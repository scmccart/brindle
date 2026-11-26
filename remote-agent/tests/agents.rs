//! Agent sessions (OpenSpec change `agents-sessions`), on Linux and
//! Windows: the fake ACP agent itself (task 6.2), `brindle-fake-acp-agent`
//! run on short scripts with its stdin, stdout and stderr piped to the
//! test; and agent sessions run through a real core and holder (tasks
//! 7.1–7.3), driven by the test client's agent helpers.

#![cfg(any(target_os = "linux", windows))]

mod support;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::{self, AgentActivity, AgentInfo, ToClient};
use brindle_remote_protocol::serde_json::{self, Value, json};
use brindle_transcript::{
    AgentState, Entry, EntryId, HoldReason, LoginKind, LoginMethod, PermissionOutcome, Slot,
    StopReason, ToolKind, ToolStatus, TurnState,
};

use support::fake_agent::{
    AgentScript, Gate, LOAD_ONLY, NEITHER, RESUME_AND_LOAD, RESUME_ONLY, TERMINAL_LOGIN, TWO_LOGINS,
    ask, continuable, initialized, methods, offering_logins, prompts, recorded, responses, said,
    started, stop, title, update, user_said,
};
use support::*;

/// A fake agent run on a script, its output read line by line.
struct Agent {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<String>,
    stderr: Receiver<String>,
}

/// The lines `reader` gives, sent as they come, without their line ends.
fn lines(reader: impl Read + Send + 'static) -> Receiver<String> {
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            let line = line.strip_suffix('\r').map(str::to_owned).unwrap_or(line);
            if send.send(line).is_err() {
                break;
            }
        }
    });
    receive
}

impl Agent {
    fn start(script: &Path) -> Self {
        Self::start_with(script, &[], &[])
    }

    /// Started with `args` after the script and `env` added to its
    /// environment.
    fn start_with(script: &Path, args: &[&str], env: &[(&str, &str)]) -> Self {
        let mut child = Command::new(FAKE_AGENT)
            .arg(script)
            .args(args)
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("starting {FAKE_AGENT}: {error}"));
        let stdin = child.stdin.take();
        let stdout = lines(child.stdout.take().expect("piped stdout"));
        let stderr = lines(child.stderr.take().expect("piped stderr"));
        Self {
            child,
            stdin,
            stdout,
            stderr,
        }
    }

    /// Writes `line` and its line end to the agent's stdin.
    fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin still open");
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .and_then(|()| stdin.flush())
            .expect("writing to the fake agent");
    }

    fn close_stdin(&mut self) {
        self.stdin = None;
    }

    /// The next line on stdout.
    fn out(&self) -> String {
        next(&self.stdout, "stdout")
    }

    /// The next line on stderr.
    fn err(&self) -> String {
        next(&self.stderr, "stderr")
    }

    /// Its exit status, once it has exited.
    fn status(&mut self) -> Option<i32> {
        let child = &mut self.child;
        until("the fake agent exits", TIMEOUT, || child.try_wait().ok().flatten()).code()
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn next(lines: &Receiver<String>, name: &str) -> String {
    lines
        .recv_timeout(limit(TIMEOUT))
        .unwrap_or_else(|error| panic!("no line on the fake agent's {name}: {error}"))
}

fn parsed(line: &str) -> Value {
    serde_json::from_str(line).unwrap_or_else(|error| panic!("{line:?} isn't JSON: {error}"))
}

#[test]
fn fake_agent_plays_its_script_and_records_what_it_matched() {
    let dir = TempDir::new("fake-agent");
    let mut script = AgentScript::new(&dir.path, "play");
    let record = script.record();
    script
        .step("expect initialize")
        .step(r#"respond {"protocolVersion":1,"agentInfo":{"name":"fake","version":"1.0"}}"#)
        .step("stderr fake: initialized");
    let gate = script.gate();
    let update = r#"{"jsonrpc":"2.0", "method":"session/update","params":{"sessionId":"s1"}}"#;
    script
        .step(format!("send {update}"))
        .step("send-raw not json {")
        .step("expect session/new")
        .step(r#"respond-error -32022 "Unsupported protocol version" {"requested":1,"supported":[2]}"#)
        .step("exit 4");
    let mut agent = Agent::start(&script.write());

    let initialize = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":1}}"#;
    agent.send(initialize);
    assert_eq!(
        parsed(&agent.out()),
        json!({"jsonrpc": "2.0", "id": 0, "result": {"protocolVersion": 1, "agentInfo": {"name": "fake", "version": "1.0"}}})
    );
    assert_eq!(agent.err(), "fake: initialized");
    // The stderr step is written, so the agent is at the gate: it has sent
    // nothing more, and recorded only what it matched.
    assert_eq!(agent.stdout.try_recv(), Err(TryRecvError::Empty));
    assert_eq!(recorded(&record), vec![parsed(initialize)]);

    // Lines before the expected method are skipped, and not recorded; the
    // agent reads them only once the gate opens.
    agent.send("not json either");
    agent.send(r#"{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"s1"}}"#);
    let new = r#"{"jsonrpc":"2.0","id":"n1","method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#;
    agent.send(new);
    gate.open();

    assert_eq!(agent.out(), update, "send writes its JSON as written");
    assert_eq!(agent.out(), "not json {");
    assert_eq!(
        parsed(&agent.out()),
        json!({"jsonrpc": "2.0", "id": "n1", "error": {
            "code": -32022,
            "message": "Unsupported protocol version",
            "data": {"requested": 1, "supported": [2]},
        }})
    );
    assert_eq!(agent.status(), Some(4));
    assert_eq!(recorded(&record), vec![parsed(initialize), parsed(new)]);
    // Recorded as the lines came in.
    let text = std::fs::read_to_string(&record).expect("the record");
    assert_eq!(text, format!("{initialize}\n{new}\n"));
}

#[test]
fn fake_agent_idles_until_stdin_closes_once_its_script_runs_out() {
    let dir = TempDir::new("fake-agent-idle");
    let mut script = AgentScript::new(&dir.path, "idle");
    script.step("expect initialize").step(r#"respond {"protocolVersion":1}"#);
    let mut agent = Agent::start(&script.write());

    agent.send(r#"{"jsonrpc":"2.0","id":7,"method":"initialize","params":{}}"#);
    assert_eq!(
        parsed(&agent.out()),
        json!({"jsonrpc": "2.0", "id": 7, "result": {"protocolVersion": 1}})
    );
    // Still running, reading what comes, until its stdin closes.
    agent.send(r#"{"jsonrpc":"2.0","id":8,"method":"session/new","params":{}}"#);
    agent.close_stdin();
    assert_eq!(agent.status(), Some(0));
    assert_eq!(agent.stdout.recv_timeout(limit(TIMEOUT)), Err(mpsc::RecvTimeoutError::Disconnected));
}

#[test]
fn fake_agent_fails_when_stdin_closes_before_the_expected_method() {
    let dir = TempDir::new("fake-agent-closed");
    let mut script = AgentScript::new(&dir.path, "closed");
    script.step("expect session/prompt").step("respond {}");
    let mut agent = Agent::start(&script.write());

    agent.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    agent.close_stdin();
    assert_eq!(agent.status(), Some(3));
    assert_eq!(agent.err(), "brindle-fake-acp-agent: stdin closed before session/prompt came");
}

#[test]
fn fake_agent_refuses_a_bad_script_naming_its_line() {
    let dir = TempDir::new("fake-agent-bad");
    let mut script = AgentScript::new(&dir.path, "bad");
    script.step("expect initialize").step("sleep 5");
    let path = script.write();
    let mut agent = Agent::start(&path);

    assert_eq!(agent.status(), Some(2));
    assert_eq!(
        agent.err(),
        format!("brindle-fake-acp-agent: {}: line 2: \"sleep\" isn't a step", path.display())
    );
}

/// OpenSpec change `agents-login`, tasks 4.1 and 4.2: the arguments after
/// the script choose its `args` section, `record-env` writes the agent's
/// environment, and `respond-to` answers a request by its id.
#[test]
fn fake_agent_plays_the_section_its_arguments_name() {
    let dir = TempDir::new("fake-agent-args");
    let env_file = dir.path.join("env.txt");
    let mut script = AgentScript::new(&dir.path, "args");
    script
        .step("exit 5")
        .step("args login --device")
        .step(format!("record-env {}", env_file.display()))
        .step(r#"respond-to 3 {"loggedIn":true}"#)
        .step(r#"respond-error-to "a" -32000 "not yet""#)
        .step("exit 0");
    let path = script.write();

    let mut login = Agent::start_with(&path, &["login", "--device"], &[("LOGIN_MODE", "device")]);
    assert_eq!(parsed(&login.out()), json!({"jsonrpc": "2.0", "id": 3, "result": {"loggedIn": true}}));
    assert_eq!(
        parsed(&login.out()),
        json!({"jsonrpc": "2.0", "id": "a", "error": {"code": -32000, "message": "not yet"}})
    );
    assert_eq!(login.status(), Some(0));
    let text = std::fs::read_to_string(&env_file).expect("the recorded environment");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.contains(&"LOGIN_MODE=device"), "{text}");
    // Sorted by name, not by whole line: `=` sorts after `(`, so
    // `ProgramFiles=…` comes before `ProgramFiles(x86)=…` on Windows.
    let names: Vec<&str> = lines
        .iter()
        .map(|line| line.split_once('=').map_or(*line, |(name, _)| name))
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);

    // With no arguments, the steps before the first section.
    assert_eq!(Agent::start(&path).status(), Some(5));
    // Arguments no section is for are a usage error.
    let mut other = Agent::start_with(&path, &["logout"], &[]);
    assert_eq!(other.status(), Some(2));
    assert_eq!(
        other.err(),
        format!("brindle-fake-acp-agent: {} has no args section for \"logout\"", path.display())
    );
}

// 7.1–7.3: agent sessions through the core.

/// A fake agent that starts as `fake` 1.0, takes session `s1`, and answers
/// one prompt per entry of `answers` with that text and `end_turn`; then it
/// idles until its stdin closes. Gives the script and its record.
fn agent_script(dir: &TempDir, name: &str, answers: &[&str]) -> (PathBuf, PathBuf) {
    let (mut script, record) = started(&dir.path, name);
    for answer in answers {
        script.step("expect session/prompt").step(said(answer)).step(stop("end_turn"));
    }
    (script.write(), record)
}

/// A core in a fresh directory, with a connected protocol 4 client.
fn agent_core(tag: &str, env: &[(&str, &str)]) -> (TempDir, TestClient) {
    let mut dir = TempDir::new(tag);
    start_core(&dir.path, env).unwrap();
    let client = TestClient::hello(&dir.path, &format!("token-{tag}"));
    dir.track(client.core_pid());
    (dir, client)
}

fn ready(log: &Transcript) -> bool {
    matches!(log.agent(), AgentState::Ready { .. })
}

/// Whether `turns` turns ended with `end_turn` (counted by the agent's
/// answers, each its own entry).
fn ended(turns: usize) -> impl FnMut(&Transcript) -> bool {
    move |log| {
        matches!(log.turn(), TurnState::Ended { stop: StopReason::EndTurn })
            && says(log).iter().filter(|line| line.starts_with("agent: ")).count() == turns
    }
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

/// The listed session `id`.
fn listed(client: &mut TestClient, id: SessionId) -> Option<client::SessionInfo> {
    client.list().into_iter().find(|info| info.session == id)
}

/// The next `ERROR` about `session` that answers no request (other
/// messages are skipped): its text.
fn unrequested_error(client: &mut TestClient, session: SessionId) -> String {
    loop {
        match client.recv() {
            ToClient::Error(error) if error.req.is_none() && error.session == Some(session) => {
                return error.message;
            }
            _ => {}
        }
    }
}

/// Every message already sent to `client`, read until it is quiet for a
/// moment. Only for checking that something didn't come after a request
/// whose reply did.
fn already_sent(client: &mut TestClient) -> Vec<ToClient> {
    let mut messages = Vec::new();
    while let Recv::Message(message) = client.next(Duration::ZERO) {
        messages.push(message);
    }
    messages
}

#[test]
fn an_agent_session_runs_a_turn_through_the_core() {
    let (mut dir, mut client) = agent_core("agent-turn", &[]);
    let (script, record) = agent_script(&dir, "turn", &["hello back"]);
    let (id, pid) = client.spawn_agent(client_agent(&script));
    dir.track(pid);

    let log = client.transcript_until(id, "is ready", ready);
    assert_eq!(
        *log.agent(),
        AgentState::Ready { name: Some("fake".into()), version: Some("1.0".into()) }
    );
    client.prompt(id, "hi").unwrap();
    let log = client.transcript_until(id, "has the turn's end", ended(1));
    assert_eq!(says(&log), ["user: hi", "agent: hello back"]);

    // Listed at 0×0, with its activity and agent's name, in the absolute
    // directory the agent was given.
    let info = listed(&mut client, id).expect("the agent session is listed");
    assert_eq!((info.cols, info.rows, info.pid), (0, 0, pid));
    assert_eq!(info.agent, Some(AgentInfo { activity: AgentActivity::Done, name: Some("fake".into()) }));
    assert!(info.command.contains("brindle-fake-acp-agent"), "{}", info.command);
    let cwd = info.cwd.expect("a listed directory");
    assert!(Path::new(&cwd).is_absolute(), "{cwd}");
    let sent = recorded(&record);
    let method = |value: &Value| value["method"].as_str().map(str::to_owned);
    assert_eq!(
        sent.iter().filter_map(method).collect::<Vec<_>>(),
        ["initialize", "session/new", "session/prompt"]
    );
    assert_eq!(sent[1]["params"]["cwd"], json!(cwd));
    assert_eq!(sent[2]["params"]["prompt"][0]["text"], json!("hi"));

    // It has no size, and takes no terminal input.
    client.send(client::ToCore::Resize(client::Resize { session: id, cols: 80, rows: 24, cell_w: 0, cell_h: 0 }));
    let refusal = unrequested_error(&mut client, id);
    assert_eq!(refusal, format!("session {id} is an agent session, which takes no size"));
    client.input(id, b"x");
    let refusal = unrequested_error(&mut client, id);
    assert_eq!(refusal, format!("session {id} is an agent session, which takes no input"));
    // A cancel with no turn running is taken, and answered with nothing.
    client.cancel(id);
    assert_eq!(listed(&mut client, id).and_then(|info| info.agent).map(|agent| agent.activity), Some(AgentActivity::Done));
}

#[test]
fn an_agent_session_needs_a_command_and_its_commands_its_client() {
    let (_dir, mut client) = agent_core("agent-refusals", &[]);
    let refusal = client.spawn_agent_reply(client::SpawnAgent { command: " ".into(), ..client_agent(Path::new("x")) });
    match refusal {
        ToClient::Error(error) => assert_eq!(error.message, "an agent session needs a command"),
        other => panic!("expected ERROR, got {other:?}"),
    }
    let unknown = client.prompt(9, "hi").unwrap_err();
    assert_eq!(unknown.message, "no session 9");
    // A terminal session takes no agent commands.
    let (terminal, _) = client.spawn(programs::sleeps());
    let refusal = client.prompt(terminal, "hi").unwrap_err();
    assert_eq!(refusal.message, format!("session {terminal} isn't an agent session"));
}

#[test]
fn an_attach_replays_the_agent_sessions_log_and_another_client_takes_it_over() {
    let (mut dir, mut first) = agent_core("agent-replay", &[]);
    let (script, _) = agent_script(&dir, "replay", &["one", "two"]);
    let (id, pid) = first.spawn_agent(client_agent(&script));
    dir.track(pid);
    first.transcript_until(id, "is ready", ready);
    first.prompt(id, "first").unwrap();
    let live = first.transcript_until(id, "has the first turn's end", ended(1));
    first.detach(id);

    // The replay folds into the very log the live events built.
    let mut second = TestClient::hello(&dir.path, "someone-else");
    let start = second.attach_agent(id).unwrap();
    assert_eq!((start.session, start.pid), (id, pid));
    assert_eq!(second.transcript(id), live);

    // Commands are its client's alone.
    let refusal = first.prompt(id, "mine").unwrap_err();
    assert_eq!(refusal.message, format!("session {id} isn't attached to this client"));
    second.prompt(id, "second").unwrap();
    let log = second.transcript_until(id, "has the second turn's end", ended(2));
    assert_eq!(says(&log), ["user: first", "agent: one", "user: second", "agent: two"]);
}

#[test]
fn clients_below_protocol_4_never_see_agent_sessions() {
    let (mut dir, mut client) = agent_core("agent-hidden", &[]);
    let (script, _) = agent_script(&dir, "hidden", &[]);
    let token = "shared";
    let mut owner = TestClient::hello(&dir.path, token);
    let spawn = client::SpawnAgent { policy: client::Policy::Detach, ..client_agent(&script) };
    let (agent, pid) = owner.spawn_agent(spawn);
    dir.track(pid);
    let live = owner.transcript_until(agent, "is ready", ready);
    let (terminal, _) = owner.spawn(client::Spawn { policy: client::Policy::Detach, ..programs::sleeps() });
    drop(owner);
    assert!(eventually(TIMEOUT, || listed(&mut client, agent).is_some_and(|info| !info.attached)));

    let three = |hello: &mut client::Hello| hello.max = 3;
    // The owner coming back on protocol 3 gets its terminal back, not its
    // agent session.
    let mut old = TestClient::hello_with(&dir.path, token, three);
    let (start, _) = old.reconnect_replay();
    assert_eq!(start.session, terminal);
    let sessions = old.list();
    let ids: Vec<SessionId> = sessions.iter().map(|info| info.session).collect();
    assert_eq!(ids, [terminal], "only the terminal session: {sessions:?}");
    assert!(sessions.iter().all(|info| info.agent.is_none()), "{sessions:?}");
    let replays: Vec<_> = already_sent(&mut old)
        .into_iter()
        .filter(|message| matches!(message, ToClient::ReplayStart(_)))
        .collect();
    assert!(replays.is_empty(), "{replays:?}");
    let refusal = old.attach(agent, 80, 24).unwrap_err();
    assert_eq!(
        refusal.message,
        format!("session {agent} is an agent session, which needs client protocol 4")
    );
    match old.spawn_agent_reply(client_agent(&script)) {
        ToClient::Error(error) => assert_eq!(error.message, "SPAWN_AGENT needs client protocol 4"),
        other => panic!("expected ERROR, got {other:?}"),
    }
    assert_eq!(old.prompt(agent, "hi").unwrap_err().message, "AGENT_PROMPT needs client protocol 4");

    // On protocol 4 it gets its agent session back, with its log.
    let mut new = TestClient::hello(&dir.path, token);
    let (start, _) = new.reconnect_replay();
    assert_eq!(start.session, agent);
    assert_eq!(new.transcript(agent), live);
    let info = listed(&mut new, agent).expect("listed on protocol 4");
    assert_eq!(info.agent.map(|agent| agent.activity), Some(AgentActivity::Idle));
}

#[test]
fn a_new_core_adopts_an_agent_session_with_its_conversation() {
    let (mut dir, mut client) = agent_core("agent-adopt", &[]);
    let (script, record) = agent_script(&dir, "adopt", &["before", "after"]);
    let token = "token-agent-adopt";
    let spawn = client::SpawnAgent { policy: client::Policy::Detach, ..client_agent(&script) };
    let (id, pid) = client.spawn_agent(spawn);
    dir.track(pid);
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    let before = client.transcript_until(id, "has the first turn's end", ended(1));
    // A round trip: the turn's checkpoint is in the holder's connection.
    listed(&mut client, id);

    kill(client.core_pid());
    assert!(client.closes());
    start_core(&dir.path, &[]).unwrap();
    let mut client = TestClient::hello(&dir.path, token);
    dir.track(client.core_pid());
    let (start, _) = client.reconnect_replay();
    assert_eq!((start.session, start.pid), (id, pid));
    assert_eq!(client.transcript(id), before);

    // The same conversation goes on: no second `initialize`.
    client.prompt(id, "two").unwrap();
    let log = client.transcript_until(id, "has the second turn's end", ended(2));
    assert_eq!(says(&log), ["user: one", "agent: before", "user: two", "agent: after"]);
    let methods: Vec<_> = recorded(&record)
        .iter()
        .filter_map(|value| value["method"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(methods, ["initialize", "session/new", "session/prompt", "session/prompt"]);
}

/// The conversation's title, when the log has one.
fn title_of(log: &Transcript) -> Option<&str> {
    log.conversation().and_then(|conversation| conversation.title.as_deref())
}

/// OpenSpec change `agents-session-title`, task 2.6: the agent names its
/// conversation mid-turn, the core is killed, and a client of the next core
/// gets the title in the replay.
#[test]
fn a_conversations_title_survives_a_core_kill() {
    let token = "token-agent-titled";
    let (mut dir, mut client) = agent_core("agent-titled", &[]);
    let (mut script, _) = started(&dir.path, "titled");
    script
        .step("expect session/prompt")
        .step(title(Some("Hello.txt\nfile creation")))
        .step(said("named"))
        .step(stop("end_turn"));
    let (id, pid) = spawned(&mut client, &mut dir, detached(&script.write()));
    let log = client.transcript_until(id, "is ready", ready);
    assert_eq!(title_of(&log), None, "untitled until the agent names it");
    client.prompt(id, "go").unwrap();
    let before = client.transcript_until(id, "has the titled turn's end", |log| {
        title_of(log).is_some() && ended(1)(log)
    });
    assert_eq!(title_of(&before), Some("Hello.txt file creation"), "made one line");
    // A round trip: the turn's checkpoint is in the holder's connection.
    listed(&mut client, id);

    let mut client = kill_and_restart_core(&mut dir, &mut client, token, || {});
    let (start, _) = client.reconnect_replay();
    assert_eq!((start.session, start.pid), (id, pid));
    let after = client.transcript(id);
    assert_eq!(title_of(&after), Some("Hello.txt file creation"));
    assert_eq!(after, before);
}

#[test]
fn a_panic_in_acp_handling_fails_only_its_session() {
    const SWITCH: &str = "BRINDLE_TEST_ACP_PANIC";
    if skip_without_switch(SWITCH) {
        return;
    }
    let marker = "acp-panic-marker";
    let (mut dir, mut client) = agent_core("agent-panic", &[(SWITCH, marker)]);
    let (script, _) = agent_script(&dir, "panic", &[]);
    let (broken, broken_pid) = client.spawn_agent(client_agent(&script));
    dir.track(broken_pid);
    client.transcript_until(broken, "is ready", ready);

    let refusal = client.prompt(broken, &format!("say {marker}")).unwrap_err();
    assert_eq!(refusal.message, format!("session {broken}: the terminal server's ACP handling failed"));
    let failed = |log: &Transcript| {
        matches!(log.agent(), AgentState::Failed { message } if message.contains(&format!("{SWITCH} matched")))
    };
    client.transcript_until(broken, "says its handling failed", failed);
    let info = listed(&mut client, broken).expect("still listed");
    assert_eq!(info.agent.map(|agent| agent.activity), Some(AgentActivity::Error));
    // Its prompts are refused from then on, and its agent runs on.
    let refusal = client.prompt(broken, "again").unwrap_err();
    assert!(refusal.message.contains("the agent can't take prompts"), "{}", refusal.message);
    assert!(alive(broken_pid), "the agent runs until the session is closed");

    // The core runs on, and so do its other agent sessions.
    let (script, _) = agent_script(&dir, "fine", &["fine"]);
    let (fine, pid) = client.spawn_agent(client_agent(&script));
    dir.track(pid);
    client.transcript_until(fine, "is ready", ready);
    client.prompt(fine, "hi").unwrap();
    client.transcript_until(fine, "has the turn's end", ended(1));
    assert!(dir.logs().contains("ACP handling panicked"), "{}", dir.logs());

    // Closing it ends its agent.
    client.send(client::ToCore::Close(client::SessionRef { session: broken }));
    exit_of(&mut client, broken);
    assert!(gone(broken_pid), "the broken session's agent outlived its close");
}

// 7.4: a test per scenario of the `agent-sessions` spec.

/// Starts an agent session and tracks its agent's pid.
fn spawned(client: &mut TestClient, dir: &mut TempDir, spawn: client::SpawnAgent) -> (SessionId, u32) {
    let (id, pid) = client.spawn_agent(spawn);
    dir.track(pid);
    (id, pid)
}

/// A `SPAWN_AGENT` of `script` whose session outlives its client.
fn detached(script: &Path) -> client::SpawnAgent {
    client::SpawnAgent { policy: client::Policy::Detach, ..client_agent(script) }
}

fn turn_is(state: TurnState) -> impl FnMut(&Transcript) -> bool {
    move |log| *log.turn() == state
}

fn slot_is(slot: Slot) -> impl FnMut(&Transcript) -> bool {
    move |log| *log.slot() == slot
}

fn waiting(text: &str) -> Slot {
    Slot::Waiting { text: text.into() }
}

fn held(text: &str, reason: HoldReason) -> Slot {
    Slot::Held { text: text.into(), reason }
}

fn end(stop: StopReason) -> TurnState {
    TurnState::Ended { stop }
}

/// An event as a short line, for checking what came in which order.
fn shape(event: &Event) -> String {
    match event {
        Event::Added { entry, .. } => match entry {
            Entry::User { text } => format!("user {text}"),
            Entry::Agent { text } => format!("agent {text}"),
            Entry::Thought { text } => format!("thought {text}"),
            Entry::Tool { call, .. } => format!("tool {call}"),
            Entry::Permission { title, .. } => format!("permission {title}"),
            Entry::Notice { text, .. } => format!("notice {text}"),
        },
        Event::Appended { text, .. } => format!("appended {text}"),
        Event::ToolChanged { .. } => "tool changed".into(),
        Event::Answered { outcome, .. } => format!("answered {outcome:?}"),
        Event::Queue { slot } => match slot {
            Slot::Empty => "queue empty".into(),
            Slot::Waiting { text } => format!("queue waiting {text}"),
            Slot::Held { text, reason } => format!("queue held {text} {reason:?}"),
        },
        Event::Turn { state } => format!("turn {state:?}"),
        Event::Agent { state } => format!("state {state:?}"),
        Event::Conversation { conversation } => format!("conversation {}", conversation.id),
        Event::Trimmed { dropped, .. } => format!("trimmed {dropped}"),
        Event::Window { from, earlier } => format!("window {from} {earlier}"),
    }
}

/// Asserts that events starting as `wanted` come in `events` in that
/// order, others between them allowed.
fn assert_in_order(events: &[Event], wanted: &[&str]) {
    let shapes: Vec<String> = events.iter().map(shape).collect();
    let mut rest = shapes.iter();
    for want in wanted {
        assert!(
            rest.any(|shape| shape.starts_with(want)),
            "no {want:?} in order in {shapes:#?}"
        );
    }
}

/// The agent states among `events`.
fn agent_states(events: &[Event]) -> Vec<AgentState> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Agent { state } => Some(state.clone()),
            _ => None,
        })
        .collect()
}

/// The texts of the log's notices.
fn notices(log: &Transcript) -> Vec<String> {
    log.entries()
        .filter_map(|(_, entry)| match entry {
            Entry::Notice { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The log's permission requests: their ids and entries.
fn permissions(log: &Transcript) -> Vec<(u64, Entry)> {
    log.entries()
        .filter(|(_, entry)| matches!(entry, Entry::Permission { .. }))
        .map(|(id, entry)| (id.0, entry.clone()))
        .collect()
}

/// The session's `EXIT`, skipping other messages (each `TRANSCRIPT` is
/// folded as it is read).
fn exit_of(client: &mut TestClient, session: SessionId) -> client::Exit {
    loop {
        if let ToClient::Exit(exit) = client.recv()
            && exit.session == session
        {
            return exit;
        }
    }
}

fn activity_of(client: &mut TestClient, id: SessionId) -> Option<AgentActivity> {
    listed(client, id).and_then(|info| info.agent).map(|agent| agent.activity)
}

/// Waits until the agent has recorded as many prompts as `expected` has,
/// and asserts they are those. (An `OK` to a prompt can come before the
/// agent reads it.)
fn prompted(record: &Path, expected: &[&str]) {
    let got = until("the agent records its prompts", TIMEOUT, || {
        let got = prompts(record);
        (got.len() >= expected.len()).then_some(got)
    });
    assert_eq!(got, expected);
}

/// Waits until a fake agent got to a [`AgentScript::reached`] mark.
fn reaches(mark: &Path) {
    assert!(eventually(TIMEOUT, || mark.exists()), "the agent never got to {}", mark.display());
}

/// Waits until the agent has been sent `count` responses to its own
/// requests.
fn answered(record: &Path, count: usize) -> Vec<Value> {
    until("the agent is answered", TIMEOUT, || {
        let got = responses(record);
        (got.len() >= count).then_some(got)
    })
}

/// What the agent was answered for its permission request `id`.
fn outcome_sent(response: &Value, id: &str) -> Value {
    assert_eq!(response["id"], json!(id), "{response}");
    response["result"]["outcome"].clone()
}

#[test]
fn the_agent_is_initialized_and_given_a_session_in_an_absolute_directory() {
    let (mut dir, mut client) = agent_core("agent-init", &[]);
    let (script, record) = agent_script(&dir, "init", &[]);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    client.transcript_until(id, "is ready", ready);

    assert_eq!(methods(&record), ["initialize", "session/new"]);
    let sent = recorded(&record);
    let init = &sent[0]["params"];
    assert_eq!(init["protocolVersion"], json!(1));
    // No file system and no terminal offered: left out, or all false.
    let offers_nothing = |value: &Value| match value {
        Value::Null | Value::Bool(false) => true,
        Value::Object(fields) => fields.values().all(|field| *field == json!(false)),
        _ => false,
    };
    let capabilities = &init["clientCapabilities"];
    assert!(capabilities.is_object(), "{init}");
    for capability in ["fs", "terminal"] {
        assert!(offers_nothing(&capabilities[capability]), "{capability} offered: {init}");
    }
    assert_eq!(init["clientInfo"]["name"], json!("brindle"));
    assert!(init["clientInfo"]["version"].is_string(), "{init}");
    let new = &sent[1]["params"];
    let cwd = new["cwd"].as_str().expect("a directory");
    assert!(Path::new(cwd).is_absolute(), "{cwd}");
    assert_eq!(new["mcpServers"], json!([]));
}

#[test]
fn a_home_relative_directory_is_given_and_listed_absolute() {
    let (mut dir, mut client) = agent_core("agent-home", &[]);
    let home = dir.path.join("home");
    std::fs::create_dir_all(home.join("src")).unwrap();
    let (script, record) = agent_script(&dir, "home", &[]);
    let mut spawn = client_agent(&script);
    spawn.cwd = Some("~/src".into());
    for name in ["HOME", "USERPROFILE"] {
        spawn.env.insert(name.into(), home.display().to_string());
    }
    let (id, _) = spawned(&mut client, &mut dir, spawn);
    client.transcript_until(id, "is ready", ready);

    let sent = recorded(&record);
    let given = sent[1]["params"]["cwd"].as_str().expect("a directory").to_owned();
    assert_eq!(Path::new(&given), home.join("src"));
    let info = listed(&mut client, id).expect("listed");
    assert_eq!(info.cwd.as_deref(), Some(given.as_str()));
}

#[test]
fn an_agent_that_cant_start_is_refused_naming_it_and_isnt_listed() {
    let (dir, mut client) = agent_core("agent-missing", &[]);
    let missing = dir.path.join("no-such-agent").display().to_string();
    let spawn = client::SpawnAgent { command: missing, ..client_agent(Path::new("x")) };
    match client.spawn_agent_reply(spawn) {
        ToClient::Error(error) => {
            assert!(error.message.contains("no-such-agent"), "{}", error.message);
        }
        other => panic!("expected ERROR, got {other:?}"),
    }
    assert!(client.list().is_empty());
}

/// A start the agent fails (`answer` to `initialize`): its exit doesn't
/// replace the failed state, which says `message`, and its stderr is a
/// notice; the session then ends.
fn a_failed_start_is_the_last_agent_state(tag: &str, answer: &str, message: &str) {
    let (mut dir, mut client) = agent_core(tag, &[]);
    let mut script = AgentScript::new(&dir.path, tag);
    let record = script.record();
    script.step("expect initialize").step(answer).step("stderr fake: speaks version 2 only");
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));

    let exit = exit_of(&mut client, id);
    assert_eq!(exit.lost, None);
    let events = client.events(id);
    let states = agent_states(&events);
    assert_eq!(states.last(), Some(&AgentState::Failed { message: message.into() }), "{states:?}");
    assert!(!states.iter().any(|state| matches!(state, AgentState::Exited { .. })), "{states:?}");
    let log = client.transcript(id);
    assert!(
        notices(&log).iter().any(|notice| notice.contains("fake: speaks version 2 only")),
        "{:?}",
        notices(&log)
    );
    assert_eq!(methods(&record), ["initialize"], "nothing more was sent");
    // Gone, and nothing came after its exit.
    assert!(listed(&mut client, id).is_none());
    assert_eq!(client.events(id), events);
}

#[test]
fn an_agent_refusing_acp_version_1_fails_with_the_versions_it_speaks() {
    a_failed_start_is_the_last_agent_state(
        "agent-refuses-v1",
        r#"respond-error -32022 "Unsupported protocol version" {"requested":1,"supported":[2]}"#,
        "the agent supports ACP version 2; Brindle speaks 1",
    );
}

#[test]
fn an_agent_answering_acp_version_2_fails_with_its_version() {
    a_failed_start_is_the_last_agent_state(
        "agent-answers-v2",
        r#"respond {"protocolVersion":2,"agentInfo":{"name":"fake","version":"2.0"}}"#,
        "the agent speaks ACP version 2; Brindle speaks 1",
    );
}

#[test]
fn an_unoffered_request_gets_32601_and_an_unknown_notification_nothing() {
    let (mut dir, mut client) = agent_core("agent-unoffered", &[]);
    let (mut script, record) = started(&dir.path, "unoffered");
    let read = json!({"jsonrpc": "2.0", "id": "fs-1", "method": "fs/read_text_file",
                      "params": {"sessionId": "s1", "path": "/etc/hostname"}});
    let ping = json!({"jsonrpc": "2.0", "method": "_vendor/ping", "params": {"n": 1}});
    script
        .step("expect session/prompt")
        .step(format!("send {read}"))
        .step(r#"expect-response "fs-1""#)
        .step(format!("send {ping}"))
        .step(said("carried on"))
        .step(stop("end_turn"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "read it").unwrap();
    let log = client.transcript_until(id, "has the turn's end", ended(1));

    let answers = responses(&record);
    assert_eq!(answers.len(), 1, "{answers:?}");
    assert_eq!(answers[0]["id"], json!("fs-1"));
    assert_eq!(answers[0]["error"]["code"], json!(-32601), "{}", answers[0]);
    // Neither the request nor the notification is in the log.
    assert_eq!(says(&log), ["user: read it", "agent: carried on"]);
    assert_eq!(log.entries().count(), 2);
}

#[test]
fn chunks_of_one_message_merge_into_one_entry() {
    let (mut dir, mut client) = agent_core("agent-chunks", &[]);
    let (mut script, _) = started(&dir.path, "chunks");
    script
        .step("expect session/prompt")
        .step(said("Hel"))
        .step(said("lo"))
        .step(said("!"))
        .step(stop("end_turn"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "hi").unwrap();
    let log = client.transcript_until(id, "has the turn's end", turn_is(end(StopReason::EndTurn)));
    assert_eq!(says(&log), ["user: hi", "agent: Hello!"]);
}

#[test]
fn an_unknown_update_kind_changes_nothing() {
    let (mut dir, mut client) = agent_core("agent-unknown-update", &[]);
    let (mut script, _) = started(&dir.path, "unknown-update");
    script
        .step("expect session/prompt")
        .step(update(json!({"sessionUpdate": "hologram", "frames": 3})))
        .step(said("after"))
        .step(stop("end_turn"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "hi").unwrap();
    let log = client.transcript_until(id, "has the turn's end", ended(1));
    assert_eq!(says(&log), ["user: hi", "agent: after"]);
    assert_eq!(log.entries().count(), 2, "{:?}", notices(&log));
}

#[test]
fn a_tool_call_is_one_entry_updated_by_its_id() {
    let (mut dir, mut client) = agent_core("agent-tool", &[]);
    let (mut script, _) = started(&dir.path, "tool");
    script
        .step("expect session/prompt")
        .step(update(json!({"sessionUpdate": "tool_call", "toolCallId": "call_1",
                            "title": "Edit main.rs", "kind": "edit", "status": "pending"})))
        .step(update(json!({"sessionUpdate": "tool_call_update", "toolCallId": "call_1", "status": "completed"})))
        .step(stop("end_turn"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "edit").unwrap();
    let log = client.transcript_until(id, "has the turn's end", turn_is(end(StopReason::EndTurn)));
    let tools: Vec<&Entry> = log.entries().map(|(_, entry)| entry).filter(|entry| matches!(entry, Entry::Tool { .. })).collect();
    assert_eq!(
        tools,
        [&Entry::Tool {
            call: "call_1".into(),
            title: "Edit main.rs".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Completed,
        }]
    );
}

#[test]
fn text_passes_unchanged_and_control_characters_are_replaced() {
    let (mut dir, mut client) = agent_core("agent-text", &[]);
    let (mut script, _) = started(&dir.path, "text");
    let kept = "tab\there\r\nnext line: é, 漢字, 🦀";
    script
        .step("expect session/prompt")
        .step(said(kept))
        .step("send {\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"s1\",\"update\":{\"sessionUpdate\":\"agent_thought_chunk\",\"content\":{\"type\":\"text\",\"text\":\"clear \\u001b[2J screen\"}}}}")
        .step(stop("end_turn"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "hi").unwrap();
    let log = client.transcript_until(id, "has the turn's end", turn_is(end(StopReason::EndTurn)));
    let entries: Vec<&Entry> = log.entries().map(|(_, entry)| entry).collect();
    assert_eq!(
        entries,
        [
            &Entry::User { text: "hi".into() },
            &Entry::Agent { text: kept.into() },
            &Entry::Thought { text: "clear \u{FFFD}[2J screen".into() },
        ]
    );
}

#[test]
fn a_turn_reaches_the_client_in_order() {
    let (mut dir, mut client) = agent_core("agent-order", &[]);
    let (script, _) = agent_script(&dir, "order", &["hello"]);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "hi").unwrap();
    client.transcript_until(id, "has the turn's end", ended(1));
    assert_in_order(&client.events(id), &["user hi", "turn Working", "agent hello", "turn Ended { stop: EndTurn }"]);
}

#[test]
fn a_prompt_answered_with_an_error_fails_its_turn_with_the_message() {
    let (mut dir, mut client) = agent_core("agent-prompt-error", &[]);
    let (mut script, _) = started(&dir.path, "prompt-error");
    script.step("expect session/prompt").step(r#"respond-error -32603 "rate limited""#);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "hi").unwrap();
    let log = client.transcript_until(id, "fails the turn", |log| matches!(log.turn(), TurnState::Failed { .. }));
    let TurnState::Failed { message } = log.turn() else { unreachable!() };
    assert!(message.contains("rate limited"), "{message}");
    assert_eq!(activity_of(&mut client, id), Some(AgentActivity::Error));
}

/// A script whose first turn waits at a gate before `ending` (its steps);
/// then it answers each later prompt with `end_turn`. Gives the script,
/// its record and the gate.
fn gated_turn(dir: &TempDir, name: &str, ending: &[String], later: usize) -> (PathBuf, PathBuf, Gate) {
    let (mut script, record) = started(&dir.path, name);
    script.step("expect session/prompt");
    let gate = script.gate();
    for step in ending {
        script.step(step.clone());
    }
    for n in 0..later {
        script.step("expect session/prompt").step(said(&format!("later {n}"))).step(stop("end_turn"));
    }
    (script.write(), record, gate)
}

#[test]
fn a_prompt_during_a_turn_waits_and_is_sent_after_end_turn() {
    let (mut dir, mut client) = agent_core("agent-waits", &[]);
    let ending = [said("first"), stop("end_turn")];
    let (script, record, gate) = gated_turn(&dir, "waits", &ending, 1);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    client.transcript_until(id, "has the prompt waiting", slot_is(waiting("two")));
    prompted(&record, &["one"]);

    gate.open();
    client.transcript_until(id, "has both turns' ends", ended(2));
    assert_in_order(
        &client.events(id),
        &[
            "user one",
            "queue waiting two",
            "agent first",
            "turn Ended { stop: EndTurn }",
            "queue empty",
            "user two",
            "turn Working",
            "agent later 0",
            "turn Ended { stop: EndTurn }",
        ],
    );
    assert_eq!(prompts(&record), ["one", "two"]);
}

#[test]
fn a_third_prompt_is_refused_while_one_waits() {
    let (mut dir, mut client) = agent_core("agent-third", &[]);
    let ending = [stop("end_turn")];
    let (script, record, gate) = gated_turn(&dir, "third", &ending, 1);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    let refusal = client.prompt(id, "three").unwrap_err();
    assert_eq!(refusal.message, format!("session {id}: a prompt is already queued"));
    assert_eq!(*client.transcript_until(id, "has the prompt waiting", slot_is(waiting("two"))).slot(), waiting("two"));

    gate.open();
    let log = client.transcript_until(id, "has the second turn's end", |log| {
        *log.slot() == Slot::Empty && *log.turn() == end(StopReason::EndTurn) && says(log).len() == 3
    });
    assert_eq!(says(&log), ["user one", "user two", "agent later 0"].map(|line| line.replacen(' ', ": ", 1)));
    assert_eq!(prompts(&record), ["one", "two"]);
}

#[test]
fn a_prompt_before_the_agent_is_ready_is_refused() {
    let (mut dir, mut client) = agent_core("agent-not-ready", &[]);
    let mut script = AgentScript::new(&dir.path, "not-ready");
    let record = script.record();
    script
        .step("expect initialize")
        .step(r#"respond {"protocolVersion":1,"agentInfo":{"name":"fake","version":"1.0"}}"#)
        .step("expect session/new");
    let gate = script.gate();
    script.step(r#"respond {"sessionId":"s1"}"#);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));

    let refusal = client.prompt(id, "too soon").unwrap_err();
    assert_eq!(refusal.message, format!("session {id}: the agent isn't ready"));
    gate.open();
    let log = client.transcript_until(id, "is ready", ready);
    assert_eq!((log.entries().count(), log.slot()), (0, &Slot::Empty));
    assert_eq!(methods(&record), ["initialize", "session/new"]);
}

#[test]
fn a_prompt_over_256_kib_is_refused_and_one_at_the_limit_is_sent() {
    let (mut dir, mut client) = agent_core("agent-oversized", &[]);
    let (script, record) = agent_script(&dir, "oversized", &["took it"]);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    client.transcript_until(id, "is ready", ready);

    let refusal = client.prompt(id, &"x".repeat(300 * 1024)).unwrap_err();
    assert!(refusal.message.contains("256 KiB"), "{}", refusal.message);
    let log = client.transcript(id);
    assert_eq!((log.entries().count(), log.slot()), (0, &Slot::Empty));

    let most = "y".repeat(256 * 1024);
    client.prompt(id, &most).unwrap();
    client.transcript_until(id, "has the turn's end", ended(1));
    assert_eq!(prompts(&record), [most]);
}

/// A prompt waiting while the first turn runs is held as `reason` once
/// `finish` (the client's part, after the gate) and the gated `ending`
/// end that turn; the hold is told before the turn's end, and the agent
/// never gets it: a third prompt, sent then, is the next it gets.
fn held_when(tag: &str, ending: &[String], finish: impl FnOnce(&mut TestClient, SessionId, &Gate), reason: HoldReason, turn: &str) {
    let (mut dir, mut client) = agent_core(tag, &[]);
    let (script, record, gate) = gated_turn(&dir, tag, ending, 1);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    finish(&mut client, id, &gate);

    let hold = held("two", reason);
    let log = client.transcript_until(id, "holds the prompt", |log| {
        *log.slot() == hold && !matches!(log.turn(), TurnState::Working | TurnState::Cancelling)
    });
    assert_in_order(&client.events(id), &["queue held two", turn]);
    assert!(says(&log).iter().all(|line| line != "user: two"), "{:?}", says(&log));

    client.prompt(id, "three").unwrap();
    let log = client.transcript_until(id, "has the third prompt's turn", |log| says(log).contains(&"agent: later 0".to_owned()));
    assert_eq!(*log.slot(), hold, "a new prompt leaves the held one held");
    let sent = prompts(&record);
    assert_eq!(sent, ["one", "three"]);
}

#[test]
fn a_failed_turn_holds_the_waiting_prompt() {
    let ending = [r#"respond-error -32603 "the model is down""#.to_owned()];
    held_when("agent-hold-failed", &ending, |_, _, gate| gate.open(), HoldReason::Failed, "turn Failed");
}

#[test]
fn a_max_tokens_ending_holds_the_waiting_prompt_with_its_reason() {
    let ending = [stop("max_tokens")];
    let reason = HoldReason::Stopped { stop: StopReason::MaxTokens };
    held_when("agent-hold-max", &ending, |_, _, gate| gate.open(), reason, "turn Ended { stop: MaxTokens }");
}

#[test]
fn a_cancel_holds_the_waiting_prompt_before_the_turn_ends() {
    let ending = ["expect session/cancel".to_owned(), stop("cancelled")];
    let cancel = |client: &mut TestClient, id: SessionId, gate: &Gate| {
        client.cancel(id);
        client.transcript_until(id, "is cancelling", turn_is(TurnState::Cancelling));
        gate.open();
    };
    held_when("agent-hold-cancel", &ending, cancel, HoldReason::Cancelled, "turn Ended { stop: Cancelled }");
}

#[test]
fn an_agent_exit_holds_the_waiting_prompt_then_reports_the_exit() {
    let (mut dir, mut client) = agent_core("agent-hold-exit", &[]);
    let ending = ["stderr fake: crashed".to_owned(), "exit 1".to_owned()];
    let (script, record, gate) = gated_turn(&dir, "hold-exit", &ending, 0);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    gate.open();

    assert_eq!(exit_of(&mut client, id).code, Some(1));
    assert_in_order(&client.events(id), &["queue held two AgentStopped", "state Exited"]);
    assert_eq!(*client.transcript(id).slot(), held("two", HoldReason::AgentStopped));
    assert_eq!(prompts(&record), ["one"]);
}

#[test]
fn a_prompt_held_while_detached_is_in_the_replay() {
    let (mut dir, mut client) = agent_core("agent-hold-detached", &[]);
    let ending = [r#"respond-error -32603 "the model is down""#.to_owned()];
    let (script, record, gate) = gated_turn(&dir, "hold-detached", &ending, 1);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    client.detach(id);
    gate.open();
    assert!(eventually(TIMEOUT, || activity_of(&mut client, id) == Some(AgentActivity::Error)));

    let mut later = TestClient::hello(&dir.path, "later");
    later.attach_agent(id).unwrap();
    assert_eq!(*later.transcript(id).slot(), held("two", HoldReason::Failed));
    assert!(later.events(id).iter().any(|event| shape(event).starts_with("queue held two Failed")));
    later.prompt(id, "three").unwrap();
    later.transcript_until(id, "has the next turn's end", |log| says(log).contains(&"agent: later 0".to_owned()));
    assert_eq!(prompts(&record), ["one", "three"]);
}

/// A session whose first turn held `two` (`max_tokens`), the agent
/// answering later prompts as [`gated_turn`] does.
fn with_a_held_prompt(tag: &str, later: usize) -> (TempDir, TestClient, SessionId, PathBuf) {
    let (mut dir, mut client) = agent_core(tag, &[]);
    let (script, record, gate) = gated_turn(&dir, tag, &[stop("max_tokens")], later);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    gate.open();
    let reason = HoldReason::Stopped { stop: StopReason::MaxTokens };
    client.transcript_until(id, "holds the prompt", slot_is(held("two", reason)));
    (dir, client, id, record)
}

#[test]
fn a_held_prompt_sent_again_when_idle_is_a_new_turn() {
    let (_dir, mut client, id, record) = with_a_held_prompt("agent-resend-idle", 1);
    client.resend(id).unwrap();
    let log = client.transcript_until(id, "has the resent prompt's turn", |log| says(log).contains(&"agent: later 0".to_owned()));
    assert_eq!(*log.slot(), Slot::Empty);
    assert_in_order(&client.events(id), &["queue empty", "user two", "turn Working", "agent later 0"]);
    assert_eq!(prompts(&record), ["one", "two"]);
}

#[test]
fn a_held_prompt_sent_again_during_a_turn_waits_for_it() {
    let (mut dir, mut client) = agent_core("agent-resend-busy", &[]);
    let (mut script, record) = started(&dir.path, "resend-busy");
    script.step("expect session/prompt");
    let first = script.gate();
    script.step(stop("max_tokens")).step("expect session/prompt");
    let second = script.gate();
    script
        .step(said("three done"))
        .step(stop("end_turn"))
        .step("expect session/prompt")
        .step(said("two done"))
        .step(stop("end_turn"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    first.open();
    let reason = HoldReason::Stopped { stop: StopReason::MaxTokens };
    client.transcript_until(id, "holds the prompt", slot_is(held("two", reason.clone())));

    client.prompt(id, "three").unwrap();
    assert_eq!(*client.transcript(id).slot(), held("two", reason));
    client.resend(id).unwrap();
    client.transcript_until(id, "has it waiting again", slot_is(waiting("two")));
    prompted(&record, &["one", "three"]);
    second.open();
    let log = client.transcript_until(id, "has its turn", |log| says(log).contains(&"agent: two done".to_owned()));
    assert_eq!(*log.slot(), Slot::Empty);
    assert_eq!(prompts(&record), ["one", "three", "two"]);
}

#[test]
fn a_waiting_prompt_discarded_is_never_sent_and_an_empty_slot_refuses_both() {
    let (mut dir, mut client) = agent_core("agent-discard", &[]);
    let (script, record, gate) = gated_turn(&dir, "discard", &[stop("end_turn")], 1);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    client.transcript_until(id, "is ready", ready);
    let empty = client.unqueue(id).unwrap_err();
    assert_eq!(empty.message, format!("session {id}: nothing is queued"));
    let none = client.resend(id).unwrap_err();
    assert_eq!(none.message, format!("session {id}: no prompt is held"));

    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    // A waiting prompt isn't held.
    assert_eq!(client.resend(id).unwrap_err().message, format!("session {id}: no prompt is held"));
    client.unqueue(id).unwrap();
    client.transcript_until(id, "has the slot empty", slot_is(Slot::Empty));
    assert_in_order(&client.events(id), &["queue waiting two", "queue empty"]);
    gate.open();
    client.transcript_until(id, "has the turn's end", turn_is(end(StopReason::EndTurn)));

    client.prompt(id, "three").unwrap();
    client.transcript_until(id, "has the next turn", |log| says(log).contains(&"agent: later 0".to_owned()));
    assert_eq!(prompts(&record), ["one", "three"]);
    assert_eq!(client.unqueue(id).unwrap_err().message, format!("session {id}: nothing is queued"));
}

/// A script whose first prompt asks permission `perm-1` and, once it is
/// answered, ends the turn with `end_turn`; later prompts as
/// [`gated_turn`]'s.
fn asks_permission(dir: &TempDir, name: &str) -> (PathBuf, PathBuf) {
    let (mut script, record) = started(&dir.path, name);
    script
        .step("expect session/prompt")
        .step(ask("perm-1"))
        .step(r#"expect-response "perm-1""#)
        .step(said("done"))
        .step(stop("end_turn"))
        .step("expect session/prompt")
        .step(said("later 0"))
        .step(stop("end_turn"));
    (script.write(), record)
}

fn blocked(log: &Transcript) -> bool {
    *log.turn() == TurnState::Blocked
}

#[test]
fn a_cancel_answers_a_pending_permission_cancelled() {
    let (mut dir, mut client) = agent_core("agent-cancel-permission", &[]);
    let (mut script, record) = started(&dir.path, "cancel-permission");
    script
        .step("expect session/prompt")
        .step(ask("perm-1"))
        .step("expect session/cancel")
        .step(r#"expect-response "perm-1""#)
        .step(stop("cancelled"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "write").unwrap();
    client.transcript_until(id, "waits on the permission", blocked);
    client.cancel(id);

    let log = client.transcript_until(id, "has the turn's end", turn_is(end(StopReason::Cancelled)));
    assert_eq!(methods(&record), ["initialize", "session/new", "session/prompt", "session/cancel"]);
    let answers = responses(&record);
    assert_eq!(outcome_sent(&answers[0], "perm-1"), json!({"outcome": "cancelled"}));
    let asked = permissions(&log);
    let [(_, Entry::Permission { outcome, .. })] = asked.as_slice() else {
        panic!("{asked:?}");
    };
    assert_eq!(*outcome, Some(PermissionOutcome::Cancelled));
}

#[test]
fn a_permission_waits_for_a_client_and_is_answered_after_an_attach() {
    let (mut dir, mut client) = agent_core("agent-permission-detached", &[]);
    let (script, record) = asks_permission(&dir, "permission-detached");
    let (id, _) = spawned(&mut client, &mut dir, detached(&script));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "write").unwrap();
    client.transcript_until(id, "waits on the permission", blocked);
    drop(client);

    let mut later = TestClient::hello(&dir.path, "later");
    assert!(eventually(TIMEOUT, || listed(&mut later, id).is_some_and(|info| !info.attached)));
    assert_eq!(activity_of(&mut later, id), Some(AgentActivity::Blocked));
    later.attach_agent(id).unwrap();
    // Nobody answered it meanwhile.
    let log = later.transcript(id);
    let asked = permissions(&log);
    let [(entry, Entry::Permission { options, outcome, .. })] = asked.as_slice() else {
        panic!("{asked:?}");
    };
    assert_eq!(*outcome, None);
    let offered: Vec<&str> = options.iter().map(|option| option.id.as_str()).collect();
    assert_eq!(offered, ["allow-once", "reject-once"]);
    assert!(responses(&record).is_empty());

    later.answer(id, *entry, Some("reject-once")).unwrap();
    let answers = answered(&record, 1);
    assert_eq!(outcome_sent(&answers[0], "perm-1"), json!({"outcome": "selected", "optionId": "reject-once"}));
    later.transcript_until(id, "has the turn's end", turn_is(end(StopReason::EndTurn)));
    assert_in_order(&later.events(id), &["answered Selected", "turn Working", "agent done"]);
}

#[test]
fn a_permission_answered_twice_is_refused_naming_it() {
    let (mut dir, mut client) = agent_core("agent-permission-twice", &[]);
    let (script, record) = asks_permission(&dir, "permission-twice");
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "write").unwrap();
    let log = client.transcript_until(id, "waits on the permission", blocked);
    let entry = permissions(&log)[0].0;

    let unknown = client.answer(id, entry, Some("allow-always")).unwrap_err();
    assert_eq!(unknown.message, format!("session {id}: permission request {entry} doesn't offer option \"allow-always\""));
    client.answer(id, entry, Some("allow-once")).unwrap();
    let again = client.answer(id, entry, Some("reject-once")).unwrap_err();
    assert_eq!(again.message, format!("session {id}: permission request {entry} was already answered"));
    let log = client.transcript_until(id, "has the turn's end", turn_is(end(StopReason::EndTurn)));
    let asked = permissions(&log);
    let [(_, Entry::Permission { outcome, .. })] = asked.as_slice() else {
        panic!("{asked:?}");
    };
    assert_eq!(*outcome, Some(PermissionOutcome::Selected { option: "allow-once".into() }));
    let answers = answered(&record, 1);
    assert_eq!(outcome_sent(&answers[0], "perm-1"), json!({"outcome": "selected", "optionId": "allow-once"}));
}

#[test]
fn a_replay_then_live_events_give_nothing_twice() {
    let (mut dir, mut first) = agent_core("agent-replay-live", &[]);
    let (mut script, _) = started(&dir.path, "replay-live");
    script.step("expect session/prompt").step(said("a"));
    let gate = script.gate();
    script
        .step(said("b"))
        .step(stop("end_turn"))
        .step("expect session/prompt")
        .step(said("c"))
        .step(stop("end_turn"));
    let (id, _) = spawned(&mut first, &mut dir, client_agent(&script.write()));
    first.transcript_until(id, "is ready", ready);
    first.prompt(id, "one").unwrap();
    first.transcript_until(id, "has the first chunk", |log| says(log).contains(&"agent: a".to_owned()));
    first.prompt(id, "two").unwrap();
    first.detach(id);

    let mut second = TestClient::hello(&dir.path, "second");
    second.attach_agent(id).unwrap();
    let replayed = second.transcript(id);
    assert_eq!(says(&replayed), ["user: one", "agent: a"]);
    assert_eq!(*replayed.slot(), waiting("two"), "the waiting prompt is in the replay");
    gate.open();
    let log = second.transcript_until(id, "has both turns", ended(2));
    assert_eq!(says(&log), ["user: one", "agent: ab", "user: two", "agent: c"]);
}

#[test]
fn attaching_again_replays_the_whole_log_from_the_start() {
    let (mut dir, mut client) = agent_core("agent-reattach", &[]);
    let (mut script, _) = started(&dir.path, "reattach");
    script.step("expect session/prompt").step(said("part one"));
    let gate = script.gate();
    script.step(said(", part two")).step(stop("end_turn"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "go").unwrap();
    let seen = client.transcript_until(id, "has part of the turn", |log| says(log).contains(&"agent: part one".to_owned()));
    client.detach(id);

    client.attach_agent(id).unwrap();
    assert_eq!(client.transcript(id), seen, "the replay is the whole log");
    let events = client.events(id);
    assert!(matches!(events.first(), Some(Event::Added { id: EntryId::FIRST, .. })), "{events:?}");
    gate.open();
    let log = client.transcript_until(id, "has the turn's end", turn_is(end(StopReason::EndTurn)));
    assert_eq!(says(&log), ["user: go", "agent: part one, part two"]);
}

fn layout(keys: &[&str]) -> Value {
    json!({"v": 1, "quit": "q", "index": 0, "keys": keys})
}

#[test]
fn agent_sessions_detached_with_a_layout_list_it_until_attached() {
    let (mut dir, mut client) = agent_core("agent-layout", &[]);
    let (script, _) = agent_script(&dir, "layout", &[]);
    let (left, _) = spawned(&mut client, &mut dir, client_agent(&script));
    let (right, _) = spawned(&mut client, &mut dir, client_agent(&script));
    let key = |client: &mut TestClient, id| listed(client, id).expect("listed").key;
    let keys = [key(&mut client, left), key(&mut client, right)];
    let tab = layout(&[&keys[0], &keys[1]]);
    client.detach_with(left, Some(tab.clone()));
    client.detach_with(right, Some(tab.clone()));
    for id in [left, right] {
        assert_eq!(listed(&mut client, id).and_then(|info| info.layout), Some(tab.clone()));
    }

    client.attach_agent(left).unwrap();
    assert_eq!(listed(&mut client, left).and_then(|info| info.layout), None, "dropped on attach");
    assert_eq!(listed(&mut client, right).and_then(|info| info.layout), Some(tab));
}

#[test]
fn an_agent_sessions_set_layout_outlives_its_killed_client_and_core() {
    let (mut dir, mut client) = agent_core("agent-layout-kept", &[]);
    let (script, _) = agent_script(&dir, "layout-kept", &[]);
    let (id, _) = spawned(&mut client, &mut dir, detached(&script));
    client.transcript_until(id, "is ready", ready);
    client.set_layout(id, Some(layout(&["agent"])));
    // A round trip orders it before the client's end.
    client.list();
    let core = client.core_pid();
    let mut gone_client = client.into_process();
    kill(gone_client.id());
    let _ = gone_client.wait();

    let mut other = TestClient::hello(&dir.path, "other");
    assert!(eventually(TIMEOUT, || listed(&mut other, id).is_some_and(|info| !info.attached)));
    assert_eq!(listed(&mut other, id).and_then(|info| info.layout), Some(layout(&["agent"])));
    kill(core);
    assert!(other.closes());
    start_core(&dir.path, &[]).unwrap();
    let mut after = TestClient::hello(&dir.path, "after");
    dir.track(after.core_pid());
    let info = listed(&mut after, id).expect("adopted");
    assert_eq!((info.layout, info.cols, info.rows), (Some(layout(&["agent"])), 0, 0));
}

#[test]
fn an_agent_session_has_no_size_and_refuses_a_resize() {
    let (mut dir, mut client) = agent_core("agent-no-size", &[]);
    let (script, record) = agent_script(&dir, "no-size", &[]);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    let ready_log = client.transcript_until(id, "is ready", ready);
    let info = listed(&mut client, id).expect("listed");
    assert_eq!((info.cols, info.rows), (0, 0));
    client.detach(id);
    client.attach_agent(id).unwrap();
    assert_eq!(client.transcript(id), ready_log, "the attach replays it");

    client.send(client::ToCore::Resize(client::Resize { session: id, cols: 80, rows: 24, cell_w: 0, cell_h: 0 }));
    let refusal = unrequested_error(&mut client, id);
    assert!(refusal.contains(&format!("session {id}")), "{refusal}");
    let info = listed(&mut client, id).expect("listed");
    assert_eq!((info.cols, info.rows), (0, 0));
    assert_eq!(methods(&record), ["initialize", "session/new"], "the agent got nothing");
}

#[test]
fn a_turn_finished_while_detached_is_all_in_the_next_replay() {
    let (mut dir, mut client) = agent_core("agent-detached-turn", &[]);
    let (mut script, _) = started(&dir.path, "detached-turn");
    script.step("expect session/prompt").step(said("started"));
    let gate = script.gate();
    script.step(said(" and finished")).step(stop("end_turn"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "go").unwrap();
    client.transcript_until(id, "has started", |log| says(log).contains(&"agent: started".to_owned()));
    client.detach(id);
    gate.open();
    assert!(eventually(TIMEOUT, || activity_of(&mut client, id) == Some(AgentActivity::Done)));

    let mut later = TestClient::hello(&dir.path, "later");
    later.attach_agent(id).unwrap();
    let log = later.transcript(id);
    assert_eq!(says(&log), ["user: go", "agent: started and finished"]);
    assert_eq!(*log.turn(), end(StopReason::EndTurn));
}

#[test]
fn closing_ends_an_agent_that_ignores_the_end_of_its_input() {
    let (mut dir, mut client) = agent_core("agent-close", &[]);
    let (mut script, _) = started(&dir.path, "close");
    // Never opened: the agent reads no more of its stdin.
    let _gate = script.gate();
    let script = script.write();
    #[cfg(unix)]
    let spawn = client::SpawnAgent {
        command: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            r#"sleep 600 & exec "$0" "$1""#.into(),
            FAKE_AGENT.into(),
            script.display().to_string(),
        ],
        ..client_agent(&script)
    };
    #[cfg(windows)]
    let spawn = client_agent(&script);
    let (id, pid) = spawned(&mut client, &mut dir, spawn);
    client.transcript_until(id, "is ready", ready);
    // On Linux it started a process of its own; the holder tests check a
    // Windows job's children.
    #[cfg(unix)]
    let started_child = until("the agent's child", TIMEOUT, || children_named(pid, "sleep").pop());

    client.send(client::ToCore::Close(client::SessionRef { session: id }));
    exit_of(&mut client, id);
    assert!(eventually(Duration::from_secs(10), || !alive(pid)), "the agent outlived its close");
    #[cfg(unix)]
    assert!(eventually(Duration::from_secs(10), || !alive(started_child)), "its child outlived the close");
    assert!(listed(&mut client, id).is_none());
}

#[test]
fn an_agent_exit_is_reported_with_its_status_and_stderr() {
    let (mut dir, mut client) = agent_core("agent-crash", &[]);
    let (mut script, _) = started(&dir.path, "crash");
    script.step("expect session/prompt").step("stderr fatal: no model").step("exit 1");
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "hi").unwrap();

    assert_eq!(exit_of(&mut client, id).code, Some(1));
    let states = agent_states(&client.events(id));
    match states.last() {
        Some(AgentState::Exited { code: Some(1), stderr }) => {
            assert!(stderr.trim_end().ends_with("fatal: no model"), "{stderr:?}");
        }
        other => panic!("the last agent state is {other:?}"),
    }
}

// OpenSpec change `agents-login`, task 5.1: logging the agent in through
// the core.

/// [`TWO_LOGINS`] as the transcript carries them.
fn two_logins() -> Vec<LoginMethod> {
    vec![
        LoginMethod { id: "oauth-personal".into(), name: "Log in with Google".into(), description: None, kind: LoginKind::Agent },
        LoginMethod {
            id: "api-key".into(),
            name: "API Key".into(),
            description: Some("Use an API key".into()),
            kind: LoginKind::Agent,
        },
    ]
}

/// The agent needs a login with `message`, offering [`TWO_LOGINS`].
fn needs_login(message: &str) -> AgentState {
    AgentState::LoginRequired { message: message.into(), methods: two_logins() }
}

fn logging_in(method: &str, name: &str) -> AgentState {
    AgentState::LoggingIn { method: method.into(), name: name.into() }
}

/// The agent states from the first that needs a login on.
fn from_login(events: &[Event]) -> Vec<AgentState> {
    agent_states(events)
        .into_iter()
        .skip_while(|state| !matches!(state, AgentState::LoginRequired { .. }))
        .collect()
}

/// The `idx`th recorded message with `method`.
fn sent(record: &Path, method: &str, idx: usize) -> Value {
    recorded(record)
        .into_iter()
        .filter(|value| value["method"] == method)
        .nth(idx)
        .unwrap_or_else(|| panic!("no {method} #{idx} in {:?}", methods(record)))
}

/// A script whose agent offers [`TWO_LOGINS`] and answers `session/new`
/// with `-32000` "log in first"; the test adds the rest.
fn login_at_start(dir: &TempDir, name: &str) -> (AgentScript, PathBuf) {
    let (mut script, record) = offering_logins(&dir.path, name, NEITHER, TWO_LOGINS);
    script.step("expect session/new").step(r#"respond-error -32000 "log in first""#);
    (script, record)
}

/// Starts `script` and waits until its agent needs a login with `message`.
fn needing_login(client: &mut TestClient, dir: &mut TempDir, script: &Path, message: &str) -> (SessionId, u32) {
    let (id, pid) = spawned(client, dir, client_agent(script));
    let wanted = needs_login(message);
    client.transcript_until(id, "needs a login", |log| *log.agent() == wanted);
    (id, pid)
}

#[test]
fn login_required_by_session_new_refuses_prompts() {
    let (mut dir, mut client) = agent_core("agent-login-new", &[]);
    let (script, _) = login_at_start(&dir, "login-new");
    let (id, pid) = needing_login(&mut client, &mut dir, &script.write(), "log in first");

    let refusal = client.prompt(id, "hi").unwrap_err();
    assert_eq!(refusal.message, format!("session {id}: the agent needs a login"));
    assert_eq!(activity_of(&mut client, id), Some(AgentActivity::Error));
    assert!(alive(pid), "the session stays running");
}

#[test]
fn login_demanded_by_a_prompt_holds_the_waiting_one() {
    let (mut dir, mut client) = agent_core("agent-login-prompt", &[]);
    let (mut script, record) = offering_logins(&dir.path, "login-prompt", NEITHER, TWO_LOGINS);
    script.step("expect session/new").step(r#"respond {"sessionId":"s1"}"#).step("expect session/prompt");
    let gate = script.gate();
    script.step(r#"respond-error -32000 "session expired""#);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    gate.open();

    let wanted = needs_login("session expired");
    let log = client.transcript_until(id, "needs a login", |log| *log.agent() == wanted);
    assert!(matches!(log.turn(), TurnState::Failed { .. }), "{:?}", log.turn());
    assert_eq!(*log.slot(), held("two", HoldReason::LoginRequired));
    assert_in_order(&client.events(id), &["queue held two LoginRequired", "turn Failed", "state LoginRequired"]);
    let refusal = client.resend(id).unwrap_err();
    assert_eq!(refusal.message, format!("session {id}: the agent needs a login"));
    assert_eq!(activity_of(&mut client, id), Some(AgentActivity::Error));
    client.unqueue(id).unwrap();
    assert_eq!(prompts(&record), ["one"]);
}

#[test]
fn a_login_at_the_start_sends_authenticate_then_session_new_and_takes_prompts() {
    let (mut dir, mut client) = agent_core("agent-login-start", &[]);
    let (mut script, record) = login_at_start(&dir, "login-start");
    script.step("expect authenticate");
    let gate = script.gate();
    script
        .step("respond {}")
        .step("expect session/new")
        .step(r#"respond {"sessionId":"s1"}"#)
        .step("expect session/prompt")
        .step(said("logged in"))
        .step(stop("end_turn"));
    let (id, _) = needing_login(&mut client, &mut dir, &script.write(), "log in first");

    client.login(id, "oauth-personal").unwrap();
    let wanted = logging_in("oauth-personal", "Log in with Google");
    client.transcript_until(id, "is logging in", |log| *log.agent() == wanted);
    assert_eq!(activity_of(&mut client, id), Some(AgentActivity::Working));
    let refusal = client.prompt(id, "too soon").unwrap_err();
    assert_eq!(refusal.message, format!("session {id}: the agent is logging in"));
    let refusal = client.login(id, "api-key").unwrap_err();
    assert_eq!(refusal.message, format!("session {id}: a login is already running"));
    gate.open();

    client.transcript_until(id, "is ready", ready);
    assert_eq!(
        from_login(&client.events(id)),
        [
            needs_login("log in first"),
            logging_in("oauth-personal", "Log in with Google"),
            AgentState::Starting,
            AgentState::Ready { name: Some("fake".into()), version: Some("1.0".into()) },
        ]
    );
    assert_in_order(&client.events(id), &["state LoggingIn", "state Starting", "conversation s1", "state Ready"]);
    assert_eq!(methods(&record), ["initialize", "session/new", "authenticate", "session/new"]);
    let authenticate = sent(&record, "authenticate", 0);
    assert_eq!((authenticate["id"].clone(), authenticate["params"].clone()), (json!(3), json!({"methodId": "oauth-personal"})));
    assert_eq!(sent(&record, "session/new", 1)["id"], json!(4));

    client.prompt(id, "hi").unwrap();
    let log = client.transcript_until(id, "has the turn's end", ended(1));
    assert_eq!(says(&log), ["user: hi", "agent: logged in"]);
}

#[test]
fn a_failed_login_offers_the_methods_again() {
    let (mut dir, mut client) = agent_core("agent-login-failed", &[]);
    let (mut script, record) = login_at_start(&dir, "login-failed");
    script
        .step("expect authenticate")
        .step(r#"respond-error -32603 "browser login timed out""#)
        .step("expect authenticate")
        .step("respond {}")
        .step("expect session/new")
        .step(r#"respond {"sessionId":"s1"}"#);
    let (id, _) = needing_login(&mut client, &mut dir, &script.write(), "log in first");

    client.login(id, "api-key").unwrap();
    let wanted = needs_login("browser login timed out");
    client.transcript_until(id, "needs a login again", |log| *log.agent() == wanted);
    assert_eq!(activity_of(&mut client, id), Some(AgentActivity::Error));

    client.login(id, "oauth-personal").unwrap();
    client.transcript_until(id, "is ready", ready);
    assert_eq!(
        from_login(&client.events(id)),
        [
            needs_login("log in first"),
            logging_in("api-key", "API Key"),
            needs_login("browser login timed out"),
            logging_in("oauth-personal", "Log in with Google"),
            AgentState::Starting,
            AgentState::Ready { name: Some("fake".into()), version: Some("1.0".into()) },
        ]
    );
    let chosen: Vec<Value> = (0..2).map(|n| sent(&record, "authenticate", n)["params"]["methodId"].clone()).collect();
    assert_eq!(chosen, [json!("api-key"), json!("oauth-personal")]);
}

#[test]
fn a_cancelled_login_sends_cancel_request_and_ignores_the_late_answer() {
    let (mut dir, mut client) = agent_core("agent-login-cancel", &[]);
    let (mut script, record) = login_at_start(&dir, "login-cancel");
    // The late answer to the cancelled `authenticate` (id 3), then a
    // request of the agent's own: its answer, recorded, says the core has
    // read the late one.
    let ping = json!({"jsonrpc": "2.0", "id": "ping", "method": "_vendor/ping", "params": {}});
    script
        .step("expect authenticate")
        .step("expect $/cancel_request")
        .step("respond-to 3 {}")
        .step(format!("send {ping}"))
        .step(r#"expect-response "ping""#)
        .step("expect authenticate")
        .step("respond {}")
        .step("expect session/new")
        .step(r#"respond {"sessionId":"s1"}"#);
    let (id, _) = needing_login(&mut client, &mut dir, &script.write(), "log in first");

    client.login(id, "oauth-personal").unwrap();
    let wanted = logging_in("oauth-personal", "Log in with Google");
    client.transcript_until(id, "is logging in", |log| *log.agent() == wanted);
    client.cancel(id);
    let wanted = needs_login("log in first");
    client.transcript_until(id, "needs a login again", |log| *log.agent() == wanted);

    // The agent was told which request was cancelled, and its late `{}`
    // started nothing.
    answered(&record, 1);
    assert_eq!(methods(&record), ["initialize", "session/new", "authenticate", "$/cancel_request"]);
    let authenticate = sent(&record, "authenticate", 0)["id"].clone();
    assert_eq!(sent(&record, "$/cancel_request", 0)["params"], json!({"requestId": authenticate}));
    assert!(sent(&record, "$/cancel_request", 0).get("id").is_none(), "a notification");
    assert_eq!(*client.transcript(id).agent(), needs_login("log in first"));

    // Choosing again is taken.
    client.login(id, "api-key").unwrap();
    client.transcript_until(id, "is ready", ready);
    assert_eq!(
        from_login(&client.events(id)),
        [
            needs_login("log in first"),
            logging_in("oauth-personal", "Log in with Google"),
            needs_login("log in first"),
            logging_in("api-key", "API Key"),
            AgentState::Starting,
            AgentState::Ready { name: Some("fake".into()), version: Some("1.0".into()) },
        ]
    );
    assert_eq!(
        methods(&record),
        ["initialize", "session/new", "authenticate", "$/cancel_request", "authenticate", "session/new"]
    );
}

#[test]
fn a_login_after_a_prompts_demand_carries_on_and_the_held_prompt_goes_by_a_resend() {
    let (mut dir, mut client) = agent_core("agent-login-resend", &[]);
    let (mut script, record) = offering_logins(&dir.path, "login-resend", NEITHER, TWO_LOGINS);
    script.step("expect session/new").step(r#"respond {"sessionId":"s1"}"#).step("expect session/prompt");
    let gate = script.gate();
    script
        .step(r#"respond-error -32000 "session expired""#)
        .step("expect authenticate")
        .step("respond {}")
        .step("expect session/prompt")
        .step(said("sent again"))
        .step(stop("end_turn"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    gate.open();
    let wanted = needs_login("session expired");
    client.transcript_until(id, "needs a login", |log| *log.agent() == wanted);

    client.login(id, "oauth-personal").unwrap();
    let log = client.transcript_until(id, "is ready again", ready);
    assert_in_order(&client.events(id), &["state LoginRequired", "state LoggingIn", "state Starting", "state Ready"]);
    // Neither prompt went again on its own, and no new session was made.
    assert_eq!(*log.slot(), held("two", HoldReason::LoginRequired));
    assert!(matches!(log.turn(), TurnState::Failed { .. }), "{:?}", log.turn());
    assert_eq!(methods(&record), ["initialize", "session/new", "session/prompt", "authenticate"]);

    client.resend(id).unwrap();
    let log = client.transcript_until(id, "has the resent prompt's turn", |log| {
        says(log).contains(&"agent: sent again".to_owned()) && *log.turn() == end(StopReason::EndTurn)
    });
    assert_eq!(*log.slot(), Slot::Empty);
    prompted(&record, &["one", "two"]);
    assert_eq!(sent(&record, "session/prompt", 1)["params"]["sessionId"], json!("s1"));
}

#[test]
fn a_login_for_an_unknown_method_or_while_ready_is_refused() {
    let (mut dir, mut client) = agent_core("agent-login-refused", &[]);
    let (mut script, record) = login_at_start(&dir, "login-refused");
    script.step("expect authenticate").step("respond {}").step("expect session/new").step(r#"respond {"sessionId":"s1"}"#);
    let (id, _) = needing_login(&mut client, &mut dir, &script.write(), "log in first");

    let refusal = client.login(id, "no-such-method").unwrap_err();
    assert_eq!(refusal.message, format!("session {id}: the agent offers no login method \"no-such-method\""));
    assert_eq!(*client.transcript(id).agent(), needs_login("log in first"));

    // The one login that went out is the next one's.
    client.login(id, "api-key").unwrap();
    client.transcript_until(id, "is ready", ready);
    assert_eq!(methods(&record), ["initialize", "session/new", "authenticate", "session/new"]);
    assert_eq!(sent(&record, "authenticate", 0)["params"]["methodId"], json!("api-key"));

    let refusal = client.login(id, "api-key").unwrap_err();
    assert_eq!(refusal.message, format!("session {id}: the agent isn't waiting for a login"));
    let terminal = client.list().into_iter().find(|info| info.session == id).expect("listed");
    assert_eq!(terminal.agent.map(|agent| agent.activity), Some(AgentActivity::Idle));
}

#[test]
fn the_session_list_shows_each_agents_activity() {
    let (mut dir, mut client) = agent_core("agent-activities", &[]);
    // Starting: initialize never answered.
    let mut starting = AgentScript::new(&dir.path, "starting");
    starting.step("expect initialize");
    let (starting, _) = spawned(&mut client, &mut dir, client_agent(&starting.write()));
    // A new session, ready.
    let (script, _) = agent_script(&dir, "new", &[]);
    let (new, _) = spawned(&mut client, &mut dir, client_agent(&script));
    // Waiting on a permission.
    let (script, _) = asks_permission(&dir, "blocked");
    let (blocked_id, _) = spawned(&mut client, &mut dir, client_agent(&script));
    // A turn ended with each stop reason that tells.
    let stopped = |client: &mut TestClient, dir: &mut TempDir, reason: &str| {
        let (mut script, _) = started(&dir.path, reason);
        script.step("expect session/prompt");
        if reason == "cancelled" {
            script.step("expect session/cancel");
        }
        script.step(stop(reason));
        spawned(client, dir, client_agent(&script.write())).0
    };
    let done = stopped(&mut client, &mut dir, "end_turn");
    let refused = stopped(&mut client, &mut dir, "refusal");
    let cancelled = stopped(&mut client, &mut dir, "cancelled");

    client.transcript_until(new, "is ready", ready);
    for id in [blocked_id, done, refused, cancelled] {
        client.transcript_until(id, "is ready", ready);
        client.prompt(id, "go").unwrap();
    }
    client.transcript_until(blocked_id, "waits on the permission", blocked);
    client.transcript_until(done, "has the turn's end", turn_is(end(StopReason::EndTurn)));
    client.transcript_until(refused, "has the turn's end", turn_is(end(StopReason::Refusal)));
    client.cancel(cancelled);
    client.transcript_until(cancelled, "has the turn's end", turn_is(end(StopReason::Cancelled)));

    for (id, activity) in [
        (starting, AgentActivity::Working),
        (new, AgentActivity::Idle),
        (blocked_id, AgentActivity::Blocked),
        (done, AgentActivity::Done),
        (refused, AgentActivity::Error),
        (cancelled, AgentActivity::Idle),
    ] {
        assert_eq!(activity_of(&mut client, id), Some(activity), "session {id}");
    }
}

#[test]
fn a_line_that_isnt_json_is_a_notice_and_the_session_carries_on() {
    let (mut dir, mut client) = agent_core("agent-not-json", &[]);
    let (mut script, _) = started(&dir.path, "not-json");
    script
        .step("expect session/prompt")
        .step("send-raw hello world")
        .step(said("still fine"))
        .step(stop("end_turn"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "hi").unwrap();
    let log = client.transcript_until(id, "has the turn's end", ended(1));
    assert_eq!(says(&log), ["user: hi", "agent: still fine"]);
    let notices = notices(&log);
    assert!(
        notices.iter().any(|notice| notice.contains("wasn't an ACP message")),
        "{notices:?}"
    );
}

/// The name of an event's kind.
fn kind(event: &Event) -> &'static str {
    match event {
        Event::Added { .. } => "added",
        Event::Appended { .. } => "appended",
        Event::ToolChanged { .. } => "tool_changed",
        Event::Answered { .. } => "answered",
        Event::Queue { .. } => "queue",
        Event::Turn { .. } => "turn",
        Event::Agent { .. } => "agent",
        Event::Conversation { .. } => "conversation",
        Event::Trimmed { .. } => "trimmed",
        Event::Window { .. } => "window",
    }
}

/// Every event the client folds is decoded with `Event::from_value` and
/// must encode back to the very JSON it came as (the test client's
/// `fold`): so each is one of protocol 4's forms. This run has the
/// conversation, a turn, a queue, a permission, a cancel and an exit; the
/// trim's are in
/// [`a_log_past_16_mib_drops_its_oldest_entries`].
#[test]
fn every_event_of_a_session_decodes_as_protocol_4s() {
    let (mut dir, mut client) = agent_core("agent-decodes", &[]);
    let (mut script, _) = started(&dir.path, "decodes");
    script
        .step("expect session/prompt")
        .step(said("thinking"))
        .step(said(" aloud"))
        .step(ask("perm-1"))
        .step("expect session/cancel")
        .step(r#"expect-response "perm-1""#)
        .step(stop("cancelled"))
        .step("expect session/prompt")
        .step(update(json!({"sessionUpdate": "tool_call", "toolCallId": "call_2",
                            "title": "Run tests", "kind": "execute", "status": "in_progress"})))
        .step(update(json!({"sessionUpdate": "tool_call_update", "toolCallId": "call_2", "status": "failed"})))
        .step(update(json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "hmm"}})))
        .step("send-raw not json")
        .step("stderr fake: giving up")
        .step("exit 3");
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.prompt(id, "two").unwrap();
    client.transcript_until(id, "waits on the permission", blocked);
    client.cancel(id);
    client.transcript_until(id, "has the turn's end", turn_is(end(StopReason::Cancelled)));
    client.resend(id).unwrap();
    assert_eq!(exit_of(&mut client, id).code, Some(3));

    let events = client.events(id);
    let mut kinds: Vec<&str> = events.iter().map(kind).collect();
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(
        kinds,
        ["added", "agent", "answered", "appended", "conversation", "queue", "tool_changed", "turn"]
    );
}

/// The client's log, and a fresh attach's, after an agent said 17
/// messages of 1 MiB each in one turn.
#[test]
fn a_log_past_16_mib_drops_its_oldest_entries() {
    const MESSAGES: usize = 17;
    let (mut dir, mut client) = agent_core("agent-trim", &[]);
    let (mut script, _) = started(&dir.path, "trim");
    let text = |n: usize| format!("{n:02}:{}", "x".repeat(1024 * 1024));
    script.step("expect session/prompt");
    for n in 0..MESSAGES {
        // A thought between two messages makes each its own entry.
        script
            .step(said(&text(n)))
            .step(update(json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "next"}})));
    }
    script.step(stop("end_turn"));
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "talk").unwrap();
    let live = client.transcript_until(id, "has the turn's end", turn_is(end(StopReason::EndTurn)));

    assert!(live.dropped() > 0, "nothing was dropped");
    assert!(live.encoded_len() <= 16 * 1024 * 1024, "{}", live.encoded_len());
    let messages: Vec<String> = says(&live).into_iter().filter(|line| line.starts_with("agent: ")).collect();
    assert!(messages.len() < MESSAGES, "{} messages kept", messages.len());
    assert_eq!(messages.last(), Some(&format!("agent: {}", text(MESSAGES - 1))), "the latest is intact");
    assert!(says(&live).iter().all(|line| line != "user: talk"), "the oldest entry went first");
    let dropped = live.dropped();
    assert!(live.entries().next().is_some_and(|(first, _)| first.0 == dropped + 1));

    client.detach(id);
    let mut later = TestClient::hello(&dir.path, "later");
    later.attach_agent(id).unwrap();
    let events = later.events(id);
    assert!(
        matches!(events.first(), Some(Event::Trimmed { dropped: replayed, .. }) if *replayed == dropped),
        "the replay starts with {:?}",
        events.first().map(shape)
    );
    assert_eq!(later.transcript(id), live);
}

// 7.5: core restarts.

/// Kills the client's core, and starts another that adopts the holders;
/// gives a client of it with `token`.
fn kill_and_restart_core(dir: &mut TempDir, client: &mut TestClient, token: &str, before_start: impl FnOnce()) -> TestClient {
    kill(client.core_pid());
    assert!(client.closes());
    before_start();
    start_core(&dir.path, &[]).unwrap();
    let client = TestClient::hello(&dir.path, token);
    dir.track(client.core_pid());
    client
}

#[test]
fn a_turn_the_agent_finished_while_no_core_ran_is_replayed_whole() {
    let token = "token-agent-killed-mid-turn";
    let (mut dir, mut client) = agent_core("agent-killed-mid-turn", &[]);
    let (mut script, _) = started(&dir.path, "killed-mid-turn");
    script.step("expect session/prompt").step(said("before"));
    let gate = script.gate();
    script.step(said(" the kill")).step(stop("end_turn"));
    let finished = script.reached();
    let (id, pid) = spawned(&mut client, &mut dir, detached(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "go").unwrap();
    // The agent got the prompt, so the holder has the checkpoint the core
    // queued before it.
    client.transcript_until(id, "has the turn's start", |log| says(log).contains(&"agent: before".to_owned()));

    let mut client = kill_and_restart_core(&mut dir, &mut client, token, || {
        gate.open();
        reaches(&finished);
    });
    let (start, _) = client.reconnect_replay();
    assert_eq!((start.session, start.pid), (id, pid));
    assert!(alive(pid));
    let log = client.transcript_until(id, "has the turn's end", turn_is(end(StopReason::EndTurn)));
    assert_eq!(says(&log), ["user: go", "agent: before the kill"]);
}

/// A script whose first turn asks permission `perm-1` and waits at a gate
/// once answered; the turn then ends with `ending`, and the next prompt
/// is answered with `end_turn`. The client's answer reaching the agent
/// orders the core's checkpoint of a waiting prompt before a kill: the
/// core queues a checkpoint before each answer's bytes.
fn answered_then_gated(dir: &TempDir, name: &str, ending: &[String]) -> (PathBuf, PathBuf, Gate, PathBuf) {
    let (mut script, record) = started(&dir.path, name);
    script.step("expect session/prompt").step(ask("perm-1")).step(r#"expect-response "perm-1""#);
    let gate = script.gate();
    for step in ending {
        script.step(step.clone());
    }
    let finished = script.reached();
    script.step("expect session/prompt").step(said("next turn")).step(stop("end_turn"));
    (script.write(), record, gate, finished)
}

/// Starts `script`'s session detached, sends `one`, queues `two` while the
/// permission waits, and answers it; returns once the agent has the answer.
fn queued_behind_a_permission(client: &mut TestClient, dir: &mut TempDir, script: &Path, record: &Path) -> (SessionId, u32) {
    let (id, pid) = spawned(client, dir, detached(script));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    let log = client.transcript_until(id, "waits on the permission", blocked);
    client.prompt(id, "two").unwrap();
    client.answer(id, permissions(&log)[0].0, Some("allow-once")).unwrap();
    answered(record, 1);
    assert_eq!(*client.transcript(id).slot(), waiting("two"));
    (id, pid)
}

#[test]
fn a_waiting_prompt_survives_a_core_kill_and_is_sent_once_after_end_turn() {
    let token = "token-agent-killed-waiting";
    let (mut dir, mut client) = agent_core("agent-killed-waiting", &[]);
    let ending = [said("first turn"), stop("end_turn")];
    let (script, record, gate, finished) = answered_then_gated(&dir, "killed-waiting", &ending);
    let (id, pid) = queued_behind_a_permission(&mut client, &mut dir, &script, &record);

    let mut client = kill_and_restart_core(&mut dir, &mut client, token, || {
        gate.open();
        reaches(&finished);
    });
    let (start, _) = client.reconnect_replay();
    assert_eq!((start.session, start.pid), (id, pid));
    let log = client.transcript_until(id, "has the second turn's end", |log| {
        says(log).contains(&"agent: next turn".to_owned()) && *log.turn() == end(StopReason::EndTurn)
    });
    assert_eq!(says(&log), ["user: one", "agent: first turn", "user: two", "agent: next turn"]);
    assert_eq!(*log.slot(), Slot::Empty);
    assert_eq!(prompts(&record), ["one", "two"], "sent once");
}

#[test]
fn a_truncated_tail_fails_the_turn_and_holds_the_waiting_prompt() {
    let token = "token-agent-truncated";
    let (mut dir, mut client) = agent_core("agent-truncated", &[]);
    // The turn's end, then more than the holder's 1 MiB tail keeps: the
    // end is lost.
    let filler: Vec<String> = (0..40)
        .map(|n| {
            let ping = json!({"jsonrpc": "2.0", "method": "_vendor/ping", "params": {"n": n, "pad": "p".repeat(64 * 1024)}});
            format!("send {ping}")
        })
        .collect();
    let ending: Vec<String> = [said("lost words"), stop("end_turn")].into_iter().chain(filler).collect();
    let (script, record, gate, finished) = answered_then_gated(&dir, "truncated", &ending);
    let (id, pid) = queued_behind_a_permission(&mut client, &mut dir, &script, &record);

    // With no core a holder keeps the tail whole (the agent blocks once it
    // is full), so a core that reads the output and never checkpoints
    // (the test's) stands in for a core that fell behind and died.
    kill(client.core_pid());
    assert!(client.closes());
    let mut test_core = TestCore::connect_to(&dir);
    test_core.hello(7);
    gate.open();
    test_core.output_until(id, br#""n":39,"#);
    reaches(&finished);
    drop(test_core);
    start_core(&dir.path, &[]).unwrap();
    let mut client = TestClient::hello(&dir.path, token);
    dir.track(client.core_pid());
    let (start, _) = client.reconnect_replay();
    assert_eq!((start.session, start.pid), (id, pid));
    let log = client.transcript_until(id, "has the prompt held", slot_is(held("two", HoldReason::OutputLost)));
    assert_eq!(
        *log.turn(),
        TurnState::Failed { message: "output was lost while the terminal server restarted".into() }
    );
    assert!(says(&log).iter().all(|line| !line.contains("lost words")), "{:?}", says(&log));
    let notices = notices(&log);
    assert!(notices.iter().any(|notice| notice.contains("lost while the terminal server restarted")), "{notices:?}");
    assert!(dir.logs().contains("its stdout was truncated"), "{}", dir.logs());

    // The agent runs on, and the next prompt works.
    client.prompt(id, "three").unwrap();
    let log = client.transcript_until(id, "has the next turn's end", |log| {
        says(log).contains(&"agent: next turn".to_owned()) && *log.turn() == end(StopReason::EndTurn)
    });
    assert_eq!(*log.slot(), held("two", HoldReason::OutputLost));
    assert_eq!(prompts(&record), ["one", "three"]);
}

/// OpenSpec change `agents-login`, task 5.2: the core is killed while the
/// agent logs in, and the agent answers `authenticate` only once the next
/// core has adopted it; that core carries the login on.
#[test]
fn a_login_running_when_the_core_is_killed_is_carried_on_by_the_next_core() {
    let token = "token-agent-killed-logging-in";
    let (mut dir, mut client) = agent_core("agent-killed-logging-in", &[]);
    let (mut script, record) = login_at_start(&dir, "killed-logging-in");
    script.step("expect authenticate");
    let gate = script.gate();
    script.step("respond {}").step("expect session/new").step(r#"respond {"sessionId":"s1"}"#);
    let (id, pid) = spawned(&mut client, &mut dir, detached(&script.write()));
    client.transcript_until(id, "needs a login", |log| *log.agent() == needs_login("log in first"));
    client.login(id, "oauth-personal").unwrap();
    // The agent has the request, so the holder has the checkpoint the core
    // queued before it.
    until("the agent gets authenticate", TIMEOUT, || methods(&record).contains(&"authenticate".to_owned()).then_some(()));

    let mut client = kill_and_restart_core(&mut dir, &mut client, token, || {});
    let (start, _) = client.reconnect_replay();
    assert_eq!((start.session, start.pid), (id, pid));
    assert_eq!(*client.transcript(id).agent(), logging_in("oauth-personal", "Log in with Google"));
    gate.open();
    client.transcript_until(id, "is ready", ready);
    // The replay's state, then the login's answer carried on.
    assert_eq!(
        agent_states(&client.events(id)),
        [
            logging_in("oauth-personal", "Log in with Google"),
            AgentState::Starting,
            AgentState::Ready { name: Some("fake".into()), version: Some("1.0".into()) },
        ]
    );
    assert_eq!(methods(&record), ["initialize", "session/new", "authenticate", "session/new"]);
    assert_eq!(sent(&record, "session/new", 1)["id"], json!(4), "the request ids go on");
}

// OpenSpec change `agents-session-recovery`, task 3.4: an agent that ended
// is kept for its conversation and continued (`AGENT_RESTART`, or a
// `SPAWN_AGENT` that names a conversation).

/// The notice a restart adds to the log.
const RESTARTED: &str = "the agent was started again to continue the conversation";

fn agent_exited(log: &Transcript) -> bool {
    matches!(log.agent(), AgentState::Exited { .. })
}

/// A session whose agent offers `capabilities`, answered prompt `one` with
/// `first`, and exits with status 1 once the gate opens. Gives the session
/// and the gate.
fn one_turn_then_exit(client: &mut TestClient, dir: &mut TempDir, name: &str, capabilities: &str) -> (SessionId, Gate) {
    let (mut script, _) = continuable(&dir.path, name, capabilities);
    script.step("expect session/prompt").step(said("first")).step(stop("end_turn"));
    let gate = script.gate();
    script.step("exit 1");
    let (id, _) = spawned(client, dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.transcript_until(id, "has the first turn", ended(1));
    (id, gate)
}

/// Waits until the session's log says its agent `is` what `done` checks,
/// then asserts the session was kept: listed with activity `error` and no
/// program, and no `EXIT` came.
fn kept(client: &mut TestClient, id: SessionId, is: &str, done: impl FnMut(&Transcript) -> bool) -> Transcript {
    let log = client.transcript_until(id, is, done);
    let info = listed(client, id).expect("the ended session is still listed");
    assert_eq!(info.agent.map(|agent| agent.activity), Some(AgentActivity::Error));
    assert_eq!(info.pid, 0, "an ended session has no program");
    let exits: Vec<ToClient> = already_sent(client)
        .into_iter()
        .filter(|message| matches!(message, ToClient::Exit(exit) if exit.session == id))
        .collect();
    assert!(exits.is_empty(), "{exits:?}");
    log
}

/// A [`one_turn_then_exit`] session whose agent has exited and is kept.
fn ended_and_kept(client: &mut TestClient, dir: &mut TempDir, name: &str, capabilities: &str) -> SessionId {
    let (id, gate) = one_turn_then_exit(client, dir, name, capabilities);
    gate.open();
    let log = kept(client, id, "has the agent's exit", agent_exited);
    assert_eq!(log.conversation().map(|conversation| conversation.id.as_str()), Some("s1"));
    id
}

/// Waits for the continued agent to be ready, and tracks its process.
fn ready_again(client: &mut TestClient, dir: &mut TempDir, id: SessionId) -> Transcript {
    let log = client.transcript_until(id, "is ready again", ready);
    let pid = listed(client, id).expect("listed").pid;
    assert_ne!(pid, 0);
    dir.track(pid);
    log
}

#[test]
fn an_exit_is_kept_for_its_conversation_and_continued_by_resume() {
    let (mut dir, mut client) = agent_core("agent-continue-resume", &[]);
    let id = ended_and_kept(&mut client, &mut dir, "resume", RESUME_AND_LOAD);
    let cwd = listed(&mut client, id).and_then(|info| info.cwd).expect("its directory");

    let (mut again, record) = initialized(&dir.path, "resume-again", RESUME_ONLY);
    again
        .step("expect session/resume")
        .step("respond {}")
        .step("expect session/prompt")
        .step(said("second"))
        .step(stop("end_turn"));
    client.restart(id, &again.write()).unwrap();
    let log = ready_again(&mut client, &mut dir, id);
    assert_in_order(&client.events(id), &[
        "state Exited",
        &format!("notice {RESTARTED}"),
        "state Starting",
        "conversation s1",
        "state Ready",
    ]);
    // The new agent was sent `initialize`, then `session/resume` for the
    // conversation in its directory, with no MCP servers; never
    // `session/new`.
    let sent = recorded(&record);
    assert_eq!(methods(&record), ["initialize", "session/resume"]);
    assert_eq!(sent[1]["params"], json!({"sessionId": "s1", "cwd": cwd, "mcpServers": []}));
    // Listed with the command it was continued with.
    let info = listed(&mut client, id).expect("listed");
    assert!(info.command.contains("resume-again.script"), "{}", info.command);
    assert_eq!(info.cwd.as_deref(), Some(cwd.as_str()));

    // Entry ids follow the log's last.
    let next = log.next_id();
    client.prompt(id, "two").unwrap();
    let log = client.transcript_until(id, "has the second turn", ended(2));
    assert_eq!(says(&log), ["user: one", "agent: first", "user: two", "agent: second"]);
    let two = log
        .entries()
        .find_map(|(entry, kept)| matches!(kept, Entry::User { text } if text == "two").then_some(entry));
    assert_eq!(two, Some(next));
    assert_eq!(prompts(&record), ["two"]);
}

#[test]
fn a_load_only_agent_continues_with_its_replay_muted() {
    let (mut dir, mut client) = agent_core("agent-continue-load", &[]);
    let id = ended_and_kept(&mut client, &mut dir, "load", LOAD_ONLY);

    let (mut again, record) = initialized(&dir.path, "load-again", LOAD_ONLY);
    again
        .step("expect session/load")
        .step(user_said("one"))
        .step(said("first"))
        .step(said(" again"))
        .step("respond {}");
    client.restart(id, &again.write()).unwrap();
    let log = ready_again(&mut client, &mut dir, id);
    assert_eq!(methods(&record), ["initialize", "session/load"]);
    // The turn once, and the notice.
    assert_eq!(says(&log), ["user: one", "agent: first"]);
    assert_eq!(
        notices(&log).last().map(String::as_str),
        Some("conversation continued; the agent's replay of 3 updates wasn't shown")
    );
}

/// `remote-login-environment` 4.5: a restarted agent runs with exactly the
/// environment its `AGENT_RESTART` carries; the core adds nothing, so the
/// endpoint's environment is the client's to merge. Linux, as remote
/// endpoints are (a Windows program needs more than two variables to start).
#[cfg(target_os = "linux")]
#[test]
fn a_restarted_agent_gets_exactly_the_restarts_environment() {
    let (mut dir, mut client) = agent_core("agent-continue-env", &[]);
    let id = ended_and_kept(&mut client, &mut dir, "env", LOAD_ONLY);

    let env_file = dir.path.join("restart.env");
    let (mut again, _) = initialized(&dir.path, "env-again", LOAD_ONLY);
    again
        .step(format!("record-env {}", env_file.display()))
        .step("expect session/load")
        .step("respond {}");
    let script = again.write().display().to_string();
    let env = std::collections::BTreeMap::from([
        ("HOME".to_string(), "/x".to_string()),
        ("ONLY".to_string(), "1".to_string()),
    ]);
    client.restart_in(id, FAKE_AGENT, &[&script], env).unwrap();
    ready_again(&mut client, &mut dir, id);
    let recorded = std::fs::read_to_string(&env_file).expect("the restarted agent's environment");
    assert_eq!(recorded.lines().collect::<Vec<_>>(), ["HOME=/x", "ONLY=1"]);
}

#[test]
fn a_holder_killed_under_a_continuable_agent_leaves_it_to_continue() {
    let (mut dir, mut client) = agent_core("agent-continue-holder", &[]);
    let (script, _) = continuable(&dir.path, "holder", LOAD_ONLY);
    let (id, pid) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    let holder = parent(pid).expect("the agent's holder");
    kill(holder);

    let log = kept(&mut client, id, "has lost the agent", |log| matches!(log.agent(), AgentState::Lost { .. }));
    assert_eq!(*log.agent(), AgentState::Lost { message: "its holder ended".into() });

    let (mut again, record) = initialized(&dir.path, "holder-again", LOAD_ONLY);
    again.step("expect session/load").step(said("hello")).step("respond {}");
    client.restart(id, &again.write()).unwrap();
    let log = ready_again(&mut client, &mut dir, id);
    assert_eq!(methods(&record), ["initialize", "session/load"]);
    assert_eq!(
        notices(&log).last().map(String::as_str),
        Some("conversation continued; the agent's replay of 1 update wasn't shown")
    );
}

#[test]
fn a_continuing_spawn_folds_the_agents_replay_into_a_new_log() {
    let (mut dir, mut client) = agent_core("agent-continue-fresh", &[]);
    let (mut script, record) = initialized(&dir.path, "fresh", RESUME_AND_LOAD);
    script.step("expect session/load").step(user_said("hi")).step(said("hello")).step("respond {}");
    let cwd = dir.path.display().to_string();
    let spawn = client::SpawnAgent {
        conversation: Some(client::AgentConversation { id: "s1".into(), cwd: cwd.clone() }),
        ..client_agent(&script.write())
    };
    let (id, _) = spawned(&mut client, &mut dir, spawn);
    let log = client.transcript_until(id, "is ready", ready);

    assert_eq!(
        notices(&log),
        ["this continues an earlier conversation; what follows is the agent's replay of it"]
    );
    assert_eq!(says(&log), ["user: hi", "agent: hello"]);
    let conversation = log.conversation().expect("the conversation");
    assert_eq!((conversation.id.as_str(), conversation.cwd.as_str()), ("s1", cwd.as_str()));
    assert_eq!(methods(&record), ["initialize", "session/load"]);
    assert_eq!(recorded(&record)[1]["params"], json!({"sessionId": "s1", "cwd": cwd, "mcpServers": []}));
    assert_eq!(listed(&mut client, id).and_then(|info| info.cwd), Some(cwd));
}

/// Continues an ended session with an agent whose script is `steps` after
/// `initialize` (offering `capabilities`): its start fails with a message
/// `failure` checks, and the session ends; the agent was sent the methods
/// `sent`, and nothing else.
fn continue_fails(tag: &str, capabilities: &str, steps: &[&str], sent: &[&str], failure: impl FnOnce(&str)) {
    let (mut dir, mut client) = agent_core(tag, &[]);
    let id = ended_and_kept(&mut client, &mut dir, tag, RESUME_AND_LOAD);
    let (mut again, record) = initialized(&dir.path, &format!("{tag}-again"), capabilities);
    for step in steps {
        again.step(*step);
    }
    client.restart(id, &again.write()).unwrap();
    exit_of(&mut client, id);
    match agent_states(&client.events(id)).last() {
        Some(AgentState::Failed { message }) => failure(message),
        other => panic!("the last agent state is {other:?}"),
    }
    assert!(listed(&mut client, id).is_none());
    assert_eq!(methods(&record), sent, "never session/new");
}

#[test]
fn a_continue_offered_neither_fails_and_ends_the_session() {
    continue_fails("agent-continue-neither", NEITHER, &[], &["initialize"], |message| {
        assert_eq!(
            message,
            "the agent can't continue conversations: it offers neither session/load nor session/resume"
        );
    });
}

#[test]
fn a_resume_error_fails_the_agent_and_ends_the_session() {
    let steps = ["expect session/resume", r#"respond-error -32603 "no such session""#];
    let sent = ["initialize", "session/resume"];
    continue_fails("agent-continue-unknown", RESUME_AND_LOAD, &steps, &sent, |message| {
        assert_eq!(message, "the agent couldn't continue the conversation: no such session");
    });
}

#[test]
fn a_turn_in_flight_is_never_sent_again_and_a_held_prompt_is_once_ready() {
    let (mut dir, mut client) = agent_core("agent-continue-held", &[]);
    let (mut script, first) = continuable(&dir.path, "held", RESUME_AND_LOAD);
    script.step("expect session/prompt");
    let gate = script.gate();
    script.step("exit 1");
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "fix it").unwrap();
    client.prompt(id, "next").unwrap();
    prompted(&first, &["fix it"]);
    gate.open();
    let log = kept(&mut client, id, "has the agent's exit", agent_exited);
    assert_eq!(*log.turn(), TurnState::Failed { message: "the agent exited".into() });
    assert_eq!(*log.slot(), held("next", HoldReason::AgentStopped));

    let (mut again, record) = initialized(&dir.path, "held-again", RESUME_AND_LOAD);
    again.step("expect session/resume");
    let resume = again.gate();
    again
        .step("respond {}")
        .step("expect session/prompt")
        .step(said("done"))
        .step(stop("end_turn"));
    client.restart(id, &again.write()).unwrap();
    // Not before the continued agent is ready.
    let refused = client.resend(id).unwrap_err();
    assert_eq!(refused.message, format!("session {id}: the agent isn't ready"));
    resume.open();
    ready_again(&mut client, &mut dir, id);
    client.resend(id).unwrap();
    let log = client.transcript_until(id, "has the resent prompt's turn", |log| says(log).contains(&"agent: done".to_owned()));
    assert_eq!(*log.slot(), Slot::Empty);
    prompted(&record, &["next"]);
    assert_eq!(prompts(&first), ["fix it"]);
}

#[test]
fn a_pending_permission_at_the_exit_stays_cancelled_after_a_continue() {
    let (mut dir, mut client) = agent_core("agent-continue-permission", &[]);
    let (mut script, first) = continuable(&dir.path, "permission", RESUME_AND_LOAD);
    script.step("expect session/prompt").step(ask("perm-1"));
    let gate = script.gate();
    script.step("exit 1");
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "write").unwrap();
    client.transcript_until(id, "waits on the permission", blocked);
    gate.open();
    kept(&mut client, id, "has the agent's exit", agent_exited);

    let (mut again, record) = initialized(&dir.path, "permission-again", RESUME_AND_LOAD);
    again.step("expect session/resume").step("respond {}");
    client.restart(id, &again.write()).unwrap();
    let log = ready_again(&mut client, &mut dir, id);
    let asked = permissions(&log);
    let [(_, Entry::Permission { outcome, .. })] = asked.as_slice() else {
        panic!("{asked:?}");
    };
    assert_eq!(*outcome, Some(PermissionOutcome::Cancelled));
    assert!(responses(&first).is_empty() && responses(&record).is_empty());
}

#[test]
fn detaching_an_ended_session_ends_it() {
    let (mut dir, mut client) = agent_core("agent-continue-detach", &[]);
    let id = ended_and_kept(&mut client, &mut dir, "detach", RESUME_AND_LOAD);
    client.detach(id);
    assert!(listed(&mut client, id).is_none());
}

#[test]
fn closing_an_ended_session_ends_it_with_its_exit() {
    let (mut dir, mut client) = agent_core("agent-continue-close", &[]);
    let id = ended_and_kept(&mut client, &mut dir, "close", RESUME_AND_LOAD);
    client.send(client::ToCore::Close(client::SessionRef { session: id }));
    assert_eq!(exit_of(&mut client, id).code, Some(1));
    assert!(listed(&mut client, id).is_none());
}

#[test]
fn a_restart_is_refused_while_the_agent_runs_and_from_a_client_without_it() {
    let (mut dir, mut client) = agent_core("agent-continue-refused", &[]);
    let (mut script, _) = continuable(&dir.path, "refused", RESUME_AND_LOAD);
    let gate = script.gate();
    script.step("exit 1");
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    let (unused, record) = initialized(&dir.path, "refused-again", RESUME_AND_LOAD);
    let unused = unused.write();

    let refused = client.restart(id, &unused).unwrap_err();
    assert_eq!((refused.session, refused.message), (Some(id), format!("session {id}: the agent hasn't ended")));
    gate.open();
    kept(&mut client, id, "has the agent's exit", agent_exited);
    let mut other = TestClient::hello(&dir.path, "other");
    let refused = other.restart(id, &unused).unwrap_err();
    assert_eq!((refused.session, refused.message), (Some(id), format!("session {id} isn't attached to this client")));
    let refused = client.restart_with(id, "  ", &[]).unwrap_err();
    assert_eq!(refused.message, format!("session {id}: AGENT_RESTART needs a command"));
    // Nothing was started.
    assert!(!record.exists());
    assert!(listed(&mut client, id).is_some());
}

#[test]
fn an_unrunnable_restart_returns_the_session_to_ended_for_another_try() {
    let (mut dir, mut client) = agent_core("agent-continue-unrunnable", &[]);
    let id = ended_and_kept(&mut client, &mut dir, "unrunnable", RESUME_AND_LOAD);
    client.restart_with(id, "brindle-no-such-agent", &[]).unwrap();
    let log = kept(&mut client, id, "has the failed start", |log| {
        matches!(log.agent(), AgentState::Exited { code: None, stderr } if !stderr.is_empty())
    });
    assert_eq!(notices(&log).last().map(String::as_str), Some(RESTARTED));

    let (mut again, record) = initialized(&dir.path, "unrunnable-again", RESUME_AND_LOAD);
    again.step("expect session/resume").step("respond {}");
    client.restart(id, &again.write()).unwrap();
    ready_again(&mut client, &mut dir, id);
    assert_eq!(methods(&record), ["initialize", "session/resume"]);
}

// OpenSpec change `agents-login`, task 5.3: terminal login methods, and a
// restart while the agent needs a login (decision 8).

/// The notice a restart while a login is needed adds.
const RESTARTED_AFTER_LOGIN: &str = "the agent is started again after its login";

fn needs_any_login(log: &Transcript) -> bool {
    matches!(log.agent(), AgentState::LoginRequired { .. })
}

/// The `EXIT`s already sent for session `id`.
fn exits_of(client: &mut TestClient, id: SessionId) -> Vec<ToClient> {
    already_sent(client)
        .into_iter()
        .filter(|message| matches!(message, ToClient::Exit(exit) if exit.session == id))
        .collect()
}

#[test]
fn initialize_advertises_terminal_logins_and_a_terminal_method_is_the_clients_to_run() {
    let (mut dir, mut client) = agent_core("agent-login-terminal", &[]);
    let (mut script, record) = offering_logins(&dir.path, "login-terminal", NEITHER, TERMINAL_LOGIN);
    script.step("expect session/new").step(r#"respond-error -32000 "log in first""#);
    let (id, _) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    let log = client.transcript_until(id, "needs a login", needs_any_login);

    // `auth.terminal`, and no `fs` or `terminal` capability.
    assert_eq!(sent(&record, "initialize", 0)["params"]["clientCapabilities"], json!({"auth": {"terminal": true}}));
    let terminal = LoginMethod {
        id: "cli-login".into(),
        name: "Log in from the terminal".into(),
        description: None,
        kind: LoginKind::Terminal {
            args: vec!["login".into()],
            env: [("LOGIN_MODE".to_owned(), "device".to_owned())].into(),
        },
    };
    let AgentState::LoginRequired { methods: offered, .. } = log.agent() else {
        panic!("{:?}", log.agent());
    };
    assert_eq!(offered.iter().map(|method| method.id.as_str()).collect::<Vec<_>>(), ["oauth-personal", "cli-login"]);
    assert_eq!(offered[1], terminal);

    let refusal = client.login(id, "cli-login").unwrap_err();
    assert_eq!(refusal.message, format!("session {id}: login method \"cli-login\" runs in a terminal, which Brindle opens"));
    assert!(needs_any_login(&client.transcript(id)));
    assert_eq!(methods(&record), ["initialize", "session/new"]);
}

#[test]
fn a_restart_while_a_login_is_needed_at_the_start_runs_a_new_agent_with_session_new() {
    let (mut dir, mut client) = agent_core("agent-login-restart-new", &[]);
    let (mut script, first) = offering_logins(&dir.path, "restart-new", NEITHER, TERMINAL_LOGIN);
    script.step("expect session/new").step(r#"respond-error -32000 "log in first""#);
    let (id, old) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "needs a login", needs_any_login);

    let (mut again, record) = offering_logins(&dir.path, "restart-new-again", NEITHER, TERMINAL_LOGIN);
    again
        .step("expect session/new")
        .step(r#"respond {"sessionId":"s1"}"#)
        .step("expect session/prompt")
        .step(said("logged in"))
        .step(stop("end_turn"));
    client.restart(id, &again.write()).unwrap();
    ready_again(&mut client, &mut dir, id);
    let info = listed(&mut client, id).expect("listed");
    assert_ne!(info.pid, old, "a new agent process");
    assert!(info.command.contains("restart-new-again.script"), "{}", info.command);
    assert!(gone(old), "the old agent was hung up");

    // No exit in the log, and no `EXIT`: the session went on.
    let events = client.events(id);
    assert_in_order(&events, &[
        "state LoginRequired",
        &format!("notice {RESTARTED_AFTER_LOGIN}"),
        "state Starting",
        "conversation s1",
        "state Ready",
    ]);
    let states = agent_states(&events);
    assert!(!states.iter().any(|state| matches!(state, AgentState::Exited { .. })), "{states:?}");
    assert!(exits_of(&mut client, id).is_empty());
    // The new agent was sent `initialize` with `auth.terminal`, then
    // `session/new`; the old one nothing more.
    assert_eq!(methods(&first), ["initialize", "session/new"]);
    assert_eq!(methods(&record), ["initialize", "session/new"]);
    assert_eq!(sent(&record, "initialize", 0)["params"]["clientCapabilities"], json!({"auth": {"terminal": true}}));

    client.prompt(id, "hi").unwrap();
    let log = client.transcript_until(id, "has the turn's end", ended(1));
    assert_eq!(says(&log), ["user: hi", "agent: logged in"]);
}

#[test]
fn a_restart_after_a_prompts_login_demand_takes_up_the_same_session_keeping_the_log() {
    let (mut dir, mut client) = agent_core("agent-login-restart-resume", &[]);
    let (mut script, first) = offering_logins(&dir.path, "restart-resume", RESUME_AND_LOAD, TERMINAL_LOGIN);
    script
        .step("expect session/new")
        .step(r#"respond {"sessionId":"s1"}"#)
        .step("expect session/prompt")
        .step(said("first"))
        .step(stop("end_turn"))
        .step("expect session/prompt");
    let gate = script.gate();
    script.step(r#"respond-error -32000 "session expired""#);
    let (id, old) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "is ready", ready);
    client.prompt(id, "one").unwrap();
    client.transcript_until(id, "has the first turn", ended(1));
    client.prompt(id, "two").unwrap();
    client.prompt(id, "three").unwrap();
    gate.open();
    let before = client.transcript_until(id, "needs a login", needs_any_login);
    assert_eq!(*before.slot(), held("three", HoldReason::LoginRequired));

    let (mut again, record) = initialized(&dir.path, "restart-resume-again", RESUME_ONLY);
    again
        .step("expect session/resume")
        .step("respond {}")
        .step("expect session/prompt")
        .step(said("third"))
        .step(stop("end_turn"));
    client.restart(id, &again.write()).unwrap();
    let log = ready_again(&mut client, &mut dir, id);
    assert!(gone(old), "the old agent was hung up");
    assert!(exits_of(&mut client, id).is_empty());

    // The same ACP session is taken up, and the log and the held prompt
    // are kept.
    assert_eq!(methods(&record), ["initialize", "session/resume"]);
    assert_eq!(sent(&record, "session/resume", 0)["params"]["sessionId"], json!("s1"));
    assert_eq!(says(&log), says(&before));
    assert_eq!(*log.slot(), held("three", HoldReason::LoginRequired));
    assert_in_order(&client.events(id), &[
        "state LoginRequired",
        &format!("notice {RESTARTED_AFTER_LOGIN}"),
        "state Starting",
        "conversation s1",
        "state Ready",
    ]);

    client.resend(id).unwrap();
    client.transcript_until(id, "has the resent prompt's turn", |log| says(log).contains(&"agent: third".to_owned()));
    prompted(&record, &["three"]);
    assert_eq!(prompts(&first), ["one", "two"]);
}

#[test]
fn a_close_before_the_old_agent_exits_drops_the_restart() {
    let (mut dir, mut client) = agent_core("agent-login-restart-closed", &[]);
    let (mut script, _) = offering_logins(&dir.path, "restart-closed", NEITHER, TERMINAL_LOGIN);
    script.step("expect session/new").step(r#"respond-error -32000 "log in first""#);
    // Past the demand it waits here, reading nothing: the hang-up doesn't
    // end it until the gate opens.
    let gate = script.gate();
    let (id, old) = spawned(&mut client, &mut dir, client_agent(&script.write()));
    client.transcript_until(id, "needs a login", needs_any_login);

    let (again, record) = offering_logins(&dir.path, "restart-closed-again", NEITHER, TERMINAL_LOGIN);
    client.restart(id, &again.write()).unwrap();
    client.send(client::ToCore::Close(client::SessionRef { session: id }));
    gate.open();
    exit_of(&mut client, id);
    assert!(gone(old));
    assert!(listed(&mut client, id).is_none());
    assert!(!record.exists(), "no new agent was started");
    assert!(matches!(agent_states(&client.events(id)).last(), Some(AgentState::Exited { .. })));
}
