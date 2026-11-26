//! An agent session's checkpoint (design decision 11 of the OpenSpec change
//! `agents-sessions`): what the core stores in the session's holder so that
//! the next core, after a crash or an upgrade, carries on the conversation.
//!
//! **The format.** A header, then the JSON state, then the raw bytes of the
//! stdout line in progress:
//!
//! | bytes | what |
//! | --- | --- |
//! | 4 | [`MAGIC`], `BACP` |
//! | 2 | the format, big-endian: [`FORMAT`] (1) |
//! | 4 | the JSON state's length in bytes, big-endian |
//! | that many | the JSON state: one object |
//! | the rest | the partial line's bytes, when [`Partial::Collecting`] |
//!
//! The state holds the log as the events of [`Transcript::replay`] (the
//! queue slot, the turn and the agent's state among them), the ACP version
//! (`acp`, always 1 here), the [`Phase`] (with the ACP session id once
//! ready, what a spawn starts or continues, what `initialize` offered, and
//! a load's muted count, and a login's message and what follows it), the
//! login methods the agent advertised (`methods`, left out when there are
//! none; OpenSpec change `agents-login`), the [`Rpc`] parts (the next outgoing id, the core's requests
//! awaiting a response and what each was for, the agent's waiting
//! permission requests by entry with their JSON-RPC ids as sent), the chunk
//! run, the tool entries by call id, the count of a bad-line run not yet
//! written as a notice, and the [`Lines`] state: which partial it is and
//! `covers`, every stdout byte fed, the partial line's included.
//!
//! **Bounds.** A checkpoint is at most [`MAX_CHECKPOINT_LEN`] (64 MiB, what a
//! holder keeps): the log's 16 MiB, an 8 MiB partial line, a 256 KiB prompt
//! and the bookkeeping fit with room to spare (decision 14), and
//! [`encode`] refuses one past it rather than send what the holder drops.
//!
//! **Reading one never panics** (`railway R5`). A checkpoint that isn't one,
//! that a newer core wrote in a format this one doesn't know, or whose state
//! doesn't hold together is adopted as a session whose conversation could
//! not be restored ([`adopt`]): a log holding one notice, the agent
//! `Failed` and left running, its lines ignored, so prompts are refused
//! rather than sent with lost ids.
//!
//! A whole pure module (`fcis` R2, R3; `PURE_MODULES`): time is an argument.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Instant;

use brindle_remote_protocol::holder::MAX_CHECKPOINT_LEN;
use brindle_remote_protocol::serde_json::{self, Map, Value, json};
use brindle_transcript::{
    AgentState, Continuation, Conversation, Entry, EntryId, Event, LoginMethod, NoticeLevel,
    Transcript,
};

use super::lines::{Lines, Partial};
use super::reduce::{
    self, AfterLogin, AgentSession, BAD_RUN_QUIET, BadRun, Greeting, Input, Log, Login, Output,
    Phase, Replay, Run, RunKind, Start,
};
use super::rpc::{BadParts, PROTOCOL_VERSION, Purpose, Rpc};
use super::wire::Id;

/// The four bytes a checkpoint starts with.
pub const MAGIC: [u8; 4] = *b"BACP";

/// The format [`encode`] writes and the newest [`decode`] reads.
pub const FORMAT: u16 = 1;

/// The magic, the format and the state's length.
const HEADER_LEN: usize = 4 + 2 + 4;

/// What the log of a session whose checkpoint couldn't be read says, and
/// its agent's failure.
pub const NOT_RESTORED: &str = "the conversation could not be restored";

/// [`encode`]'s result would be past [`MAX_CHECKPOINT_LEN`]; `len` is its
/// length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLarge {
    pub len: u64,
}

impl fmt::Display for TooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the agent session's checkpoint is {} bytes, over the limit of {} MiB",
            self.len,
            MAX_CHECKPOINT_LEN / (1024 * 1024)
        )
    }
}

/// Why a checkpoint can't be read; its text follows [`NOT_RESTORED`] in
/// the notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    /// Shorter than its header.
    Short { len: usize },
    /// It doesn't start with [`MAGIC`].
    NotACheckpoint,
    /// A format this core doesn't read: a newer core wrote it, or it is 0.
    Format { format: u16 },
    /// The state's length runs past the end.
    Truncated,
    /// The state isn't a JSON object.
    NotJson { error: String },
    /// A field is missing or has the wrong form.
    Field { name: &'static str },
    /// The conversation speaks another ACP version.
    Acp { version: u64 },
    /// A logged event doesn't decode or doesn't fold.
    Log { error: String },
    /// The request bookkeeping doesn't hold together.
    Rpc { parts: BadParts },
    /// The state names an entry the log doesn't have, or has of another
    /// kind.
    Entry { name: &'static str, entry: EntryId },
    /// Partial line bytes come with a partial that isn't being collected.
    StrayBytes { len: usize },
}

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short { len } => write!(f, "its checkpoint is only {len} bytes"),
            Self::NotACheckpoint => f.write_str("its checkpoint isn't an agent session's"),
            Self::Format { format } => write!(
                f,
                "its checkpoint is in format {format}, and this terminal server reads format {FORMAT}"
            ),
            Self::Truncated => f.write_str("its checkpoint is cut short"),
            Self::NotJson { error } => write!(f, "its checkpoint's state isn't JSON: {error}"),
            Self::Field { name } => write!(f, "its checkpoint's {name} is missing or malformed"),
            Self::Acp { version } => write!(
                f,
                "its conversation speaks ACP version {version}, and this terminal server speaks {PROTOCOL_VERSION}"
            ),
            Self::Log { error } => write!(f, "its checkpoint's log doesn't hold together: {error}"),
            Self::Rpc { parts } => {
                let why = match parts {
                    BadParts::IdNotBelowNext => "a request id at or past the next one",
                    BadParts::TooManyPending => "more waiting permission requests than allowed",
                };
                write!(f, "its checkpoint's requests don't hold together: {why}")
            }
            Self::Entry { name, entry } => {
                write!(
                    f,
                    "its checkpoint's {name} names entry {entry}, which the log doesn't hold"
                )
            }
            Self::StrayBytes { len } => {
                write!(
                    f,
                    "its checkpoint carries {len} bytes of a line it isn't collecting"
                )
            }
        }
    }
}

/// Encodes `session` whole, or refuses past [`MAX_CHECKPOINT_LEN`].
pub fn encode(session: &AgentSession) -> Result<Vec<u8>, TooLarge> {
    encode_within(session, MAX_CHECKPOINT_LEN)
}

fn encode_within(session: &AgentSession, limit: u64) -> Result<Vec<u8>, TooLarge> {
    let (partial, bytes): (Value, &[u8]) = match session.lines.partial() {
        Partial::Collecting(bytes) => (json!({ "partial": "collecting" }), bytes),
        Partial::TooLong { len } => (json!({ "partial": "too_long", "len": len }), &[]),
        Partial::Resyncing { len } => (json!({ "partial": "resyncing", "len": len }), &[]),
    };
    let mut lines = partial;
    if let Some(object) = lines.as_object_mut() {
        object.insert("covers".into(), session.lines.covers().into());
    }
    let events: Vec<Value> = session
        .transcript
        .replay()
        .iter()
        .map(Event::to_value)
        .collect();
    let mut state = json!({
        "acp": PROTOCOL_VERSION,
        "cwd": session.cwd,
        "version": session.version,
        "phase": phase_value(&session.phase),
        "log": events,
        "rpc": rpc_value(&session.rpc),
        "run": session.run.map(|run| json!({ "kind": run_kind_name(run.kind), "entry": run.entry.0 })),
        "tools": session.tools.iter().map(|(call, entry)| (call.clone(), Value::from(entry.0))).collect::<Map<String, Value>>(),
        "bad_lines": session.bad.map_or(0, |bad| bad.lines),
        "lines": lines,
    });
    if !session.methods.is_empty() {
        state["methods"] = json!(session.methods);
    }
    let state = state.to_string();
    let len = HEADER_LEN as u64 + state.len() as u64 + bytes.len() as u64;
    let state_len = u32::try_from(state.len()).map_err(|_| TooLarge { len })?;
    if len > limit {
        return Err(TooLarge { len });
    }
    let mut out = Vec::with_capacity(HEADER_LEN + state.len() + bytes.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT.to_be_bytes());
    out.extend_from_slice(&state_len.to_be_bytes());
    out.extend_from_slice(state.as_bytes());
    out.extend_from_slice(bytes);
    Ok(out)
}

/// A phase as the state's `phase` object. Of the fields
/// `agents-session-recovery` added, `start` is left out for a new
/// conversation, so its spawning and initializing phases encode as before;
/// a state written without them reads as a new conversation's, its
/// `creating` phase's agent offering nothing ([`restore_phase`]).
fn phase_value(phase: &Phase) -> Value {
    match phase {
        Phase::Spawning { start } => with_start(json!({ "phase": "spawning" }), start),
        Phase::Initializing { start } => with_start(json!({ "phase": "initializing" }), start),
        Phase::Restarting { start } => with_start(json!({ "phase": "restarting" }), start),
        Phase::Creating { greeting } => greeting_value("creating", greeting),
        Phase::Resuming {
            greeting,
            conversation,
            log,
        } => {
            let mut value = greeting_value("resuming", greeting);
            value["conversation"] = json!(conversation);
            value["log"] = json!(log_name(*log));
            value
        }
        Phase::Loading {
            greeting,
            conversation,
            replay,
        } => {
            let mut value = greeting_value("loading", greeting);
            value["conversation"] = json!(conversation);
            match replay {
                Replay::Muted { updates } => {
                    value["replay"] = json!("muted");
                    value["updates"] = json!(updates);
                }
                Replay::Shown => value["replay"] = json!("shown"),
            }
            value
        }
        Phase::Ready { session } => json!({ "phase": "ready", "session": session }),
        Phase::LoginRequired(login) => login_value("login_required", login),
        Phase::LoggingIn { login, method } => {
            let mut value = login_value("logging_in", login);
            value["method"] = json!(method);
            value
        }
        Phase::Refused => json!({ "phase": "refused" }),
        Phase::Broken => json!({ "phase": "broken" }),
        Phase::Exited => json!({ "phase": "exited" }),
        Phase::Lost => json!({ "phase": "lost" }),
    }
}

fn with_start(mut value: Value, start: &Start) -> Value {
    match start {
        Start::New => {}
        Start::Continue { conversation, log } => {
            value["start"] = json!({
                "conversation": conversation_value(conversation),
                "log": log_name(*log),
            });
        }
    }
    value
}

/// A login phase (`agents-login` decision 15): the greeting's fields, the
/// message, and how it goes on: `session` after a prompt's demand, else
/// the start sent again (`start` left out for a new conversation, as a
/// spawning phase's).
fn login_value(phase: &str, login: &Login) -> Value {
    let mut value = greeting_value(phase, &login.greeting);
    value["message"] = json!(login.message);
    match &login.then {
        AfterLogin::Start(start) => with_start(value, start),
        AfterLogin::Ready { session } => {
            value["session"] = json!(session);
            value
        }
    }
}

fn greeting_value(phase: &str, greeting: &Greeting) -> Value {
    json!({
        "phase": phase,
        "name": greeting.name,
        "version": greeting.version,
        "continuation": continuation_name(greeting.continuation),
    })
}

fn conversation_value(conversation: &Conversation) -> Value {
    json!({
        "id": conversation.id,
        "cwd": conversation.cwd,
        "continuation": continuation_name(conversation.continuation),
    })
}

fn continuation_name(continuation: Continuation) -> &'static str {
    match continuation {
        Continuation::ResumeAndLoad => "resume_and_load",
        Continuation::Resume => "resume",
        Continuation::Load => "load",
        Continuation::Neither => "neither",
    }
}

fn log_name(log: Log) -> &'static str {
    match log {
        Log::Kept => "kept",
        Log::Fresh => "fresh",
    }
}

fn purpose_name(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Initialize => "initialize",
        Purpose::NewSession => "new_session",
        Purpose::Prompt => "prompt",
        Purpose::Resume => "resume",
        Purpose::Load => "load",
        Purpose::Authenticate => "authenticate",
    }
}

fn run_kind_name(kind: RunKind) -> &'static str {
    match kind {
        RunKind::User => "user",
        RunKind::Agent => "agent",
        RunKind::Thought => "thought",
    }
}

fn rpc_value(rpc: &Rpc) -> Value {
    let outgoing: Vec<Value> = rpc
        .outgoing()
        .iter()
        .map(|(id, purpose)| json!([id, purpose_name(*purpose)]))
        .collect();
    let permissions: Vec<Value> = rpc
        .permissions()
        .iter()
        .map(|(entry, id)| json!([entry.0, id.to_value()]))
        .collect();
    json!({ "next_id": rpc.next_id(), "outgoing": outgoing, "permissions": permissions })
}

/// The session `bytes` encode, or why they don't. A bad-line run comes
/// back with `now` as its last line's time, so its notice is written a
/// quiet second after the adoption.
pub fn decode(bytes: &[u8], now: Instant) -> Result<AgentSession, Unreadable> {
    let header = bytes
        .get(..HEADER_LEN)
        .ok_or(Unreadable::Short { len: bytes.len() })?;
    let (magic, rest) = header.split_at(4);
    if magic != MAGIC {
        return Err(Unreadable::NotACheckpoint);
    }
    let (format, state_len) = rest.split_at(2);
    let format = u16::from_be_bytes([format[0], format[1]]);
    if format != FORMAT {
        return Err(Unreadable::Format { format });
    }
    let state_len = u32::from_be_bytes([state_len[0], state_len[1], state_len[2], state_len[3]]);
    let body = &bytes[HEADER_LEN..];
    let state_len = usize::try_from(state_len).map_err(|_| Unreadable::Truncated)?;
    if state_len > body.len() {
        return Err(Unreadable::Truncated);
    }
    let (state, partial) = body.split_at(state_len);
    let state: Value = serde_json::from_slice(state).map_err(|error| Unreadable::NotJson {
        error: error.to_string(),
    })?;
    let state = state.as_object().ok_or(Unreadable::NotJson {
        error: "not an object".into(),
    })?;
    restore(state, partial, now)
}

/// A field of `object`.
fn field<'a>(object: &'a Map<String, Value>, name: &'static str) -> Result<&'a Value, Unreadable> {
    object.get(name).ok_or(Unreadable::Field { name })
}

fn text<'a>(object: &'a Map<String, Value>, name: &'static str) -> Result<&'a str, Unreadable> {
    field(object, name)?
        .as_str()
        .ok_or(Unreadable::Field { name })
}

fn number(object: &Map<String, Value>, name: &'static str) -> Result<u64, Unreadable> {
    field(object, name)?
        .as_u64()
        .ok_or(Unreadable::Field { name })
}

fn optional_text(
    object: &Map<String, Value>,
    name: &'static str,
) -> Result<Option<String>, Unreadable> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(Unreadable::Field { name }),
    }
}

fn object<'a>(value: &'a Value, name: &'static str) -> Result<&'a Map<String, Value>, Unreadable> {
    value.as_object().ok_or(Unreadable::Field { name })
}

fn array<'a>(
    object: &'a Map<String, Value>,
    name: &'static str,
) -> Result<&'a [Value], Unreadable> {
    field(object, name)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or(Unreadable::Field { name })
}

/// A two-element array.
fn pair<'a>(value: &'a Value, name: &'static str) -> Result<(&'a Value, &'a Value), Unreadable> {
    match value.as_array().map(Vec::as_slice) {
        Some([first, second]) => Ok((first, second)),
        Some(_) | None => Err(Unreadable::Field { name }),
    }
}

fn restore(
    state: &Map<String, Value>,
    partial: &[u8],
    now: Instant,
) -> Result<AgentSession, Unreadable> {
    let acp = number(state, "acp")?;
    if acp != u64::from(PROTOCOL_VERSION) {
        return Err(Unreadable::Acp { version: acp });
    }
    let cwd = text(state, "cwd")?.to_owned();
    let version = text(state, "version")?.to_owned();
    let phase = restore_phase(object(field(state, "phase")?, "phase")?)?;
    let transcript = restore_log(array(state, "log")?)?;
    let rpc = restore_rpc(object(field(state, "rpc")?, "rpc")?)?;
    let run = field(state, "run")?;
    let run = if run.is_null() {
        None
    } else {
        Some(restore_run(object(run, "run")?)?)
    };
    let mut tools = BTreeMap::new();
    for (call, entry) in object(field(state, "tools")?, "tools")? {
        let entry = EntryId(entry.as_u64().ok_or(Unreadable::Field { name: "tools" })?);
        tools.insert(call.clone(), entry);
    }
    let bad_lines = number(state, "bad_lines")?;
    let lines = restore_lines(object(field(state, "lines")?, "lines")?, partial)?;
    let methods: Vec<LoginMethod> = match state.get("methods") {
        None => Vec::new(),
        Some(methods) => serde_json::from_value(methods.clone())
            .map_err(|_| Unreadable::Field { name: "methods" })?,
    };

    // Each entry the state names is the log's, of the kind it is named as.
    if let Some(run) = run {
        let kind_matches = match (run.kind, transcript.entry(run.entry)) {
            (RunKind::User, Some(Entry::User { .. }))
            | (RunKind::Agent, Some(Entry::Agent { .. }))
            | (RunKind::Thought, Some(Entry::Thought { .. })) => true,
            (RunKind::User | RunKind::Agent | RunKind::Thought, Some(_) | None) => false,
        };
        if !kind_matches {
            return Err(Unreadable::Entry {
                name: "run",
                entry: run.entry,
            });
        }
    }
    for entry in tools.values() {
        if !matches!(transcript.entry(*entry), Some(Entry::Tool { .. })) {
            return Err(Unreadable::Entry {
                name: "tools",
                entry: *entry,
            });
        }
    }
    for entry in rpc.permissions().keys() {
        if !matches!(
            transcript.entry(*entry),
            Some(Entry::Permission { outcome: None, .. })
        ) {
            return Err(Unreadable::Entry {
                name: "permissions",
                entry: *entry,
            });
        }
    }
    Ok(AgentSession {
        transcript,
        rpc,
        lines,
        phase,
        cwd,
        version,
        run,
        tools,
        bad: (bad_lines > 0).then_some(BadRun {
            lines: bad_lines,
            last: now,
        }),
        methods,
    })
}

fn restore_phase(phase: &Map<String, Value>) -> Result<Phase, Unreadable> {
    let name = "phase";
    Ok(match text(phase, name)? {
        "spawning" => Phase::Spawning {
            start: restore_start(phase)?,
        },
        "initializing" => Phase::Initializing {
            start: restore_start(phase)?,
        },
        "restarting" => Phase::Restarting {
            start: restore_start(phase)?,
        },
        "creating" => Phase::Creating {
            greeting: restore_greeting(phase)?,
        },
        "resuming" => Phase::Resuming {
            greeting: restore_greeting(phase)?,
            conversation: text(phase, "conversation")?.to_owned(),
            log: restore_log_kind(text(phase, "log")?)?,
        },
        "loading" => Phase::Loading {
            greeting: restore_greeting(phase)?,
            conversation: text(phase, "conversation")?.to_owned(),
            replay: match text(phase, "replay")? {
                "muted" => Replay::Muted {
                    updates: number(phase, "updates")?,
                },
                "shown" => Replay::Shown,
                _ => return Err(Unreadable::Field { name: "replay" }),
            },
        },
        "ready" => Phase::Ready {
            session: text(phase, "session")?.to_owned(),
        },
        "login_required" => Phase::LoginRequired(restore_login(phase)?),
        "logging_in" => Phase::LoggingIn {
            login: restore_login(phase)?,
            method: text(phase, "method")?.to_owned(),
        },
        "refused" => Phase::Refused,
        "broken" => Phase::Broken,
        "exited" => Phase::Exited,
        "lost" => Phase::Lost,
        _ => return Err(Unreadable::Field { name }),
    })
}

/// A spawning or initializing phase's start: new without a `start`.
fn restore_start(phase: &Map<String, Value>) -> Result<Start, Unreadable> {
    let name = "start";
    let start = match phase.get(name) {
        None | Some(Value::Null) => return Ok(Start::New),
        Some(start) => object(start, name)?,
    };
    let conversation = object(field(start, "conversation")?, "conversation")?;
    Ok(Start::Continue {
        conversation: Conversation {
            id: text(conversation, "id")?.to_owned(),
            cwd: text(conversation, "cwd")?.to_owned(),
            continuation: restore_continuation(Some(field(conversation, "continuation")?))?,
            // A start names the session to take up; its title is the
            // log's (`agents-session-title` decision 3).
            title: None,
        },
        log: restore_log_kind(text(start, "log")?)?,
    })
}

/// A login phase's [`Login`]: with a `session`, ready again in it after the
/// login; else the start, as a spawning phase's.
fn restore_login(phase: &Map<String, Value>) -> Result<Login, Unreadable> {
    let then = match optional_text(phase, "session")? {
        Some(session) => AfterLogin::Ready { session },
        None => AfterLogin::Start(restore_start(phase)?),
    };
    Ok(Login {
        greeting: restore_greeting(phase)?,
        message: text(phase, "message")?.to_owned(),
        then,
    })
}

/// What `initialize` said; a state without `continuation` predates it, and
/// its agent offers nothing.
fn restore_greeting(phase: &Map<String, Value>) -> Result<Greeting, Unreadable> {
    Ok(Greeting {
        name: optional_text(phase, "name")?,
        version: optional_text(phase, "version")?,
        continuation: restore_continuation(phase.get("continuation"))?,
    })
}

fn restore_continuation(value: Option<&Value>) -> Result<Continuation, Unreadable> {
    let name = "continuation";
    match value.map(Value::as_str) {
        None => Ok(Continuation::Neither),
        Some(Some("resume_and_load")) => Ok(Continuation::ResumeAndLoad),
        Some(Some("resume")) => Ok(Continuation::Resume),
        Some(Some("load")) => Ok(Continuation::Load),
        Some(Some("neither")) => Ok(Continuation::Neither),
        Some(Some(_) | None) => Err(Unreadable::Field { name }),
    }
}

fn restore_log_kind(log: &str) -> Result<Log, Unreadable> {
    match log {
        "kept" => Ok(Log::Kept),
        "fresh" => Ok(Log::Fresh),
        _ => Err(Unreadable::Field { name: "log" }),
    }
}

/// The log the replayed events fold into.
fn restore_log(events: &[Value]) -> Result<Transcript, Unreadable> {
    let mut transcript = Transcript::new();
    for value in events {
        let event = Event::from_value(value).map_err(|error| Unreadable::Log {
            error: error.to_string(),
        })?;
        transcript.apply(&event).map_err(|error| Unreadable::Log {
            error: error.to_string(),
        })?;
    }
    Ok(transcript)
}

fn restore_rpc(parts: &Map<String, Value>) -> Result<Rpc, Unreadable> {
    let next_id = number(parts, "next_id")?;
    let mut outgoing = BTreeMap::new();
    for value in array(parts, "outgoing")? {
        let name = "outgoing";
        let (id, purpose) = pair(value, name)?;
        let id = id.as_u64().ok_or(Unreadable::Field { name })?;
        let purpose = match purpose.as_str() {
            Some("initialize") => Purpose::Initialize,
            Some("new_session") => Purpose::NewSession,
            Some("prompt") => Purpose::Prompt,
            Some("resume") => Purpose::Resume,
            Some("load") => Purpose::Load,
            Some("authenticate") => Purpose::Authenticate,
            Some(_) | None => return Err(Unreadable::Field { name }),
        };
        outgoing.insert(id, purpose);
    }
    let mut permissions = BTreeMap::new();
    for value in array(parts, "permissions")? {
        let name = "permissions";
        let (entry, id) = pair(value, name)?;
        let entry = EntryId(entry.as_u64().ok_or(Unreadable::Field { name })?);
        let id = Id::parse(id).ok_or(Unreadable::Field { name })?;
        permissions.insert(entry, id);
    }
    Rpc::from_parts(next_id, outgoing, permissions).map_err(|parts| Unreadable::Rpc { parts })
}

fn restore_run(run: &Map<String, Value>) -> Result<Run, Unreadable> {
    let name = "run";
    let kind = match text(run, "kind").map_err(|_| Unreadable::Field { name })? {
        "user" => RunKind::User,
        "agent" => RunKind::Agent,
        "thought" => RunKind::Thought,
        _ => return Err(Unreadable::Field { name }),
    };
    let entry = EntryId(number(run, "entry").map_err(|_| Unreadable::Field { name })?);
    Ok(Run { kind, entry })
}

fn restore_lines(lines: &Map<String, Value>, bytes: &[u8]) -> Result<Lines, Unreadable> {
    let covers = number(lines, "covers")?;
    let partial = match text(lines, "partial")? {
        "collecting" => Partial::Collecting(bytes.to_vec()),
        kind @ ("too_long" | "resyncing") => {
            if !bytes.is_empty() {
                return Err(Unreadable::StrayBytes { len: bytes.len() });
            }
            let len = number(lines, "len")?;
            if kind == "too_long" {
                Partial::TooLong { len }
            } else {
                Partial::Resyncing { len }
            }
        }
        _ => return Err(Unreadable::Field { name: "partial" }),
    };
    Ok(Lines::restore(partial, covers))
}

/// A session whose checkpoint couldn't be read (decision 11): a log of one
/// notice saying why, the agent `Failed` with [`NOT_RESTORED`], and
/// [`Phase::Broken`], so its lines are ignored, prompts are refused and the
/// agent runs until the session is closed. `cwd` and `version` are the
/// session's directory and the core's version, which it never sends.
pub fn not_restored(why: &Unreadable, cwd: &str, version: &str) -> AgentSession {
    let mut session = AgentSession::new(cwd.to_owned(), version.to_owned());
    session.phase = Phase::Broken;
    let events = [
        Event::Added {
            id: EntryId::FIRST,
            entry: Entry::Notice {
                level: NoticeLevel::Error,
                text: format!("{NOT_RESTORED}: {}", reduce::clean(&why.to_string())),
            },
        },
        Event::Agent {
            state: AgentState::Failed {
                message: NOT_RESTORED.to_owned(),
            },
        },
    ];
    for event in &events {
        // A fresh log takes both: the notice is its first entry.
        let _ = session.transcript.apply(event);
    }
    session
}

/// An agent session adopted from its holder.
#[derive(Debug, Clone, PartialEq)]
pub struct Adopted {
    pub session: AgentSession,
    /// What adopting did, applied as [`reduce::step`]'s is: a truncated
    /// tail's notice and failed turn, a waiting prompt held, a never
    /// checkpointed session's `initialize`, and when to tick for a bad-line
    /// run that was waiting for its notice.
    pub output: Output,
    /// Whether the checkpoint was read, and why not.
    pub restored: Result<(), Unreadable>,
}

/// Adopts an agent session from its holder's `checkpoint`, whose tail was
/// `truncated` (its front cut, decision 11). The core then feeds the tail
/// through [`reduce::step`] as `Stdout`.
///
/// - A checkpoint that reads: the session as it was. After a truncated
///   tail, [`Input::OutputLost`] is stepped: a prompt awaiting its response
///   is forgotten and its turn `Failed`, a waiting prompt is held
///   (`OutputLost`), a held one stays as it was. One still spawning (the
///   core stores a continue's state with its spawn, change
///   `agents-session-recovery`) is started ([`Input::Spawned`]), so its
///   agent continues the conversation rather than start another.
/// - An empty one: the core never checkpointed the session, which it does
///   before it first writes to the agent, so the agent was sent nothing.
///   It is adopted as a new session in `cwd` by `version`, and started
///   ([`Input::Spawned`]: `initialize` goes out), after `OutputLost` when
///   the tail was truncated.
/// - One that doesn't read: [`not_restored`]. Its lines are ignored, so a
///   truncation changes nothing but where the next line starts.
pub fn adopt(
    checkpoint: &[u8],
    truncated: bool,
    cwd: &str,
    version: &str,
    now: Instant,
) -> Adopted {
    if checkpoint.is_empty() {
        let mut session = AgentSession::new(cwd.to_owned(), version.to_owned());
        let mut output = Output::default();
        if truncated {
            output = reduce::step(&mut session, Input::OutputLost, now);
        }
        merge(&mut output, reduce::step(&mut session, Input::Spawned, now));
        return Adopted {
            session,
            output,
            restored: Ok(()),
        };
    }
    match decode(checkpoint, now) {
        Ok(mut session) => {
            let mut output = if truncated {
                reduce::step(&mut session, Input::OutputLost, now)
            } else {
                Output {
                    wake: session.bad.map(|bad| bad.last + BAD_RUN_QUIET),
                    ..Output::default()
                }
            };
            // Stored with a continue's spawn, before its `SPAWNED`: the
            // agent was sent nothing, so it starts now, continuing.
            if let Phase::Spawning { .. } = session.phase {
                merge(&mut output, reduce::step(&mut session, Input::Spawned, now));
            }
            Adopted {
                session,
                output,
                restored: Ok(()),
            }
        }
        Err(why) => {
            let mut session = not_restored(&why, cwd, version);
            if truncated {
                session.lines.cut_front();
            }
            Adopted {
                session,
                output: Output::default(),
                restored: Err(why),
            }
        }
    }
}

/// `later`'s output after `earlier`'s, as one.
fn merge(earlier: &mut Output, later: Output) {
    earlier.events.extend(later.events);
    earlier.stdin.extend(later.stdin);
    if later.reply.is_some() {
        earlier.reply = later.reply;
    }
    earlier.checkpoint = match (earlier.checkpoint, later.checkpoint) {
        (Some(reduce::Moment::BeforeRequest), _) | (_, Some(reduce::Moment::BeforeRequest)) => {
            Some(reduce::Moment::BeforeRequest)
        }
        (first, None) => first,
        (_, Some(moment)) => Some(moment),
    };
    earlier.hang_up |= later.hang_up;
    earlier.wake = later.wake;
}

#[cfg(test)]
mod tests {
    use super::*;
    use brindle_transcript::{HoldReason, Slot, TurnState};
    use reduce::{Command, Refused, step};

    /// A fixed-seed xorshift, so the fuzz run is the same every time.
    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    fn line(value: Value) -> Vec<u8> {
        let mut line = value.to_string().into_bytes();
        line.push(b'\n');
        line
    }

    fn result(id: u64, result: Value) -> Vec<u8> {
        line(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    fn update(update: Value) -> Vec<u8> {
        line(json!({"jsonrpc": "2.0", "method": "session/update",
                    "params": {"sessionId": "s1", "update": update}}))
    }

    fn chunk(text: &str) -> Vec<u8> {
        update(json!({"sessionUpdate": "agent_message_chunk",
                      "content": {"type": "text", "text": text}}))
    }

    fn permission(id: Value) -> Vec<u8> {
        line(
            json!({"jsonrpc": "2.0", "id": id, "method": "session/request_permission",
            "params": {"sessionId": "s1", "toolCall": {"toolCallId": "c1"},
                "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                            {"optionId": "reject", "name": "Reject", "kind": "reject_once"}]}}),
        )
    }

    /// The JSON lines in `stdin`.
    fn sent(stdin: &[u8]) -> Vec<Value> {
        stdin
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_slice(l).expect("a JSON line"))
            .collect()
    }

    struct Agent {
        s: AgentSession,
        now: Instant,
    }

    impl Agent {
        fn new() -> Self {
            Self {
                s: AgentSession::new("/home/u/src".into(), "0.3.0".into()),
                now: Instant::now(),
            }
        }

        /// Spawned, `initialize` (id 1, offering load and resume) and
        /// `session/new` (id 2) answered.
        fn ready() -> Self {
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

        fn step(&mut self, input: Input<'_>) -> Output {
            step(&mut self.s, input, self.now)
        }

        fn feed(&mut self, bytes: &[u8]) -> Output {
            self.step(Input::Stdout(bytes))
        }

        fn prompt(&mut self, text: &str) -> Output {
            self.step(Input::Command(Command::Prompt { text: text.into() }))
        }

        /// Mid-turn (prompt 3 out) with a tool call, a waiting permission
        /// request (id `"p1"`, a string), a prompt waiting in the slot, a
        /// chunk run and half a line in progress.
        fn busy() -> Self {
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

        fn round_trip(&self) -> AgentSession {
            let bytes = encode(&self.s).expect("it fits");
            decode(&bytes, self.now).expect("it reads")
        }
    }

    fn entries(session: &AgentSession) -> Vec<Entry> {
        session
            .transcript
            .entries()
            .map(|(_, e)| e.clone())
            .collect()
    }

    // --- round trips ----------------------------------------------------

    #[test]
    fn a_new_session_round_trips() {
        let agent = Agent::new();
        assert_eq!(agent.round_trip(), agent.s);
    }

    #[test]
    fn a_session_mid_turn_round_trips_whole() {
        let agent = Agent::busy();
        assert_eq!(agent.s.rpc.permissions().len(), 1);
        assert!(matches!(agent.s.transcript.slot(), Slot::Waiting { .. }));
        assert!(agent.s.run.is_none(), "the tool call ended the run");
        assert_eq!(agent.round_trip(), agent.s);
    }

    #[test]
    fn a_chunk_run_round_trips_and_keeps_growing_its_entry() {
        let mut agent = Agent::ready();
        agent.prompt("go");
        agent.feed(&chunk("Hel"));
        let mut restored = agent.round_trip();
        assert_eq!(restored, agent.s);
        let out = step(&mut restored, Input::Stdout(&chunk("lo")), agent.now);
        assert!(
            matches!(out.events.as_slice(), [Event::Appended { text, .. }] if text == "lo"),
            "{:?}",
            out.events
        );
    }

    fn greeting(name: Option<&str>, version: Option<&str>, continuation: Continuation) -> Greeting {
        Greeting {
            name: name.map(Into::into),
            version: version.map(Into::into),
            continuation,
        }
    }

    fn continuing(log: Log) -> Start {
        Start::Continue {
            conversation: Conversation {
                id: "s1".into(),
                cwd: "/home/u/src".into(),
                continuation: Continuation::Load,
                title: None,
            },
            log,
        }
    }

    fn login(then: AfterLogin) -> Login {
        Login {
            greeting: greeting(Some("fake"), Some("1.0"), Continuation::ResumeAndLoad),
            message: "Authentication required".into(),
            then,
        }
    }

    #[test]
    fn every_phase_round_trips() {
        let phases = [
            Phase::Spawning { start: Start::New },
            Phase::Spawning {
                start: continuing(Log::Kept),
            },
            Phase::Initializing { start: Start::New },
            Phase::Initializing {
                start: continuing(Log::Fresh),
            },
            Phase::Creating {
                greeting: greeting(Some("fake"), None, Continuation::ResumeAndLoad),
            },
            Phase::Creating {
                greeting: greeting(None, Some("2"), Continuation::Neither),
            },
            Phase::Resuming {
                greeting: greeting(Some("fake"), Some("1.0"), Continuation::Resume),
                conversation: "s1".into(),
                log: Log::Kept,
            },
            Phase::Resuming {
                greeting: greeting(None, None, Continuation::ResumeAndLoad),
                conversation: "s1".into(),
                log: Log::Fresh,
            },
            Phase::Loading {
                greeting: greeting(Some("Copilot"), None, Continuation::Load),
                conversation: "s1".into(),
                replay: Replay::Muted { updates: 7 },
            },
            Phase::Loading {
                greeting: greeting(None, None, Continuation::ResumeAndLoad),
                conversation: "s1".into(),
                replay: Replay::Shown,
            },
            Phase::Ready {
                session: "s1".into(),
            },
            Phase::LoginRequired(login(AfterLogin::Start(Start::New))),
            Phase::LoginRequired(login(AfterLogin::Start(continuing(Log::Kept)))),
            Phase::LoginRequired(login(AfterLogin::Ready {
                session: "s1".into(),
            })),
            Phase::LoggingIn {
                login: login(AfterLogin::Start(continuing(Log::Fresh))),
                method: "oauth-personal".into(),
            },
            Phase::LoggingIn {
                login: login(AfterLogin::Ready {
                    session: "s1".into(),
                }),
                method: "chat-gpt".into(),
            },
            Phase::Restarting { start: Start::New },
            Phase::Restarting {
                start: continuing(Log::Kept),
            },
            Phase::Refused,
            Phase::Broken,
            Phase::Exited,
            Phase::Lost,
        ];
        for phase in phases {
            let mut agent = Agent::new();
            agent.s.phase = phase;
            assert_eq!(agent.round_trip(), agent.s);
        }
    }

    /// A state written before `agents-session-recovery`: no `start`, and a
    /// `creating` phase without `continuation`.
    #[test]
    fn an_older_shaped_state_reads_as_a_new_conversation_offering_nothing() {
        let agent = Agent::new();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        assert_eq!(state["phase"], json!({"phase": "spawning"}), "encoded as before");
        for (phase, expected) in [
            (json!({"phase": "spawning"}), Phase::Spawning { start: Start::New }),
            (
                json!({"phase": "initializing"}),
                Phase::Initializing { start: Start::New },
            ),
            (
                json!({"phase": "creating", "name": "fake", "version": null}),
                Phase::Creating {
                    greeting: greeting(Some("fake"), None, Continuation::Neither),
                },
            ),
        ] {
            state["phase"] = phase;
            let restored = decode(&rebuild(&state, &partial), agent.now).expect("it reads");
            assert_eq!(restored.phase, expected);
        }
    }

    #[test]
    fn a_malformed_continuing_phase_is_unreadable_naming_its_field() {
        let agent = Agent::new();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        for (phase, name) in [
            (json!({"phase": "spawning", "start": 7}), "start"),
            (
                json!({"phase": "spawning", "start": {"log": "kept"}}),
                "conversation",
            ),
            (
                json!({"phase": "spawning", "start": {"log": "sometimes",
                    "conversation": {"id": "s1", "cwd": "/", "continuation": "load"}}}),
                "log",
            ),
            (
                json!({"phase": "initializing", "start": {"log": "kept",
                    "conversation": {"id": "s1", "cwd": "/", "continuation": "maybe"}}}),
                "continuation",
            ),
            (
                json!({"phase": "resuming", "conversation": "s1", "log": "kept", "continuation": 1}),
                "continuation",
            ),
            (
                json!({"phase": "loading", "conversation": "s1", "replay": "muted"}),
                "updates",
            ),
            (
                json!({"phase": "loading", "conversation": "s1", "replay": "loud"}),
                "replay",
            ),
            (json!({"phase": "loading", "replay": "shown"}), "conversation"),
            (json!({"phase": "login_required", "name": "fake"}), "message"),
            (
                json!({"phase": "login_required", "message": "m", "session": 3}),
                "session",
            ),
            (json!({"phase": "logging_in", "message": "m"}), "method"),
            (json!({"phase": "logged_out"}), "phase"),
        ] {
            state["phase"] = phase.clone();
            assert_eq!(
                decode(&rebuild(&state, &partial), agent.now),
                Err(Unreadable::Field { name }),
                "{phase}"
            );
        }
    }

    // agents-login 3.4: the login methods and phases.

    fn auth_methods() -> Value {
        json!([{"id": "oauth-personal", "name": "Log in with Google"},
               {"id": "cli", "name": "CLI", "type": "terminal", "args": ["--login"], "env": {"A": "1"}}])
    }

    /// Spawned and `initialize` (id 1, offering both ways to continue and
    /// two login methods) answered; `session/new` (id 2) is out.
    fn creating() -> Agent {
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

    fn auth_error(id: u64, message: &str) -> Vec<u8> {
        line(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message}}))
    }

    fn login_cmd(agent: &mut Agent, method: &str) -> Output {
        agent.step(Input::Command(Command::Login {
            method: method.into(),
        }))
    }

    #[test]
    fn the_methods_round_trip_and_are_left_out_when_there_are_none() {
        let agent = creating();
        assert_eq!(agent.s.methods.len(), 2);
        let (state, _) = state_of(&encode(&agent.s).expect("it fits"));
        assert_eq!(state["methods"][1]["kind"], json!("terminal"));
        assert_eq!(agent.round_trip(), agent.s);
        let (state, _) = state_of(&encode(&Agent::new().s).expect("it fits"));
        assert!(state.get("methods").is_none(), "{state}");
        // Methods that don't read are unreadable.
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        state["methods"] = json!([{"id": "x"}]);
        assert_eq!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Field { name: "methods" })
        );
    }

    #[test]
    fn a_login_needed_at_the_start_round_trips_and_logs_in_after_restoring() {
        let mut agent = creating();
        agent.feed(&auth_error(2, "Authentication required"));
        assert!(matches!(
            &agent.s.phase,
            Phase::LoginRequired(Login {
                then: AfterLogin::Start(Start::New),
                ..
            })
        ));
        let (state, _) = state_of(&encode(&agent.s).expect("it fits"));
        assert_eq!(
            state["phase"],
            json!({"phase": "login_required", "name": "fake", "version": "1.0",
                   "continuation": "resume_and_load", "message": "Authentication required"})
        );
        let mut restored = Agent {
            s: agent.round_trip(),
            now: agent.now,
        };
        assert_eq!(restored.s, agent.s);
        let out = login_cmd(&mut restored, "oauth-personal");
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(sent(&out.stdin)[0]["method"], json!("authenticate"));
    }

    #[test]
    fn a_login_demanded_by_a_prompt_round_trips_with_its_session() {
        let mut agent = Agent::ready();
        agent.s.methods = creating().s.methods;
        assert_eq!(agent.prompt("hi").reply, Some(Ok(())));
        agent.feed(&auth_error(3, "expired"));
        let (state, _) = state_of(&encode(&agent.s).expect("it fits"));
        assert_eq!(state["phase"]["session"], json!("s1"));
        assert_eq!(state["phase"]["message"], json!("expired"));
        assert_eq!(agent.round_trip(), agent.s);
    }

    /// A core restarted while logging in carries the `authenticate` id, so
    /// the agent's answer to the earlier core's request is applied.
    #[test]
    fn a_login_running_round_trips_and_its_answer_is_applied_after_restoring() {
        let mut agent = creating();
        agent.feed(&auth_error(2, "Authentication required"));
        let out = login_cmd(&mut agent, "oauth-personal");
        let id = sent(&out.stdin)[0]["id"].as_u64().expect("an id");
        assert_eq!(id, 3);
        let (state, _) = state_of(&encode(&agent.s).expect("it fits"));
        assert_eq!(state["phase"]["phase"], json!("logging_in"));
        assert_eq!(state["phase"]["method"], json!("oauth-personal"));
        assert_eq!(state["rpc"]["outgoing"], json!([[3, "authenticate"]]));
        let mut restored = Agent {
            s: agent.round_trip(),
            now: agent.now,
        };
        assert_eq!(restored.s, agent.s);
        let out = restored.feed(&result(id, json!({})));
        assert_eq!(
            sent(&out.stdin)
                .iter()
                .map(|line| line["method"].clone())
                .collect::<Vec<_>>(),
            [json!("session/new")]
        );
        assert_eq!(*restored.s.transcript.agent(), AgentState::Starting);
        restored.feed(&result(4, json!({"sessionId": "s9"})));
        assert_eq!(
            restored.s.phase,
            Phase::Ready {
                session: "s9".into()
            }
        );
    }

    /// A login cancelled before a checkpoint is stored with its id
    /// forgotten: the late answer, after restoring, matches nothing.
    #[test]
    fn a_cancelled_login_round_trips_with_its_id_forgotten() {
        let mut agent = creating();
        agent.feed(&auth_error(2, "Authentication required"));
        login_cmd(&mut agent, "oauth-personal");
        agent.step(Input::Command(Command::Cancel));
        let (state, _) = state_of(&encode(&agent.s).expect("it fits"));
        assert_eq!(state["phase"]["phase"], json!("login_required"));
        assert_eq!(state["rpc"]["outgoing"], json!([]));
        let mut restored = Agent {
            s: agent.round_trip(),
            now: agent.now,
        };
        let out = restored.feed(&result(3, json!({})));
        assert!(out.stdin.is_empty() && out.events.is_empty(), "{out:?}");
        assert!(matches!(restored.s.phase, Phase::LoginRequired(_)));
    }

    /// Mid-load with a muted count, a permission request waiting: the
    /// adopted core finishes the load, notice and count included.
    #[test]
    fn a_checkpoint_taken_mid_load_round_trips_and_finishes_the_load() {
        let mut agent = Agent::ready();
        agent.prompt("first");
        agent.feed(&chunk("Done."));
        agent.feed(&result(3, json!({"stopReason": "end_turn"})));
        agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        });
        assert_eq!(agent.step(Input::Restart).reply, Some(Ok(())));
        agent.step(Input::Spawned);
        let out = agent.feed(&result(
            4,
            json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}),
        ));
        assert_eq!(sent(&out.stdin)[0]["method"], "session/load");
        agent.feed(&chunk("replayed"));
        agent.feed(&chunk(" twice"));
        agent.feed(&permission(json!(9)));
        assert!(matches!(
            agent.s.phase,
            Phase::Loading {
                replay: Replay::Muted { updates: 2 },
                ..
            }
        ));
        let mut restored = agent.round_trip();
        assert_eq!(restored, agent.s);
        let out = step(&mut restored, Input::Stdout(&result(5, json!({}))), agent.now);
        assert_eq!(restored.phase, Phase::Ready { session: "s1".into() });
        assert!(out.events.iter().any(|event| matches!(event,
            Event::Added { entry: Entry::Notice { text, .. }, .. }
                if text == "conversation continued; the agent's replay of 2 updates wasn't shown")));
        assert_eq!(restored.rpc.permissions().len(), 1, "the request still waits");
    }

    #[test]
    fn a_lost_agent_with_its_conversation_round_trips_continuable() {
        let mut agent = Agent::ready();
        agent.step(Input::Lost {
            message: "its holder ended",
        });
        assert!(agent.s.continuable());
        let restored = agent.round_trip();
        assert_eq!(restored, agent.s);
        assert!(restored.continuable());
        assert_eq!(
            restored.transcript.conversation().map(|c| c.id.as_str()),
            Some("s1")
        );
    }

    /// The conversation events in a checkpoint's log, as written.
    fn logged_conversations(bytes: &[u8]) -> Vec<Value> {
        let (state, _) = state_of(bytes);
        state["log"]
            .as_array()
            .expect("a log")
            .iter()
            .filter(|event| event["event"] == "conversation")
            .map(|event| event["conversation"].clone())
            .collect()
    }

    #[test]
    fn a_titled_conversation_round_trips_with_its_title() {
        let mut agent = Agent::ready();
        agent.feed(&update(json!({"sessionUpdate": "session_info_update",
            "title": "Hello.txt file creation"})));
        let bytes = encode(&agent.s).expect("it fits");
        assert_eq!(
            logged_conversations(&bytes),
            [json!({"id": "s1", "cwd": "/home/u/src", "continuation": "resume_and_load",
                "title": "Hello.txt file creation"})]
        );
        let restored = agent.round_trip();
        assert_eq!(restored, agent.s);
        assert_eq!(
            restored.transcript.conversation().and_then(|c| c.title.as_deref()),
            Some("Hello.txt file creation")
        );
        // A restored session goes on from the title it kept: the same one
        // again sends nothing.
        let mut restored = Agent {
            s: restored,
            now: agent.now,
        };
        let out = restored.feed(&update(json!({"sessionUpdate": "session_info_update",
            "title": "Hello.txt file creation"})));
        assert!(out.events.is_empty());
    }

    #[test]
    fn a_checkpoint_written_without_titles_reads_unchanged() {
        let agent = Agent::ready();
        let bytes = encode(&agent.s).expect("it fits");
        // An untitled conversation is written as before titles existed.
        assert_eq!(
            logged_conversations(&bytes),
            [json!({"id": "s1", "cwd": "/home/u/src", "continuation": "resume_and_load"})]
        );
        let restored = decode(&bytes, agent.now).expect("it reads");
        assert_eq!(restored, agent.s);
        assert_eq!(
            restored.transcript.conversation().map(|c| c.title.clone()),
            Some(None)
        );
    }

    #[test]
    fn a_trimmed_log_round_trips_with_its_marker_and_next_id() {
        let mut agent = Agent::ready();
        agent.prompt("go");
        for _ in 0..4 {
            agent.feed(&chunk("x"));
            agent.feed(&update(json!({"sessionUpdate": "agent_thought_chunk",
                "content": {"type": "text", "text": "y"}})));
        }
        let trimmed = agent.s.transcript.trim_to(1).expect("entries dropped");
        assert!(matches!(trimmed, Event::Trimmed { .. }));
        agent.s.run = None;
        let restored = agent.round_trip();
        assert_eq!(restored, agent.s);
        assert_eq!(restored.transcript.next_id(), agent.s.transcript.next_id());
        assert_eq!(restored.transcript.dropped(), agent.s.transcript.dropped());
    }

    #[test]
    fn a_held_prompt_round_trips_and_can_be_sent_again() {
        let mut agent = Agent::ready();
        agent.prompt("first");
        agent.prompt("second");
        agent.feed(&result(3, json!({"stopReason": "max_tokens"})));
        assert!(matches!(
            agent.s.transcript.slot(),
            Slot::Held {
                reason: HoldReason::Stopped { .. },
                ..
            }
        ));
        let mut restored = agent.round_trip();
        assert_eq!(restored, agent.s);
        let out = step(&mut restored, Input::Command(Command::Resend), agent.now);
        assert_eq!(out.reply, Some(Ok(())));
        let lines = sent(&out.stdin);
        assert_eq!(lines[0]["method"], "session/prompt");
        assert_eq!(lines[0]["id"], 4, "the next id survived");
        assert_eq!(lines[0]["params"]["prompt"][0]["text"], "second");
    }

    #[test]
    fn a_restored_session_answers_the_permission_request_by_its_own_id() {
        let agent = Agent::busy();
        let mut restored = agent.round_trip();
        let entry = *restored.rpc.permissions().keys().next().expect("a request");
        let out = step(
            &mut restored,
            Input::Command(Command::Answer {
                entry,
                option: Some("reject".into()),
            }),
            agent.now,
        );
        assert_eq!(out.reply, Some(Ok(())));
        let lines = sent(&out.stdin);
        assert_eq!(lines[0]["id"], "p1");
        assert_eq!(lines[0]["result"]["outcome"]["optionId"], "reject");
    }

    #[test]
    fn a_restored_session_matches_the_prompts_response_and_sends_the_waiting_one() {
        let agent = Agent::busy();
        let mut restored = agent.round_trip();
        let mut original = agent.s.clone();
        let entry = *original.rpc.permissions().keys().next().expect("a request");
        let answer = || {
            Input::Command(Command::Answer {
                entry,
                option: Some("allow".into()),
            })
        };
        assert_eq!(
            step(&mut restored, answer(), agent.now),
            step(&mut original, answer(), agent.now)
        );
        // The rest of the line in progress, then the turn's end.
        let mut tail = b"date\",\"params\":{\"sessionId\":\"s1\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"!\"}}}}\n".to_vec();
        tail.extend(result(3, json!({"stopReason": "end_turn"})));
        let expected = step(&mut original, Input::Stdout(&tail), agent.now);
        let out = step(&mut restored, Input::Stdout(&tail), agent.now);
        assert_eq!(out, expected);
        assert_eq!(restored, original);
        assert_eq!(sent(&out.stdin)[0]["params"]["prompt"][0]["text"], "second");
        assert!(
            entries(&restored)
                .iter()
                .any(|e| matches!(e, Entry::Agent { text } if text == "!"))
        );
    }

    #[test]
    fn covers_counts_every_byte_fed_and_survives() {
        let agent = Agent::busy();
        let restored = agent.round_trip();
        assert!(agent.s.lines.covers() > 0);
        assert_eq!(restored.lines.covers(), agent.s.lines.covers());
    }

    #[test]
    fn a_partial_line_travels_as_raw_bytes_after_the_state() {
        let mut agent = Agent::ready();
        agent.feed(b"\xff\xfe not UTF-8");
        let bytes = encode(&agent.s).expect("it fits");
        assert!(bytes.ends_with(b"\xff\xfe not UTF-8"));
        assert_eq!(decode(&bytes, agent.now).expect("it reads"), agent.s);
    }

    #[test]
    fn an_overlong_or_resyncing_partial_round_trips_without_bytes() {
        for partial in [
            Partial::TooLong { len: 9_000_000 },
            Partial::Resyncing { len: 12 },
        ] {
            let mut agent = Agent::ready();
            agent.s.lines = Lines::restore(partial, 9_000_100);
            let bytes = encode(&agent.s).expect("it fits");
            let state_len = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
            assert_eq!(bytes.len(), HEADER_LEN + state_len, "no line bytes follow");
            assert_eq!(decode(&bytes, agent.now).expect("it reads"), agent.s);
        }
    }

    #[test]
    fn the_header_is_the_magic_format_1_and_the_states_length() {
        let bytes = encode(&Agent::new().s).expect("it fits");
        assert_eq!(&bytes[..4], b"BACP");
        assert_eq!(&bytes[4..6], &[0, 1]);
        let state_len = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
        let state: Value = serde_json::from_slice(&bytes[HEADER_LEN..HEADER_LEN + state_len])
            .expect("the state is JSON");
        assert_eq!(state["acp"], 1);
        assert_eq!(state["phase"], json!({"phase": "spawning"}));
    }

    #[test]
    fn a_state_with_fields_it_doesnt_know_still_reads() {
        let agent = Agent::busy();
        let bytes = encode(&agent.s).expect("it fits");
        let state_len = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
        let mut state: Value =
            serde_json::from_slice(&bytes[HEADER_LEN..HEADER_LEN + state_len]).expect("JSON");
        state["later"] = json!({"added": true});
        let rebuilt = rebuild(&state, &bytes[HEADER_LEN + state_len..]);
        assert_eq!(decode(&rebuilt, agent.now).expect("it reads"), agent.s);
    }

    /// A checkpoint of `state` and the line bytes `partial`.
    fn rebuild(state: &Value, partial: &[u8]) -> Vec<u8> {
        let state = state.to_string();
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&FORMAT.to_be_bytes());
        out.extend_from_slice(&(state.len() as u32).to_be_bytes());
        out.extend_from_slice(state.as_bytes());
        out.extend_from_slice(partial);
        out
    }

    fn state_of(bytes: &[u8]) -> (Value, Vec<u8>) {
        let state_len = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
        let state =
            serde_json::from_slice(&bytes[HEADER_LEN..HEADER_LEN + state_len]).expect("JSON");
        (state, bytes[HEADER_LEN + state_len..].to_vec())
    }

    #[test]
    fn a_bad_line_run_keeps_its_count_and_writes_its_notice_a_second_after_adoption() {
        let mut agent = Agent::ready();
        agent.feed(b"hello\nworld\n");
        assert_eq!(agent.s.bad.map(|bad| bad.lines), Some(2));
        let later = agent.now + std::time::Duration::from_secs(60);
        let bytes = encode(&agent.s).expect("it fits");
        let adopted = adopt(&bytes, false, "/x", "0.4.0", later);
        assert_eq!(adopted.restored, Ok(()));
        assert_eq!(
            adopted.session.bad,
            Some(BadRun {
                lines: 2,
                last: later
            })
        );
        assert_eq!(adopted.output.wake, Some(later + BAD_RUN_QUIET));
        let mut session = adopted.session;
        let out = step(&mut session, Input::Tick, later + BAD_RUN_QUIET);
        assert!(
            matches!(out.events.as_slice(), [Event::Added { entry: Entry::Notice { text, .. }, .. }]
            if text == "2 lines from the agent weren't ACP messages")
        );
    }

    // --- bounds ---------------------------------------------------------

    #[test]
    fn a_checkpoint_past_the_limit_is_refused_with_its_length() {
        let agent = Agent::busy();
        let len = encode(&agent.s).expect("it fits").len() as u64;
        assert_eq!(
            encode_within(&agent.s, len),
            Ok(encode(&agent.s).expect("it fits"))
        );
        assert_eq!(encode_within(&agent.s, len - 1), Err(TooLarge { len }));
    }

    #[test]
    fn a_full_log_a_full_partial_line_and_a_full_prompt_fit_in_64_mib() {
        let mut agent = Agent::ready();
        agent.prompt("go");
        // Entries of text that escapes to twice its length, past the log's
        // bound: the reducer trims it to 16 MiB encoded. Agent and thought
        // chunks alternate, so each is an entry of its own.
        let piece = "\"".repeat(32 * 1024);
        let thought = update(json!({"sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": piece}}));
        for _ in 0..(reduce::LOG_LIMIT / (4 * piece.len())) + 4 {
            agent.feed(&chunk(&piece));
            agent.feed(&thought);
        }
        assert!(agent.s.transcript.encoded_len() <= reduce::LOG_LIMIT);
        assert!(agent.s.transcript.encoded_len() > reduce::LOG_LIMIT - 512 * 1024);
        agent.prompt(&"q".repeat(reduce::MAX_PROMPT));
        assert!(matches!(agent.s.transcript.slot(), Slot::Waiting { .. }));
        agent.s.lines = Lines::restore(
            Partial::Collecting(vec![b'"'; super::super::lines::MAX_LINE + 1]),
            0,
        );
        let bytes = encode(&agent.s).expect("it fits in 64 MiB");
        assert!(
            (bytes.len() as u64) < MAX_CHECKPOINT_LEN / 2,
            "{}",
            bytes.len()
        );
        assert_eq!(decode(&bytes, agent.now).expect("it reads"), agent.s);
    }

    // --- checkpoints that don't read ------------------------------------

    fn assert_not_restored(adopted: &Adopted) {
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

    /// A phase this core doesn't know (a later build's) is adopted as a
    /// conversation that could not be restored, its agent left running.
    #[test]
    fn an_unknown_phase_is_adopted_as_not_restored() {
        let agent = Agent::ready();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        state["phase"] = json!({"phase": "logging_out", "method": "m"});
        let adopted = adopt(&rebuild(&state, &partial), false, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Err(Unreadable::Field { name: "phase" }));
        assert_not_restored(&adopted);
    }

    #[test]
    fn a_newer_format_is_adopted_as_a_conversation_that_could_not_be_restored() {
        let mut bytes = encode(&Agent::busy().s).expect("it fits");
        bytes[4..6].copy_from_slice(&2u16.to_be_bytes());
        let adopted = adopt(&bytes, false, "/home/u/src", "0.3.0", Instant::now());
        assert_eq!(adopted.restored, Err(Unreadable::Format { format: 2 }));
        assert_not_restored(&adopted);
        let entries = entries(&adopted.session);
        assert!(matches!(&entries[0], Entry::Notice { text, .. } if text.contains("format 2")));
    }

    #[test]
    fn bytes_that_arent_a_checkpoint_are_adopted_as_not_restored() {
        for (bytes, why) in [
            (&b"BAC"[..], Unreadable::Short { len: 3 }),
            (&b"\x1bc\x1b[2Jhello there"[..], Unreadable::NotACheckpoint),
            (
                &b"BACP\x00\x01\x00\x00\x00\x09{}"[..],
                Unreadable::Truncated,
            ),
            (
                &b"BACP\x00\x00\x00\x00\x00\x00"[..],
                Unreadable::Format { format: 0 },
            ),
        ] {
            let adopted = adopt(bytes, false, "/home/u/src", "0.3.0", Instant::now());
            assert_eq!(adopted.restored, Err(why));
            assert_not_restored(&adopted);
        }
    }

    #[test]
    fn a_session_that_wasnt_restored_refuses_prompts_and_ignores_its_agent() {
        let adopted = adopt(
            b"garbage!!!!!",
            false,
            "/home/u/src",
            "0.3.0",
            Instant::now(),
        );
        let mut session = adopted.session;
        let now = Instant::now();
        let out = step(
            &mut session,
            Input::Command(Command::Prompt { text: "hi".into() }),
            now,
        );
        assert_eq!(
            out.reply,
            Some(Err(Refused::Failed {
                message: NOT_RESTORED.into()
            }))
        );
        let out = step(&mut session, Input::Stdout(&chunk("ignored")), now);
        assert!(out.events.is_empty() && out.stdin.is_empty());
        assert!(!out.hang_up, "the agent is left running");
    }

    #[test]
    fn a_state_that_names_an_entry_the_log_doesnt_hold_is_unreadable() {
        let agent = Agent::busy();
        let (state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        let mut bad_run = state.clone();
        bad_run["run"] = json!({"kind": "agent", "entry": 99});
        let mut bad_tool = state.clone();
        bad_tool["tools"] = json!({"c9": 99});
        let mut bad_permission = state.clone();
        bad_permission["rpc"]["permissions"] = json!([[1, 0]]);
        for (state, name, entry) in [
            (bad_run, "run", 99),
            (bad_tool, "tools", 99),
            (bad_permission, "permissions", 1),
        ] {
            assert_eq!(
                decode(&rebuild(&state, &partial), agent.now),
                Err(Unreadable::Entry {
                    name,
                    entry: EntryId(entry)
                })
            );
        }
    }

    #[test]
    fn another_acp_version_is_unreadable() {
        let agent = Agent::ready();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        state["acp"] = json!(2);
        assert_eq!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Acp { version: 2 })
        );
    }

    #[test]
    fn a_log_that_doesnt_fold_is_unreadable() {
        let agent = Agent::busy();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        let log = state["log"].as_array_mut().expect("an array");
        log.insert(0, log[0].clone());
        assert!(matches!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Log { .. })
        ));
    }

    #[test]
    fn requests_that_dont_hold_together_are_unreadable() {
        let agent = Agent::busy();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        state["rpc"]["next_id"] = json!(3);
        assert_eq!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Rpc {
                parts: BadParts::IdNotBelowNext
            })
        );
    }

    #[test]
    fn line_bytes_beside_a_partial_that_isnt_collected_are_unreadable() {
        let mut agent = Agent::ready();
        agent.s.lines = Lines::restore(Partial::Resyncing { len: 3 }, 10);
        let (state, _) = state_of(&encode(&agent.s).expect("it fits"));
        assert_eq!(
            decode(&rebuild(&state, b"abc"), agent.now),
            Err(Unreadable::StrayBytes { len: 3 })
        );
    }

    #[test]
    fn a_missing_field_is_unreadable_naming_it() {
        let agent = Agent::ready();
        let (state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        for name in [
            "acp",
            "cwd",
            "version",
            "phase",
            "log",
            "rpc",
            "run",
            "tools",
            "bad_lines",
            "lines",
        ] {
            let mut state = state.clone();
            state.as_object_mut().expect("an object").remove(name);
            assert_eq!(
                decode(&rebuild(&state, &partial), agent.now),
                Err(Unreadable::Field { name }),
                "{name}"
            );
        }
    }

    // --- adoption -------------------------------------------------------

    #[test]
    fn an_untruncated_adoption_changes_nothing() {
        let agent = Agent::busy();
        let bytes = encode(&agent.s).expect("it fits");
        let adopted = adopt(&bytes, false, "/elsewhere", "9.9.9", agent.now);
        assert_eq!(adopted.restored, Ok(()));
        assert_eq!(adopted.session, agent.s);
        assert_eq!(adopted.output, Output::default());
    }

    #[test]
    fn a_truncated_adoption_fails_the_pending_prompt_and_holds_the_waiting_one() {
        let agent = Agent::busy();
        let bytes = encode(&agent.s).expect("it fits");
        let adopted = adopt(&bytes, true, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Ok(()));
        let session = &adopted.session;
        assert_eq!(
            *session.transcript.slot(),
            Slot::Held {
                text: "second".into(),
                reason: HoldReason::OutputLost
            }
        );
        assert_eq!(
            *session.transcript.turn(),
            TurnState::Failed {
                message: "output was lost while the terminal server restarted".into()
            }
        );
        assert!(
            !session
                .rpc
                .outgoing()
                .values()
                .any(|p| *p == Purpose::Prompt)
        );
        assert!(
            adopted
                .output
                .events
                .iter()
                .any(|e| matches!(e, Event::Queue { .. }))
        );
        assert!(matches!(session.lines.partial(), Partial::Resyncing { .. }));
    }

    #[test]
    fn a_truncated_adoption_keeps_a_held_prompt_as_it_was() {
        let mut agent = Agent::ready();
        agent.prompt("first");
        agent.prompt("second");
        agent.feed(&result(3, json!({"stopReason": "refusal"})));
        let held = agent.s.transcript.slot().clone();
        assert!(matches!(held, Slot::Held { .. }));
        assert_eq!(agent.prompt("third").reply, Some(Ok(())));
        let bytes = encode(&agent.s).expect("it fits");
        let adopted = adopt(&bytes, true, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(*adopted.session.transcript.slot(), held);
        assert!(matches!(
            adopted.session.transcript.turn(),
            TurnState::Failed { .. }
        ));
        assert!(
            !adopted
                .output
                .events
                .iter()
                .any(|e| matches!(e, Event::Queue { .. }))
        );
    }

    #[test]
    fn a_session_never_checkpointed_is_adopted_new_and_started() {
        let adopted = adopt(&[], false, "/home/u/src", "0.3.0", Instant::now());
        assert_eq!(adopted.restored, Ok(()));
        assert_eq!(adopted.session.phase, Phase::Initializing { start: Start::New });
        assert_eq!(sent(&adopted.output.stdin)[0]["method"], "initialize");
        assert_eq!(
            adopted.output.checkpoint,
            Some(reduce::Moment::BeforeRequest)
        );
        assert_eq!(adopted.session.cwd, "/home/u/src");
    }

    /// A continue's state, stored with its spawn before the core saw its
    /// `SPAWNED` (agents-session-recovery 3.3): the adopted agent is sent
    /// `initialize` and then continues the conversation, never
    /// `session/new`.
    #[test]
    fn a_continue_stored_before_its_spawned_is_started_and_continues() {
        let mut agent = Agent::ready();
        agent.prompt("first");
        agent.feed(&result(3, json!({"stopReason": "end_turn"})));
        agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        });
        assert_eq!(agent.step(Input::Restart).reply, Some(Ok(())));
        let entries = agent.s.transcript.entries().count();
        let bytes = encode(&agent.s).expect("it fits");
        let mut adopted = adopt(&bytes, false, "/elsewhere", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Ok(()));
        assert!(matches!(adopted.session.phase, Phase::Initializing { .. }));
        assert_eq!(sent(&adopted.output.stdin)[0]["method"], "initialize");
        assert_eq!(adopted.output.checkpoint, Some(reduce::Moment::BeforeRequest));
        assert_eq!(adopted.session.transcript.entries().count(), entries, "the log is kept");
        let out = step(
            &mut adopted.session,
            Input::Stdout(&result(
                4,
                json!({"protocolVersion": 1, "agentCapabilities": {
                    "loadSession": true, "sessionCapabilities": {"resume": {}}}}),
            )),
            agent.now,
        );
        assert_eq!(sent(&out.stdin)[0]["method"], "session/resume");

        // A fresh continue's likewise.
        let conversation = Conversation {
            id: "s1".into(),
            cwd: "/w".into(),
            continuation: Continuation::Neither,
            title: None,
        };
        let fresh = AgentSession::continuing(conversation, "0.3.0".into());
        let bytes = encode(&fresh).expect("it fits");
        let mut adopted = adopt(&bytes, false, "/elsewhere", "0.3.0", agent.now);
        assert_eq!(sent(&adopted.output.stdin)[0]["method"], "initialize");
        let out = step(
            &mut adopted.session,
            Input::Stdout(&result(1, json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}))),
            agent.now,
        );
        let load = &sent(&out.stdin)[0];
        assert_eq!((load["method"].as_str(), load["params"]["cwd"].as_str()), (Some("session/load"), Some("/w")));
    }

    #[test]
    fn a_session_never_checkpointed_with_a_truncated_tail_says_so_then_starts() {
        let adopted = adopt(&[], true, "/home/u/src", "0.3.0", Instant::now());
        let notices: Vec<&Event> = adopted
            .output
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Event::Added {
                        entry: Entry::Notice { .. },
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(notices.len(), 1);
        assert_eq!(sent(&adopted.output.stdin)[0]["method"], "initialize");
        assert!(matches!(
            adopted.session.lines.partial(),
            Partial::Resyncing { .. }
        ));
    }

    #[test]
    fn a_truncated_unreadable_checkpoint_only_resyncs_its_lines() {
        let adopted = adopt(
            b"not a checkpoint",
            true,
            "/home/u/src",
            "0.3.0",
            Instant::now(),
        );
        assert_not_restored(&adopted);
        assert!(matches!(
            adopted.session.lines.partial(),
            Partial::Resyncing { .. }
        ));
    }

    // --- fuzz -----------------------------------------------------------

    #[test]
    fn fuzz_decoding_never_panics_on_random_or_mutated_checkpoints() {
        let mut rng = XorShift(0x5eed_ac01_c4ec_0001);
        let now = Instant::now();
        let valid = encode(&Agent::busy().s).expect("it fits");
        for round in 0..3000 {
            let bytes: Vec<u8> = match round % 4 {
                // Random bytes, sometimes behind a good header.
                0 => (0..rng.below(64)).map(|_| rng.next() as u8).collect(),
                1 => {
                    let mut bytes = valid[..HEADER_LEN].to_vec();
                    bytes.extend((0..rng.below(256)).map(|_| rng.next() as u8));
                    bytes
                }
                // A valid checkpoint cut short or with bytes flipped.
                2 => valid[..rng.below(valid.len())].to_vec(),
                _ => {
                    let mut bytes = valid.clone();
                    for _ in 0..1 + rng.below(4) {
                        let at = rng.below(bytes.len());
                        bytes[at] ^= 1 << rng.below(8);
                    }
                    bytes
                }
            };
            let adopted = adopt(&bytes, round % 2 == 0, "/home/u/src", "0.3.0", now);
            if adopted.restored.is_ok() {
                // What reads encodes again.
                encode(&adopted.session).expect("it fits");
            }
        }
    }
}
