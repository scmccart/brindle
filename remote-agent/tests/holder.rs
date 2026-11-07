//! `brindle-remote hold`, driven by a test core over H1 (tasks 3.2–3.4).

#![cfg(target_os = "linux")]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use brindle_remote_protocol::Frame;
use brindle_remote_protocol::holder::{self as h1, Drain, Goodbye, Previous, ToCore, ToHolder};

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

/// Reads at most `chunk` bytes per call, pausing before each: a core
/// slower than the program's output.
struct Throttled<R> {
    inner: R,
    chunk: usize,
    pause: Duration,
}

impl<R: std::io::Read> std::io::Read for Throttled<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        std::thread::sleep(self.pause);
        let len = buffer.len().min(self.chunk);
        self.inner.read(&mut buffer[..len])
    }
}

/// A flood followed by the program's exit, read by a core much slower than
/// the holder's queue fills: the holder must deliver every byte and the
/// `EXITED` before it exits, however long that takes (within its bound),
/// and leave without resetting the connection while the core still writes.
#[test]
fn an_exiting_holder_delivers_a_flood_and_its_exit_to_a_slow_core() {
    const FLOOD: usize = 6 * 1024 * 1024;
    let mut dir = TempDir::new("flood");
    start_holder(&dir.path);
    let socket = dir.holder_sockets().remove(0);
    let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    let send = |stream: &mut std::os::unix::net::UnixStream, message: ToHolder| {
        message.encode().unwrap().write_to(stream).unwrap()
    };
    send(
        &mut stream,
        ToHolder::Hello(h1::Hello {
            min: h1::PROTOCOLS.min,
            max: h1::PROTOCOLS.max,
            core_build: "test-core".into(),
            instance: 1,
        }),
    );
    // No newlines, so the PTY passes the bytes through unchanged.
    send(
        &mut stream,
        ToHolder::Spawn(shell(
            1,
            &format!("head -c {FLOOD} /dev/zero | tr '\\0' x; exit 0"),
        )),
    );
    // About 1.3 MB/s: the flood takes about 5 s to read, longer than the
    // holder used to wait.
    let mut reader = std::io::BufReader::with_capacity(
        32 * 1024,
        Throttled {
            inner: stream.try_clone().unwrap(),
            chunk: 32 * 1024,
            pause: Duration::from_millis(25),
        },
    );
    let mut output = 0usize;
    let mut code = None;
    let mut last_poke = std::time::Instant::now();
    loop {
        let frame = match Frame::read_from_or_eof(&mut reader) {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) => panic!("after {output} bytes of output: {error}"),
        };
        match ToCore::decode(&frame).unwrap() {
            ToCore::Welcome(welcome) => dir.track(welcome.pid),
            ToCore::Spawned(_) => {}
            ToCore::Output(data) => {
                assert_eq!(code, None, "output after EXITED");
                assert!(data.bytes.iter().all(|&byte| byte == b'x'));
                output += data.bytes.len();
            }
            ToCore::Exited(exited) => {
                assert_eq!(exited.session, 1);
                code = Some(exited.code);
            }
            other => panic!("{other:?}"),
        }
        // A core keeps talking to its holder (pauses, resumes,
        // checkpoints); an unknown session isn't an error.
        if last_poke.elapsed() >= Duration::from_millis(100) {
            last_poke = std::time::Instant::now();
            let _ = ToHolder::Resume(h1::SessionRef { session: 99 })
                .encode()
                .unwrap()
                .write_to(&mut stream);
        }
    }
    assert_eq!(output, FLOOD, "every byte of output");
    assert_eq!(code, Some(Some(0)), "the exit, with its status");
    // The holder closed its side; once the core closes too, it is gone.
    drop(reader);
    drop(stream);
    assert!(
        eventually(TIMEOUT, || dir.holder_sockets().is_empty()
            && server_processes(&dir.path, "hold").is_empty()),
        "the holder didn't exit"
    );
    let logs = dir.logs();
    assert!(!logs.contains("didn't take"), "{logs}");
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
fn ptys_are_utf8() {
    let (_dir, mut core, _) = holder("iutf8");
    core.spawn(shell(1, "stty -a; echo done.; exit 0"));
    let output = String::from_utf8_lossy(&core.output_until(1, b"done.")).into_owned();
    assert!(
        output.split_whitespace().any(|flag| flag == "iutf8"),
        "{output}"
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

#[test]
fn hangup_kills_a_child_that_outlives_its_exited_leader() {
    let (_dir, mut core, _) = holder("hangup-orphan");
    // The leader goes on SIGHUP; its child ignores it and stays in the group.
    let pid = core.spawn(shell(
        1,
        "(trap '' HUP; exec sleep 1000) & echo \"bg=$!.\"; wait",
    ));
    let background = reported_pid(&core.output_until(1, b"."), "bg");
    assert!(alive(background));

    core.hang_up(1);
    assert_eq!(core.exit_code(1), None);
    assert!(!alive(pid), "the leader is reaped");
    assert!(alive(background), "the child outlived its leader");
    assert!(
        eventually(TIMEOUT, || !alive(background)),
        "the child survived the hang-up"
    );
}

/// `SPAWN`'s meta, decoded from JSON as a core would send it.
fn meta() -> h1::Spawn {
    let frame = Frame::new(
        h1::MSG_SPAWN,
        br#"{"session":0,"cols":1,"rows":1,"meta":{"owner":"t1","policy":"kill"}}"#.to_vec(),
    )
    .unwrap();
    match ToHolder::decode(&frame).unwrap() {
        ToHolder::Spawn(spawn) => spawn,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_new_core_gets_the_sessions_checkpoints_and_tails() {
    let (dir, mut first, holder_pid) = holder("adopt");
    let mut request = shell(1, "echo one; read x; echo \"two:$x\"; read y");
    request.meta = meta().meta;
    let pid = first.spawn(request.clone());
    first.output_until(1, b"one\r\n");

    // A newer HELLO takes over while the first core is still connected.
    let mut second = TestCore::connect_to(&dir);
    let adopted = second.hello(2);
    assert!(first.closes(), "the older core is dropped");
    let welcome = adopted.welcome();
    assert_eq!(
        (welcome.previous, welcome.pid),
        (Previous::Crash, holder_pid)
    );
    assert_eq!(welcome.sessions.len(), 1);
    let held = &welcome.sessions[0];
    assert_eq!(
        (held.session, held.pid, held.cols, held.rows),
        (1, pid, 80, 24)
    );
    assert_eq!((held.exit, held.truncated), (None, false));
    assert_eq!(held.meta, request.meta);
    assert_eq!(adopted.checkpoints[&1], b"");
    assert_eq!(adopted.tails[&1], b"one\r\n");

    // A checkpoint replaces the tail it covers ("one\r\n").
    for message in ToHolder::checkpoint(1, b"CP1", 5) {
        second.send(message);
    }
    second.input(1, b"abc\n");
    second.output_until(1, b"two:abc\r\n");
    second.send(ToHolder::Goodbye(Goodbye { handover: true }));
    drop(second);

    let mut third = TestCore::connect_to(&dir);
    let adopted = third.hello(3);
    assert_eq!(adopted.welcome().previous, Previous::Handover);
    assert_eq!(adopted.checkpoints[&1], b"CP1");
    let tail = &adopted.tails[&1];
    assert!(
        contains(tail, b"two:abc\r\n") && !contains(tail, b"one"),
        "{tail:?}"
    );

    // An exit while no core is connected is reported in the next WELCOME.
    drop(third);
    let mut fourth = TestCore::connect_to(&dir);
    fourth.hello(4);
    fourth.input(1, b"\n");
    drop(fourth);
    assert!(eventually(TIMEOUT, || !alive(pid)));
    let mut fifth = TestCore::connect_to(&dir);
    let adopted = fifth.hello(5);
    match adopted.welcome().sessions[0].exit {
        Some(exit) => assert_eq!(exit.code, Some(0)),
        // Reaped, but its output was still being drained.
        None => assert_eq!(fifth.exit_code(1), Some(0)),
    }
    assert!(fifth.closes(), "the holder had nothing left");
    assert!(holder_exits(&dir, holder_pid));
}

#[test]
fn a_checkpoint_keeps_the_output_it_doesnt_cover() {
    let (dir, mut core, _) = holder("covers");
    core.spawn(spawn(1, "cat", &[]));
    core.input(1, b"a\n");
    core.output_until(1, b"a\r\na\r\n");
    // As if the core had rendered after the echo but before cat's line.
    for message in ToHolder::checkpoint(1, b"CP", 3) {
        core.send(message);
    }
    core.input(1, b"b\n");
    core.output_until(1, b"b\r\nb\r\n");
    drop(core);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    assert_eq!(adopted.checkpoints[&1], b"CP");
    assert_eq!(adopted.tails[&1], b"a\r\nb\r\nb\r\n");
    assert!(!adopted.welcome().sessions[0].truncated);
    core.hang_up(1);
    assert_eq!(core.exit_code(1), None);
}

#[test]
fn a_tail_past_its_limit_is_truncated() {
    let (dir, mut core, _) = holder("truncate");
    core.spawn(shell(
        1,
        "head -c 1500000 /dev/zero | tr '\\0' x; echo; echo END; read z",
    ));
    core.output_until(1, b"END\r\n");
    drop(core);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    assert!(adopted.welcome().sessions[0].truncated);
    let tail = &adopted.tails[&1];
    assert_eq!(tail.len() as u64, h1::TAIL_LIMIT);
    assert!(tail.ends_with(b"x\r\nEND\r\n") && tail.starts_with(b"xxx"));

    // A checkpoint clears the flag.
    for message in ToHolder::checkpoint(1, b"CP", h1::TAIL_LIMIT) {
        core.send(message);
    }
    drop(core);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(3);
    assert!(!adopted.welcome().sessions[0].truncated);
    assert_eq!(adopted.tails[&1], b"");
    core.hang_up(1);
    assert_eq!(core.exit_code(1), None);
}

#[test]
fn without_a_core_a_program_blocks_instead_of_losing_output() {
    let (dir, mut core, holder_pid) = holder("block");
    let lines = 300_000;
    let pid = core.spawn(shell(
        1,
        &format!("read go; seq 1 {lines}; echo DONE; read z"),
    ));
    // The holder handles the input before it sees the connection end.
    core.input(1, b"go\n");
    drop(core);

    // seq stays blocked on the full PTY once the holder stops reading.
    let mut seq = Vec::new();
    assert!(eventually(TIMEOUT, || {
        seq = children_named(pid, "seq");
        !seq.is_empty()
    }));
    std::thread::sleep(Duration::from_millis(500));
    assert!(alive(seq[0]), "seq finished, so output went somewhere");
    assert_eq!(state(seq[0]), Some('S'));

    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    assert_eq!(adopted.welcome().previous, Previous::Crash);
    assert!(!adopted.welcome().sessions[0].truncated);
    let mut stream = adopted.tails[&1].clone();
    assert_eq!(stream.len() as u64, h1::TAIL_LIMIT, "the tail filled up");
    stream.extend(core.output_until(1, b"DONE\r\n"));

    let mut expected = b"go\r\n".to_vec();
    for line in 1..=lines {
        expected.extend(format!("{line}\r\n").as_bytes());
    }
    expected.extend(b"DONE\r\n");
    assert!(stream.len() >= expected.len());
    assert!(
        stream[..expected.len()] == expected[..],
        "the output has a gap or is out of order"
    );
    core.hang_up(1);
    assert_eq!(core.exit_code(1), None);
    assert!(holder_exits(&dir, holder_pid));
}

#[test]
fn a_paused_session_blocks_until_resumed() {
    let (_dir, mut core, _) = holder("pause");
    let pid = core.spawn(shell(1, "read go; seq 1 300000; echo DONE; read z"));
    core.send(ToHolder::Pause(h1::SessionRef { session: 1 }));
    core.input(1, b"go\n");
    let mut seq = Vec::new();
    assert!(eventually(TIMEOUT, || {
        seq = children_named(pid, "seq");
        !seq.is_empty()
    }));
    std::thread::sleep(Duration::from_millis(500));
    assert!(alive(seq[0]), "seq finished, so the PTY was read");
    assert_eq!(state(seq[0]), Some('S'));

    core.send(ToHolder::Resume(h1::SessionRef { session: 1 }));
    let output = core.output_until(1, b"DONE\r\n");
    assert!(contains(&output, b"\r\n300000\r\n"));
    core.hang_up(1);
    assert_eq!(core.exit_code(1), None);
}

#[test]
fn a_drained_holder_refuses_spawn_and_exits_after_its_last_session() {
    let (dir, mut core, holder_pid) = holder("drain");
    core.spawn(shell(1, "read x"));
    core.send(ToHolder::Drain(Drain {}));
    core.send(ToHolder::Spawn(shell(2, "exit 0")));
    match core.recv() {
        ToCore::SpawnFailed(failed) => {
            assert_eq!(failed.session, 2);
            assert!(failed.message.contains("draining"), "{}", failed.message);
        }
        other => panic!("{other:?}"),
    }
    assert!(alive(holder_pid));
    core.input(1, b"\n");
    assert_eq!(core.exit_code(1), Some(0));
    assert!(holder_exits(&dir, holder_pid));
}

#[test]
fn a_holder_without_a_core_starts_one_from_its_own_binary_even_after_it_was_replaced() {
    let mut dir = TempDir::new("supervise");
    let bin_dir = TempDir::new("supervise-bin");
    let copy = bin_dir.path.join("brindle-remote");
    fs::copy(BIN, &copy).unwrap();
    // Instead of `serve`, which comes later, the started "core" prints the
    // version of whichever binary ran, into the holder's log.
    start(
        &copy,
        "hold",
        &dir.path,
        &[("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version")],
    )
    .unwrap();
    let mut core = TestCore::connect_to(&dir);
    let holder_pid = core.hello(1).welcome().pid;
    dir.track(holder_pid);
    core.spawn(shell(1, "read x"));

    // An install replaces the file.
    let replacement = bin_dir.path.join("new");
    fs::write(&replacement, "#!/bin/sh\necho replaced binary ran\n").unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o755)).unwrap();
    fs::rename(&replacement, &copy).unwrap();

    // The core goes away, as if SIGKILLed.
    drop(core);
    let version = env!("CARGO_PKG_VERSION");
    let starts = |logs: &str| logs.lines().filter(|line| *line == version).count();
    assert!(
        eventually(Duration::from_secs(60), || starts(&dir.logs()) >= 2),
        "no repeated core starts: {}",
        dir.logs()
    );
    let logs = dir.logs();
    assert!(!logs.contains("replaced binary ran"), "{logs}");
    assert!(logs.contains("started /proc/self/exe --version"), "{logs}");

    // A core that connects stops the retries and keeps the session.
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    assert_eq!(adopted.welcome().sessions.len(), 1);
    core.input(1, b"\n");
    assert_eq!(core.exit_code(1), Some(0));
    assert!(holder_exits(&dir, holder_pid));
}
