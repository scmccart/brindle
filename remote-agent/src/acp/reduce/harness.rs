use brindle_remote_protocol::serde_json::{self, json};
use brindle_transcript::{Activity, Continuation, LoginKind, StopReason, ToolItem};

use super::list::Page;
use super::*;

/// ACP v1's login demand, which the tests' agents answer with; the reducer
/// reads it only through `Dialect::failure`.
pub(super) const AUTH_REQUIRED: i64 = -32000;

/// An agent session under test, with a client's copy of its log folded
/// from every event `step` returned (checked equal to the session's
/// after each step) and every line written to the agent.
pub(super) struct Agent {
    pub(super) s: AgentSession,
    pub(super) client: Transcript,
    pub(super) now: Instant,
    pub(super) events: Vec<Event>,
    pub(super) sent: Vec<Value>,
    /// The emit guard's limit, `MAX_EVENT` unless a test lowers it.
    pub(super) event_limit: usize,
}

impl Agent {
    pub(super) fn new() -> Self {
        Self::in_dir("/home/u/src")
    }

    pub(super) fn in_dir(cwd: &str) -> Self {
        Self {
            s: AgentSession::new(cwd.into(), "0.3.0".into()),
            client: Transcript::new(),
            now: Instant::now(),
            events: Vec::new(),
            sent: Vec::new(),
            event_limit: MAX_EVENT,
        }
    }

    /// Spawned, `initialize` and `session/new` answered: ready in ACP
    /// session `s1`.
    pub(super) fn ready() -> Self {
        Self::ready_in("s1")
    }

    pub(super) fn ready_in(session: &str) -> Self {
        let mut agent = Self::new();
        agent.step(Input::Spawned);
        agent.feed(&result(
            1,
            json!({"protocolVersion": 1, "agentInfo": {"name": "fake", "version": "1.0"}}),
        ));
        agent.feed(&result(2, json!({"sessionId": session})));
        assert_eq!(
            *agent.s.phase(),
            Phase::Ready {
                session: session.into()
            }
        );
        agent
    }

    pub(super) fn step(&mut self, input: Input<'_>) -> Output {
        let out = run(&mut self.s, input, self.now, self.event_limit);
        for event in &out.events {
            assert_eq!(Event::from_value(&event.to_value()).as_ref(), Ok(event));
            self.client
                .apply(event)
                .unwrap_or_else(|e| panic!("{event:?}: {e}"));
        }
        assert_eq!(self.client, self.s.transcript, "the client's fold differs");
        assert_eq!(
            self.s.process.book.tools,
            tool_index(&self.s.transcript, self.s.process.book.tool_epoch),
            "the tool book isn't the log's tool entries from its epoch"
        );
        assert_eq!(
            pending_permissions(&self.s.transcript),
            self.s.rpc.waiting_permissions(),
            "the log's pending permission requests aren't the ones awaited"
        );
        if matches!(
            self.s.phase,
            Phase::Restarting { .. } | Phase::Refused | Phase::Broken | Phase::Exited | Phase::Lost
        ) {
            assert!(
                self.s.rpc.permissions().is_empty() && self.s.rpc.listing().is_none(),
                "a process the session is done with has requests out: {:?}",
                self.s.rpc
            );
        }
        for line in out.stdin.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            self.sent
                .push(serde_json::from_slice(line).expect("a JSON line"));
        }
        self.events.extend(out.events.iter().cloned());
        out
    }

    pub(super) fn feed(&mut self, line: &str) -> Output {
        self.step(Input::Stdout(format!("{line}\n").as_bytes()))
    }

    pub(super) fn cmd(&mut self, command: Command) -> Output {
        self.step(Input::Command(command))
    }

    pub(super) fn prompt(&mut self, text: &str) -> Output {
        self.cmd(Command::Prompt { text: text.into() })
    }

    /// The id of the last `session/prompt` written.
    pub(super) fn prompt_id(&self) -> u64 {
        self.last("session/prompt")["id"].as_u64().expect("an id")
    }

    pub(super) fn last(&self, method: &str) -> &Value {
        self.sent
            .iter()
            .rev()
            .find(|line| line["method"] == method)
            .unwrap_or_else(|| panic!("no {method} was sent"))
    }

    pub(super) fn sent_methods(&self) -> Vec<&str> {
        self.sent
            .iter()
            .filter_map(|line| line["method"].as_str())
            .collect()
    }

    /// Starts a turn with `text` and returns its prompt's id.
    pub(super) fn turn(&mut self, text: &str) -> u64 {
        assert_eq!(self.prompt(text).reply, Some(Ok(())));
        self.prompt_id()
    }

    pub(super) fn end(&mut self, id: u64, stop: &str) -> Output {
        self.feed(&result(id, json!({"stopReason": stop})))
    }

    pub(super) fn slot(&self) -> &Slot {
        self.s.transcript().slot()
    }

    pub(super) fn turn_state(&self) -> &TurnState {
        self.s.transcript.turn()
    }

    pub(super) fn agent_state(&self) -> &AgentState {
        self.s.transcript.agent()
    }

    pub(super) fn activity(&self) -> Activity {
        self.s.transcript.activity()
    }

    pub(super) fn entries(&self) -> Vec<Entry> {
        self.s
            .transcript
            .entries()
            .map(|(_, entry)| entry.clone())
            .collect()
    }

    pub(super) fn last_entry(&self) -> (EntryId, Entry) {
        self.s
            .transcript
            .entries()
            .last()
            .map(|(id, entry)| (id, entry.clone()))
            .expect("an entry")
    }

    pub(super) fn notices(&self) -> Vec<String> {
        self.entries()
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Notice { text, .. } => Some(text),
                Entry::User { .. }
                | Entry::Agent { .. }
                | Entry::Thought { .. }
                | Entry::Tool { .. }
                | Entry::Permission { .. } => None,
            })
            .collect()
    }

    /// Asks permission with request id `id` and these `(id, kind)`
    /// options; returns its entry.
    pub(super) fn ask(&mut self, id: Value, options: &[(&str, &str)]) -> EntryId {
        self.feed(&permission(id, "call_1", options));
        let (entry, _) = self.last_permission();
        entry
    }

    pub(super) fn last_permission(&self) -> (EntryId, Entry) {
        self.s
            .transcript
            .entries()
            .filter(|(_, entry)| matches!(entry, Entry::Permission { .. }))
            .last()
            .map(|(id, entry)| (id, entry.clone()))
            .expect("a permission entry")
    }
}

pub(super) fn result(id: u64, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

pub(super) fn error(id: u64, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

pub(super) fn update(update: Value) -> String {
    json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1", "update": update}})
        .to_string()
}

pub(super) fn chunk(kind: &str, text: &str) -> String {
    update(json!({"sessionUpdate": kind, "content": {"type": "text", "text": text}}))
}

pub(super) fn said(text: &str) -> String {
    chunk("agent_message_chunk", text)
}

pub(super) fn tool_call(call: &str, title: &str, status: &str) -> String {
    update(
        json!({"sessionUpdate": "tool_call", "toolCallId": call, "title": title, "kind": "edit", "status": status}),
    )
}

pub(super) fn tool_update(call: &str, status: &str) -> String {
    update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": status}))
}

pub(super) fn permission(id: Value, call: &str, options: &[(&str, &str)]) -> String {
    let options: Vec<Value> = options
        .iter()
        .map(|(id, kind)| json!({"optionId": id, "name": id.to_uppercase(), "kind": kind}))
        .collect();
    json!({"jsonrpc": "2.0", "id": id, "method": "session/request_permission",
           "params": {"sessionId": "s1", "toolCall": {"toolCallId": call, "title": "Write a file"}, "options": options}})
        .to_string()
}

pub(super) const ALLOW_REJECT: [(&str, &str); 2] =
    [("allow-once", "allow_once"), ("reject-once", "reject_once")];

pub(super) fn waiting(text: &str) -> Slot {
    Slot::Waiting { text: text.into() }
}

pub(super) fn held(text: &str, reason: HoldReason) -> Slot {
    Slot::Held {
        text: text.into(),
        reason,
    }
}

pub(super) fn ended(stop: StopReason) -> TurnState {
    TurnState::Ended { stop }
}

/// The events of kinds that order a turn, as short names.
pub(super) fn shape(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|event| match event {
            Event::Added { entry, .. } => format!("added {}", entry_kind(entry)),
            Event::Appended { .. } => "appended".into(),
            Event::ToolChanged { .. } => "tool changed".into(),
            Event::ToolItems { .. } => "tool items".into(),
            Event::ToolOutput { .. } => "tool output".into(),
            Event::Answered { .. } => "answered".into(),
            Event::Queue { slot } => match slot {
                Slot::Empty => "queue empty".into(),
                Slot::Waiting { .. } => "queue waiting".into(),
                Slot::Held { .. } => "queue held".into(),
            },
            Event::Turn { state } => format!("turn {}", turn_name(state)),
            Event::Agent { state } => format!("agent {}", agent_name(state)),
            Event::Conversation { .. } => "conversation".into(),
            Event::Trimmed { .. } => "trimmed".into(),
            Event::Window { .. } => "window".into(),
            Event::Commands { commands } => format!("commands {}", commands.len()),
            Event::Options { options } => format!("options {}", options.len()),
        })
        .collect()
}

pub(super) fn entry_kind(entry: &Entry) -> &'static str {
    match entry {
        Entry::User { .. } => "user",
        Entry::Agent { .. } => "agent",
        Entry::Thought { .. } => "thought",
        Entry::Tool { .. } => "tool",
        Entry::Permission { .. } => "permission",
        Entry::Notice { .. } => "notice",
    }
}

pub(super) fn turn_name(state: &TurnState) -> &'static str {
    match state {
        TurnState::Idle => "idle",
        TurnState::Working => "working",
        TurnState::Blocked => "blocked",
        TurnState::Cancelling => "cancelling",
        TurnState::Ended { .. } => "ended",
        TurnState::Failed { .. } => "failed",
    }
}

pub(super) fn agent_name(state: &AgentState) -> &'static str {
    match state {
        AgentState::Starting => "starting",
        AgentState::Ready { .. } => "ready",
        AgentState::LoginRequired { .. } => "login required",
        AgentState::LoggingIn { .. } => "logging in",
        AgentState::Failed { .. } => "failed",
        AgentState::Exited { .. } => "exited",
        AgentState::Lost { .. } => "lost",
        AgentState::Choosing { .. } => "choosing",
    }
}

pub(super) fn fixture_line(file: &str, n: usize) -> String {
    let text = match file {
        "initialize-32022" => include_str!("../../../tests/fixtures/acp/initialize-32022.jsonl"),
        "initialize-v2-reply" => {
            include_str!("../../../tests/fixtures/acp/initialize-v2-reply.jsonl")
        }
        "prompt-auth-required" => {
            include_str!("../../../tests/fixtures/acp/prompt-auth-required.jsonl")
        }
        other => panic!("no fixture {other}"),
    };
    text.lines().nth(n).expect("a line").to_owned()
}

// --- tool content (`agents-tool-content` decisions 3 to 7) ---------

pub(super) fn tool_entry(agent: &Agent, call: &str) -> Entry {
    let entry = *agent.s.process.book.tools.get(call).expect("a tool entry");
    agent.s.transcript.entry(entry).cloned().expect("held")
}

pub(super) fn content_of(entry: &Entry) -> Vec<ToolItem> {
    match entry {
        Entry::Tool { content, .. } => content.clone(),
        other => panic!("{other:?}"),
    }
}

pub(super) fn diff_item(path: &str, old: &str, new: &str) -> Value {
    json!({"type": "diff", "path": path, "oldText": old, "newText": new})
}

pub(super) fn meta(call: &str, meta: Value) -> String {
    update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "_meta": meta}))
}

// --- the conversation's title (agents-session-title) ---------------

pub(super) fn titled(title: Value) -> String {
    update(json!({"sessionUpdate": "session_info_update", "title": title}))
}

pub(super) fn title_of(agent: &Agent) -> Option<&str> {
    agent
        .s
        .transcript()
        .conversation()
        .and_then(|conversation| conversation.title.as_deref())
}

// --- continuing a conversation (agents-session-recovery) -------------

/// `initialize`'s answer offering `capabilities`.
pub(super) fn initialized(id: u64, capabilities: Value) -> String {
    result(
        id,
        json!({"protocolVersion": 1, "agentCapabilities": capabilities,
               "agentInfo": {"name": "fake", "version": "2.0"}}),
    )
}

pub(super) fn both() -> Value {
    json!({"loadSession": true, "sessionCapabilities": {"resume": {}}})
}

pub(super) fn load_only() -> Value {
    json!({"loadSession": true, "sessionCapabilities": {"list": {}}})
}

pub(super) fn resume_only() -> Value {
    json!({"sessionCapabilities": {"resume": {}}})
}

pub(super) fn conversation_s1(continuation: Continuation) -> Conversation {
    Conversation {
        id: "s1".into(),
        cwd: "/home/u/src".into(),
        continuation,
        title: None,
    }
}

impl Agent {
    /// Ready in ACP session `s1`, its agent offering `capabilities`.
    pub(super) fn ready_offering(capabilities: Value) -> Self {
        let mut agent = Self::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, capabilities));
        agent.feed(&result(2, json!({"sessionId": "s1"})));
        agent
    }

    /// Ready offering both, one turn done ("first" → "Done."), then the
    /// agent exited with status 1.
    pub(super) fn ended() -> Self {
        let mut agent = Self::ready_offering(both());
        let id = agent.turn("first");
        agent.feed(&said("Done."));
        agent.end(id, "end_turn");
        agent.exit();
        agent
    }

    pub(super) fn exit(&mut self) -> Output {
        self.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        })
    }

    /// `AGENT_RESTART`, then the holder's `SPAWNED`: returns the
    /// `initialize` id sent.
    pub(super) fn restart(&mut self) -> u64 {
        assert_eq!(self.step(Input::Restart).reply, Some(Ok(())));
        self.step(Input::Spawned);
        self.last("initialize")["id"].as_u64().expect("an id")
    }

    /// The lines sent from the `n`th on.
    pub(super) fn sent_since(&self, n: usize) -> &[Value] {
        &self.sent[n..]
    }
}

// agents-login 3.3: logging in.

/// Two agent methods and a terminal one.
pub(super) fn auth_methods() -> Value {
    json!([
        {"id": "oauth-personal", "name": "Log in with Google", "description": "Use your\u{1b}[31m Google account"},
        {"id": "api-key", "name": "API Key"},
        {"id": "cli", "name": "CLI", "type": "terminal", "args": ["--login"], "env": {"MODE": "device"}},
    ])
}

pub(super) fn methods() -> Vec<LoginMethod> {
    vec![
        LoginMethod {
            id: "oauth-personal".into(),
            name: "Log in with Google".into(),
            description: Some("Use your\u{FFFD}[31m Google account".into()),
            kind: LoginKind::Agent,
        },
        LoginMethod {
            id: "api-key".into(),
            name: "API Key".into(),
            description: None,
            kind: LoginKind::Agent,
        },
        LoginMethod {
            id: "cli".into(),
            name: "CLI".into(),
            description: None,
            kind: LoginKind::Terminal {
                args: vec!["--login".into()],
                env: [("MODE".to_owned(), "device".to_owned())].into(),
            },
        },
    ]
}

pub(super) fn initialized_with_methods(id: u64, capabilities: Value) -> String {
    result(
        id,
        json!({"protocolVersion": 1, "agentCapabilities": capabilities,
               "agentInfo": {"name": "fake", "version": "2.0"}, "authMethods": auth_methods()}),
    )
}

pub(super) fn login_required(message: &str) -> AgentState {
    AgentState::LoginRequired {
        message: message.into(),
        methods: methods(),
    }
}

pub(super) fn logging_in() -> AgentState {
    AgentState::LoggingIn {
        method: "oauth-personal".into(),
        name: "Log in with Google".into(),
    }
}

impl Agent {
    /// `initialize` (id 1) answered with [`auth_methods`] and
    /// `session/new` (id 2) with `-32000`.
    pub(super) fn needing_login() -> Self {
        let mut agent = Self::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized_with_methods(1, both()));
        agent.feed(&error(2, AUTH_REQUIRED, "Authentication required"));
        agent
    }

    /// Ready in `s1`, its agent having advertised [`auth_methods`].
    pub(super) fn ready_with_methods() -> Self {
        let mut agent = Self::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized_with_methods(1, both()));
        agent.feed(&result(2, json!({"sessionId": "s1"})));
        agent
    }

    pub(super) fn login(&mut self, method: &str) -> Output {
        self.cmd(Command::Login {
            method: method.into(),
        })
    }

    /// The id of the last `authenticate` written.
    pub(super) fn authenticate_id(&self) -> u64 {
        self.last("authenticate")["id"].as_u64().expect("an id")
    }
}

pub(super) fn listing_offered() -> Value {
    json!({"loadSession": true, "sessionCapabilities": {"resume": {}, "list": {}}})
}

impl Agent {
    /// A choosing start in `/home/u/src`, its `initialize` (id 1)
    /// answered offering `capabilities`.
    pub(super) fn choosing_offering(capabilities: Value) -> Self {
        let mut agent = Self::new();
        agent.s = AgentSession::choosing("/home/u/src".into(), "0.3.0".into());
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, capabilities));
        agent
    }

    pub(super) fn choose(&mut self, conversation: Option<&str>) -> Output {
        self.cmd(Command::Choose {
            conversation: conversation.map(str::to_owned),
        })
    }

    pub(super) fn list(&mut self, req: RequestId, cursor: Option<&str>) -> Output {
        self.cmd(Command::List {
            req,
            cursor: cursor.map(str::to_owned),
        })
    }
}

pub(super) fn listed_ok(out: &Output) -> &Page {
    match &out.listed {
        Some(ListAnswer { result: Ok(page), .. }) => page,
        other => panic!("no page: {other:?}"),
    }
}
