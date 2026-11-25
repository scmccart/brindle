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
use brindle_transcript::{AgentState, Entry, StopReason, TurnState};

use support::fake_agent::{AgentScript, recorded};
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
        let mut child = Command::new(FAKE_AGENT)
            .arg(script)
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

// 7.1–7.3: agent sessions through the core.

/// A fake agent that starts as `fake` 1.0, takes session `s1`, and answers
/// one prompt per entry of `answers` with that text and `end_turn`; then it
/// idles until its stdin closes. Gives the script and its record.
fn agent_script(dir: &TempDir, name: &str, answers: &[&str]) -> (PathBuf, PathBuf) {
    let mut script = AgentScript::new(&dir.path, name);
    let record = script.record();
    script
        .step("expect initialize")
        .step(r#"respond {"protocolVersion":1,"agentInfo":{"name":"fake","version":"1.0"}}"#)
        .step("expect session/new")
        .step(r#"respond {"sessionId":"s1"}"#);
    for answer in answers {
        let chunk = json!({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": "s1",
            "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": answer}},
        }});
        script
            .step("expect session/prompt")
            .step(format!("send {chunk}"))
            .step(r#"respond {"stopReason":"end_turn"}"#);
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
    assert!(sessions.iter().all(|info| info.session != agent && info.agent.is_none()), "{sessions:?}");
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

#[test]
fn a_panic_in_acp_handling_fails_only_its_session() {
    const SWITCH: &str = "BRINDLE_TEST_ACP_PANIC";
    if skip_without_switch(SWITCH) {
        return;
    }
    let marker = "acp-panic-marker";
    let (mut dir, mut client) = agent_core("agent-panic", &[(SWITCH, marker)]);
    let (script, _) = agent_script(&dir, "panic", &[]);
    let (broken, pid) = client.spawn_agent(client_agent(&script));
    dir.track(pid);
    client.transcript_until(broken, "is ready", ready);

    let refusal = client.prompt(broken, &format!("say {marker}")).unwrap_err();
    assert_eq!(refusal.message, format!("session {broken}: the terminal server's ACP handling failed"));
    let failed = |log: &Transcript| {
        matches!(log.agent(), AgentState::Failed { message } if message.contains(&format!("{SWITCH} matched")))
    };
    client.transcript_until(broken, "says its handling failed", failed);
    let info = listed(&mut client, broken).expect("still listed");
    assert_eq!(info.agent.map(|agent| agent.activity), Some(AgentActivity::Error));

    // The core runs on, and so do its other agent sessions.
    let (script, _) = agent_script(&dir, "fine", &["fine"]);
    let (fine, pid) = client.spawn_agent(client_agent(&script));
    dir.track(pid);
    client.transcript_until(fine, "is ready", ready);
    client.prompt(fine, "hi").unwrap();
    client.transcript_until(fine, "has the turn's end", ended(1));
    assert!(dir.logs().contains("ACP handling panicked"), "{}", dir.logs());
}
