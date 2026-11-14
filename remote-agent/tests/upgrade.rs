//! Upgrades (task 6.1): `STEP_DOWN`, adoption by the next core, draining
//! holders of another build, the `check` command, and a next core that
//! won't start.
//!
//! Every test runs on both platforms. On Windows the console host renders
//! what a program prints, so a replay is compared with what a client saw
//! rather than with the program's bytes, and since a running image can't
//! be replaced, the older build is another copy of the binary.

#![cfg(any(target_os = "linux", windows))]

mod support;

use std::collections::HashMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::{self, ReplayStart};

use support::programs::{self, ENTER, say};
use support::*;

/// How long a Windows session that has printed everything stays quiet
/// before its output counts as all there.
#[cfg(windows)]
const QUIET: Duration = Duration::from_secs(1);

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

/// Waits until a [`fixture_spawn`] session has printed `fixture` and
/// returns what its replay must show: on Linux the fixture itself, which
/// the PTY passes through and which ends with `end`; on Windows what the
/// client saw once `text` was shown and the session went quiet.
fn printed(
    client: &mut TestClient,
    session: SessionId,
    fixture: &[u8],
    end: &[u8],
    text: &[u8],
) -> Vec<u8> {
    #[cfg(unix)]
    {
        let _ = text;
        client.output_until(session, end);
        fixture.to_vec()
    }
    #[cfg(windows)]
    {
        let _ = (fixture, end);
        client.output_until_quiet(session, text, QUIET)
    }
}

/// The running process `pid`'s binary is the built one, and the binary's
/// file time is what Brindle's upgrade check reads for it: through
/// `/proc/<pid>/exe` on Linux, `QueryFullProcessImageNameW` on Windows.
fn runs_the_built_binary(pid: u32) {
    #[cfg(unix)]
    let image = fs::read_link(format!("/proc/{pid}/exe")).unwrap();
    #[cfg(windows)]
    let image = brindle_remote_protocol::win::image_path_of(pid).unwrap();
    assert_eq!(
        fs::canonicalize(&image).unwrap(),
        fs::canonicalize(BIN).unwrap()
    );
    let modified = |path: &Path| fs::metadata(path).unwrap().modified().unwrap();
    assert_eq!(modified(&image), modified(Path::new(BIN)));
}

#[test]
fn an_upgrade_to_the_same_binary_keeps_every_session() {
    let mut dir = TempDir::new("upgrade");
    start_core(&dir.path, &[]).unwrap();
    let mut a = TestClient::hello(&dir.path, "a");
    let old_core = a.core_pid();
    dir.track(old_core);
    runs_the_built_binary(old_core);

    // Client a: an attached full-screen program, and one to exit later.
    let fixture = full_screen_fixture(1);
    let mut spawn = fixture_spawn(&dir, "full", &fixture, 1000);
    spawn.policy = client::Policy::Kill;
    let (full, full_pid) = a.spawn(spawn);
    let full_seen = printed(&mut a, full, &fixture, b"frame 1\x1b[12;30H", b"Quit frame 1");
    let (later, later_pid) = a.spawn(programs::exits_after_a_line(7));
    let holder = parent(full_pid).unwrap();
    dir.track(holder);

    // Client b: a detached session with scrollback, and an attached one.
    let mut b = TestClient::hello(&dir.path, "b");
    let lines: String = (1..=100).map(|n| format!("line {n}\r\n")).collect();
    let (kept, kept_pid) = b.spawn(fixture_spawn(&dir, "lines", lines.as_bytes(), 1000));
    let kept_seen = printed(&mut b, kept, lines.as_bytes(), b"line 100\r\n", b"line 100");
    b.detach(kept);
    let (echo, echo_pid) = b.spawn(programs::echoer());
    say(&mut b, echo, "before");
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
    let got_b = replays(&mut b, 1);
    let kept_expected = snapshot(&replayed(&kept_seen, 80, 24, 1000));
    let (kept_start, kept_screen) =
        b.attach_until(kept, 1000, |screen| snapshot(screen) == kept_expected);
    for (start, id, pid) in [
        (&got_a[&full].0, full, full_pid),
        (&got_a[&later].0, later, later_pid),
        (&kept_start, kept, kept_pid),
        (&got_b[&echo].0, echo, echo_pid),
    ] {
        assert_eq!(start.pid, pid, "session {id}'s pid");
        assert!(alive(pid));
        assert_eq!(parent(pid), Some(holder));
    }
    assert_eq!(
        snapshot(&replayed(&got_a[&full].1, 80, 24, 1000)),
        snapshot(&replayed(&full_seen, 80, 24, 1000))
    );
    assert_eq!(snapshot(&kept_screen), kept_expected);

    // They keep working, and later exit statuses arrive.
    say(&mut b, echo, "after");
    a.input(later, ENTER);
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
#[cfg(unix)]
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
#[cfg(unix)]
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

/// A copy of the built binary with another build id: its bytes with a few
/// appended (a PE file's id is a hash of its bytes, and it runs with bytes
/// after its sections). It is where Windows runs a build from, the build's
/// per-build copy under `dir`'s `bin`, so the next core's pruning finds it
/// in use. Returns it and the built binary's own id.
#[cfg(windows)]
fn other_build_copy(dir: &TempDir) -> (PathBuf, String) {
    use brindle_remote_protocol::{build_id, server_dir};

    let mut bytes = fs::read(BIN).unwrap();
    let build = build_id::of_binary(&bytes);
    bytes.extend_from_slice(b"another build");
    let path = server_dir::helper_copy(&dir.path.join("bin"), &build_id::of_binary(&bytes));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
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
    let (one, one_pid) = client.spawn(programs::echoer());
    let (two, two_pid) = client.spawn(programs::echoer());
    let old_holder = parent(one_pid).unwrap();
    dir.track(old_holder);
    let old_endpoint = dir.holder_endpoints().pop().unwrap();
    assert!(
        old_endpoint.to_string_lossy().contains(&old_build),
        "{old_endpoint:?}"
    );

    step_down(&mut client);
    start_core(&dir.path, &[]).unwrap();
    let mut client = TestClient::hello(&dir.path, "t");
    dir.track(client.core_pid());
    assert_eq!(client.welcome.as_ref().unwrap().core_build, new_build);
    replays(&mut client, 2);

    // The third session runs under a new holder of the new build.
    let (three, three_pid) = client.spawn(programs::echoer());
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
        say(&mut client, session, "ping");
    }
    for session in [one, two] {
        client.send(client::ToCore::Close(client::SessionRef { session }));
        programs::hung_up(&mut client, session);
    }
    assert!(gone(old_holder), "the drained holder didn't exit");
    assert!(eventually(TIMEOUT, || !dir
        .holder_endpoints()
        .contains(&old_endpoint)));
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
    let (session, pid) = client.spawn(programs::echoer());
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
    say(&mut client, session, "still");
    assert!(cli(&dir.path, &["check", "--holder-protocols"]).status.code() == Some(1));
}

/// A program that exits at once whatever its arguments, standing in for a
/// new core that won't start.
fn exits_at_startup() -> PathBuf {
    #[cfg(unix)]
    return PathBuf::from("/bin/false");
    #[cfg(windows)]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        Path::new(&root).join("System32").join("whoami.exe")
    }
}

#[test]
fn a_next_core_that_wont_start_is_replaced_by_the_holders_own() {
    let mut dir = TempDir::new("no-next");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut client = TestClient::hello(&dir.path, "t");
    let old_core = client.core_pid();
    let (killed, killed_pid) = client.spawn(programs::echoer());
    let mut detach = programs::echoer();
    detach.policy = client::Policy::Detach;
    let (kept, kept_pid) = client.spawn(detach);
    dir.track(parent(killed_pid).unwrap());

    step_down(&mut client);
    // The upgrade's core exits at startup.
    assert!(start(&exits_at_startup(), "serve", &dir.path, &[]).is_err());
    let stepped_down = Instant::now();
    assert!(
        eventually(Duration::from_secs(20), || {
            server_processes(&dir.path, "serve")
                .iter()
                .any(|&pid| pid != old_core)
                && core_listens(&dir.path)
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
    say(&mut client, killed, "back");
}
