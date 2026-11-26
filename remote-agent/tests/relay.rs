//! `brindle-remote relay` (tasks 4.1 and 4.2): reaching or starting the
//! core, `RELAY_HOST` and `RELAY_ERROR`, a client's sessions through it, and
//! the link's liveness; and (`remote-login-environment`) the login shell's
//! environment, the passwd basics and the `login-env` role. Upgrades
//! through a relay are in `upgrade.rs`.
//!
//! Linux only, as relays are. Each relay's stdio is a socket pair whose other
//! end the test holds, as `ssh` would.

#![cfg(target_os = "linux")]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use brindle_remote_protocol::Frame;
use brindle_remote_protocol::client::{self, ToClient};
use brindle_remote_protocol::relay::{LoginEnv, RelayMessage};
use brindle_remote_protocol::serde_json::json;
use brindle_transcript::{AgentState, Entry, HoldReason, Slot, StopReason, TurnState};

use support::fake_agent::{ask, methods, prompts, responses, said, started, stop};
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
    // Not asked for, so no login shell ran.
    assert_eq!(host.login, None);

    let (echo, pid) = client.spawn(programs::echoer());
    dir.track(parent(pid).unwrap());
    say(&mut client, echo, "through the relay");

    let logs = dir.logs();
    assert!(
        logs.lines().any(|line| line.contains("relay[")
            && line.contains("without a scope of its own (BRINDLE_NO_SCOPE=1)")
            && !line.contains("may not outlive")),
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
    let status = relay.exits_within(TIMEOUT, "the relay's exit once its client's stdin ended");
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
    let status = relay.exits_within(IDLE + TIMEOUT, "the relay's exit with a silent client");
    let after = silent.elapsed();
    assert!(!status.success());
    // Its last frame came just before `silent`; the wait above bounds the
    // other side.
    assert!(after >= IDLE / 2, "the relay ended after {after:?}");
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
    // Blocked, as `stays_blocked` tells it: the program never ran for
    // `BLOCKED_FOR`, and it and the holder, which reads its terminal, were
    // asleep at every look.
    let holder = parent(pid).unwrap();
    let asleep = |pid| state(pid) == Some('S');
    let stopped = Instant::now();
    let mut next_heartbeat = stopped;
    let mut quiet = Quiet::new(BLOCKED_FOR);
    assert!(
        eventually(TIMEOUT, || {
            if Instant::now() >= next_heartbeat {
                client.send_frame(&heartbeat());
                next_heartbeat += HEARTBEAT;
            }
            let blocked = quiet
                .observe(context_switches(pid).filter(|_| asleep(pid) && asleep(holder)));
            blocked && stopped.elapsed() >= IDLE * 3
        }),
        "the flooding program never blocked"
    );
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

/// An agent session through a relay (`agents-sessions` 8.2): every agent
/// message a client sends, `SPAWN_AGENT` and `AGENT_PROMPT`, `_PERMISSION`,
/// `_UNQUEUE`, `_CANCEL` and `_RESEND` (frames 15–20), reaches the core as
/// sent, and its `TRANSCRIPT`s (frame 42) reach the client as the core sent
/// them: what the agent was sent is what the client asked, and the log the
/// client folded from them is the one a client on the core's own socket is
/// replayed.
#[test]
fn an_agent_sessions_frames_pass_through_a_relay_unchanged() {
    let mut dir = TempDir::new("relay-agent");
    let mut relay = Relay::start(Path::new(BIN), &dir.path, &[]);
    let host = relay.host();
    dir.track(host.core_pid);
    let mut client = relay.client("r");
    assert_eq!(client.welcome.as_ref().unwrap().protocol, 4);

    // The first turn asks permission and ends; the second waits for a
    // cancel, and the third is the held prompt sent again.
    let (mut script, record) = started(&dir.path, "relayed");
    script
        .step("expect session/prompt")
        .step(ask("perm-1"))
        .step(r#"expect-response "perm-1""#)
        .step(said("first"))
        .step(stop("end_turn"))
        .step("expect session/prompt")
        .step("expect session/cancel")
        .step(stop("cancelled"))
        .step("expect session/prompt")
        .step(said("third"))
        .step(stop("end_turn"));
    let (id, pid) = client.spawn_agent(client_agent(&script.write()));
    dir.track(pid);
    client.transcript_until(id, "is ready", |log| matches!(log.agent(), AgentState::Ready { .. }));

    // One turn, with a prompt waiting while it is blocked on a permission.
    client.prompt(id, "one").unwrap();
    let log = client.transcript_until(id, "waits on the permission", |log| *log.turn() == TurnState::Blocked);
    let entry = log
        .entries()
        .find_map(|(entry, kind)| matches!(kind, Entry::Permission { .. }).then_some(entry.0))
        .expect("the permission request is in the log");
    client.prompt(id, "two").unwrap();
    client.transcript_until(id, "has the prompt waiting", |log| *log.slot() == Slot::Waiting { text: "two".into() });
    client.answer(id, entry, Some("allow-once")).unwrap();
    let user_two = |log: &Transcript| {
        log.entries().any(|(_, kind)| matches!(kind, Entry::User { text } if text == "two"))
    };
    client.transcript_until(id, "runs the waiting prompt's turn", |log| {
        user_two(log) && *log.slot() == Slot::Empty && *log.turn() == TurnState::Working
    });

    // A prompt waits, is taken back, waits again, and is held by a cancel.
    client.prompt(id, "three").unwrap();
    client.unqueue(id).unwrap();
    client.transcript_until(id, "has the slot empty", |log| *log.slot() == Slot::Empty);
    client.prompt(id, "three").unwrap();
    client.cancel(id);
    let held = Slot::Held { text: "three".into(), reason: HoldReason::Cancelled };
    client.transcript_until(id, "holds the prompt", |log| {
        *log.slot() == held && *log.turn() == TurnState::Ended { stop: StopReason::Cancelled }
    });
    // Sent again, it is the next turn.
    client.resend(id).unwrap();
    let says_third = |log: &Transcript| {
        log.entries().any(|(_, kind)| matches!(kind, Entry::Agent { text } if text == "third"))
    };
    let relayed = client.transcript_until(id, "has the resent prompt's turn", |log| {
        says_third(log) && *log.turn() == TurnState::Ended { stop: StopReason::EndTurn }
    });

    // The agent was sent what the client asked, once each.
    assert_eq!(prompts(&record), ["one", "two", "three"]);
    let answers = responses(&record);
    assert_eq!(answers.len(), 1, "{answers:?}");
    assert_eq!(answers[0]["id"], json!("perm-1"));
    assert_eq!(answers[0]["result"]["outcome"], json!({"outcome": "selected", "optionId": "allow-once"}));
    assert_eq!(
        methods(&record),
        ["initialize", "session/new", "session/prompt", "session/prompt", "session/cancel", "session/prompt"]
    );
    // The relayed events folded into the log the core keeps.
    client.detach(id);
    // The DETACH is handled before the next request on this connection.
    assert!(client.list().iter().any(|info| info.session == id && !info.attached));
    let mut direct = TestClient::hello(&dir.path, "direct");
    direct.attach_agent(id).unwrap();
    assert_eq!(direct.transcript(id), relayed);
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
    assert!(!relay.exits_within(TIMEOUT, "the relay's exit after RELAY_ERROR").success());

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
    assert!(!relay.exits_within(TIMEOUT, "the relay's exit after RELAY_ERROR").success());
    assert!(!core_listens(&long));
}

/// A home of the test's own under `dir`, with `profile` as its `.profile`.
fn login_home(dir: &TempDir, profile: &str) -> String {
    let home = dir.path.join("home");
    fs::create_dir(&home).unwrap();
    fs::write(home.join(".profile"), profile).unwrap();
    home.display().to_string()
}

/// A relay started with `--login-environment`, its `HOME` and `SHELL` as
/// given, and `env` on top.
fn login_relay(dir: &TempDir, home: &str, shell: &str, env: &[(&str, &str)]) -> Relay {
    let mut all = vec![("HOME", home), ("SHELL", shell)];
    all.extend_from_slice(env);
    Relay::start_with(Path::new(BIN), &dir.path, &["--login-environment"], &all, &[])
}

/// The process ids in the process group `group`.
fn group_members(group: u32) -> Vec<u32> {
    fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&pid| {
            fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                // pgrp is the third field after the command's `)`.
                stat.rfind(')').and_then(|end| stat[end + 1..].split_whitespace().nth(2))
                    == Some(group.to_string().as_str())
            }) && alive(pid)
        })
        .collect()
}

/// A number a startup file wrote to `path`, once it has.
fn written_pid(path: &Path) -> u32 {
    until("a startup file's pid", TIMEOUT, || {
        fs::read_to_string(path).ok()?.trim().parse().ok()
    })
}

/// `remote-login-environment` 4.4: a noisy `.profile` that exports a variable and
/// leaves a program holding the dump pipe. The relay reports the login
/// environment without waiting for that program, nothing the profile
/// printed reaches the link, and the relay relays.
#[test]
fn a_relay_reports_the_login_shells_environment_without_its_noise() {
    let mut dir = TempDir::new("relay-login");
    let home = login_home(
        &dir,
        "echo profile-out\necho profile-err >&2\nexport RELAY_LOGIN_VAR=1\n\
         sleep 30 &\necho $! > \"$HOME/sleep.pid\"\n",
    );
    let mut relay = login_relay(&dir, &home, "/bin/sh", &[]);
    // Nothing came before the marker (`Relay::first`), and the frames after
    // it decode.
    let host = relay.host();
    dir.track(host.core_pid);
    let sleep = written_pid(&Path::new(&home).join("sleep.pid"));
    dir.track(sleep);

    let Some(LoginEnv::Captured(login)) = &host.login else {
        panic!("expected a captured login environment, got {:?}", host.login);
    };
    assert_eq!(login.get("RELAY_LOGIN_VAR").map(String::as_str), Some("1"));
    assert_eq!(login.get("HOME"), Some(&home));
    assert!(!host.env.contains_key("RELAY_LOGIN_VAR"));
    // The variables the shell set for itself are the relay's own.
    for name in ["SHLVL", "PWD", "OLDPWD", "_"] {
        assert_eq!(login.get(name), host.env.get(name), "{name}");
    }
    // The program the profile left running still holds the pipe.
    assert!(alive(sleep));

    let mut client = relay.client("r");
    let (echo, pid) = client.spawn(programs::echoer());
    dir.track(parent(pid).unwrap());
    say(&mut client, echo, "after the login shell");
}

/// bash as the login shell: an interactive login bash marks descriptors 3 to
/// 19 close-on-exec, so the dump pipe must be above them for the role it
/// `exec`s to find it. bash reads `.profile` when there is no
/// `.bash_profile`.
#[test]
fn bash_as_the_login_shell_hands_its_environment_over() {
    let mut dir = TempDir::new("relay-login-bash");
    let home = login_home(&dir, "export RELAY_LOGIN_VAR=bash\n");
    let mut relay = login_relay(&dir, &home, "/bin/bash", &[]);
    let host = relay.host();
    dir.track(host.core_pid);
    let Some(LoginEnv::Captured(login)) = &host.login else {
        panic!("expected a captured login environment, got {:?}", host.login);
    };
    assert_eq!(login.get("RELAY_LOGIN_VAR").map(String::as_str), Some("bash"));
}

/// `remote-login-environment` 4.4: a `.profile` that never finishes. The relay
/// reports it took too long, the shell's process group is gone, `core.log`
/// says why, and the relay relays.
#[test]
fn a_login_shell_that_takes_too_long_is_ended_and_reported() {
    let mut dir = TempDir::new("relay-login-slow");
    let home = login_home(&dir, "echo $$ > \"$HOME/shell.pid\"\nsleep 60\n");
    let started = Instant::now();
    let mut relay = login_relay(
        &dir,
        &home,
        "/bin/sh",
        &[("BRINDLE_TEST_RELAY_LOGIN_ENV_MS", "300")],
    );
    let host = relay.host();
    dir.track(host.core_pid);
    assert!(
        started.elapsed() < limit(Duration::from_secs(10)),
        "RELAY_HOST took {:?}",
        started.elapsed()
    );
    assert_eq!(
        host.login,
        Some(LoginEnv::Failed("it took longer than 300 ms".into()))
    );
    let shell = written_pid(&Path::new(&home).join("shell.pid"));
    assert!(
        eventually(TIMEOUT, || group_members(shell).is_empty()),
        "left running: {:?}",
        group_members(shell)
    );
    let logs = dir.logs();
    assert!(
        logs.lines().any(|line| line.contains("relay[")
            && line.contains("the login shell's environment wasn't read: it took longer than 300 ms")),
        "{logs}"
    );

    let mut client = relay.client("r");
    let (echo, pid) = client.spawn(programs::echoer());
    dir.track(parent(pid).unwrap());
    say(&mut client, echo, "after the timeout");
}

/// `remote-login-environment` 4.4: a shell that isn't there, and one Brindle
/// can't run as an interactive login shell (named, never run), are
/// failures with their reason, and the relay relays.
#[test]
fn a_missing_or_unsupported_login_shell_is_reported() {
    for (shell, reason) in [
        // A name Brindle runs, so it is tried: one it doesn't know is
        // unsupported before it is looked for.
        (
            "/nonexistent/relay-test/bash",
            "the login shell /nonexistent/relay-test/bash couldn't start: ",
        ),
        (
            "/bin/tcsh",
            "SHELL is /bin/tcsh, which can't be run as an interactive login shell to read it",
        ),
    ] {
        let mut dir = TempDir::new("relay-login-shell");
        let home = login_home(&dir, "");
        let mut relay = login_relay(&dir, &home, shell, &[]);
        let host = relay.host();
        dir.track(host.core_pid);
        let Some(LoginEnv::Failed(text)) = &host.login else {
            panic!("{shell}: expected a failed login environment, got {:?}", host.login);
        };
        assert!(text.starts_with(reason), "{shell}: {text}");
        let mut client = relay.client("r");
        let (echo, pid) = client.spawn(programs::echoer());
        dir.track(parent(pid).unwrap());
        say(&mut client, echo, "without a login shell");
    }
}

/// `remote-login-environment` 4.1: an endpoint that sets no `HOME` or `SHELL` (as
/// `docker exec` may not) gets them, `USER` and `LOGNAME` from the passwd
/// entry, and no login environment is reported when none was asked for.
#[test]
fn a_relays_environment_gets_the_passwd_basics() {
    // SAFETY: the passwd entry is read before any other call could reuse
    // its static buffer; the test's threads don't call getpw*.
    let (name, home, shell) = unsafe {
        let entry = libc::getpwuid(libc::getuid());
        assert!(!entry.is_null(), "no passwd entry for this user");
        let field = |field: *const libc::c_char| {
            std::ffi::CStr::from_ptr(field).to_string_lossy().into_owned()
        };
        (field((*entry).pw_name), field((*entry).pw_dir), field((*entry).pw_shell))
    };
    let mut dir = TempDir::new("relay-basics");
    let mut relay = Relay::start_with(
        Path::new(BIN),
        &dir.path,
        &[],
        &[("USER", "")],
        &["HOME", "SHELL", "LOGNAME"],
    );
    let host = relay.host();
    dir.track(host.core_pid);
    assert_eq!(host.login, None);
    let get = |name: &str| host.env.get(name).map(String::as_str);
    assert_eq!(get("HOME"), Some(home.as_str()));
    assert_eq!(get("SHELL"), Some(shell.as_str()));
    assert_eq!(get("USER"), Some(name.as_str()));
    assert_eq!(get("LOGNAME"), Some(name.as_str()));
    assert_eq!(host.shell, shell);
}

/// `remote-login-environment` 4.2: the hidden role writes its environment to fd 63
/// as `NAME=VALUE` records and an empty one, and refuses to run without fd
/// 63 or with arguments. bash runs the scripts, since dash's redirections
/// take one digit.
#[test]
fn the_login_env_role_writes_its_environment_to_fd_63() {
    // `63>&1`: fd 63 is the shell's stdout, which the test reads.
    let output = std::process::Command::new("/bin/bash")
        .args(["-c", "exec \"$0\" login-env 63>&1 1>/dev/null", BIN])
        .env_clear()
        .env("ROLE_TEST", "a b=c")
        .env("EMPTY", "")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let records: Vec<&[u8]> = output.stdout.split(|&byte| byte == 0).collect();
    let end = records
        .iter()
        .position(|record| record.is_empty())
        .expect("a terminator");
    let vars: Vec<&str> = records[..end]
        .iter()
        .map(|record| std::str::from_utf8(record).unwrap())
        .collect();
    assert!(vars.contains(&"ROLE_TEST=a b=c"), "{vars:?}");
    assert!(vars.contains(&"EMPTY="), "{vars:?}");
    // Nothing after the terminator but the end of the last record.
    assert_eq!(&output.stdout[output.stdout.len() - 2..], b"\0\0");

    for script in ["exec \"$0\" login-env 63>&-", "exec \"$0\" login-env extra 63>/dev/null"] {
        let status = std::process::Command::new("/bin/bash")
            .args(["-c", script, BIN])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "{script}: {status}");
    }
}
