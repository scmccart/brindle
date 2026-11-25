//! The management commands (task 6.2, and the owner columns of
//! `terminal-server-remote` 3.4): `brindle-remote list`, `kill`,
//! `status` and `stop`, run against a real core, on Linux and Windows; and
//! agent sessions in them (`agents-sessions` 8.1).

#![cfg(any(target_os = "linux", windows))]

mod support;

use std::process::Output;

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::{self, SessionInfo, ToClient};
use brindle_remote_protocol::serde_json::{self, Value, json};
use brindle_transcript::TurnState;

use support::fake_agent::{ask, started};
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

/// The seconds a `list` duration column (an age, a time detached) shows:
/// digits, each followed by its unit (`s`, `m`, `h` or `d`), as in `3s`,
/// `2m05s` or `1h02m`. `None` for anything else, `-` included.
fn duration_column(text: &str) -> Option<u64> {
    if text.is_empty() {
        return None;
    }
    let mut total = 0u64;
    let mut digits = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let unit = match c {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86_400,
            _ => return None,
        };
        let count: u64 = digits.parse().ok()?;
        digits.clear();
        total = total.checked_add(count.checked_mul(unit)?)?;
    }
    digits.is_empty().then_some(total)
}

#[test]
fn duration_columns_are_digits_and_units() {
    assert_eq!(duration_column("3s"), Some(3));
    assert_eq!(duration_column("2m5s"), Some(125));
    assert_eq!(duration_column("2m05s"), Some(125));
    assert_eq!(duration_column("1h"), Some(3600));
    assert_eq!(duration_column("1h02m"), Some(3720));
    for rejected in ["-", "x", "", "5", "s", "3s4", "1y"] {
        assert_eq!(duration_column(rejected), None, "{rejected:?}");
    }
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
fn the_helper_lists_its_fault_switches() {
    let dir = TempDir::new("cli-switches");
    let output = succeeded(cli(&dir.path, &["--list-fault-switches"]));
    assert!(output.stderr.is_empty(), "{output:?}");
    for line in stdout(&output).lines() {
        assert!(line.starts_with("BRINDLE_TEST_"), "{line:?}");
    }
    // The listing never connects to a core, so it starts none.
    assert!(server_processes(&dir.path, "serve").is_empty());
    assert!(core_endpoint_gone(&dir.path));
    let output = cli(&dir.path, &["--list-fault-switches", "extra"]);
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(stderr(&output).contains("takes no arguments"), "{output:?}");
}

#[test]
fn a_failed_command_names_its_action() {
    let dir = TempDir::new("cli-file");
    // The server directory is a file: there may be a server, but it can't
    // be reached, which isn't "no server".
    let file = dir.path.join("server");
    std::fs::write(&file, b"").unwrap();
    let output = cli(&file, &["list"]);
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let text = stderr(&output);
    let prefix = "can't read the server directory ";
    assert!(text.contains(prefix), "{text}");
    // Then the system's own words for the failure, after the path.
    let (_, cause) = text.split_once(prefix).unwrap().1.split_once(": ").unwrap();
    assert!(!cause.trim().is_empty(), "{text}");
    assert!(file.is_file(), "the file was left alone");
}

#[test]
fn list_shows_one_line_per_session() {
    let began = std::time::Instant::now();
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
    // The age (column 7) and the time detached (column 5) depend on the
    // clock: durations, no longer than this test has run.
    let ran = began.elapsed().as_secs() + 1;
    let within = |column: &str| duration_column(column).is_some_and(|secs| secs <= ran);
    let mut detached_for = Vec::new();
    for line in &mut lines {
        let age = line.remove(6);
        assert!(within(&age), "age {age:?}, after {ran}s");
        detached_for.push(line.remove(4));
    }
    assert_eq!(detached_for[0], "-", "attached");
    assert!(within(&detached_for[1]), "{detached_for:?}, after {ran}s");
    let line = |session: u32, pid: u32, state: &str, owner: &str, policy: &str, command: &str, cwd: &str| {
        [&*session.to_string(), &*pid.to_string(), state, owner, policy, &title(session), command, cwd]
            .map(String::from)
            .to_vec()
    };
    assert_eq!(
        lines,
        [
            line(first, first_pid, "attached", "-", "kill", &first_command, &first_dir),
            // Its client let it go, so it has no owner.
            line(second, second_pid, "detached", "unowned", "detach", &second_command, &second_dir),
        ]
    );
    // Nothing titles a Linux session here.
    #[cfg(unix)]
    assert_eq!((title(first), title(second)), ("-".into(), "-".into()));

    // The time detached ticks on between two lists.
    let unclocked = |sessions: Vec<SessionInfo>| -> Vec<SessionInfo> {
        sessions
            .into_iter()
            .map(|session| SessionInfo { detached_secs: 0, ..session })
            .collect()
    };
    let output = succeeded(cli(&dir.path, &["list", "--json"]));
    let sessions: Vec<SessionInfo> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(unclocked(sessions.clone()), unclocked(client.list()));
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[1].policy, client::Policy::Detach);
}

#[test]
fn list_shows_whether_a_detached_sessions_owner_may_come_back() {
    let (dir, mut owner) = core("cli-owned");
    let detached = || client::Spawn {
        policy: client::Policy::Detach,
        ..programs::sleeps()
    };
    let (dropped, _) = owner.spawn(detached());
    let (released, _) = owner.spawn(detached());
    owner.detach(released);
    // Its connection ends without a DETACH for `dropped`.
    drop(owner);
    let listed = || -> Vec<Vec<String>> {
        stdout(&succeeded(cli(&dir.path, &["list"])))
            .lines()
            .map(|line| line.split('\t').map(String::from).collect())
            .collect()
    };
    assert!(
        eventually(TIMEOUT, || listed().iter().all(|line| line[2] == "detached")),
        "{:?}",
        listed()
    );
    // Both have been detached for a second, in the text and in the JSON.
    let listed_json = || -> Vec<Value> {
        let output = succeeded(cli(&dir.path, &["list", "--json"]));
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let (lines, sessions) = until("both sessions listed as detached for a second", TIMEOUT, || {
        let lines = listed();
        let sessions = listed_json();
        let text_ticked = lines.iter().all(|line| line[4] != "0s");
        let json_ticked = sessions
            .iter()
            .all(|info| info["detached_secs"].as_u64().is_some_and(|secs| secs >= 1));
        (text_ticked && json_ticked).then_some((lines, sessions))
    });
    let line = |session: u32| lines.iter().find(|line| line[0] == session.to_string()).unwrap();
    assert_eq!(line(dropped)[3], "owned", "{lines:?}");
    assert_eq!(line(released)[3], "unowned", "{lines:?}");
    for session in [dropped, released] {
        let detached = &line(session)[4];
        assert!(duration_column(detached).is_some_and(|secs| secs >= 1), "{lines:?}");
    }

    let json = |session: u32| {
        sessions
            .iter()
            .find(|info| info["session"] == session)
            .unwrap()
    };
    assert_eq!(json(dropped)["owned"], true, "{sessions:?}");
    assert!(json(released).get("owned").is_none(), "false is left out: {sessions:?}");
    for session in [dropped, released] {
        assert!(json(session)["detached_secs"].as_u64().is_some_and(|secs| secs >= 1), "{sessions:?}");
    }
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
    assert_eq!(status["protocols"], serde_json::json!({ "min": 1, "max": 4 }));
    assert_eq!(status["holder_protocols"], serde_json::json!({ "min": 1, "max": 3 }));
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
            "protocol": 3,
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

/// A fake agent that is given one prompt and then waits on permission
/// request `perm-1` until its stdin closes. Gives its `SPAWN_AGENT`.
fn asks_and_waits(dir: &TempDir, name: &str) -> client::SpawnAgent {
    let (mut script, _) = started(&dir.path, name);
    script.step("expect session/prompt").step(ask("perm-1")).step(r#"expect-response "perm-1""#);
    client_agent(&script.write())
}

/// A fake agent that starts and idles until its stdin closes.
fn idles(dir: &TempDir, name: &str) -> client::SpawnAgent {
    let (script, _) = started(&dir.path, name);
    client_agent(&script.write())
}

/// Starts an agent session, tracking its agent, and waits until it is
/// ready.
fn agent_session(client: &mut TestClient, dir: &mut TempDir, spawn: client::SpawnAgent) -> (SessionId, u32) {
    let (id, pid) = client.spawn_agent(spawn);
    dir.track(pid);
    client.transcript_until(id, "is ready", |log| {
        matches!(log.agent(), brindle_transcript::AgentState::Ready { .. })
    });
    (id, pid)
}

/// An agent session in the middle of a turn, waiting on a permission
/// request nobody answers.
fn blocked_agent(client: &mut TestClient, dir: &mut TempDir, name: &str) -> (SessionId, u32) {
    let spawn = asks_and_waits(dir, name);
    let (id, pid) = agent_session(client, dir, spawn);
    client.prompt(id, "write").unwrap();
    client.transcript_until(id, "waits on the permission", |log| *log.turn() == TurnState::Blocked);
    (id, pid)
}

/// Waits for the `EXIT` of each of `sessions`, in whatever order they
/// come, and gives their codes in `sessions`' order; other messages are
/// skipped (each `TRANSCRIPT` is folded as it is read).
fn exits(client: &mut TestClient, sessions: &[SessionId]) -> Vec<Option<i32>> {
    let mut codes = vec![None; sessions.len()];
    let mut left = sessions.len();
    while left > 0 {
        if let ToClient::Exit(exit) = client.recv()
            && let Some(n) = sessions.iter().position(|&session| session == exit.session)
        {
            codes[n] = exit.code;
            left -= 1;
        }
    }
    codes
}

#[test]
fn list_shows_an_agent_sessions_activity_in_its_titles_place() {
    let (mut dir, mut client) = core("cli-agent-list");
    let [terminal_dir, _] = directories();
    let mut terminal = programs::echoer();
    terminal.cwd = Some(terminal_dir.clone());
    let terminal_command = command_of(&terminal);
    let (shell, shell_pid) = client.spawn(terminal);
    #[cfg(windows)]
    say(&mut client, shell, "up");
    let (agent, agent_pid) = blocked_agent(&mut client, &mut dir, "list");
    let listed = client.list();
    let info = |session| listed.iter().find(|info: &&SessionInfo| info.session == session).unwrap().clone();
    let agent_info = info(agent);

    let output = succeeded(cli(&dir.path, &["list"]));
    let lines: Vec<Vec<String>> = stdout(&output)
        .lines()
        .map(|line| line.split('\t').map(String::from).collect())
        .collect();
    let line = |session: SessionId| lines.iter().find(|line| line[0] == session.to_string()).unwrap();
    assert_eq!(lines.len(), 2, "{lines:?}");
    // The terminal's line is as before: its title, or `-`.
    let shell_title = info(shell).title;
    let shell_title = if shell_title.is_empty() { "-".to_string() } else { shell_title };
    let shell_line = line(shell);
    assert_eq!(
        [&shell_line[..4], &shell_line[5..6], &shell_line[7..]].concat(),
        [&*shell.to_string(), &*shell_pid.to_string(), "attached", "-", "kill", &shell_title, &terminal_command, &terminal_dir]
    );
    // The agent's shows its activity where the title goes, then its
    // command and its absolute directory.
    let agent_line = line(agent);
    assert_eq!(
        [&agent_line[..4], &agent_line[5..6], &agent_line[7..8]].concat(),
        [&*agent.to_string(), &*agent_pid.to_string(), "attached", "-", "kill", "agent: blocked"]
    );
    assert_eq!(agent_line[8], agent_info.command);
    assert!(agent_line[8].contains("brindle-fake-acp-agent"), "{agent_line:?}");
    assert_eq!(Some(agent_line[9].clone()), agent_info.cwd);

    let output = succeeded(cli(&dir.path, &["list", "--json"]));
    let sessions: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    let entry = |session: SessionId| sessions.iter().find(|entry| entry["session"] == session).unwrap();
    assert_eq!(entry(agent)["agent"], json!({ "activity": "blocked", "name": "fake" }));
    assert_eq!((&entry(agent)["cols"], &entry(agent)["rows"]), (&json!(0), &json!(0)));
    assert!(entry(shell).get("agent").is_none(), "{sessions:?}");
    // The same entries the core lists to a client.
    let decoded: Vec<SessionInfo> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(decoded, listed);
}

#[test]
fn kill_ends_an_agent_session_mid_turn_without_asking() {
    let (mut dir, mut client) = core("cli-agent-kill");
    let (blocked, blocked_pid) = blocked_agent(&mut client, &mut dir, "kill-one");
    let spawn = idles(&dir, "kill-other");
    let (idle, idle_pid) = agent_session(&mut client, &mut dir, spawn);

    // Mid-turn, with a permission request pending: nothing is asked.
    let output = succeeded(cli(&dir.path, &["kill", &blocked.to_string()]));
    assert!(output.stdout.is_empty() && output.stderr.is_empty(), "{output:?}");
    exits(&mut client, &[blocked]);
    assert!(gone(blocked_pid));
    assert!(alive(idle_pid));
    assert!(client.list().iter().all(|info| info.session != blocked));

    let (shell, shell_pid) = client.spawn(programs::echoer());
    say(&mut client, shell, "up");
    succeeded(cli(&dir.path, &["kill", "--all"]));
    let codes = exits(&mut client, &[idle, shell]);
    assert_hung_up(shell, codes[1]);
    assert!(gone(idle_pid) && gone(shell_pid));
    assert!(client.list().is_empty());
}

#[test]
fn stop_ends_agent_sessions_with_the_rest() {
    let (mut dir, mut client) = core("cli-agent-stop");
    let core_pid = client.core_pid();
    let (blocked, blocked_pid) = blocked_agent(&mut client, &mut dir, "stop-blocked");
    let spawn = client::SpawnAgent { policy: client::Policy::Detach, ..idles(&dir, "stop-detached") };
    let (kept, kept_pid) = agent_session(&mut client, &mut dir, spawn);
    let (shell, shell_pid) = client.spawn(programs::echoer());
    say(&mut client, shell, "up");
    let holder = parent(blocked_pid).unwrap();

    let output = succeeded(cli(&dir.path, &["stop"]));
    assert!(output.stdout.is_empty() && output.stderr.is_empty(), "{output:?}");
    let codes = exits(&mut client, &[blocked, kept, shell]);
    assert_hung_up(shell, codes[2]);
    assert!(client.closes());
    assert!(gone(blocked_pid) && gone(kept_pid) && gone(shell_pid));
    assert!(gone(core_pid) && gone(holder));
    assert!(dir.holder_endpoints().is_empty());
}
