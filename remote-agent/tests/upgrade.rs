//! Upgrades (task 6.1): `STEP_DOWN`, adoption by the next core, draining
//! holders of another build, the `check` command, and a next core that
//! won't start.

#![cfg(target_os = "linux")]

mod support;

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::{self, ReplayStart};

use support::*;

/// Every reconnect replay a returning client gets, by session.
fn replays(client: &mut TestClient, count: usize) -> HashMap<SessionId, (ReplayStart, Vec<u8>)> {
    (0..count)
        .map(|_| {
            let (start, bytes) = client.reconnect_replay();
            (start.session, (start, bytes))
        })
        .collect()
}

/// The client's core steps down; returns once it has exited.
fn step_down(client: &mut TestClient) {
    let old = client.core_pid();
    client.step_down();
    assert!(client.closes());
    assert!(gone(old), "the core didn't exit after stepping down");
}

#[test]
fn an_upgrade_to_the_same_binary_keeps_every_session() {
    let mut dir = TempDir::new("upgrade");
    start_core(&dir.path, &[]).unwrap();
    let mut a = TestClient::hello(&dir.path, "a");
    let old_core = a.core_pid();
    dir.track(old_core);

    // Client a: an attached full-screen program, and one to exit later.
    let fixture = full_screen_fixture(1);
    let mut spawn = fixture_spawn(&dir, "full", &fixture, 1000);
    spawn.policy = client::Policy::Kill;
    let (full, full_pid) = a.spawn(spawn);
    a.output_until(full, b"frame 1\x1b[12;30H");
    let (later, later_pid) = a.spawn(client_shell("read x; exit 7"));
    let holder = parent(full_pid).unwrap();
    dir.track(holder);

    // Client b: a detached session with scrollback, and an attached one.
    let mut b = TestClient::hello(&dir.path, "b");
    let lines: String = (1..=100).map(|n| format!("line {n}\r\n")).collect();
    let (kept, kept_pid) = b.spawn(fixture_spawn(&dir, "lines", lines.as_bytes(), 1000));
    b.output_until(kept, b"line 100\r\n");
    b.detach(kept);
    let (echo, echo_pid) = b.spawn(client_spawn("cat", &[]));
    b.input(echo, b"before\n");
    b.output_until(echo, b"before\r\nbefore\r\n");
    assert!(
        b.list().iter().any(|session| session.session == kept && !session.attached),
        "the detached session is listed as detached"
    );

    step_down(&mut a);
    assert!(b.closes());
    start_core(&dir.path, &[]).unwrap();

    // Both clients come back with their tokens and get the sessions they
    // were attached to; the one b detached stays detached, and attaching
    // it gives its replay.
    let mut a = TestClient::hello(&dir.path, "a");
    assert_ne!(a.core_pid(), old_core);
    dir.track(a.core_pid());
    let mut b = TestClient::hello(&dir.path, "b");
    let got_a = replays(&mut a, 2);
    let mut got_b = replays(&mut b, 1);
    let attached = b.attach(kept, 80, 24).unwrap();
    got_b.insert(kept, attached);
    for (replays, id, pid) in [
        (&got_a, full, full_pid),
        (&got_a, later, later_pid),
        (&got_b, kept, kept_pid),
        (&got_b, echo, echo_pid),
    ] {
        let (start, _) = &replays[&id];
        assert_eq!(start.pid, pid, "session {id}'s pid");
        assert!(alive(pid));
        assert_eq!(parent(pid), Some(holder));
    }
    assert_eq!(
        snapshot(&replayed(&got_a[&full].1, 80, 24, 1000)),
        snapshot(&replayed(&fixture, 80, 24, 1000))
    );
    assert_eq!(
        snapshot(&replayed(&got_b[&kept].1, 80, 24, 1000)),
        snapshot(&replayed(lines.as_bytes(), 80, 24, 1000))
    );

    // They keep working, and later exit statuses arrive.
    b.input(echo, b"after\n");
    b.output_until(echo, b"after\r\nafter\r\n");
    a.input(later, b"\n");
    assert_eq!(a.exit_code(later), Some(7));

    let logs = dir.logs();
    for entry in [
        "upgrade: client",
        "upgrade: stepped down",
        "upgrade: took holder",
        "previous core Handover",
    ] {
        assert!(logs.contains(entry), "no {entry:?} in\n{logs}");
    }
}

/// The build id's bytes, from its hex.
fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
        .collect()
}

/// A copy of the built binary with another build id, in `dir`: the ELF
/// build-id note's bytes with one flipped, or (for a binary without one,
/// whose id is a hash of its bytes) a few bytes appended. Returns it and
/// the built binary's own id.
fn other_build_copy(dir: &TempDir) -> (PathBuf, String) {
    let probe = TempDir::new("probe");
    start_core(&probe.path, &[]).unwrap();
    let build = TestClient::hello(&probe.path, "probe")
        .welcome
        .unwrap()
        .core_build;
    let mut bytes = fs::read(BIN).unwrap();
    let id = unhex(&build);
    match bytes.windows(id.len()).position(|window| window == id) {
        Some(at) => bytes[at] ^= 0xff,
        None => bytes.extend_from_slice(b"another build"),
    }
    let path = dir.path.join("old-brindle-remote");
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    (path, build)
}

#[test]
fn an_old_build_holder_drains_and_new_sessions_go_to_a_new_holder() {
    let mut dir = TempDir::new("drain");
    let (old_bin, new_build) = other_build_copy(&dir);
    start(
        &old_bin,
        "serve",
        &dir.path,
        &[("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version")],
    )
    .unwrap();
    let mut client = TestClient::hello(&dir.path, "t");
    dir.track(client.core_pid());
    let old_build = client.welcome.as_ref().unwrap().core_build.clone();
    assert_ne!(old_build, new_build);
    let (one, one_pid) = client.spawn(client_spawn("cat", &[]));
    let (two, two_pid) = client.spawn(client_spawn("cat", &[]));
    let old_holder = parent(one_pid).unwrap();
    dir.track(old_holder);
    let old_socket = dir.holder_endpoints().pop().unwrap();
    assert!(
        old_socket
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(&old_build)
    );

    step_down(&mut client);
    start_core(&dir.path, &[]).unwrap();
    let mut client = TestClient::hello(&dir.path, "t");
    dir.track(client.core_pid());
    assert_eq!(client.welcome.as_ref().unwrap().core_build, new_build);
    replays(&mut client, 2);

    // The third session runs under a new holder of the new build.
    let (three, three_pid) = client.spawn(client_spawn("cat", &[]));
    let new_holder = parent(three_pid).unwrap();
    dir.track(new_holder);
    assert_ne!(new_holder, old_holder);
    assert_eq!(parent(one_pid), Some(old_holder));
    assert_eq!(parent(two_pid), Some(old_holder));
    let status = client.status();
    let by_pid = |pid: u32| {
        status
            .holders
            .iter()
            .find(|holder| holder.pid == pid)
            .unwrap_or_else(|| panic!("holder {pid} not in {:?}", status.holders))
    };
    let old = by_pid(old_holder);
    assert_eq!((old.build.as_str(), old.sessions, old.draining), (&*old_build, 2, true));
    let new = by_pid(new_holder);
    assert_eq!((new.build.as_str(), new.sessions, new.draining), (&*new_build, 1, false));

    // The old holder's sessions keep working, and it exits after them.
    for session in [one, two, three] {
        client.input(session, b"ping\n");
        client.output_until(session, b"ping\r\nping\r\n");
    }
    for session in [one, two] {
        client.send(client::ToCore::Close(client::SessionRef { session }));
        assert_eq!(client.exit_code(session), None);
    }
    assert!(gone(old_holder), "the drained holder didn't exit");
    assert!(eventually(TIMEOUT, || !old_socket.exists()));
    assert!(alive(new_holder) && alive(three_pid));
    assert!(
        eventually(TIMEOUT, || dir
            .logs()
            .contains("ended its last session and exited")),
        "{}",
        dir.logs()
    );
    let logs = dir.logs();
    assert!(logs.contains("drain: holder"), "{logs}");
    assert!(logs.contains("draining: no new sessions"), "{logs}");
}

#[test]
fn a_failing_check_leaves_the_core_running_with_the_reason_in_its_log() {
    let mut dir = TempDir::new("check");
    start_core(&dir.path, &[]).unwrap();
    let mut client = TestClient::hello(&dir.path, "t");
    let core = client.core_pid();
    dir.track(core);
    let (session, pid) = client.spawn(client_spawn("cat", &[]));
    dir.track(parent(pid).unwrap());
    let in_use = TestClient::hello(&dir.path, "other")
        .welcome
        .unwrap()
        .holder_protocols;
    assert_eq!(in_use, [2]);

    let passed = cli(&dir.path, &["check", "--holder-protocols", "1,2"]);
    assert!(passed.status.success(), "{passed:?}");
    assert!(passed.stdout.is_empty() && passed.stderr.is_empty());

    let refused = cli(&dir.path, &["check", "--holder-protocols", "1,99"]);
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("upgrade refused") && stderr.contains("not H99"),
        "{stderr}"
    );
    let logs = fs::read_to_string(dir.path.join("core.log")).unwrap();
    assert!(
        logs.lines()
            .any(|line| line.contains("upgrade refused") && line.contains("H99")),
        "{logs}"
    );

    // Nothing changed for the running core.
    assert!(alive(core) && alive(pid));
    client.input(session, b"still\n");
    client.output_until(session, b"still\r\nstill\r\n");
    assert!(cli(&dir.path, &["check", "--holder-protocols"]).status.code() == Some(1));
}

#[test]
fn a_next_core_that_wont_start_is_replaced_by_the_holders_own() {
    let mut dir = TempDir::new("no-next");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut client = TestClient::hello(&dir.path, "t");
    let old_core = client.core_pid();
    let (killed, killed_pid) = client.spawn(client_spawn("cat", &[]));
    let mut detach = client_spawn("cat", &[]);
    detach.policy = client::Policy::Detach;
    let (kept, kept_pid) = client.spawn(detach);
    dir.track(parent(killed_pid).unwrap());

    step_down(&mut client);
    // The upgrade's core exits at startup.
    assert!(start(Path::new("/bin/false"), "serve", &dir.path, &[]).is_err());
    let stepped_down = Instant::now();
    assert!(
        eventually(Duration::from_secs(20), || {
            server_processes(&dir.path, "serve")
                .iter()
                .any(|&pid| pid != old_core)
                && std::os::unix::net::UnixStream::connect(core_endpoint(&dir.path)).is_ok()
        }),
        "no core: {}",
        dir.logs()
    );
    assert!(
        stepped_down.elapsed() < Duration::from_secs(10),
        "a core came back after {:?}",
        stepped_down.elapsed()
    );

    let mut observer = TestClient::hello(&dir.path, "observer");
    dir.track(observer.core_pid());
    let mut listed: Vec<(SessionId, u32)> = observer
        .list()
        .iter()
        .map(|session| (session.session, session.pid))
        .collect();
    listed.sort_unstable();
    assert_eq!(listed, [(killed, killed_pid), (kept, kept_pid)]);
    let mut client = TestClient::hello(&dir.path, "t");
    replays(&mut client, 2);
    client.input(killed, b"back\n");
    client.output_until(killed, b"back\r\nback\r\n");
}
