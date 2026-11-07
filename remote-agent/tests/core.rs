//! `brindle-remote serve`, driven by test clients over protocol 1, with real
//! holders under it (task group 4).

#![cfg(target_os = "linux")]

mod support;

use std::thread;
use std::time::{Duration, Instant};

use brindle_remote_protocol::client::{self, ToClient};

use support::*;

/// A core in a fresh directory, with a connected client.
fn core(tag: &str) -> (TempDir, TestClient) {
    let mut dir = TempDir::new(tag);
    start_core(&dir.path, &[]).unwrap();
    let client = TestClient::hello(&dir.path, &format!("token-{tag}"));
    dir.track(client.core_pid());
    (dir, client)
}

fn gone(pid: u32) -> bool {
    eventually(TIMEOUT, || !alive(pid))
}

// 4.1

#[test]
fn racing_starts_leave_one_detached_core() {
    let mut dir = TempDir::new("race");
    let starts: Vec<_> = (0..2)
        .map(|_| {
            let path = dir.path.clone();
            thread::spawn(move || start_core(&path, &[]))
        })
        .collect();
    for start in starts {
        // Both starters are told a core is listening.
        start.join().unwrap().unwrap();
    }
    let client = TestClient::hello(&dir.path, "t");
    let pid = client.core_pid();
    dir.track(pid);
    assert!(
        eventually(TIMEOUT, || server_processes(&dir.path, "serve") == [pid]),
        "cores: {:?}",
        server_processes(&dir.path, "serve")
    );
    assert_ne!(parent(pid), Some(std::process::id()));

    let welcome = client.welcome.as_ref().unwrap();
    assert_eq!(welcome.protocol, 1);
    assert_eq!(welcome.core_version, env!("CARGO_PKG_VERSION"));
    assert!(!welcome.core_build.is_empty());
    assert!(welcome.holder_protocols.is_empty(), "no holder yet");
}

#[test]
fn sessions_share_one_holder_that_is_neither_the_core_nor_the_client() {
    let (mut dir, mut client) = core("share");
    let core_pid = client.core_pid();
    let pids: Vec<u32> = (0..3)
        .map(|_| client.spawn(client_shell("read x")).1)
        .collect();
    let holder = parent(pids[0]).unwrap();
    dir.track(holder);
    assert!(pids.iter().all(|&pid| parent(pid) == Some(holder)));
    assert_eq!(server_processes(&dir.path, "hold"), [holder]);
    assert_eq!(dir.holder_sockets().len(), 1);
    for pid in [core_pid, holder] {
        let parent = parent(pid);
        assert_ne!(parent, Some(std::process::id()));
        assert_ne!(parent, Some(core_pid));
    }

    // The next client hears of the holder's protocol.
    let other = TestClient::hello(&dir.path, "other");
    assert_eq!(other.welcome.as_ref().unwrap().holder_protocols, [1]);
}

#[test]
fn an_unused_core_and_its_holder_exit_and_remove_their_sockets() {
    // Never used at all.
    let idle = TempDir::new("unused-idle");
    start_core(&idle.path, &[]).unwrap();
    let pid = server_processes(&idle.path, "serve")[0];
    let started = Instant::now();
    assert!(gone(pid));
    assert!(started.elapsed() >= Duration::from_secs(4));
    assert!(!core_socket(&idle.path).exists());

    // Used, then left.
    let (dir, mut client) = core("unused");
    let core_pid = client.core_pid();
    let (session, _) = client.spawn(client_shell("read x; exit 0"));
    let holder = server_processes(&dir.path, "hold")[0];
    client.input(session, b"\n");
    assert_eq!(client.exit_code(session), Some(0));
    drop(client);
    assert!(gone(holder) && gone(core_pid));
    assert!(!core_socket(&dir.path).exists());
    assert!(dir.holder_sockets().is_empty());
}

#[test]
fn a_new_core_adopts_the_holder_and_continues_the_session_ids() {
    let (mut dir, mut client) = core("adopt");
    let first = client.spawn(client_shell("read x"));
    let second = client.spawn(client_shell("read x"));
    assert_eq!((first.0, second.0), (1, 2));
    let holder = parent(first.1).unwrap();
    dir.track(holder);

    unsafe { libc::kill(client.core_pid() as libc::pid_t, libc::SIGKILL) };
    assert!(client.closes());
    assert!(gone(client.core_pid()));
    start_core(&dir.path, &[]).unwrap();
    let mut client = TestClient::hello(&dir.path, "someone-else");
    dir.track(client.core_pid());
    let (third, pid) = client.spawn(client_shell("read x"));
    assert_eq!(third, 3);
    assert_eq!(parent(pid), Some(holder), "the adopted holder is reused");
    assert!(alive(first.1) && alive(second.1));
    let logs = dir.logs();
    assert!(
        logs.contains("previous core Crash) with 2 session(s)"),
        "{logs}"
    );
}

// 4.2

#[test]
fn a_session_gets_exactly_the_requests_environment() {
    let mut dir = TempDir::new("env");
    // Set on the core (and so its holder): the program must not see it.
    start_core(&dir.path, &[("FOO", "1")]).unwrap();
    let mut client = TestClient::hello(&dir.path, "t");
    dir.track(client.core_pid());

    let mut request = client_spawn("env", &[]);
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
    let (session, _) = client.spawn(request);
    let (output, code) = client.output_and_exit(session);
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
fn a_missing_command_is_an_error_naming_it() {
    let (_dir, mut client) = core("missing");
    match client.spawn_reply(client_spawn("/nonexistent/brindle-no-such-program", &[])) {
        ToClient::Error(error) => {
            assert!(error.session.is_none());
            assert!(
                error
                    .message
                    .contains("/nonexistent/brindle-no-such-program")
                    && error.message.contains("No such file"),
                "{}",
                error.message
            );
        }
        other => panic!("{other:?}"),
    }
    // The core and its holder still work.
    let (session, _) = client.spawn(client_shell("exit 0"));
    assert_eq!(client.exit_code(session), Some(0));
}

#[test]
fn exit_statuses_reach_the_client() {
    let (_dir, mut client) = core("exit");
    let (three, _) = client.spawn(client_shell("read x; exit 3"));
    let (signalled, _) = client.spawn(client_shell("kill -TERM $$"));
    assert_eq!(client.exit_code(signalled), None, "killed by a signal");
    client.input(three, b"\n");
    assert_eq!(client.exit_code(three), Some(3));
    // It is gone: input for it is refused.
    client.input(three, b"x");
    match client.recv() {
        ToClient::Error(error) => assert_eq!(
            (error.session, error.message.as_str()),
            (Some(three), &*format!("no session {three}"))
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn spawning_as_the_last_session_ends_still_works() {
    // An empty holder exits, maybe with the next SPAWN unread; the core
    // sends it to a new holder.
    let (_dir, mut client) = core("respawn");
    for round in 0..10 {
        let (session, _) = client.spawn(client_shell(&format!("exit {round}")));
        assert_eq!(client.exit_code(session), Some(round));
    }
}

#[test]
fn input_resize_and_close() {
    let (dir, mut client) = core("io");
    let (session, _) = client.spawn(client_shell(
        "read line; stty size; echo \"got $line\"; read z",
    ));
    client.send(client::ToCore::Resize(client::Resize {
        session,
        cols: 100,
        rows: 30,
        cell_w: 8,
        cell_h: 16,
    }));
    client.input(session, b"hello\n");
    let output = client.output_until(session, b"got hello");
    assert!(
        contains(&output, b"30 100"),
        "{}",
        String::from_utf8_lossy(&output)
    );

    let (closed, pid) = client.spawn(client_shell("sleep 1000"));
    client.send(client::ToCore::Close(client::SessionRef {
        session: closed,
    }));
    assert_eq!(client.exit_code(closed), None);
    assert!(!alive(pid));

    // Another client can't use these sessions.
    let mut other = TestClient::hello(&dir.path, "other");
    other.input(session, b"x");
    match other.recv() {
        ToClient::Error(error) => {
            assert_eq!(error.session, Some(session));
            assert!(
                error.message.contains("isn't attached"),
                "{}",
                error.message
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_stalled_client_doesnt_delay_another_clients_session() {
    let (dir, mut stalled) = core("stall");
    let (flood, flood_pid) = stalled.spawn(client_shell("exec yes flood"));
    // From here on it stops reading.
    let mut reader = TestClient::hello(&dir.path, "reader");
    let (echo, _) = reader.spawn(client_shell(
        "while read line; do echo \"pong $line\"; done",
    ));

    // The flood's program ends up blocked, not the core.
    let mut samples = 0;
    assert!(
        eventually(TIMEOUT, || {
            samples = if state(flood_pid) == Some('S') {
                samples + 1
            } else {
                0
            };
            samples >= 10
        }),
        "the flooding program never blocked"
    );
    let core_rss = rss_kib(stalled.core_pid());
    assert!(core_rss < 128 * 1024, "the core holds {core_rss} KiB");

    for round in 0..5 {
        let sent = Instant::now();
        reader.input(echo, format!("{round}\n").as_bytes());
        reader.output_until(echo, format!("pong {round}\r\n").as_bytes());
        assert!(
            sent.elapsed() < Duration::from_secs(5),
            "round {round} took {:?}",
            sent.elapsed()
        );
    }

    // Once the stalled client reads again, its session flows again: far
    // more arrives than was queued when it was paused.
    let mut received = 0usize;
    let deadline = Instant::now() + TIMEOUT;
    while received < 32 * 1024 * 1024 {
        assert!(Instant::now() < deadline, "only {received} bytes resumed");
        match stalled.recv() {
            ToClient::Data(data) if data.session == flood => received += data.bytes.len(),
            _ => {}
        }
    }
}

// 4.3

/// Holders start a real core of their own once theirs is gone.
const SUPERVISED: (&str, &str) = (
    "BRINDLE_TEST_HOLDER_CORE_ARGS",
    "serve --ready-fd {ready_fd}",
);
/// A little more than the core's reconnect grace period.
const PAST_GRACE: Duration = Duration::from_secs(12);

fn detach(mut spawn: client::Spawn) -> client::Spawn {
    spawn.policy = client::Policy::Detach;
    spawn
}

fn kill(pid: u32) {
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}

/// SIGKILLs the client's core and waits until another core (the holders'
/// own) accepts connections.
fn kill_core_and_wait_for_another(dir: &TempDir, client: &mut TestClient) {
    let old = client.core_pid();
    kill(old);
    assert!(client.closes());
    assert!(
        eventually(Duration::from_secs(60), || {
            server_processes(&dir.path, "serve")
                .iter()
                .any(|&pid| pid != old)
                && std::os::unix::net::UnixStream::connect(core_socket(&dir.path)).is_ok()
        }),
        "no new core: {}",
        dir.logs()
    );
}

#[test]
fn a_sigkilled_clients_kill_session_ends_with_its_children_and_detach_survives() {
    let (mut dir, mut client) = core("sigkill");
    let core_pid = client.core_pid();
    let (with_child, pid) = client.spawn(client_shell("sleep 1000 & echo \"bg=$!.\"; wait"));
    let background = reported_pid(&client.output_until(with_child, b"."), "bg");
    let (kept, kept_pid) = client.spawn(detach(client_spawn("cat", &[])));
    dir.track(parent(pid).unwrap());

    let mut process = client.into_process();
    process.kill().unwrap();
    process.wait().unwrap();
    assert!(
        eventually(Duration::from_secs(5), || !alive(pid) && !alive(background)),
        "the kill session outlived its client"
    );
    thread::sleep(Duration::from_millis(500));
    assert!(alive(kept_pid), "the detach session ended with its client");
    assert!(alive(core_pid), "the core keeps the detach session");

    // Listed as detached, through the holder (LIST is task 6.2): with the
    // core gone, a test core takes the holder over and reads its list.
    kill(core_pid);
    assert!(gone(core_pid));
    let mut holder = TestCore::connect_to(&dir);
    let adopted = holder.hello(99);
    let sessions = &adopted.welcome().sessions;
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    let held = &sessions[0];
    assert_eq!((held.session, held.pid, held.exit), (kept, kept_pid, None));
    let meta = held.meta.as_object().unwrap();
    let keys: Vec<&str> = meta.keys().map(String::as_str).collect();
    assert_eq!(keys, ["command", "cwd", "owner", "policy", "scrollback"]);
    assert_eq!(meta["policy"], "detach");
    assert_eq!(meta["owner"], "token-sigkill");
    assert_eq!(meta["command"], "cat");
    assert_eq!(meta["scrollback"], 1000);
}

#[test]
fn a_detach_session_goes_back_to_its_owners_token() {
    let (dir, mut client) = core("detach");
    let (session, pid) = client.spawn(detach(client_spawn("cat", &[])));
    drop(client);
    thread::sleep(Duration::from_millis(500));
    assert!(alive(pid));

    let mut stranger = TestClient::hello(&dir.path, "stranger");
    stranger.input(session, b"x\n");
    match stranger.recv() {
        ToClient::Error(error) => assert_eq!(error.session, Some(session)),
        other => panic!("{other:?}"),
    }
    let mut owner = TestClient::hello(&dir.path, "token-detach");
    owner.input(session, b"back\n");
    owner.output_until(session, b"back\r\nback\r\n");
}

#[test]
fn a_client_back_within_the_grace_period_keeps_its_kill_sessions() {
    let mut dir = TempDir::new("grace-back");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut client = TestClient::hello(&dir.path, "back");
    let old_core = client.core_pid();
    let (session, pid) = client.spawn(client_spawn("cat", &[]));
    dir.track(parent(pid).unwrap());

    kill_core_and_wait_for_another(&dir, &mut client);
    let mut client = TestClient::hello(&dir.path, "back");
    assert_ne!(client.core_pid(), old_core);
    client.input(session, b"still here\n");
    client.output_until(session, b"still here\r\nstill here\r\n");

    thread::sleep(PAST_GRACE);
    assert!(alive(pid));
    client.input(session, b"later\n");
    client.output_until(session, b"later\r\nlater\r\n");
}

#[test]
fn kill_sessions_whose_owner_doesnt_come_back_end_after_the_grace_period() {
    let mut dir = TempDir::new("grace-gone");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut client = TestClient::hello(&dir.path, "away");
    let (_, pid) = client.spawn(client_spawn("cat", &[]));
    let (kept, kept_pid) = client.spawn(detach(client_spawn("cat", &[])));
    dir.track(parent(pid).unwrap());

    kill_core_and_wait_for_another(&dir, &mut client);
    // Someone else's client keeps no session alive.
    let _observer = TestClient::hello(&dir.path, "someone-else");
    let adopted = Instant::now();
    assert!(alive(pid) && alive(kept_pid));
    assert!(gone(pid));
    assert!(
        adopted.elapsed() >= Duration::from_secs(8),
        "hung up after {:?}",
        adopted.elapsed()
    );
    assert!(alive(kept_pid), "a detach session waits for its owner");

    let mut owner = TestClient::hello(&dir.path, "away");
    owner.input(kept, b"x\n");
    owner.output_until(kept, b"x\r\nx\r\n");
}

#[test]
fn kill_and_stop_end_sessions_whatever_their_policy() {
    let (dir, mut client) = core("stop");
    let core_pid = client.core_pid();
    let (first, first_pid) = client.spawn(detach(client_spawn("cat", &[])));
    client.send(client::ToCore::Kill(client::Kill {
        req: 100,
        target: client::KillTarget::Session(first),
    }));
    assert!(matches!(client.reply(100), ToClient::Ok(_)));
    assert_eq!(client.exit_code(first), None);
    assert!(!alive(first_pid));

    client.send(client::ToCore::Kill(client::Kill {
        req: 101,
        target: client::KillTarget::Session(999),
    }));
    match client.reply(101) {
        ToClient::Error(error) => assert_eq!(error.message, "no session 999"),
        other => panic!("{other:?}"),
    }

    let (kept, kept_pid) = client.spawn(detach(client_spawn("cat", &[])));
    let (plain, plain_pid) = client.spawn(client_shell("sleep 1000"));
    let holder = parent(kept_pid).unwrap();
    client.send(client::ToCore::Stop(client::Request { req: 102 }));
    assert!(matches!(client.reply(102), ToClient::Ok(_)));
    assert_eq!(client.exit_code(kept), None);
    assert_eq!(client.exit_code(plain), None);
    assert!(client.closes());
    assert!(!alive(kept_pid) && !alive(plain_pid));
    assert!(gone(core_pid) && gone(holder));
    assert!(!core_socket(&dir.path).exists());
    assert!(dir.holder_sockets().is_empty());
}

/// A process's resident set, in KiB.
fn rss_kib(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.split_whitespace().next()?.parse().ok())
        .unwrap()
}
