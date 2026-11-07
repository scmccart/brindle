//! `brindle-remote hold`, driven by a test core over H1 (tasks 3.2–3.4).

#![cfg(target_os = "linux")]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use brindle_remote_protocol::holder::{self as h1, Previous, ToCore, ToHolder};

use support::*;

/// A holder in a fresh directory, with a connected test core.
fn holder(tag: &str) -> (TempDir, TestCore, u32) {
    let mut dir = TempDir::new(tag);
    start_holder(&dir.path);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(1);
    let pid = adopted.welcome().pid;
    dir.track(pid);
    (dir, core, pid)
}

fn holder_exits(dir: &TempDir, pid: u32) -> bool {
    eventually(TIMEOUT, || !alive(pid) && dir.holder_sockets().is_empty())
}

#[test]
fn a_new_holder_is_detached_and_welcomes_its_first_core() {
    let mut dir = TempDir::new("welcome");
    start_holder(&dir.path);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(1);
    let welcome = adopted.welcome();
    dir.track(welcome.pid);
    assert_eq!(welcome.protocol, 1);
    assert_eq!(welcome.previous, Previous::None);
    assert!(welcome.sessions.is_empty());
    assert!(!welcome.build.is_empty());
    assert_ne!(parent(welcome.pid), Some(std::process::id()));
    let socket = &dir.holder_sockets()[0];
    let name = socket.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(name, format!("holder-{}-0.sock", welcome.build));
    let mode = fs::metadata(&dir.path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);

    // REPARENT is Windows-only.
    core.send(ToHolder::Reparent(h1::Reparent {
        session: 9,
        hwnd: 1,
    }));
    match core.recv() {
        ToCore::Error(error) => {
            assert_eq!(
                (error.session, error.message.as_str()),
                (Some(9), "unsupported")
            )
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_session_gets_exactly_the_requests_environment() {
    let mut dir = TempDir::new("env");
    // Set on the holder itself: the program must not see it.
    start(
        Path::new(BIN),
        "hold",
        &dir.path,
        &[("BRINDLE_HOLDER_OWN", "leak")],
    )
    .unwrap();
    let mut core = TestCore::connect_to(&dir);
    dir.track(core.hello(1).welcome().pid);

    let mut request = spawn(1, "env", &[]);
    request.env.insert("FOO".into(), "2".into());
    request
        .env
        .insert("HOME".into(), "/nonexistent-home".into());
    request.cwd = Some("/".into());
    let expected: Vec<String> = request
        .env
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    core.spawn(request);
    let (output, code) = core.output_and_exit(1);
    assert_eq!(code, Some(0));
    let mut lines: Vec<String> = String::from_utf8_lossy(&output)
        .split("\r\n")
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect();
    lines.sort();
    assert_eq!(lines, expected);
}

#[test]
fn a_missing_command_fails_naming_the_program() {
    let (_dir, mut core, _) = holder("missing");
    core.send(ToHolder::Spawn(spawn(
        4,
        "/nonexistent/brindle-no-such-program",
        &[],
    )));
    match core.recv() {
        ToCore::SpawnFailed(failed) => {
            assert_eq!(failed.session, 4);
            assert!(
                failed
                    .message
                    .contains("/nonexistent/brindle-no-such-program")
                    && failed.message.contains("No such file"),
                "{}",
                failed.message
            );
        }
        other => panic!("{other:?}"),
    }
    // The holder still works.
    core.spawn(shell(5, "exit 0"));
    assert_eq!(core.exit_code(5), Some(0));
}

#[test]
fn exit_statuses_are_reported_and_an_empty_holder_exits() {
    let (dir, mut core, pid) = holder("exit");
    core.spawn(shell(1, "read x; exit 3"));
    core.spawn(shell(2, "kill -TERM $$"));
    assert_eq!(core.exit_code(2), None, "killed by a signal");
    assert!(alive(pid), "one session is left");
    core.input(1, b"\n");
    assert_eq!(core.exit_code(1), Some(3));
    assert!(core.closes());
    assert!(holder_exits(&dir, pid));
    assert!(
        !dir.path
            .read_dir()
            .unwrap()
            .flatten()
            .any(|entry| { entry.file_name().to_string_lossy().ends_with(".lock") })
    );
}

#[test]
fn input_resize_and_output_reach_the_program() {
    let (_dir, mut core, _) = holder("io");
    core.spawn(shell(1, "read line; stty size; echo \"got $line\"; exit 0"));
    core.send(ToHolder::Resize(h1::Resize {
        session: 1,
        cols: 100,
        rows: 30,
        cell_w: 8,
        cell_h: 16,
    }));
    core.input(1, b"hello\n");
    let output = core.output_until(1, b"got hello");
    assert!(
        contains(&output, b"30 100"),
        "{}",
        String::from_utf8_lossy(&output)
    );
    assert_eq!(core.exit_code(1), Some(0));
}

#[test]
fn hangup_kills_the_process_group() {
    let (_dir, mut core, holder_pid) = holder("hangup");
    // Started first, so the holder isn't left empty (and exits) between the two.
    let plain = core.spawn(shell(2, "sleep 1000"));
    // Ignores SIGHUP, so it takes the SIGKILL after the grace period.
    let pid = core.spawn(shell(1, "trap '' HUP; sleep 1000 & echo \"bg=$!.\"; wait"));
    let output = String::from_utf8_lossy(&core.output_until(1, b".")).into_owned();
    let background: u32 = output
        .split("bg=")
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .and_then(|pid| pid.parse().ok())
        .unwrap_or_else(|| panic!("no pid in {output:?}"));
    assert_eq!(
        parent(pid),
        Some(holder_pid),
        "the holder is the program's parent"
    );
    assert!(alive(background));

    core.hang_up(1);
    assert_eq!(core.exit_code(1), None);
    assert!(eventually(TIMEOUT, || !alive(pid) && !alive(background)));

    // A program that doesn't ignore it goes at once.
    core.hang_up(2);
    assert_eq!(core.exit_code(2), None);
    assert!(!alive(plain));
}
