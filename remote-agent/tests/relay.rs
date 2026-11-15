//! `brindle-remote relay` (tasks 4.1 and 4.2): reaching or starting the
//! core, `RELAY_HOST` and `RELAY_ERROR`, a client's sessions through it, and
//! the link's liveness. Upgrades through a relay are in `upgrade.rs`.
//!
//! Linux only, as relays are. Each relay's stdio is a socket pair whose other
//! end the test holds, as `ssh` would.

#![cfg(target_os = "linux")]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use brindle_remote_protocol::Frame;
use brindle_remote_protocol::client::{self, ToClient};
use brindle_remote_protocol::relay::RelayMessage;

use support::programs::{self, say};
use support::relay::{self, HEARTBEAT, IDLE, Relay, SHORT_TIMINGS};
use support::*;

fn heartbeat() -> Frame {
    RelayMessage::Heartbeat.encode().unwrap()
}

/// Whether the environment of the process `pid` has `var` (`NAME=value`).
fn environ_has(pid: u32, var: &str) -> bool {
    fs::read(format!("/proc/{pid}/environ"))
        .is_ok_and(|env| env.split(|&b| b == 0).any(|entry| entry == var.as_bytes()))
}

/// A `detach` session of [`programs::sleeps`].
fn detached_sleep() -> client::Spawn {
    let mut spawn = programs::sleeps();
    spawn.policy = client::Policy::Detach;
    spawn
}

#[test]
fn a_relay_starts_the_core_reports_its_host_and_relays_a_session() {
    let mut dir = TempDir::new("relay-start");
    assert!(!core_listens(&dir.path));
    let home = dir.path.display().to_string();
    let env = [
        ("BRINDLE_RELAY_TEST", "here"),
        ("SHELL", "/bin/relay-test-shell"),
        ("HOME", home.as_str()),
    ];
    let mut relay = Relay::start(Path::new(BIN), &dir.path, &env);
    let host = relay.host();
    dir.track(host.core_pid);
    // At once: the relay has opened the client's connection, which the core
    // keeps only so long without a HELLO.
    let mut client = relay.client("r");

    // The core runs detached from the relay, from this binary, and without
    // a scope, which its holders inherit.
    assert!(core_listens(&dir.path));
    assert!(alive(host.core_pid));
    assert_ne!(parent(host.core_pid), Some(relay.pid()));
    assert_eq!(server_processes(&dir.path, "serve"), [host.core_pid]);
    assert!(environ_has(host.core_pid, "BRINDLE_NO_SCOPE=1"));
    assert_eq!(client.core_pid(), host.core_pid);

    assert_eq!(host.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(host.build, client.welcome.as_ref().unwrap().core_build);
    assert_eq!(
        host.env.get("BRINDLE_RELAY_TEST").map(String::as_str),
        Some("here")
    );
    assert_eq!(
        host.env.get("BRINDLE_SERVER_DIR").map(String::as_str),
        Some(home.as_str())
    );
    assert_eq!(host.shell, "/bin/relay-test-shell");
    assert_eq!(host.home, home);
    assert_eq!(host.server_dir, home);

    let (echo, pid) = client.spawn(programs::echoer());
    dir.track(parent(pid).unwrap());
    say(&mut client, echo, "through the relay");

    let logs = dir.logs();
    assert!(
        logs.lines().any(|line| line.contains("relay[")
            && line.contains("without a scope of its own (BRINDLE_NO_SCOPE=1)")
            && line.contains("may not outlive the login")),
        "{logs}"
    );

    // Another relay reaches the same core, and starts none.
    let mut second = Relay::start(Path::new(BIN), &dir.path, &[]);
    let second_host = second.host();
    let mut other = second.client("r2");
    assert_eq!(second_host.core_pid, host.core_pid);
    assert_eq!(server_processes(&dir.path, "serve"), [host.core_pid]);
    assert!(
        other
            .list()
            .iter()
            .any(|session| session.session == echo && session.pid == pid && session.attached)
    );
}

#[test]
fn a_relay_ends_when_its_client_closes_and_the_core_lets_the_client_go() {
    let mut dir = TempDir::new("relay-close");
    let mut relay = Relay::start(Path::new(BIN), &dir.path, &[]);
    let host = relay.host();
    dir.track(host.core_pid);
    let mut client = relay.client("r");
    let (ended, ended_pid) = client.spawn(programs::sleeps());
    let (kept, kept_pid) = client.spawn(detached_sleep());
    dir.track(parent(kept_pid).unwrap());

    relay::hang_up(&client);
    let status = relay
        .exits_within(TIMEOUT)
        .expect("the relay outlived its client's stdin");
    assert!(status.success(), "{status}");
    // Nothing else held the relay's end of the link: neither the core nor a
    // holder inherited its stdio.
    assert!(client.closes());
    assert!(alive(host.core_pid));

    // The core let the client go: its `kill` session ended, and its
    // `detach` one is detached, with its owner.
    assert!(gone(ended_pid));
    let mut observer = TestClient::hello(&dir.path, "observer");
    assert!(eventually(TIMEOUT, || {
        let sessions = observer.list();
        sessions.iter().all(|session| session.session != ended)
            && sessions.iter().any(|session| {
                session.session == kept
                    && session.pid == kept_pid
                    && !session.attached
                    && session.owned
            })
    }));
}

#[test]
fn a_silent_client_ends_its_relay_and_its_sessions_stay_detached() {
    let mut dir = TempDir::new("relay-silent");
    let mut relay = Relay::start(Path::new(BIN), &dir.path, &SHORT_TIMINGS);
    let host = relay.host();
    dir.track(host.core_pid);
    let mut client = relay.client("r");
    let (kept, kept_pid) = client.spawn(detached_sleep());
    dir.track(parent(kept_pid).unwrap());

    // From here the client sends nothing, not even heartbeats.
    let silent = Instant::now();
    let status = relay
        .exits_within(IDLE + TIMEOUT)
        .expect("the relay outlived a silent client");
    let after = silent.elapsed();
    assert!(!status.success());
    // Its last frame came just before `silent`.
    assert!(after >= IDLE / 2, "the relay ended after {after:?}");
    assert!(
        after < IDLE + Duration::from_secs(5),
        "the relay ended after {after:?}"
    );
    assert!(client.closes());
    let logs = dir.logs();
    assert!(
        logs.contains("nothing from the client for 1500 ms"),
        "{logs}"
    );

    let mut observer = TestClient::hello(&dir.path, "observer");
    assert!(eventually(TIMEOUT, || observer.list().iter().any(
        |session| { session.session == kept && !session.attached && session.owned }
    )));
    assert!(alive(kept_pid));
}

#[test]
fn an_idle_client_that_sends_heartbeats_keeps_its_relay_and_gets_the_relays() {
    let mut dir = TempDir::new("relay-idle");
    let mut relay = Relay::start(Path::new(BIN), &dir.path, &SHORT_TIMINGS);
    let host = relay.host();
    dir.track(host.core_pid);
    let mut client = relay.client("r");
    let (echo, pid) = client.spawn(programs::echoer());
    dir.track(parent(pid).unwrap());

    let idle = Instant::now();
    while idle.elapsed() < IDLE * 3 {
        client.send_frame(&heartbeat());
        // Nothing but the relay's heartbeats, which the client counts.
        assert!(matches!(client.next(HEARTBEAT), Recv::TimedOut));
    }
    assert!(relay.running());
    // The relay wrote nothing else meanwhile, so it sent heartbeats.
    assert!(client.heartbeats >= 3, "{} heartbeats", client.heartbeats);
    say(&mut client, echo, "still here");
}

#[test]
fn a_client_that_stops_reading_during_a_flood_keeps_its_relay() {
    let mut dir = TempDir::new("relay-flood");
    let mut relay = Relay::start(Path::new(BIN), &dir.path, &SHORT_TIMINGS);
    let host = relay.host();
    dir.track(host.core_pid);
    let mut client = relay.client("r");
    let (flood, pid) = client.spawn(client_shell("exec yes flood"));
    dir.track(parent(pid).unwrap());
    client.output_until(flood, b"flood\r\n");

    // The client stops reading, but sends heartbeats, for three idle limits
    // and until the flood's program has stayed blocked for a while: the
    // relay's stdout is full, and the backpressure reaches the program.
    let stopped = Instant::now();
    let mut next_heartbeat = stopped;
    let mut blocked = 0;
    while stopped.elapsed() < IDLE * 3 || blocked < 10 {
        assert!(
            stopped.elapsed() < TIMEOUT,
            "the flooding program never blocked"
        );
        if Instant::now() >= next_heartbeat {
            client.send_frame(&heartbeat());
            next_heartbeat += HEARTBEAT;
        }
        blocked = if state(pid) == Some('S') {
            blocked + 1
        } else {
            0
        };
        thread::sleep(Duration::from_millis(20));
    }
    assert!(relay.running());
    // The relay held its frames back rather than buffer them.
    let relay_rss = rss_kib(relay.pid());
    assert!(relay_rss < 32 * 1024, "the relay holds {relay_rss} KiB");

    // Reading again (and still sending heartbeats, as a client does whatever
    // it reads), the flood flows again: far more arrives than the link, the
    // core and the PTY held when it stopped.
    let mut received = 0usize;
    let deadline = Instant::now() + TIMEOUT;
    while received < 16 * 1024 * 1024 {
        assert!(Instant::now() < deadline, "only {received} bytes resumed");
        if Instant::now() >= next_heartbeat {
            client.send_frame(&heartbeat());
            next_heartbeat = Instant::now() + HEARTBEAT;
        }
        if let ToClient::Data(data) = client.recv()
            && data.session == flood
        {
            received += data.bytes.len();
        }
    }
    assert!(relay.running());
}

/// A process's resident set, in KiB.
fn rss_kib(pid: u32) -> u64 {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.split_whitespace().next()?.parse().ok())
        .unwrap()
}

#[test]
fn heartbeats_never_reach_a_protocol_1_core() {
    let mut dir = TempDir::new("relay-p1");
    let mut relay = Relay::start(Path::new(BIN), &dir.path, &[]);
    let host = relay.host();
    dir.track(host.core_pid);
    let mut client = relay.client_with("r", |hello| (hello.min, hello.max) = (1, 1));
    assert_eq!(client.welcome.as_ref().unwrap().protocol, 1);

    for _ in 0..20 {
        client.send_frame(&heartbeat());
    }
    // A core that got one would have answered with an ERROR by the time
    // it answers the LIST sent after them.
    assert!(client.list().is_empty());
    assert!(matches!(
        client.next(Duration::from_millis(300)),
        Recv::TimedOut
    ));
    let (echo, pid) = client.spawn(programs::echoer());
    dir.track(parent(pid).unwrap());
    say(&mut client, echo, "after the heartbeats");

    // The check means something: a heartbeat sent to the core directly is
    // an error.
    let mut direct =
        TestClient::hello_with(&dir.path, "direct", |hello| (hello.min, hello.max) = (1, 1));
    direct.send_frame(&heartbeat());
    assert!(matches!(direct.recv(), ToClient::Error(_)));
}

#[test]
fn a_relay_that_cant_reach_a_core_says_why() {
    let dir = TempDir::new("relay-error");

    // A server directory others may enter is refused.
    let open = dir.path.join("open");
    fs::create_dir(&open).unwrap();
    fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
    let mut relay = Relay::start(Path::new(BIN), &open, &[]);
    let RelayMessage::Error(error) = relay.first() else {
        panic!("expected RELAY_ERROR");
    };
    assert!(
        error.message.contains("refusing the server directory") && error.message.contains("0755"),
        "{}",
        error.message
    );
    assert!(
        !relay
            .exits_within(TIMEOUT)
            .expect("the relay kept running")
            .success()
    );

    // A core socket that can't be connected to (its path is too long), with
    // the core's log named.
    let long = dir.path.join("x".repeat(100));
    fs::create_dir(&long).unwrap();
    private_dir(&long);
    let mut relay = Relay::start(Path::new(BIN), &long, &[]);
    let RelayMessage::Error(error) = relay.first() else {
        panic!("expected RELAY_ERROR");
    };
    let log = long.join("core.log").display().to_string();
    assert!(
        error.message.contains("can't connect") && error.message.contains(&log),
        "{}",
        error.message
    );
    assert!(
        !relay
            .exits_within(TIMEOUT)
            .expect("the relay kept running")
            .success()
    );
    assert!(!core_listens(&long));
}
