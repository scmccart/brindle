use brindle_remote_protocol::serde_json::{self, Value, json};
use brindle_transcript::{AgentState, Entry, NoticeLevel};

use crate::acp::reduce::{Command, Input, Output, Phase, step};

use super::adopt::Adopted;
use super::*;

/// A fixed-seed xorshift, so the fuzz run is the same every time.
pub(super) struct XorShift(pub(super) u64);

impl XorShift {
    pub(super) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(super) fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

pub(super) fn line(value: Value) -> Vec<u8> {
    let mut line = value.to_string().into_bytes();
    line.push(b'\n');
    line
}

pub(super) fn result(id: u64, result: Value) -> Vec<u8> {
    line(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

pub(super) fn update(update: Value) -> Vec<u8> {
    line(json!({"jsonrpc": "2.0", "method": "session/update",
                "params": {"sessionId": "s1", "update": update}}))
}

pub(super) fn chunk(text: &str) -> Vec<u8> {
    update(json!({"sessionUpdate": "agent_message_chunk",
                  "content": {"type": "text", "text": text}}))
}

pub(super) fn permission(id: Value) -> Vec<u8> {
    line(
        json!({"jsonrpc": "2.0", "id": id, "method": "session/request_permission",
        "params": {"sessionId": "s1", "toolCall": {"toolCallId": "c1"},
            "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                        {"optionId": "reject", "name": "Reject", "kind": "reject_once"}]}}),
    )
}

/// The JSON lines in `stdin`.
pub(super) fn sent(stdin: &[u8]) -> Vec<Value> {
    stdin
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice(l).expect("a JSON line"))
        .collect()
}

pub(super) struct Agent {
    pub(super) s: AgentSession,
    pub(super) now: Instant,
}

impl Agent {
    pub(super) fn new() -> Self {
        Self {
            s: AgentSession::new("/home/u/src".into(), "0.3.0".into()),
            now: Instant::now(),
        }
    }

    /// Spawned, `initialize` (id 1, offering load and resume) and
    /// `session/new` (id 2) answered.
    pub(super) fn ready() -> Self {
        let mut agent = Self::new();
        agent.step(Input::Spawned);
        agent.feed(&result(
            1,
            json!({"protocolVersion": 1,
            "agentCapabilities": {"loadSession": true, "sessionCapabilities": {"resume": {}}},
            "agentInfo": {"name": "fake", "version": "1.0"}}),
        ));
        agent.feed(&result(2, json!({"sessionId": "s1"})));
        assert_eq!(
            agent.s.phase,
            Phase::Ready {
                session: "s1".into()
            }
        );
        agent
    }

    pub(super) fn step(&mut self, input: Input<'_>) -> Output {
        step(&mut self.s, input, self.now)
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) -> Output {
        self.step(Input::Stdout(bytes))
    }

    pub(super) fn prompt(&mut self, text: &str) -> Output {
        self.step(Input::Command(Command::Prompt { text: text.into() }))
    }

    /// Mid-turn (prompt 3 out) with a tool call, a waiting permission
    /// request (id `"p1"`, a string), a prompt waiting in the slot, a
    /// chunk run and half a line in progress.
    pub(super) fn busy() -> Self {
        let mut agent = Self::ready();
        assert_eq!(agent.prompt("first").reply, Some(Ok(())));
        agent.feed(&chunk("Working on "));
        agent.feed(&chunk("it"));
        agent.feed(&update(
            json!({"sessionUpdate": "tool_call", "toolCallId": "c1",
            "title": "Write", "kind": "edit", "status": "pending"}),
        ));
        agent.feed(&permission(json!("p1")));
        assert_eq!(agent.prompt("second").reply, Some(Ok(())));
        agent.feed(b"{\"jsonrpc\":\"2.0\",\"method\":\"session/up");
        agent
    }

    pub(super) fn round_trip(&self) -> AgentSession {
        let bytes = encode(&self.s).expect("it fits");
        decode(&bytes, self.now).expect("it reads")
    }
}

pub(super) fn entries(session: &AgentSession) -> Vec<Entry> {
    session
        .transcript
        .entries()
        .map(|(_, e)| e.clone())
        .collect()
}

/// Spawned and `initialize` (id 1, offering both ways to continue and
/// two login methods) answered; `session/new` (id 2) is out.
pub(super) fn creating() -> Agent {
    let mut agent = Agent::new();
    agent.step(Input::Spawned);
    agent.feed(&result(
        1,
        json!({"protocolVersion": 1,
        "agentCapabilities": {"loadSession": true, "sessionCapabilities": {"resume": {}}},
        "agentInfo": {"name": "fake", "version": "1.0"},
        "authMethods": auth_methods()}),
    ));
    agent
}

pub(super) fn auth_error(id: u64, message: &str) -> Vec<u8> {
    line(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message}}))
}

pub(super) fn login_cmd(agent: &mut Agent, method: &str) -> Output {
    agent.step(Input::Command(Command::Login {
        method: method.into(),
    }))
}

/// A format 1 checkpoint of `state` and the line bytes `partial`, as
/// builds before format 2 wrote them (still read).
pub(super) fn rebuild(state: &Value, partial: &[u8]) -> Vec<u8> {
    let state = state.to_string();
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&FORMAT_1.to_be_bytes());
    out.extend_from_slice(&(state.len() as u32).to_be_bytes());
    out.extend_from_slice(state.as_bytes());
    out.extend_from_slice(partial);
    out
}

/// A format 2 checkpoint's base record's state and line bytes.
pub(super) fn state_of(bytes: &[u8]) -> (Value, Vec<u8>) {
    assert_eq!(&bytes[4..6], &FORMAT.to_be_bytes());
    let mut rest = &bytes[PREFIX_LEN..];
    let base = next_record(&mut rest).expect("a record").expect("a base");
    assert_eq!(base.kind, BASE);
    let state = serde_json::from_slice(base.state).expect("JSON");
    (state, base.partial.to_vec())
}

pub(super) fn assert_not_restored(adopted: &Adopted) {
    let session = &adopted.session;
    assert_eq!(session.phase, Phase::Broken);
    assert_eq!(
        *session.transcript.agent(),
        AgentState::Failed {
            message: NOT_RESTORED.into()
        }
    );
    let entries = entries(session);
    assert!(
        matches!(entries.as_slice(), [Entry::Notice { level: NoticeLevel::Error, text }]
            if text.starts_with("the conversation could not be restored: ")),
        "{entries:?}"
    );
    assert_eq!(adopted.output, Output::default());
}

/// A stored checkpoint the core appends to, as the holder keeps it.
pub(super) struct Stored {
    pub(super) bytes: Vec<u8>,
    pub(super) seq: u32,
    pub(super) unsaved: Vec<Event>,
}

impl Stored {
    pub(super) fn whole(session: &AgentSession) -> Self {
        Self {
            bytes: encode(session).expect("it fits"),
            seq: 0,
            unsaved: Vec::new(),
        }
    }

    /// Notes what a step applied, as the core keeps `unsaved`.
    pub(super) fn saw(&mut self, out: Output) -> Output {
        self.unsaved.extend(out.events.iter().cloned());
        out
    }

    pub(super) fn append(&mut self, session: &AgentSession) {
        self.seq += 1;
        let segment = encode_segment(session, &self.unsaved, self.seq).expect("it fits");
        self.bytes.extend_from_slice(&segment);
        self.unsaved.clear();
    }

    pub(super) fn decode(&self, now: Instant) -> AgentSession {
        decode(&self.bytes, now).expect("it reads")
    }
}

pub(super) fn auth_methods() -> Value {
    json!([{"id": "oauth-personal", "name": "Log in with Google"},
           {"id": "cli", "name": "CLI", "type": "terminal", "args": ["--login"], "env": {"A": "1"}}])
}
