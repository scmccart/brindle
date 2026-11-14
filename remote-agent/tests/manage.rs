//! The management commands (task 6.2): `brindle-remote list`, `kill`,
//! `status` and `stop`, run against a real core, on Linux and Windows.

#![cfg(any(target_os = "linux", windows))]

mod support;

use std::process::Output;

use brindle_remote_protocol::client::{self, SessionInfo};
use brindle_remote_protocol::serde_json::{self, Value};

use support::programs::{self, hung_up, say};
use support::*;

fn core(tag: &str) -> (TempDir, TestClient) {
    let mut dir = TempDir::new(tag);
    start_core(&dir.path, &[]).unwrap();
    let client = TestClient::hello(&dir.path, &format!("token-{tag}"));
    dir.track(client.core_pid());
    (dir, client)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn succeeded(output: Output) -> Output {
    assert!(output.status.success(), "{output:?}");
    output
}

/// What a `list` line shows of a spawn's command.
fn command_of(spawn: &client::Spawn) -> String {
    let mut words = vec![spawn.command.clone().unwrap()];
    words.extend(spawn.args.iter().cloned());
    words.join(" ")
}

/// Two working directories that exist.
#[cfg(unix)]
fn directories() -> [String; 2] {
    ["/".into(), "/tmp".into()]
}

#[cfg(windows)]
fn directories() -> [String; 2] {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let temp = std::env::temp_dir().to_string_lossy().into_owned();
    [root, temp.trim_end_matches('\\').into()]
}

#[test]
fn with_no_server_running_nothing_is_printed_and_nothing_starts() {
    let dir = TempDir::new("cli-none");
    for args in [&["list"][..], &["list", "--json"]] {
        let output = succeeded(cli(&dir.path, args));
        assert!(output.stdout.is_empty() && output.stderr.is_empty(), "{output:?}");
    }
    let output = succeeded(cli(&dir.path, &["status"]));
    assert_eq!(stdout(&output), "no server is running\n");
    let output = succeeded(cli(&dir.path, &["status", "--json"]));
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status, serde_json::json!({ "running": false }));
    for args in [&["kill", "3"][..], &["kill", "--all"], &["stop"]] {
        let output = succeeded(cli(&dir.path, args));
        assert!(output.stdout.is_empty());
        assert_eq!(stderr(&output), "no server is running\n");
    }
    assert!(server_processes(&dir.path, "serve").is_empty());
    assert!(core_endpoint_gone(&dir.path));
    // A directory that doesn't exist isn't created.
    let missing = dir.path.join("missing");
    succeeded(cli(&missing, &["list"]));
    assert!(!missing.exists());
}

#[test]
fn list_shows_one_line_per_session() {
    let (dir, mut client) = core("cli-list");
    let [first_dir, second_dir] = directories();
    let mut attached = programs::echoer();
    attached.cwd = Some(first_dir.clone());
    let first_command = command_of(&attached);
    let (first, first_pid) = client.spawn(attached);
    let mut detached = programs::sleeps();
    detached.cwd = Some(second_dir.clone());
    detached.policy = client::Policy::Detach;
    let second_command = command_of(&detached);
    let (second, second_pid) = client.spawn(detached);
    // On Windows the console host titles each session before its program
    // prints: once both have printed, their titles are settled.
    #[cfg(windows)]
    {
        say(&mut client, first, "up");
        client.output_until(second, b"127.0.0.1");
    }
    client.detach(second);
    // The DETACH is handled before the next request on this connection.
    let listed = client.list();
    let title = |session| {
        let info = listed.iter().find(|info| info.session == session).unwrap();
        if info.title.is_empty() {
            "-".to_string()
        } else {
            info.title.clone()
        }
    };

    let output = succeeded(cli(&dir.path, &["list"]));
    let mut lines: Vec<Vec<String>> = stdout(&output)
        .lines()
        .map(|line| line.split('\t').map(String::from).collect())
        .collect();
    // The age (column 5) depends on the clock: a few seconds at most.
    for line in &mut lines {
        let age = line.remove(4);
        assert!(age.ends_with('s') && !age.contains('m'), "{age}");
    }
    let line = |session: u32, pid: u32, state: &str, policy: &str, command: &str, cwd: &str| {
        [&*session.to_string(), &*pid.to_string(), state, policy, &title(session), command, cwd]
            .map(String::from)
            .to_vec()
    };
    assert_eq!(
        lines,
        [
            line(first, first_pid, "attached", "kill", &first_command, &first_dir),
            line(second, second_pid, "detached", "detach", &second_command, &second_dir),
        ]
    );
    // Nothing titles a Linux session here.
    #[cfg(unix)]
    assert_eq!((title(first), title(second)), ("-".into(), "-".into()));

    let output = succeeded(cli(&dir.path, &["list", "--json"]));
    let sessions: Vec<SessionInfo> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sessions, client.list());
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[1].policy, client::Policy::Detach);
}

#[test]
fn kill_hangs_up_one_session_or_all_and_names_an_unknown_id() {
    let (dir, mut client) = core("cli-kill");
    let (session, pid) = client.spawn(programs::echoer());
    let (other, other_pid) = client.spawn(programs::echoer());
    // Each program runs before it is hung up.
    say(&mut client, session, "up");
    say(&mut client, other, "up");

    let output = succeeded(cli(&dir.path, &["kill", &session.to_string()]));
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    hung_up(&mut client, session);
    assert!(gone(pid));
    assert!(alive(other_pid));

    let output = cli(&dir.path, &["kill", "999"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stderr(&output), "brindle-remote: no session 999\n");
    assert!(!cli(&dir.path, &["kill", "nope"]).status.success());

    let (third, third_pid) = client.spawn(programs::echoer());
    say(&mut client, third, "up");
    succeeded(cli(&dir.path, &["kill", "--all"]));
    hung_up(&mut client, other);
    hung_up(&mut client, third);
    assert!(gone(other_pid) && gone(third_pid));
}

#[test]
fn status_shows_the_core_and_each_holder() {
    let (dir, mut client) = core("cli-status");
    let (_, pid) = client.spawn(programs::echoer());
    let holder = parent(pid).unwrap();
    let welcome = client.welcome.clone().unwrap();

    let output = succeeded(cli(&dir.path, &["status", "--json"]));
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["running"], true);
    assert_eq!(status["core_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(status["core_build"], welcome.core_build.as_str());
    assert_eq!(status["protocols"], serde_json::json!({ "min": 1, "max": 1 }));
    assert_eq!(status["holder_protocols"], serde_json::json!({ "min": 1, "max": 2 }));
    assert_eq!(status["pid"], client.core_pid());
    assert!(status["uptime_secs"].is_u64());
    // The test client and the command's own connection.
    assert_eq!(status["clients"], 2);
    assert!(status.get("req").is_none());
    assert_eq!(
        status["holders"],
        serde_json::json!([{
            "pid": holder,
            "build": welcome.core_build,
            "protocol": 2,
            "sessions": 1,
            "draining": false,
        }])
    );

    let text = stdout(&succeeded(cli(&dir.path, &["status"])));
    assert!(
        text.contains(&format!("pid {}", client.core_pid()))
            && text.contains("2 client(s)")
            && text.contains(&format!("holder: pid {holder}"))
            && text.contains("1 session(s)"),
        "{text}"
    );
}

#[test]
fn stop_ends_every_session_the_core_and_the_holders() {
    let (dir, mut client) = core("cli-stop");
    let core_pid = client.core_pid();
    let (killed, killed_pid) = client.spawn(programs::echoer());
    let mut detach = programs::echoer();
    detach.policy = client::Policy::Detach;
    let (kept, kept_pid) = client.spawn(detach);
    let holder = parent(killed_pid).unwrap();
    // Each program runs before it is hung up.
    say(&mut client, killed, "up");
    say(&mut client, kept, "up");

    let output = succeeded(cli(&dir.path, &["stop"]));
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    hung_up(&mut client, killed);
    hung_up(&mut client, kept);
    assert!(client.closes());
    assert!(gone(killed_pid) && gone(kept_pid));
    assert!(gone(core_pid) && gone(holder));
    assert!(core_endpoint_gone(&dir.path));
    assert!(dir.holder_endpoints().is_empty());
    // And now there is no server.
    let output = succeeded(cli(&dir.path, &["status"]));
    assert_eq!(stdout(&output), "no server is running\n");
}
