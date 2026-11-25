//! Scripts for the fake ACP agent ([`super::FAKE_AGENT`], OpenSpec change
//! `agents-sessions`, design decision 16): [`AgentScript`] writes one into a
//! test's directory, with its record file and the file gates ([`Gate`]) the
//! test opens to let the agent go on, so a test orders the agent's steps
//! against its own without a sleep.

use std::fs;
use std::path::{Path, PathBuf};

use brindle_remote_protocol::serde_json::{self, Value, json};

/// A fake agent's script, built step by step and written with
/// [`AgentScript::write`]. Steps are the lines of `src/fake_acp/script.rs`.
pub struct AgentScript {
    dir: PathBuf,
    name: String,
    lines: Vec<String>,
    gates: usize,
    marks: usize,
    /// The file the last `record` step names.
    recording: Option<PathBuf>,
}

impl AgentScript {
    /// A script to be written as `<dir>/<name>.script`, its record and gates
    /// beside it.
    pub fn new(dir: &Path, name: &str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            name: name.to_owned(),
            lines: Vec::new(),
            gates: 0,
            marks: 0,
            recording: None,
        }
    }

    /// Adds a step, written as is (`expect initialize`, `respond {…}`).
    pub fn step(&mut self, line: impl Into<String>) -> &mut Self {
        self.lines.push(line.into());
        self
    }

    /// Adds a `record` step and gives its file, which [`recorded`] reads.
    pub fn record(&mut self) -> PathBuf {
        let path = self.dir.join(format!("{}.received.jsonl", self.name));
        self.lines.push(format!("record {}", path.display()));
        self.recording = Some(path.clone());
        path
    }

    /// Adds steps that create a new file when the agent gets to them, and
    /// gives its path: everything the agent wrote before them is written
    /// (each step flushes), so a test waits for the file to know it. It is
    /// a `record` step on the file, then one back on the record, if any.
    pub fn reached(&mut self) -> PathBuf {
        self.marks += 1;
        let path = self.dir.join(format!("{}.reached-{}", self.name, self.marks));
        let _ = fs::remove_file(&path);
        self.lines.push(format!("record {}", path.display()));
        if let Some(record) = &self.recording {
            self.lines.push(format!("record {}", record.display()));
        }
        path
    }

    /// Adds a `wait-file` step on a new gate: the agent goes past it once
    /// the test calls [`Gate::open`].
    pub fn gate(&mut self) -> Gate {
        self.gates += 1;
        let path = self.dir.join(format!("{}.gate-{}", self.name, self.gates));
        // A gate left from an earlier run would be open already.
        let _ = fs::remove_file(&path);
        self.lines.push(format!("wait-file {}", path.display()));
        Gate { path }
    }

    /// Writes the script and gives its path, the agent's argument.
    pub fn write(&self) -> PathBuf {
        let path = self.dir.join(format!("{}.script", self.name));
        let mut text = self.lines.join("\n");
        text.push('\n');
        fs::write(&path, text).unwrap_or_else(|error| panic!("writing {}: {error}", path.display()));
        path
    }
}

/// A file the fake agent waits for; creating it lets the agent go on.
pub struct Gate {
    path: PathBuf,
}

impl Gate {
    pub fn open(&self) {
        fs::write(&self.path, b"").unwrap_or_else(|error| panic!("opening gate {}: {error}", self.path.display()));
    }
}

/// The messages a fake agent recorded so far, each as JSON; none before the
/// file exists. A line still being written is left out.
pub fn recorded(path: &Path) -> Vec<Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("recorded {line:?}: {error}")))
        .collect()
}

/// The methods of the requests and notifications recorded so far.
pub fn methods(path: &Path) -> Vec<String> {
    recorded(path)
        .iter()
        .filter_map(|value| value["method"].as_str().map(str::to_owned))
        .collect()
}

/// The texts of the prompts recorded so far, in the order they came.
pub fn prompts(path: &Path) -> Vec<String> {
    recorded(path)
        .iter()
        .filter(|value| value["method"] == "session/prompt")
        .map(|value| value["params"]["prompt"][0]["text"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The responses to the agent's own requests recorded so far
/// (`expect-response`).
pub fn responses(path: &Path) -> Vec<Value> {
    recorded(path).into_iter().filter(|value| value.get("method").is_none()).collect()
}

/// A script that starts as `fake` 1.0 and takes ACP session `s1`, its
/// record kept; the test adds the rest. Gives it and the record's path.
pub fn started(dir: &Path, name: &str) -> (AgentScript, PathBuf) {
    let mut script = AgentScript::new(dir, name);
    let record = script.record();
    script
        .step("expect initialize")
        .step(r#"respond {"protocolVersion":1,"agentInfo":{"name":"fake","version":"1.0"}}"#)
        .step("expect session/new")
        .step(r#"respond {"sessionId":"s1"}"#);
    (script, record)
}

/// What an agent's `initialize` may offer for continuing a conversation
/// (OpenSpec change `agents-session-recovery`), as its `agentCapabilities`.
pub const RESUME_AND_LOAD: &str = r#"{"loadSession":true,"sessionCapabilities":{"resume":{}}}"#;
pub const LOAD_ONLY: &str = r#"{"loadSession":true}"#;
pub const RESUME_ONLY: &str = r#"{"sessionCapabilities":{"resume":{}}}"#;
pub const NEITHER: &str = r#"{}"#;

/// A script whose agent answers `initialize` as `fake` 1.0 offering
/// `capabilities` (one of [`RESUME_AND_LOAD`] and the others), its record
/// kept; the test adds the rest (`session/new`, or a continue's
/// `session/resume` or `session/load`). Gives it and the record's path.
pub fn initialized(dir: &Path, name: &str, capabilities: &str) -> (AgentScript, PathBuf) {
    let mut script = AgentScript::new(dir, name);
    let record = script.record();
    script.step("expect initialize").step(format!(
        r#"respond {{"protocolVersion":1,"agentInfo":{{"name":"fake","version":"1.0"}},"agentCapabilities":{capabilities}}}"#
    ));
    (script, record)
}

/// As [`started`], offering `capabilities` in `initialize`: an agent whose
/// conversation `s1` can be continued, unless it offers [`NEITHER`].
pub fn continuable(dir: &Path, name: &str, capabilities: &str) -> (AgentScript, PathBuf) {
    let (mut script, record) = initialized(dir, name, capabilities);
    script.step("expect session/new").step(r#"respond {"sessionId":"s1"}"#);
    (script, record)
}

/// A `send` step of a `session/update` for `s1` with `update`.
pub fn update(update: Value) -> String {
    let message = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1", "update": update}});
    format!("send {message}")
}

/// A `send` step of an agent message chunk.
pub fn said(text: &str) -> String {
    update(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}))
}

/// A `send` step of a user message chunk, as an agent's `session/load`
/// replays one.
pub fn user_said(text: &str) -> String {
    update(json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": text}}))
}

/// A `send` step of a permission request with the JSON-RPC id `id` about
/// tool call `call_1`, offering `allow-once` and `reject-once`.
pub fn ask(id: &str) -> String {
    let options = json!([
        {"optionId": "allow-once", "name": "Allow", "kind": "allow_once"},
        {"optionId": "reject-once", "name": "Reject", "kind": "reject_once"},
    ]);
    let message = json!({"jsonrpc": "2.0", "id": id, "method": "session/request_permission", "params": {
        "sessionId": "s1",
        "toolCall": {"toolCallId": "call_1", "title": "Write a file"},
        "options": options,
    }});
    format!("send {message}")
}

/// A `respond` step ending the prompt's turn with `stop`.
pub fn stop(stop: &str) -> String {
    format!(r#"respond {{"stopReason":"{stop}"}}"#)
}
