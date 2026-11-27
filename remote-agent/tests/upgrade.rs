//! Upgrades (task 6.1): `STEP_DOWN`, adoption by the next core, draining
//! holders of another build, the `check` command, a next core that won't
//! start, and (on Linux) the upgrade a relay offers.
//!
//! Every test but the relay's runs on both platforms. On Windows the console host renders
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
use std::time::Duration;

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::{self, ReplayStart};
use brindle_remote_protocol::holder as h1;
use brindle_remote_protocol::serde_json::json;
use brindle_transcript::{
    AgentState, DiffLine, Entry, Exit, FileChange, FileOp, HoldReason, Slot, StopReason, ToolItem,
    TurnState,
};

use support::fake_agent::{
    LISTS, LOAD_ONLY, NEITHER, TWO_LOGINS, ask, initialized, methods, offering_logins, prompts, recorded,
    responses, said, started, stop, update, user_said,
};
use support::programs::{self, ENTER, say};
#[cfg(unix)]
use support::relay::Relay;
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
    assert_eq!(in_use, [3]);

    let passed = cli(&dir.path, &["check", "--holder-protocols", "1,2,3"]);
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
    assert!(
        eventually(TIMEOUT, || {
            server_processes(&dir.path, "serve")
                .iter()
                .any(|&pid| pid != old_core)
                && core_listens(&dir.path)
        }),
        "no core: {}",
        dir.logs()
    );
    // The holder's own core took over.
    assert!(
        eventually(TIMEOUT, || dir.logs().contains("the core it started is listening")),
        "the holder didn't start the core: {}",
        dir.logs()
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

/// Sets `path`'s file time to an hour before the built binary's, so that
/// for the same version the built binary counts as the newer build.
#[cfg(unix)]
fn older_than_the_built_binary(path: &Path) {
    let built = fs::metadata(BIN).unwrap().modified().unwrap();
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(built - Duration::from_secs(3600)).unwrap();
}

#[cfg(unix)]
#[test]
fn a_newer_relay_upgrades_the_core_and_keeps_its_sessions() {
    let mut dir = TempDir::new("relay-upgrade");
    let (old_bin, new_build) = other_build_copy(&dir);
    older_than_the_built_binary(&old_bin);
    start(
        &old_bin,
        "serve",
        &dir.path,
        &[("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version")],
    )
    .unwrap();
    let mut a = TestClient::hello(&dir.path, "a");
    let old_core = a.core_pid();
    dir.track(old_core);
    assert_ne!(a.welcome.as_ref().unwrap().core_build, new_build);
    let (echo, echo_pid) = a.spawn(programs::echoer());
    let mut detach = programs::sleeps();
    detach.policy = client::Policy::Detach;
    let (kept, kept_pid) = a.spawn(detach);
    let holder = parent(echo_pid).unwrap();
    dir.track(holder);

    // A relay of the built binary: the same version, another build, a later
    // file.
    let mut relay = Relay::start(Path::new(BIN), &dir.path, &[]);
    let host = relay.host();
    dir.track(host.core_pid);
    let mut through = relay.client("r");
    assert_ne!(host.core_pid, old_core);
    assert!(gone(old_core), "the old core still runs");
    assert!(a.closes());
    assert_eq!(host.build, new_build);
    assert_eq!(through.welcome.as_ref().unwrap().core_build, new_build);
    assert_eq!(through.core_pid(), host.core_pid);
    runs_the_built_binary(host.core_pid);

    // The sessions kept running under their holder, and come back to their
    // client.
    let mut a = TestClient::hello(&dir.path, "a");
    let back = replays(&mut a, 2);
    assert_eq!(back[&echo].0.pid, echo_pid);
    assert_eq!(back[&kept].0.pid, kept_pid);
    for pid in [echo_pid, kept_pid] {
        assert_eq!(parent(pid), Some(holder));
    }
    say(&mut a, echo, "upgraded");

    // And the relay's client works on the new core.
    let (via, via_pid) = through.spawn(programs::echoer());
    dir.track(parent(via_pid).unwrap());
    say(&mut through, via, "through the relay");

    let logs = dir.logs();
    for entry in ["upgrading the core", "upgrade: client", "upgrade: took holder"] {
        assert!(logs.contains(entry), "no {entry:?} in
{logs}");
    }
}

#[cfg(unix)]
#[test]
fn an_older_or_equal_relay_leaves_the_core_alone() {
    let mut dir = TempDir::new("relay-keep");
    let (old_bin, built) = other_build_copy(&dir);
    older_than_the_built_binary(&old_bin);
    start_core(&dir.path, &[]).unwrap();
    let mut a = TestClient::hello(&dir.path, "a");
    let core = a.core_pid();
    dir.track(core);
    assert_eq!(a.welcome.as_ref().unwrap().core_build, built);
    let (echo, echo_pid) = a.spawn(programs::echoer());
    dir.track(parent(echo_pid).unwrap());

    for (bin, equal) in [(old_bin.as_path(), false), (Path::new(BIN), true)] {
        let mut relay = Relay::start(bin, &dir.path, &[]);
        let host = relay.host();
        let mut through = relay.client(if equal { "equal" } else { "older" });
        assert_eq!(host.core_pid, core, "equal build: {equal}");
        assert_eq!(host.build == built, equal);
        assert_eq!(through.core_pid(), core);
        assert!(
            through
                .list()
                .iter()
                .any(|session| session.session == echo && session.pid == echo_pid)
        );
    }
    assert!(alive(core));
    say(&mut a, echo, "untouched");
    let logs = dir.logs();
    assert!(
        !logs.contains("upgrading the core") && !logs.contains("upgrade: client"),
        "{logs}"
    );
}

// Agent sessions (OpenSpec change `agents-sessions`, task 7.5).

fn agent_ready(log: &Transcript) -> bool {
    matches!(log.agent(), AgentState::Ready { .. })
}

/// The log's user and agent entries, as `user: …` and `agent: …`.
fn says(log: &Transcript) -> Vec<String> {
    log.entries()
        .filter_map(|(_, entry)| match entry {
            Entry::User { text } => Some(format!("user: {text}")),
            Entry::Agent { text } => Some(format!("agent: {text}")),
            Entry::Thought { .. } | Entry::Tool { .. } | Entry::Permission { .. } | Entry::Notice { .. } => None,
        })
        .collect()
}

/// A core in a fresh directory and a client `a` of it.
fn agent_core(tag: &str) -> (TempDir, TestClient) {
    let mut dir = TempDir::new(tag);
    start_core(&dir.path, &[]).unwrap();
    let client = TestClient::hello(&dir.path, "a");
    dir.track(client.core_pid());
    (dir, client)
}

/// The client's core steps down and the next one starts; gives client `a`
/// of it, back with its session's replay.
fn upgraded(dir: &mut TempDir, client: &mut TestClient, session: SessionId, pid: u32) -> TestClient {
    step_down(client);
    start_core(&dir.path, &[]).unwrap();
    let mut client = TestClient::hello(&dir.path, "a");
    dir.track(client.core_pid());
    let (start, _) = client.reconnect_replay();
    assert_eq!((start.session, start.pid), (session, pid));
    client
}

/// OpenSpec change `agents-session-list`, task 3.2: a choosing session
/// comes through an upgrade still choosing, its agent still listing, and
/// the next core carries out a choice.
#[test]
fn a_choosing_session_survives_an_upgrade_and_then_chooses() {
    let (mut dir, mut client) = agent_core("agent-upgrade-choosing");
    let cwd = dir.path.display().to_string();
    let (mut script, record) = initialized(&dir.path, "choosing", LISTS);
    // `s1`, the conversation `said` speaks in.
    let answer = json!({"sessions": [{"sessionId": "s1", "cwd": cwd, "title": "Earlier"}]});
    script
        .step("expect session/list")
        .step(format!("respond {answer}"))
        .step("expect session/load")
        .step(said("loaded"))
        .step("respond {}");
    let spawn = client::SpawnAgent {
        cwd: Some(cwd.clone()),
        start: Some(client::SpawnStart::Choose),
        policy: client::Policy::Detach,
        ..client_agent(&script.write())
    };
    let (id, pid) = client.spawn_agent(spawn);
    dir.track(pid);
    let choosing = |log: &Transcript| matches!(log.agent(), AgentState::Choosing { .. });
    let before = client.transcript_until(id, "is choosing", choosing);

    let mut client = upgraded(&mut dir, &mut client, id, pid);
    assert_eq!(client.transcript(id), before);
    let listed = client.list_conversations(id, None).unwrap();
    assert_eq!(listed.conversations.len(), 1);
    assert_eq!(listed.conversations[0].title.as_deref(), Some("Earlier"));
    client.choose(id, Some("s1")).unwrap();
    let log = client.transcript_until(id, "is ready", agent_ready);
    assert_eq!(says(&log), ["agent: loaded"]);
    assert_eq!(methods(&record), ["initialize", "session/list", "session/load"]);
}

/// OpenSpec change `agents-commands`, task 3.2: the agent's commands and
/// options, those made from its modes included, come through an upgrade,
/// and the next core sets them by their keys.
#[test]
fn an_upgrade_keeps_the_commands_and_options_and_sets_them_after() {
    let (mut dir, mut client) = agent_core("agent-upgrade-options");
    let (mut script, record) = initialized(&dir.path, "options", NEITHER);
    let answer = json!({"sessionId": "s1",
        "modes": {"currentModeId": "default", "availableModes": [
            {"id": "default", "name": "Default"}, {"id": "plan", "name": "Plan"}]},
        "configOptions": [{"id": "fast", "name": "Fast", "type": "boolean", "currentValue": false}]});
    script
        .step("expect session/new")
        .step(format!("respond {answer}"))
        .step(update(json!({"sessionUpdate": "available_commands_update",
            "availableCommands": [{"name": "compact", "description": "Compacts", "input": null}]})))
        .step("expect session/set_mode")
        .step("respond {}");
    let spawn = client::SpawnAgent { policy: client::Policy::Detach, ..client_agent(&script.write()) };
    let (id, pid) = client.spawn_agent(spawn);
    dir.track(pid);
    let before = client.transcript_until(id, "offers its options and commands", |log| {
        agent_ready(log) && log.options().len() == 2 && log.commands().len() == 1
    });

    let mut client = upgraded(&mut dir, &mut client, id, pid);
    assert_eq!(client.transcript(id), before);
    client
        .set_option(id, "mode", client::OptionValue::Choice { value: "plan".into() })
        .unwrap();
    let log = client.transcript_until(id, "is in plan mode", |log| {
        log.options().iter().any(|option| {
            option.id == "mode"
                && matches!(&option.kind, brindle_transcript::OptionControl::Select { current, .. } if current == "plan")
        })
    });
    assert_eq!(log.commands(), before.commands());
    assert_eq!(methods(&record), ["initialize", "session/new", "session/set_mode"]);
}

#[test]
fn a_held_prompt_survives_an_upgrade_and_is_sent_again() {
    let (mut dir, mut client) = agent_core("agent-upgrade-held");
    let (mut script, record) = started(&dir.path, "held");
    script.step("expect session/prompt");
    let gate = script.gate();
    script
        .step(stop("max_tokens"))
        .step("expect session/prompt")
        .step(said("sent again"))
        .step(stop("end_turn"));
    let (id, pid) = client.spawn_agent(client_agent(&script.write()));
    dir.track(pid);
    client.transcript_until(id, "is ready", agent_ready);
    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    gate.open();
    let hold = Slot::Held {
        text: "two".into(),
        reason: HoldReason::Stopped { stop: StopReason::MaxTokens },
    };
    let before = client.transcript_until(id, "holds the prompt", |log| *log.slot() == hold);

    let mut client = upgraded(&mut dir, &mut client, id, pid);
    assert_eq!(client.transcript(id), before);
    client.resend(id).unwrap();
    let log = client.transcript_until(id, "has its turn's end", |log| {
        says(log).contains(&"agent: sent again".to_owned()) && matches!(log.turn(), TurnState::Ended { stop: StopReason::EndTurn })
    });
    assert_eq!(*log.slot(), Slot::Empty);
    assert_eq!(prompts(&record), ["one", "two"]);
}

/// A tool call's file change and its open output (OpenSpec change
/// `agents-tool-content`, task 4.2) come through an upgrade, and the
/// output carries on in the next core.
#[test]
fn an_upgrade_keeps_a_tool_calls_file_change_and_its_open_output() {
    let (mut dir, mut client) = agent_core("agent-upgrade-tool");
    let (mut script, _) = started(&dir.path, "tool");
    script
        .step("expect session/prompt")
        .step(update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Edit",
            "kind": "edit", "status": "completed",
            "content": [{"type": "diff", "path": "/w/a.txt", "oldText": "one\n", "newText": "two\n"}]})))
        .step(update(json!({"sessionUpdate": "tool_call", "toolCallId": "c2", "title": "Run",
            "kind": "execute", "status": "in_progress",
            "_meta": {"terminal_output_delta": {"terminal_id": "t1", "data": "before \u{1b}[3"}}})));
    let gate = script.gate();
    script
        .step(update(json!({"sessionUpdate": "tool_call_update", "toolCallId": "c2", "status": "completed",
            "_meta": {"terminal_output_delta": {"terminal_id": "t1", "data": "1mafter\n"},
                "terminal_exit": {"terminal_id": "t1", "exit_code": 0, "signal": null}}})))
        .step(stop("end_turn"));
    let (id, pid) = client.spawn_agent(client_agent(&script.write()));
    dir.track(pid);
    client.transcript_until(id, "is ready", agent_ready);
    client.prompt(id, "edit").unwrap();
    let has_both = |log: &Transcript| log.entries().filter(|(_, entry)| matches!(entry, Entry::Tool { .. })).count() == 2;
    let before = client.transcript_until(id, "has both tool calls", has_both);

    let mut client = upgraded(&mut dir, &mut client, id, pid);
    assert_eq!(client.transcript(id), before);
    gate.open();
    let log = client.transcript_until(id, "has its turn's end", |log| {
        matches!(log.turn(), TurnState::Ended { stop: StopReason::EndTurn })
    });
    let contents: Vec<Vec<ToolItem>> = log
        .entries()
        .filter_map(|(_, entry)| match entry {
            Entry::Tool { content, .. } => Some(content.clone()),
            Entry::User { .. } | Entry::Agent { .. } | Entry::Thought { .. } | Entry::Permission { .. } | Entry::Notice { .. } => None,
        })
        .collect();
    let [edit, run] = &contents[..] else {
        panic!("{contents:?}");
    };
    assert!(
        matches!(&edit[..], [ToolItem::Diff(FileChange { op: FileOp::Modify, path, hunks, .. })]
            if path == "/w/a.txt" && hunks[0].lines == [DiffLine::Removed("one".into()), DiffLine::Added("two".into())]),
        "{edit:?}"
    );
    assert!(
        matches!(&run[..], [ToolItem::Output(output)]
            if output.text == "before after\n" && output.exit == Some(Exit::Code { code: 0 })),
        "{run:?}"
    );
}

#[test]
fn a_pending_permission_survives_an_upgrade_and_is_answered_with_the_agents_id() {
    let (mut dir, mut client) = agent_core("agent-upgrade-permission");
    let (mut script, record) = started(&dir.path, "permission");
    script
        .step("expect session/prompt")
        .step(ask("the-agents-own-id"))
        .step(r#"expect-response "the-agents-own-id""#)
        .step(said("allowed"))
        .step(stop("end_turn"));
    let (id, pid) = client.spawn_agent(client_agent(&script.write()));
    dir.track(pid);
    client.transcript_until(id, "is ready", agent_ready);
    client.prompt(id, "write").unwrap();
    client.transcript_until(id, "waits on the permission", |log| *log.turn() == TurnState::Blocked);

    let mut client = upgraded(&mut dir, &mut client, id, pid);
    let log = client.transcript(id);
    assert_eq!(*log.turn(), TurnState::Blocked);
    let entry = log
        .entries()
        .find_map(|(entry, kept)| matches!(kept, Entry::Permission { outcome: None, .. }).then_some(entry.0))
        .expect("the permission request, still pending");
    client.answer(id, entry, Some("allow-once")).unwrap();
    let answer = until("the agent is answered", TIMEOUT, || responses(&record).pop());
    assert_eq!(answer["id"], json!("the-agents-own-id"));
    assert_eq!(answer["result"]["outcome"], json!({"outcome": "selected", "optionId": "allow-once"}));
    client.transcript_until(id, "has the turn's end", |log| {
        matches!(log.turn(), TurnState::Ended { stop: StopReason::EndTurn })
    });
}

/// An H1 2 core (the test's: an older build after a downgrade) isn't
/// shown the agent session, which runs on and comes back, with what its
/// agent said meanwhile, to the next core that speaks H1 3.
#[test]
fn an_older_core_isnt_shown_an_agent_session_and_a_newer_one_gets_it_back() {
    let (mut dir, mut client) = agent_core("agent-older-core");
    let (mut script, _) = started(&dir.path, "older-core");
    script.step("expect session/prompt").step(said("before"));
    let gate = script.gate();
    script.step(said(" and after")).step(stop("end_turn"));
    let finished = script.reached();
    let spawn = client::SpawnAgent { policy: client::Policy::Detach, ..client_agent(&script.write()) };
    let (id, pid) = client.spawn_agent(spawn);
    dir.track(pid);
    client.transcript_until(id, "is ready", agent_ready);
    client.prompt(id, "go").unwrap();
    client.transcript_until(id, "has the turn's start", |log| says(log).contains(&"agent: before".to_owned()));
    kill(client.core_pid());
    assert!(client.closes());

    let mut older = TestCore::connect_to(&dir);
    let adopted = older.hello_up_to(2, 2);
    assert_eq!(adopted.welcome().protocol, 2);
    assert!(adopted.welcome().sessions.is_empty(), "{:?}", adopted.welcome().sessions);
    gate.open();
    assert!(eventually(TIMEOUT, || finished.exists()), "the agent didn't finish its turn");
    // A round trip: the holder read what the agent wrote, and sent none.
    older.hang_up(99);
    loop {
        match older.recv() {
            h1::ToCore::Error(error) if error.session == Some(99) => break,
            h1::ToCore::Output(data) => assert_ne!(data.session, id, "{data:?}"),
            other => panic!("{other:?}"),
        }
    }

    start_core(&dir.path, &[]).unwrap();
    for message in older.until_closed() {
        if let h1::ToCore::Output(data) = message {
            assert_ne!(data.session, id, "the older core was sent its output");
        }
    }
    let mut client = TestClient::hello(&dir.path, "a");
    dir.track(client.core_pid());
    let (start, _) = client.reconnect_replay();
    assert_eq!((start.session, start.pid), (id, pid));
    let log = client.transcript_until(id, "has the turn's end", |log| {
        matches!(log.turn(), TurnState::Ended { stop: StopReason::EndTurn })
    });
    assert_eq!(says(&log), ["user: go", "agent: before and after"]);
}

/// OpenSpec change `agents-login`, task 5.2: the core steps down while the
/// agent logs in, and the agent answers `authenticate` only once the next
/// core has adopted it; that core carries the login on to `session/new`.
#[test]
fn a_login_running_at_an_upgrade_is_carried_on_by_the_next_core() {
    let (mut dir, mut client) = agent_core("agent-upgrade-logging-in");
    let (mut script, record) = offering_logins(&dir.path, "logging-in", NEITHER, TWO_LOGINS);
    script
        .step("expect session/new")
        .step(r#"respond-error -32000 "log in first""#)
        .step("expect authenticate");
    let gate = script.gate();
    script.step("respond {}").step("expect session/new").step(r#"respond {"sessionId":"s1"}"#);
    let (id, pid) = client.spawn_agent(client_agent(&script.write()));
    dir.track(pid);
    client.transcript_until(id, "needs a login", |log| matches!(log.agent(), AgentState::LoginRequired { .. }));
    client.login(id, "api-key").unwrap();
    let logging_in = AgentState::LoggingIn { method: "api-key".into(), name: "API Key".into() };
    client.transcript_until(id, "is logging in", |log| *log.agent() == logging_in);

    let mut client = upgraded(&mut dir, &mut client, id, pid);
    assert_eq!(*client.transcript(id).agent(), logging_in);
    gate.open();
    client.transcript_until(id, "is ready", agent_ready);
    let states: Vec<AgentState> = client
        .events(id)
        .into_iter()
        .filter_map(|event| if let brindle_transcript::Event::Agent { state } = event { Some(state) } else { None })
        .collect();
    assert_eq!(
        states,
        [logging_in, AgentState::Starting, AgentState::Ready { name: Some("fake".into()), version: Some("1.0".into()) }]
    );
    assert_eq!(methods(&record), ["initialize", "session/new", "authenticate", "session/new"]);
    let authenticate = recorded(&record).into_iter().find(|value| value["method"] == "authenticate").expect("authenticate");
    assert_eq!(authenticate["params"], json!({"methodId": "api-key"}));
}

/// A continuing `SPAWN_AGENT` (OpenSpec change `agents-session-recovery`,
/// task 3.5) whose agent is still replaying its `session/load` when the
/// core steps down: the next core finishes the load, with the replay
/// folded once, and the conversation goes on.
#[test]
fn an_upgrade_mid_load_leaves_the_next_core_to_finish_it() {
    let (mut dir, mut client) = agent_core("agent-upgrade-load");
    let (mut script, record) = initialized(&dir.path, "load", LOAD_ONLY);
    script.step("expect session/load").step(user_said("one")).step(said("first"));
    let gate = script.gate();
    script
        .step(said(" reply"))
        .step("respond {}")
        .step("expect session/prompt")
        .step(said("next"))
        .step(stop("end_turn"));
    let cwd = dir.path.display().to_string();
    let spawn = client::SpawnAgent {
        conversation: Some(client::AgentConversation { id: "s1".into(), cwd }),
        ..client_agent(&script.write())
    };
    let (id, pid) = client.spawn_agent(spawn);
    dir.track(pid);
    client.transcript_until(id, "has the replay's start", |log| says(log).contains(&"agent: first".to_owned()));

    let mut client = upgraded(&mut dir, &mut client, id, pid);
    gate.open();
    let log = client.transcript_until(id, "is ready", agent_ready);
    assert_eq!(says(&log), ["user: one", "agent: first reply"]);
    let notices: Vec<&str> = log
        .entries()
        .filter_map(|(_, entry)| match entry {
            Entry::Notice { text, .. } => Some(text.as_str()),
            Entry::User { .. } | Entry::Agent { .. } | Entry::Thought { .. } | Entry::Tool { .. } | Entry::Permission { .. } => None,
        })
        .collect();
    assert_eq!(notices, ["this continues an earlier conversation; what follows is the agent's replay of it"]);

    client.prompt(id, "two").unwrap();
    let log = client.transcript_until(id, "has the turn's end", |log| {
        matches!(log.turn(), TurnState::Ended { stop: StopReason::EndTurn })
    });
    assert_eq!(says(&log), ["user: one", "agent: first reply", "user: two", "agent: next"]);
    assert_eq!(methods(&record), ["initialize", "session/load", "session/prompt"]);
}

// OpenSpec change `agents-log-paging`, task 3.4: which form an agent
// checkpoint takes, read from what the holder stores.

/// The records of the checkpoint the session's holder stores (the only
/// holder in `dir`), read by a test core that adopts it while no core runs.
fn stored_records(dir: &TempDir, id: SessionId) -> Vec<(char, u32)> {
    let mut core = TestCore::connect_to(dir);
    let adopted = core.hello(77);
    let blob = adopted.checkpoints.get(&id).expect("the session's checkpoint");
    checkpoint_records(blob)
}

/// A script with a short first turn, and a second that says `before`,
/// waits at the gate it gives, then says ` and after`.
fn two_turns(dir: &Path, name: &str) -> (PathBuf, support::fake_agent::Gate) {
    let (mut script, _) = started(dir, name);
    script
        .step("expect session/prompt")
        .step(said("first"))
        .step(stop("end_turn"))
        .step("expect session/prompt")
        .step(said("before"));
    let gate = script.gate();
    script.step(said(" and after")).step(stop("end_turn"));
    (script.write(), gate)
}

/// Runs [`two_turns`]'s first turn and the second up to its gate.
fn to_the_gate(client: &mut TestClient, id: SessionId) {
    client.transcript_until(id, "is ready", agent_ready);
    client.prompt(id, "one").unwrap();
    client.transcript_until(id, "has the first turn", |log| {
        says(log).contains(&"agent: first".to_owned()) && matches!(log.turn(), TurnState::Ended { .. })
    });
    client.prompt(id, "two").unwrap();
    client.transcript_until(id, "has the second turn's start", |log| says(log).contains(&"agent: before".to_owned()));
}

#[test]
fn a_step_down_writes_the_agent_checkpoint_whole_and_the_next_core_carries_the_turn_on() {
    let (mut dir, mut client) = agent_core("agent-upgrade-whole");
    let (script, gate) = two_turns(&dir.path, "whole");
    let (id, pid) = client.spawn_agent(client_agent(&script));
    dir.track(pid);
    to_the_gate(&mut client, id);

    // The turns' checkpoints are small beside the spawn's base, so they
    // were appended to it; the step-down's replaces them with one base.
    step_down(&mut client);
    assert_eq!(stored_records(&dir, id), [('B', 0)]);

    start_core(&dir.path, &[]).unwrap();
    let mut client = TestClient::hello(&dir.path, "a");
    dir.track(client.core_pid());
    let (start, _) = client.reconnect_replay();
    assert_eq!((start.session, start.pid), (id, pid));
    gate.open();
    let log = client.transcript_until(id, "has the second turn's end", |log| {
        says(log).len() == 4 && matches!(log.turn(), TurnState::Ended { stop: StopReason::EndTurn })
    });
    assert_eq!(says(&log), ["user: one", "agent: first", "user: two", "agent: before and after"]);
}

#[test]
fn a_draining_holder_of_another_build_is_sent_only_whole_agent_checkpoints() {
    let mut dir = TempDir::new("agent-drain");
    let (old_bin, new_build) = other_build_copy(&dir);
    start(&old_bin, "serve", &dir.path, &[("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version")]).unwrap();
    let mut client = TestClient::hello(&dir.path, "a");
    dir.track(client.core_pid());
    let (script, gate) = two_turns(&dir.path, "drain");
    let (id, pid) = client.spawn_agent(client_agent(&script));
    dir.track(pid);
    dir.track(parent(pid).unwrap());
    client.transcript_until(id, "is ready", agent_ready);

    step_down(&mut client);
    start_core(&dir.path, &[]).unwrap();
    let mut client = TestClient::hello(&dir.path, "a");
    dir.track(client.core_pid());
    assert_eq!(client.welcome.as_ref().unwrap().core_build, new_build);
    client.reconnect_replay();
    // Each of the turns' checkpoints (the prompts, the first turn's end)
    // is small beside the base, so a holder of the core's own build would
    // have segments appended; this one is draining.
    to_the_gate(&mut client, id);
    kill(client.core_pid());
    assert!(client.closes());
    assert_eq!(stored_records(&dir, id), [('B', 0)]);
    drop(gate);
}
