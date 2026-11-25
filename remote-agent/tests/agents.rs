//! Agent sessions (OpenSpec change `agents-sessions`). For now the fake ACP
//! agent itself (task 6.2): `brindle-fake-acp-agent` run on short scripts,
//! with its stdin, stdout and stderr piped to the test, on Linux and
//! Windows.

#![cfg(any(target_os = "linux", windows))]

mod support;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use brindle_remote_protocol::serde_json::{self, Value, json};

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
