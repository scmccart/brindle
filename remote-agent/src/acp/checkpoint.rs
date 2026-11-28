//! An agent session's checkpoint (design decision 11 of the OpenSpec change
//! `agents-sessions`): what the core stores in the session's holder so that
//! the next core, after a crash or an upgrade, carries on the conversation.
//!
//! **The format** (2, OpenSpec change `agents-log-paging`, design decision
//! 7): a header, then a base record and any number of segment records, so
//! the core can append what changed since its last checkpoint
//! (`CHECKPOINT_APPEND`) instead of rewriting the whole log:
//!
//! | bytes | what |
//! | --- | --- |
//! | 4 | [`MAGIC`], `BACP` |
//! | 2 | the format, big-endian: [`FORMAT`] (2) |
//! | then, repeated | a record |
//! | 1 | its kind: `B` (the base, first and only once) or `S` (a segment) |
//! | 4 | its sequence number, big-endian: the base 0, each segment one more |
//! | 4 | the JSON state's length in bytes, big-endian |
//! | that many | the JSON state: one object |
//! | 4 | the partial line's length, big-endian (0 unless [`Partial::Collecting`]) |
//! | that many | the partial line's bytes |
//!
//! A segment's state has the base's fields except `log`, which becomes
//! `events`: the events the session applied since the previous record, in
//! order, with consecutive `Appended`s of one entry merged up to
//! [`TEXT_CHUNK`] and consecutive `ToolOutput`s of one item up to
//! [`ITEM_LIMIT`], the later exit kept (`agents-tool-content` decision 8).
//! Every other field is whole and replaces the previous
//! record's. [`decode`] folds the base's log, then each segment's events,
//! and takes the rest from the last record. [`encode`] writes the header
//! and a base; [`encode_segment`] one segment record.
//!
//! Format 1, which builds before this change wrote, is still read: the
//! header's format, then the JSON state's length (4 bytes, big-endian),
//! the state, and the rest the partial line's bytes.
//!
//! The base's state holds the log as the events of [`Transcript::replay`] (the
//! queue slot, the turn and the agent's state among them), the ACP version
//! (`acp`, always 1 here), the [`Phase`] (with the ACP session id once
//! ready, what a spawn starts or continues, what `initialize` offered, and
//! a load's muted count, and a login's message and what follows it), the
//! login methods the agent advertised (`methods`, left out when there are
//! none; OpenSpec change `agents-login`), the [`Rpc`] parts (the next outgoing id, the core's requests
//! awaiting a response and what each was for, the agent's waiting
//! permission requests by entry with their JSON-RPC ids as sent), the chunk
//! run, the tool entries by call id, the outputs still appended to by the
//! agent's terminal id (`outputs`: each one's entry, its `Output` item and
//! its plain-text filter's state, a half-received escape sequence or
//! character included; left out when there are none, as in a checkpoint
//! from before `agents-tool-content`), the count of a bad-line run not yet
//! written as a notice, and the [`Lines`] state: which partial it is and
//! `covers`, every stdout byte fed, the partial line's included. Since
//! OpenSpec change `agents-commands` (format 2, amended) it also holds
//! each held option's key by its id (`option_keys`) and the mode values
//! last in effect (`modes`), each left out when empty; the commands and
//! options themselves are log events. A set of an option out isn't kept.
//!
//! **Bounds.** A checkpoint is at most [`MAX_CHECKPOINT_LEN`] (64 MiB, what a
//! holder keeps). That a whole one fits is `acp::limits`' invariant I2:
//! the log's stored form is at most twice its 16 MiB measure, beside an
//! 8 MiB partial line, the login methods, 64 open outputs and the rest of
//! the state, `LARGEST_CHECKPOINT` in all, with about 12.7 MiB to spare
//! (`agents-event-bounds` decision 1). [`encode`] still refuses one past
//! it rather than send what the holder drops.
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
    AgentState, Continuation, Conversation, Entry, EntryId, Event, ITEM_LIMIT, LoginMethod,
    NoticeLevel, TEXT_CHUNK, ToolItem, Transcript,
};

use super::dialect::{Dialect, Unsupported};
use super::envelope::Id;
use super::forms::OptionKey;
use super::lines::{Lines, Partial};
use super::plain::{self, Mode};
use super::reduce::{
    self, AfterLogin, AgentSession, BAD_RUN_QUIET, BadRun, Greeting, Input, Log, Login, OpenOutput,
    Output, Phase, Replay, Run, RunKind, Start,
};
use super::rpc::{BadParts, Purpose, Rpc};

/// The four bytes a checkpoint starts with.
pub const MAGIC: [u8; 4] = *b"BACP";

/// The format [`encode`] writes and the newest [`decode`] reads.
pub const FORMAT: u16 = 2;

/// The format before segments, which [`decode`] still reads.
const FORMAT_1: u16 = 1;

/// The magic and the format.
const PREFIX_LEN: usize = 4 + 2;

/// Format 1's header: the magic, the format and the state's length.
const HEADER_LEN: usize = PREFIX_LEN + 4;

/// A format 2 record's kind, sequence number and state length.
const RECORD_HEADER_LEN: usize = 1 + 4 + 4;

/// A base record's kind.
const BASE: u8 = b'B';

/// A segment record's kind.
const SEGMENT: u8 = b'S';

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
    /// kind; or the log has a pending permission request the state's
    /// `permissions` doesn't await.
    Entry { name: &'static str, entry: EntryId },
    /// Partial line bytes come with a partial that isn't being collected.
    StrayBytes { len: usize },
    /// A format 2 checkpoint whose first record isn't a base.
    NoBase,
    /// A format 2 record of a kind other than a base first and segments
    /// after it.
    RecordKind { kind: u8 },
    /// A format 2 record whose sequence number isn't the next one: a
    /// segment was lost.
    Sequence { expected: u32, found: u32 },
}

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short { len } => write!(f, "its checkpoint is only {len} bytes"),
            Self::NotACheckpoint => f.write_str("its checkpoint isn't an agent session's"),
            Self::Format { format } => write!(
                f,
                "its checkpoint is in format {format}, and this terminal server reads formats {FORMAT_1} and {FORMAT}"
            ),
            Self::Truncated => f.write_str("its checkpoint is cut short"),
            Self::NotJson { error } => write!(f, "its checkpoint's state isn't JSON: {error}"),
            Self::Field { name } => write!(f, "its checkpoint's {name} is missing or malformed"),
            Self::Acp { version } => write!(
                f,
                "its conversation speaks ACP version {version}, and this terminal server speaks {}",
                Dialect::newest().version()
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
                    "its checkpoint's {name} and its log disagree about entry {entry}"
                )
            }
            Self::StrayBytes { len } => {
                write!(
                    f,
                    "its checkpoint carries {len} bytes of a line it isn't collecting"
                )
            }
            Self::NoBase => f.write_str("its checkpoint doesn't start with its base"),
            Self::RecordKind { kind } => {
                write!(f, "its checkpoint has a record of unknown kind {kind:#04x}")
            }
            Self::Sequence { expected, found } => write!(
                f,
                "its checkpoint's record {found} came where record {expected} was due, so a part of it is missing"
            ),
        }
    }
}

/// Encodes `session` whole (the header and a base record), or refuses
/// past [`MAX_CHECKPOINT_LEN`].
pub fn encode(session: &AgentSession) -> Result<Vec<u8>, TooLarge> {
    encode_within(session, MAX_CHECKPOINT_LEN)
}

fn encode_within(session: &AgentSession, limit: u64) -> Result<Vec<u8>, TooLarge> {
    let (mut state, partial) = state_fields(session);
    let events: Vec<Value> = session
        .transcript
        .replay()
        .iter()
        .map(Event::to_value)
        .collect();
    state.insert("log".into(), Value::Array(events));
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&FORMAT.to_be_bytes());
    push_record(&mut out, BASE, 0, &Value::Object(state), partial, limit)?;
    Ok(out)
}

/// One segment record, numbered `seq`, to append to a stored checkpoint:
/// `events`, the events `session` applied since the previous record, with
/// consecutive `Appended`s of one entry merged, and the rest of the session
/// whole. Refused past [`MAX_CHECKPOINT_LEN`] on its own; whether it fits
/// beside what is stored is the core's to judge.
pub fn encode_segment(
    session: &AgentSession,
    events: &[Event],
    seq: u32,
) -> Result<Vec<u8>, TooLarge> {
    let (mut state, partial) = state_fields(session);
    let events: Vec<Value> = merged(events).iter().map(Event::to_value).collect();
    state.insert("events".into(), Value::Array(events));
    let mut out = Vec::new();
    push_record(
        &mut out,
        SEGMENT,
        seq,
        &Value::Object(state),
        partial,
        MAX_CHECKPOINT_LEN,
    )?;
    Ok(out)
}

/// `events` with each run of `Appended`s to one entry merged into as few
/// as fit [`TEXT_CHUNK`] each: they fold to the same text.
fn merged(events: &[Event]) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    for event in events {
        if let Event::Appended { id, text } = event
            && let Some(Event::Appended {
                id: last_id,
                text: last,
            }) = out.last_mut()
            && last_id == id
            && last.len() + text.len() <= TEXT_CHUNK
        {
            last.push_str(text);
            continue;
        }
        // Output to one item, as the fold appends it: `a` then `b` folds as
        // `a + b` (`agents-tool-content` decision 8), the later exit kept.
        if let Event::ToolOutput {
            id,
            item,
            text,
            exit,
        } = event
            && let Some(Event::ToolOutput {
                id: last_id,
                item: last_item,
                text: last,
                exit: last_exit @ None,
            }) = out.last_mut()
            && (*last_id, *last_item) == (*id, *item)
            && last.len() + text.len() <= ITEM_LIMIT
        {
            last.push_str(text);
            last_exit.clone_from(exit);
            continue;
        }
        out.push(event.clone());
    }
    out
}

/// Every field of the state but the log, and the partial line's bytes.
fn state_fields(session: &AgentSession) -> (Map<String, Value>, &[u8]) {
    let (partial, bytes): (Value, &[u8]) = match session.lines.partial() {
        Partial::Collecting(bytes) => (json!({ "partial": "collecting" }), bytes),
        Partial::TooLong { len } => (json!({ "partial": "too_long", "len": len }), &[]),
        Partial::Resyncing { len } => (json!({ "partial": "resyncing", "len": len }), &[]),
    };
    let mut lines = partial;
    if let Some(object) = lines.as_object_mut() {
        object.insert("covers".into(), session.lines.covers().into());
    }
    let mut state = Map::new();
    state.insert("acp".into(), session.rpc.dialect().version().into());
    state.insert("cwd".into(), session.cwd.clone().into());
    state.insert("version".into(), session.version.clone().into());
    state.insert("phase".into(), phase_value(&session.phase));
    state.insert("rpc".into(), rpc_value(&session.rpc));
    state.insert(
        "run".into(),
        session.run.map_or(Value::Null, |run| {
            json!({ "kind": run_kind_name(run.kind), "entry": run.entry.0 })
        }),
    );
    state.insert(
        "tools".into(),
        Value::Object(
            session
                .tools
                .iter()
                .map(|(call, entry)| (call.clone(), Value::from(entry.0)))
                .collect(),
        ),
    );
    // The open outputs (`agents-tool-content` decision 8), left out when
    // there are none, as a checkpoint from before them has none.
    if !session.outputs.is_empty() {
        state.insert(
            "outputs".into(),
            Value::Object(
                session
                    .outputs
                    .iter()
                    .map(|(terminal, open)| (terminal.clone(), open_output_value(open)))
                    .collect(),
            ),
        );
    }
    state.insert(
        "bad_lines".into(),
        session.bad.map_or(0, |bad| bad.lines).into(),
    );
    state.insert("lines".into(), lines);
    if !session.methods.is_empty() {
        state.insert("methods".into(), json!(session.methods));
    }
    // Left out when false, so a state written before
    // `agents-session-list` reads as one whose agent lists nothing.
    if session.lists {
        state.insert("lists".into(), json!(true));
    }
    if session.chose {
        state.insert("chose".into(), json!(true));
    }
    // The options' keys and the modes last in effect (`agents-commands`,
    // decision 12), left out when empty, as a state from before has none.
    if !session.keys.is_empty() {
        state.insert(
            "option_keys".into(),
            Value::Object(
                session
                    .keys
                    .iter()
                    .map(|(id, key)| (id.clone(), key.to_value()))
                    .collect(),
            ),
        );
    }
    if !session.modes.is_empty() {
        state.insert("modes".into(), json!(session.modes));
    }
    (state, bytes)
}

/// Appends a format 2 record to `out`, or refuses when `out` would pass
/// `limit`.
fn push_record(
    out: &mut Vec<u8>,
    kind: u8,
    seq: u32,
    state: &Value,
    partial: &[u8],
    limit: u64,
) -> Result<(), TooLarge> {
    let state = state.to_string();
    let len = out.len() as u64
        + RECORD_HEADER_LEN as u64
        + state.len() as u64
        + 4
        + partial.len() as u64;
    let state_len = u32::try_from(state.len()).map_err(|_| TooLarge { len })?;
    let partial_len = u32::try_from(partial.len()).map_err(|_| TooLarge { len })?;
    if len > limit {
        return Err(TooLarge { len });
    }
    out.reserve(RECORD_HEADER_LEN + state.len() + 4 + partial.len());
    out.push(kind);
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&state_len.to_be_bytes());
    out.extend_from_slice(state.as_bytes());
    out.extend_from_slice(&partial_len.to_be_bytes());
    out.extend_from_slice(partial);
    Ok(())
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
        Phase::Choosing { greeting } => greeting_value("choosing", greeting),
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
        Start::Choose => value["start"] = json!("choose"),
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

/// A request's purpose as checkpointed; none for a list, which isn't
/// (`agents-session-list` flag 4): its late answer is ignored; nor for a
/// set of an option (`agents-commands` flag 3).
fn purpose_name(purpose: &Purpose) -> Option<&'static str> {
    Some(match purpose {
        Purpose::Initialize => "initialize",
        Purpose::NewSession => "new_session",
        Purpose::Prompt => "prompt",
        Purpose::Resume => "resume",
        Purpose::Load => "load",
        Purpose::Authenticate => "authenticate",
        Purpose::List { .. } | Purpose::SetOption { .. } => return None,
    })
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
        .filter_map(|(id, purpose)| Some(json!([id, purpose_name(purpose)?])))
        .collect();
    let permissions: Vec<Value> = rpc
        .permissions()
        .iter()
        .map(|(entry, id)| json!([entry.0, id.to_value()]))
        .collect();
    json!({ "next_id": rpc.next_id(), "outgoing": outgoing, "permissions": permissions })
}

/// The session `bytes` encode, in format 1 or 2, or why they don't. A
/// bad-line run comes back with `now` as its last line's time, so its
/// notice is written a quiet second after the adoption.
pub fn decode(bytes: &[u8], now: Instant) -> Result<AgentSession, Unreadable> {
    let prefix = bytes
        .get(..PREFIX_LEN)
        .ok_or(Unreadable::Short { len: bytes.len() })?;
    let (magic, format) = prefix.split_at(4);
    if magic != MAGIC {
        return Err(Unreadable::NotACheckpoint);
    }
    match u16::from_be_bytes([format[0], format[1]]) {
        FORMAT_1 => decode_1(bytes, now),
        FORMAT => decode_2(&bytes[PREFIX_LEN..], now),
        format => Err(Unreadable::Format { format }),
    }
}

/// Format 1: the state's length, the state, the partial line.
fn decode_1(bytes: &[u8], now: Instant) -> Result<AgentSession, Unreadable> {
    let header = bytes
        .get(..HEADER_LEN)
        .ok_or(Unreadable::Short { len: bytes.len() })?;
    let state_len = u32::from_be_bytes([header[6], header[7], header[8], header[9]]);
    let body = &bytes[HEADER_LEN..];
    let state_len = usize::try_from(state_len).map_err(|_| Unreadable::Truncated)?;
    if state_len > body.len() {
        return Err(Unreadable::Truncated);
    }
    let (state, partial) = body.split_at(state_len);
    let state = state_object(state)?;
    restore(&state, None, partial, now)
}

/// Format 2: a base record, then segments in sequence.
fn decode_2(mut rest: &[u8], now: Instant) -> Result<AgentSession, Unreadable> {
    let Some(base) = next_record(&mut rest)? else {
        return Err(Unreadable::NoBase);
    };
    match base.kind {
        BASE => {}
        SEGMENT => return Err(Unreadable::NoBase),
        kind => return Err(Unreadable::RecordKind { kind }),
    }
    if base.seq != 0 {
        return Err(Unreadable::Sequence {
            expected: 0,
            found: base.seq,
        });
    }
    let mut state = state_object(base.state)?;
    let mut transcript = restore_log(array(&state, "log")?)?;
    let mut partial = base.partial;
    let mut expected: u32 = 1;
    while let Some(record) = next_record(&mut rest)? {
        if record.kind != SEGMENT {
            return Err(Unreadable::RecordKind { kind: record.kind });
        }
        if record.seq != expected {
            return Err(Unreadable::Sequence {
                expected,
                found: record.seq,
            });
        }
        expected = expected.saturating_add(1);
        state = state_object(record.state)?;
        fold_log(&mut transcript, array(&state, "events")?)?;
        partial = record.partial;
    }
    restore(&state, Some(transcript), partial, now)
}

/// One format 2 record.
struct Record<'a> {
    kind: u8,
    seq: u32,
    state: &'a [u8],
    partial: &'a [u8],
}

/// The next record of `rest`, taken off it; `None` at its end.
fn next_record<'a>(rest: &mut &'a [u8]) -> Result<Option<Record<'a>>, Unreadable> {
    if rest.is_empty() {
        return Ok(None);
    }
    let (kind, bytes) = rest.split_first().ok_or(Unreadable::Truncated)?;
    let (seq, bytes) = take_u32(bytes)?;
    let (state_len, bytes) = take_u32(bytes)?;
    let (state, bytes) = take(bytes, state_len)?;
    let (partial_len, bytes) = take_u32(bytes)?;
    let (partial, bytes) = take(bytes, partial_len)?;
    *rest = bytes;
    Ok(Some(Record {
        kind: *kind,
        seq,
        state,
        partial,
    }))
}

fn take_u32(bytes: &[u8]) -> Result<(u32, &[u8]), Unreadable> {
    let (number, rest) = bytes
        .split_first_chunk::<4>()
        .ok_or(Unreadable::Truncated)?;
    Ok((u32::from_be_bytes(*number), rest))
}

fn take(bytes: &[u8], len: u32) -> Result<(&[u8], &[u8]), Unreadable> {
    let len = usize::try_from(len).map_err(|_| Unreadable::Truncated)?;
    if len > bytes.len() {
        return Err(Unreadable::Truncated);
    }
    Ok(bytes.split_at(len))
}

/// A state's bytes as its JSON object.
fn state_object(state: &[u8]) -> Result<Map<String, Value>, Unreadable> {
    let state: Value = serde_json::from_slice(state).map_err(|error| Unreadable::NotJson {
        error: error.to_string(),
    })?;
    match state {
        Value::Object(object) => Ok(object),
        Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_) => Err(Unreadable::NotJson {
            error: "not an object".into(),
        }),
    }
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

/// The session `state` holds, its log folded already (format 2) or read
/// from its `log` field (format 1).
fn restore(
    state: &Map<String, Value>,
    transcript: Option<Transcript>,
    partial: &[u8],
    now: Instant,
) -> Result<AgentSession, Unreadable> {
    let acp = number(state, "acp")?;
    let dialect = Dialect::from_version(acp)
        .map_err(|Unsupported { version }| Unreadable::Acp { version })?;
    let cwd = text(state, "cwd")?.to_owned();
    let version = text(state, "version")?.to_owned();
    let phase = restore_phase(object(field(state, "phase")?, "phase")?)?;
    let transcript = match transcript {
        Some(transcript) => transcript,
        None => restore_log(array(state, "log")?)?,
    };
    let mut rpc = restore_rpc(object(field(state, "rpc")?, "rpc")?)?;
    rpc.speak(dialect);
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
    let mut outputs = BTreeMap::new();
    if let Some(open) = state.get("outputs") {
        for (terminal, open) in object(open, "outputs")? {
            outputs.insert(terminal.clone(), restore_open_output(open)?);
        }
    }
    let bad_lines = number(state, "bad_lines")?;
    let lines = restore_lines(object(field(state, "lines")?, "lines")?, partial)?;
    let methods: Vec<LoginMethod> = match state.get("methods") {
        None => Vec::new(),
        Some(methods) => serde_json::from_value(methods.clone())
            .map_err(|_| Unreadable::Field { name: "methods" })?,
    };
    let flag = |name: &'static str| match state.get(name) {
        None => Ok(false),
        Some(value) => value.as_bool().ok_or(Unreadable::Field { name }),
    };
    let (lists, chose) = (flag("lists")?, flag("chose")?);
    let mut keys = BTreeMap::new();
    if let Some(stored) = state.get("option_keys") {
        for (id, key) in object(stored, "option_keys")? {
            let key = OptionKey::from_value(key).ok_or(Unreadable::Field {
                name: "option_keys",
            })?;
            keys.insert(id.clone(), key);
        }
    }
    let modes: BTreeMap<String, String> = match state.get("modes") {
        None => BTreeMap::new(),
        Some(modes) => serde_json::from_value(modes.clone())
            .map_err(|_| Unreadable::Field { name: "modes" })?,
    };
    // Each key is a held option's.
    if keys.len() != transcript.options().len()
        || transcript
            .options()
            .iter()
            .any(|option| !keys.contains_key(&option.id))
    {
        return Err(Unreadable::Field {
            name: "option_keys",
        });
    }

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
    for open in outputs.values() {
        let holds = match transcript.entry(open.entry) {
            Some(Entry::Tool { content, .. }) => {
                matches!(content.get(open.item as usize), Some(ToolItem::Output(_)))
            }
            Some(_) | None => false,
        };
        if !holds {
            return Err(Unreadable::Entry {
                name: "outputs",
                entry: open.entry,
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
    // The converse (`agents-process-lifecycle` decision 3): a pending
    // request the log shows is one `Rpc` awaits, or no answer to it could
    // ever be sent.
    for (entry, kept) in transcript.entries() {
        if matches!(kept, Entry::Permission { outcome: None, .. })
            && !rpc.permissions().contains_key(&entry)
        {
            return Err(Unreadable::Entry {
                name: "permissions",
                entry,
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
        outputs,
        bad: (bad_lines > 0).then_some(BadRun {
            lines: bad_lines,
            last: now,
        }),
        methods,
        lists,
        chose,
        keys,
        modes,
    })
}

/// An open output: its entry and item, and its plain-text filter.
fn open_output_value(open: &OpenOutput) -> Value {
    let filter = &open.filter;
    let (mode, skipped) = match filter.mode {
        Mode::Text => ("text", 0),
        Mode::Escape => ("escape", 0),
        Mode::EscapeIntermediate => ("escape_intermediate", 0),
        Mode::Csi => ("csi", 0),
        Mode::String { skipped } => ("string", skipped),
        Mode::StringEscape { skipped } => ("string_escape", skipped),
    };
    json!({
        "entry": open.entry.0,
        "item": open.item,
        "mode": mode,
        "skipped": skipped,
        "line": filter.line,
        "cr": filter.cr,
        "partial": filter.partial,
    })
}

fn restore_open_output(value: &Value) -> Result<OpenOutput, Unreadable> {
    let name = "outputs";
    let bad = Unreadable::Field { name };
    let open = object(value, name)?;
    let skipped =
        usize::try_from(number(open, "skipped")?).map_err(|_| Unreadable::Field { name })?;
    let mode = match text(open, "mode")? {
        "text" => Mode::Text,
        "escape" => Mode::Escape,
        "escape_intermediate" => Mode::EscapeIntermediate,
        "csi" => Mode::Csi,
        "string" => Mode::String { skipped },
        "string_escape" => Mode::StringEscape { skipped },
        _ => return Err(bad),
    };
    let partial: Vec<u8> = serde_json::from_value(field(open, "partial")?.clone())
        .map_err(|_| Unreadable::Field { name })?;
    if partial.len() > 3 {
        return Err(bad);
    }
    Ok(OpenOutput {
        entry: EntryId(number(open, "entry")?),
        item: u32::try_from(number(open, "item")?).map_err(|_| Unreadable::Field { name })?,
        filter: plain::State {
            mode,
            line: text(open, "line")?.to_owned(),
            cr: open
                .get("cr")
                .and_then(Value::as_bool)
                .ok_or(Unreadable::Field { name })?,
            partial,
        },
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
        "choosing" => Phase::Choosing {
            greeting: restore_greeting(phase)?,
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
        Some(Value::String(choose)) if choose == "choose" => return Ok(Start::Choose),
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
    fold_log(&mut transcript, events)?;
    Ok(transcript)
}

/// Folds a record's events onto `transcript`.
fn fold_log(transcript: &mut Transcript, events: &[Value]) -> Result<(), Unreadable> {
    for value in events {
        let event = Event::from_value(value).map_err(|error| Unreadable::Log {
            error: error.to_string(),
        })?;
        transcript.apply(&event).map_err(|error| Unreadable::Log {
            error: error.to_string(),
        })?;
    }
    Ok(())
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
                text: format!("{NOT_RESTORED}: {}", brindle_transcript::clean(&why.to_string())),
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
    use brindle_transcript::{Exit, HoldReason, Slot, TurnState};
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
            Phase::Spawning {
                start: Start::Choose,
            },
            Phase::Initializing {
                start: Start::Choose,
            },
            Phase::Choosing {
                greeting: greeting(Some("fake"), Some("1.0"), Continuation::ResumeAndLoad),
            },
            Phase::LoginRequired(login(AfterLogin::Start(Start::Choose))),
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

    /// `agents-session-list`: a choosing session keeps that its agent
    /// lists and that a continue out was chosen; a list out isn't kept, so
    /// its late answer is ignored (flag 4).
    #[test]
    fn a_choosing_session_round_trips_without_its_list() {
        let mut agent = Agent {
            s: AgentSession::choosing("/home/u/src".into(), "0.3.0".into()),
            now: Instant::now(),
        };
        agent.step(Input::Spawned);
        agent.feed(&result(
            1,
            json!({"protocolVersion": 1,
            "agentCapabilities": {"loadSession": true, "sessionCapabilities": {"list": {}}},
            "agentInfo": {"name": "fake", "version": "1.0"}}),
        ));
        assert!(matches!(agent.s.phase, Phase::Choosing { .. }));
        assert_eq!(agent.round_trip(), agent.s);
        let (state, _) = state_of(&encode(&agent.s).expect("it fits"));
        assert_eq!(state["lists"], json!(true));
        let listed = agent.step(Input::Command(Command::List { req: 9, cursor: None }));
        assert_eq!(listed.reply, Some(Ok(())));
        let mut restored = Agent {
            s: agent.round_trip(),
            now: agent.now,
        };
        assert_eq!(restored.s.rpc.listing(), None, "a list isn't checkpointed");
        let late = restored.feed(&result(2, json!({"sessions": []})));
        assert_eq!(late.listed, None);
        assert!(restored.s.lists);
        // A choice out is kept as one.
        agent.step(Input::Command(Command::Choose {
            conversation: Some("c1".into()),
        }));
        assert!(agent.s.chose);
        assert!(agent.round_trip().chose);
        // A state written before reads as an agent that lists nothing.
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        state.as_object_mut().unwrap().remove("lists");
        assert!(!decode(&rebuild(&state, &partial), agent.now).unwrap().lists);
        state["lists"] = json!("yes");
        assert_eq!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Field { name: "lists" })
        );
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
            assert_eq!(bytes[bytes.len() - 4..], [0, 0, 0, 0], "no line bytes follow");
            assert!(state_of(&bytes).1.is_empty());
            assert_eq!(decode(&bytes, agent.now).expect("it reads"), agent.s);
        }
    }

    #[test]
    fn the_header_is_the_magic_and_format_2_then_a_base_record() {
        let bytes = encode(&Agent::new().s).expect("it fits");
        assert_eq!(&bytes[..4], b"BACP");
        assert_eq!(&bytes[4..6], &[0, 2]);
        assert_eq!(bytes[6], b'B');
        assert_eq!(&bytes[7..11], &[0, 0, 0, 0], "the base is record 0");
        let (state, partial) = state_of(&bytes);
        assert_eq!(state["acp"], 1);
        assert_eq!(state["phase"], json!({"phase": "spawning"}));
        assert!(partial.is_empty());
    }

    #[test]
    fn a_state_with_fields_it_doesnt_know_still_reads() {
        let agent = Agent::busy();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        state["later"] = json!({"added": true});
        let rebuilt = rebuild(&state, &partial);
        assert_eq!(decode(&rebuilt, agent.now).expect("it reads"), agent.s);
    }

    /// A format 1 checkpoint of `state` and the line bytes `partial`, as
    /// builds before format 2 wrote them (still read).
    fn rebuild(state: &Value, partial: &[u8]) -> Vec<u8> {
        let state = state.to_string();
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&FORMAT_1.to_be_bytes());
        out.extend_from_slice(&(state.len() as u32).to_be_bytes());
        out.extend_from_slice(state.as_bytes());
        out.extend_from_slice(partial);
        out
    }

    /// A format 2 checkpoint's base record's state and line bytes.
    fn state_of(bytes: &[u8]) -> (Value, Vec<u8>) {
        assert_eq!(&bytes[4..6], &FORMAT.to_be_bytes());
        let mut rest = &bytes[PREFIX_LEN..];
        let base = next_record(&mut rest).expect("a record").expect("a base");
        assert_eq!(base.kind, BASE);
        let state = serde_json::from_slice(base.state).expect("JSON");
        (state, base.partial.to_vec())
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
        // Entries of text that escapes to twice its length, up to just
        // below the log's bound (16 MiB encoded), where it is largest.
        // Agent and thought chunks alternate, so each is an entry of its own.
        let piece = "\"".repeat(32 * 1024);
        let thought = update(json!({"sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": piece}}));
        while agent.s.transcript.encoded_len() <= reduce::LOG_LIMIT - 300 * 1024 {
            agent.feed(&chunk(&piece));
            agent.feed(&thought);
        }
        assert_eq!(agent.s.transcript.dropped(), 0, "no trim yet");
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

    /// A format 2 checkpoint written before a replay grouped a tool entry's
    /// items, one item a `tool_items` event, still decodes to the same
    /// session (`agents-event-bounds` task 2.3).
    #[test]
    fn a_format_2_checkpoint_written_with_one_item_an_event_still_decodes() {
        let mut agent = Agent::ready();
        agent.prompt("go");
        agent.feed(&update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1",
            "title": "Read", "kind": "read", "status": "completed",
            "content": [
                {"type": "content", "content": {"type": "text", "text": "one"}},
                {"type": "content", "content": {"type": "text", "text": "two"}},
                {"type": "content", "content": {"type": "text", "text": "three"}},
            ]})));
        let bytes = encode(&agent.s).expect("it fits");
        let (mut state, partial) = state_of(&bytes);
        let log = state["log"].as_array().expect("a log").clone();
        let mut split = Vec::new();
        let mut grouped = 0;
        for event in log {
            match event["event"].as_str() {
                Some("tool_items") if event["items"].as_array().is_some_and(|items| items.len() > 1) => {
                    grouped += 1;
                    let from = event["from"].as_u64().expect("from");
                    for (at, item) in event["items"].as_array().expect("items").iter().enumerate() {
                        let mut one = event.clone();
                        one["from"] = json!(from + at as u64);
                        one["items"] = json!([item]);
                        split.push(one);
                    }
                }
                Some(_) | None => split.push(event),
            }
        }
        assert_eq!(grouped, 1, "the replay groups the three items");
        state["log"] = Value::Array(split);
        let mut old = bytes[..PREFIX_LEN].to_vec();
        push_record(&mut old, BASE, 0, &state, &partial, MAX_CHECKPOINT_LEN).expect("it fits");
        assert_eq!(decode(&old, agent.now).expect("it reads"), agent.s);
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
        bytes[4..6].copy_from_slice(&3u16.to_be_bytes());
        let adopted = adopt(&bytes, false, "/home/u/src", "0.3.0", Instant::now());
        assert_eq!(adopted.restored, Err(Unreadable::Format { format: 3 }));
        assert_not_restored(&adopted);
        let entries = entries(&adopted.session);
        assert!(matches!(&entries[0], Entry::Notice { text, .. }
            if text.contains("format 3") && text.contains("reads formats 1 and 2")));
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

    /// The converse of the check above (`agents-process-lifecycle`
    /// decision 3): a pending permission request in the log that `Rpc`
    /// doesn't await could never be answered, so the checkpoint is refused.
    #[test]
    fn a_checkpoint_with_a_pending_permission_rpc_doesnt_await_is_unreadable() {
        let agent = Agent::busy();
        let entry = *agent.s.rpc.permissions().keys().next().expect("a request");
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        state["rpc"]["permissions"] = json!([]);
        let refused = decode(&rebuild(&state, &partial), agent.now);
        assert_eq!(
            refused,
            Err(Unreadable::Entry {
                name: "permissions",
                entry
            })
        );
        assert_eq!(
            refused.unwrap_err().to_string(),
            format!("its checkpoint's permissions and its log disagree about entry {entry}")
        );
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

    // --- format 2: segments (agents-log-paging decision 7) ----------------

    /// A stored checkpoint the core appends to, as the holder keeps it.
    struct Stored {
        bytes: Vec<u8>,
        seq: u32,
        unsaved: Vec<Event>,
    }

    impl Stored {
        fn whole(session: &AgentSession) -> Self {
            Self {
                bytes: encode(session).expect("it fits"),
                seq: 0,
                unsaved: Vec::new(),
            }
        }

        /// Notes what a step applied, as the core keeps `unsaved`.
        fn saw(&mut self, out: Output) -> Output {
            self.unsaved.extend(out.events.iter().cloned());
            out
        }

        fn append(&mut self, session: &AgentSession) {
            self.seq += 1;
            let segment = encode_segment(session, &self.unsaved, self.seq).expect("it fits");
            self.bytes.extend_from_slice(&segment);
            self.unsaved.clear();
        }

        fn decode(&self, now: Instant) -> AgentSession {
            decode(&self.bytes, now).expect("it reads")
        }
    }

    #[test]
    fn a_base_and_three_segments_decode_to_the_live_session_the_last_record_winning() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("first"));
        stored.saw(agent.feed(&chunk("Working on ")));
        stored.saw(agent.feed(&chunk("it")));
        stored.append(&agent.s);
        stored.saw(agent.feed(&update(
            json!({"sessionUpdate": "tool_call", "toolCallId": "c1",
            "title": "Write", "kind": "edit", "status": "pending"}),
        )));
        stored.saw(agent.feed(&permission(json!("p1"))));
        stored.saw(agent.prompt("second"));
        stored.saw(agent.feed(b"{\"jsonrpc\":\"2.0\",\"method\":\"session/up"));
        stored.append(&agent.s);
        assert_eq!(stored.decode(agent.now), agent.s);
        assert_eq!(stored.decode(agent.now).rpc.permissions().len(), 1);
        // The partial line completes, the turn ends and the permission goes:
        // the last record's partial, phase and requests win.
        stored.saw(agent.feed(b"date\",\"params\":{\"sessionId\":\"s1\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\" done\"}}}}\n"));
        stored.saw(agent.feed(&result(3, json!({"stopReason": "end_turn"}))));
        agent.s.methods = creating().s.methods;
        stored.append(&agent.s);
        assert_eq!(stored.seq, 3);
        let decoded = stored.decode(agent.now);
        assert_eq!(decoded, agent.s);
        assert!(matches!(decoded.lines.partial(), Partial::Collecting(bytes) if bytes.is_empty()));
        assert_eq!(decoded.methods.len(), 2);
    }

    #[test]
    fn a_segment_written_while_logging_in_round_trips() {
        let mut agent = creating();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.feed(&auth_error(2, "Authentication required")));
        stored.append(&agent.s);
        assert!(matches!(stored.decode(agent.now).phase, Phase::LoginRequired(_)));
        assert_eq!(stored.decode(agent.now), agent.s);
        stored.saw(login_cmd(&mut agent, "oauth-personal"));
        stored.append(&agent.s);
        assert!(matches!(stored.decode(agent.now).phase, Phase::LoggingIn { .. }));
        assert_eq!(stored.decode(agent.now), agent.s);
    }

    #[test]
    fn a_segment_written_during_a_muted_load_round_trips() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("first"));
        stored.saw(agent.feed(&chunk("Done.")));
        stored.saw(agent.feed(&result(3, json!({"stopReason": "end_turn"}))));
        stored.saw(agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        }));
        stored.append(&agent.s);
        stored.saw(agent.step(Input::Restart));
        stored.saw(agent.step(Input::Spawned));
        stored.saw(agent.feed(&result(
            4,
            json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}),
        )));
        stored.saw(agent.feed(&chunk("replayed")));
        stored.saw(agent.feed(&permission(json!(9))));
        stored.append(&agent.s);
        let decoded = stored.decode(agent.now);
        assert!(matches!(
            decoded.phase,
            Phase::Loading {
                replay: Replay::Muted { updates: 1 },
                ..
            }
        ));
        assert_eq!(decoded, agent.s);
    }

    #[test]
    fn a_segment_written_after_a_retitle_round_trips_with_the_title() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.feed(&update(json!({"sessionUpdate": "session_info_update",
            "title": "Hello.txt file creation"}))));
        stored.append(&agent.s);
        let decoded = stored.decode(agent.now);
        assert_eq!(decoded, agent.s);
        assert_eq!(
            decoded.transcript.conversation().and_then(|c| c.title.as_deref()),
            Some("Hello.txt file creation")
        );
    }

    #[test]
    fn appended_text_is_merged_per_entry_within_a_chunk_and_folds_the_same() {
        let a = EntryId(1);
        let b = EntryId(2);
        let appended = |id, text: &str| Event::Appended {
            id,
            text: text.into(),
        };
        let events = [
            appended(a, "one "),
            appended(a, "two"),
            appended(b, "x"),
            appended(a, "three"),
        ];
        assert_eq!(
            merged(&events),
            [appended(a, "one two"), appended(b, "x"), appended(a, "three")]
        );
        let half = "y".repeat(TEXT_CHUNK / 2 + 1);
        assert_eq!(
            merged(&[appended(a, &half), appended(a, &half)]).len(),
            2,
            "a merge never passes TEXT_CHUNK"
        );
        // A run of chunks makes a segment of a few events that folds to the
        // same text.
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("go"));
        for n in 0..200 {
            stored.saw(agent.feed(&chunk(&format!("piece {n} "))));
        }
        let unsaved = stored.unsaved.len();
        stored.append(&agent.s);
        let mut rest = &stored.bytes[PREFIX_LEN..];
        next_record(&mut rest).expect("a base");
        let segment = next_record(&mut rest).expect("a record").expect("a segment");
        let state: Value = serde_json::from_slice(segment.state).expect("JSON");
        let written = state["events"].as_array().expect("events").len();
        assert!(written < unsaved / 10, "{written} of {unsaved}");
        assert_eq!(stored.decode(agent.now), agent.s);
    }

    /// A format 2 record of `kind`, numbered `seq`, with the state `state`.
    fn record(kind: u8, seq: u32, state: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        push_record(&mut out, kind, seq, state, &[], MAX_CHECKPOINT_LEN).expect("it fits");
        out
    }

    #[test]
    fn a_format_2_checkpoint_out_of_order_or_cut_short_is_adopted_as_not_restored() {
        let agent = Agent::ready();
        let base = encode(&agent.s).expect("it fits");
        let (state, _) = state_of(&base);
        let mut segment_state = state.clone();
        if let Some(object) = segment_state.as_object_mut() {
            object.remove("log");
            object.insert("events".into(), json!([]));
        }
        let header = base[..PREFIX_LEN].to_vec();
        let joined = |parts: &[&[u8]]| parts.concat();
        let segment = |seq| record(SEGMENT, seq, &segment_state);
        let cases: Vec<(Vec<u8>, Unreadable)> = vec![
            (header.clone(), Unreadable::NoBase),
            (joined(&[&header, &segment(0)]), Unreadable::NoBase),
            (
                joined(&[&header, &record(b'X', 0, &state)]),
                Unreadable::RecordKind { kind: b'X' },
            ),
            (
                joined(&[&base, &record(BASE, 1, &state)]),
                Unreadable::RecordKind { kind: BASE },
            ),
            (
                joined(&[&header, &record(BASE, 1, &state)]),
                Unreadable::Sequence {
                    expected: 0,
                    found: 1,
                },
            ),
            (
                joined(&[&base, &segment(2)]),
                Unreadable::Sequence {
                    expected: 1,
                    found: 2,
                },
            ),
            (
                joined(&[&base, &segment(1)[..7]]),
                Unreadable::Truncated,
            ),
            (base[..base.len() - 1].to_vec(), Unreadable::Truncated),
        ];
        for (bytes, why) in cases {
            let adopted = adopt(&bytes, false, "/home/u/src", "0.3.0", agent.now);
            assert_eq!(adopted.restored, Err(why));
            assert_not_restored(&adopted);
        }
        // The segment itself reads.
        let fine = joined(&[&base, &segment(1)]);
        assert_eq!(decode(&fine, agent.now), Ok(agent.s.clone()));
    }

    #[test]
    fn the_new_refusals_read_as_what_went_wrong() {
        assert_eq!(
            Unreadable::NoBase.to_string(),
            "its checkpoint doesn't start with its base"
        );
        assert_eq!(
            Unreadable::RecordKind { kind: b'X' }.to_string(),
            "its checkpoint has a record of unknown kind 0x58"
        );
        assert_eq!(
            Unreadable::Sequence {
                expected: 2,
                found: 3
            }
            .to_string(),
            "its checkpoint's record 3 came where record 2 was due, so a part of it is missing"
        );
    }

    #[test]
    fn a_format_1_checkpoint_with_a_title_and_a_login_still_adopts() {
        let mut agent = Agent::ready();
        agent.s.methods = creating().s.methods;
        agent.feed(&update(json!({"sessionUpdate": "session_info_update",
            "title": "Hello.txt file creation"})));
        assert_eq!(agent.prompt("hi").reply, Some(Ok(())));
        agent.feed(&auth_error(3, "expired"));
        let (state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        let format_1 = rebuild(&state, &partial);
        assert_eq!(&format_1[4..6], &[0, 1]);
        let adopted = adopt(&format_1, false, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Ok(()));
        assert_eq!(adopted.session, agent.s);
        assert!(matches!(adopted.session.phase, Phase::LoginRequired(_)));
    }

    /// `agents-tool-content` decision 2: the `acp` field a checkpoint from
    /// before `Dialect` recorded restores the session's dialect, in either
    /// format, and the pinned bytes below still decode.
    #[test]
    fn a_format_1_or_2_checkpoint_restores_its_dialect() {
        let agent = Agent::ready();
        let format_2 = encode(&agent.s).expect("it fits");
        let (state, partial) = state_of(&format_2);
        assert_eq!(state["acp"], 1);
        let format_1 = rebuild(&state, &partial);
        for bytes in [format_1, format_2] {
            let session = decode(&bytes, agent.now).expect("it decodes");
            assert_eq!(session.rpc.dialect(), Dialect::newest());
            assert_eq!(session.rpc.dialect().version(), 1);
        }
        let pinned: Vec<u8> = (0..PINNED.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&PINNED[at..at + 2], 16).expect("hex"))
            .collect();
        let session = decode(&pinned, Instant::now()).expect("it decodes");
        assert_eq!(session.rpc.dialect().version(), 1);
    }

    #[test]
    fn a_small_sessions_format_2_bytes_are_pinned() {
        let session = AgentSession::new("/w".into(), "0.3.0".into());
        let bytes = encode(&session).expect("it fits");
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, PINNED);
        assert_eq!(decode(&bytes, Instant::now()), Ok(session));
    }

    /// `BACP`, format 2, a base record numbered 0 with the state's length,
    /// the state, and no partial line.
    const PINNED: &str = concat!(
        "4241435000024200000000000001267b22616370223a312c22637764223a222f77222c2276657273",
        "696f6e223a22302e332e30222c227068617365223a7b227068617365223a22737061776e696e6722",
        "7d2c22727063223a7b226e6578745f6964223a312c226f7574676f696e67223a5b5d2c227065726d",
        "697373696f6e73223a5b5d7d2c2272756e223a6e756c6c2c22746f6f6c73223a7b7d2c226261645f",
        "6c696e6573223a302c226c696e6573223a7b227061727469616c223a22636f6c6c656374696e6722",
        "2c22636f76657273223a307d2c226c6f67223a5b7b226576656e74223a227475726e222c22737461",
        "7465223a7b227475726e223a2269646c65227d7d2c7b226576656e74223a226167656e74222c2273",
        "74617465223a7b226167656e74223a227374617274696e67227d7d5d7d00000000",
    );

    fn error(id: u64, code: i64, message: &str) -> Vec<u8> {
        line(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
    }

    /// The log's pending permission requests are the ones `Rpc` awaits
    /// (`agents-process-lifecycle` decision 3), and a process the session
    /// is done with has nothing out: no permission and no list.
    fn invariant(session: &AgentSession) {
        assert_eq!(
            reduce::pending_permissions(session.transcript()),
            session.rpc.waiting_permissions(),
            "the log's pending permission requests aren't the ones awaited"
        );
        let closed = match session.phase {
            Phase::Restarting { .. } | Phase::Refused | Phase::Broken | Phase::Exited | Phase::Lost => true,
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Ready { .. }
            | Phase::Choosing { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. } => false,
        };
        if closed {
            assert!(
                session.rpc.waiting_permissions().is_empty() && session.rpc.listing().is_none(),
                "{:?} left requests out: {:?}",
                session.phase,
                session.rpc
            );
        }
    }

    /// `Input::Restart` as the core steps it: after a login demand the old
    /// agent is hung up and the start is made again; once placed, the new
    /// agent's `SPAWNED`, and the answers that bring it to ready. Every
    /// output but the last is noted in `stored`; the last is returned.
    fn restarted(agent: &mut Agent, stored: &mut Stored) -> Output {
        let out = agent.step(Input::Restart);
        if out.reply != Some(Ok(())) {
            return out;
        }
        invariant(&agent.s);
        let mut outs = vec![out];
        if matches!(agent.s.phase, Phase::Restarting { .. }) {
            outs.push(agent.step(Input::Restart));
            invariant(&agent.s);
        }
        outs.push(agent.step(Input::Spawned));
        invariant(&agent.s);
        let request = |out: &Output| {
            sent(&out.stdin)
                .into_iter()
                .find_map(|m| Some((m["method"].as_str()?.to_owned(), m["id"].as_u64()?)))
        };
        if let Some((_, id)) = outs.last().and_then(request) {
            outs.push(agent.feed(&result(
                id,
                json!({"protocolVersion": 1,
                "agentCapabilities": {"loadSession": true, "sessionCapabilities": {"resume": {}}},
                "agentInfo": {"name": "fake", "version": "1.0"}}),
            )));
        }
        match outs.last().and_then(request) {
            Some((method, id)) if method == "session/new" => {
                outs.push(agent.feed(&result(id, json!({"sessionId": "s1"}))));
            }
            Some((_, id)) => outs.push(agent.feed(&result(id, json!({})))),
            None => {}
        }
        let last = outs.pop().unwrap_or_default();
        for out in outs {
            stored.saw(out);
            invariant(&agent.s);
        }
        last
    }

    /// Random reducer steps checkpointed at random points, whole or
    /// appended: what the holder stores always decodes to the live session,
    /// and the log's pending permission requests are always the ones
    /// awaited. The steps include the ends of an agent process: a login
    /// demand and its restart, an exit and the restart after it, a fault,
    /// answers and withdrawals of permissions, and a list out.
    #[test]
    fn random_steps_checkpointed_whole_or_appended_decode_to_the_live_session() {
        let mut rng = XorShift(0x5eed_5e9e_0000_0002);
        for _ in 0..300 {
            let mut agent = Agent::ready();
            // It offers lists too, so a list can be out when it stops.
            agent.s.lists = true;
            let mut stored = Stored::whole(&agent.s);
            let mut prompt_id = None;
            let mut list_id = None;
            let mut req = 0;
            for _ in 0..40 {
                let out = match rng.below(20) {
                    0 => {
                        let out = agent.prompt("go on");
                        if let Some(id) = sent(&out.stdin).first().and_then(|m| m["id"].as_u64()) {
                            prompt_id = Some(id);
                        }
                        out
                    }
                    1 | 2 => agent.feed(&chunk(&"text ".repeat(1 + rng.below(30)))),
                    3 => agent.feed(&update(json!({"sessionUpdate": "agent_thought_chunk",
                        "content": {"type": "text", "text": "hmm"}}))),
                    4 => {
                        let call = format!("c{}", rng.below(4));
                        agent.feed(&update(json!({"sessionUpdate": "tool_call",
                            "toolCallId": call, "title": "Run", "kind": "execute",
                            "status": "pending"})))
                    }
                    5 => {
                        let call = format!("c{}", rng.below(4));
                        agent.feed(&update(json!({"sessionUpdate": "tool_call_update",
                            "toolCallId": call, "status": "completed"})))
                    }
                    6 => agent.feed(&permission(json!(format!("p{}", rng.below(100))))),
                    7 => match prompt_id.take() {
                        Some(id) => agent.feed(&result(id, json!({"stopReason": "end_turn"}))),
                        None => agent.step(Input::Command(Command::Cancel)),
                    },
                    8 => agent.feed(&update(json!({"sessionUpdate": "session_info_update",
                        "title": format!("title {}", rng.below(3))}))),
                    9 => {
                        let line = chunk("split");
                        let at = rng.below(line.len());
                        agent.feed(&line[..at])
                    }
                    10 => agent.feed(b"not json\n"),
                    11 => agent.step(Input::Command(Command::Unqueue)),
                    // A login demand for the prompt out.
                    12 => match prompt_id.take() {
                        Some(id) => agent.feed(&error(id, crate::acp::forms::AUTH_REQUIRED, "log in")),
                        None => agent.step(Input::Tick),
                    },
                    // A restart: after a login demand or an exit, the agent
                    // is started again and brought to ready if it can be.
                    13 => {
                        let out = restarted(&mut agent, &mut stored);
                        // A refused restart leaves what was out.
                        prompt_id = prompt_id.filter(|id| agent.s.rpc.outgoing().contains_key(id));
                        list_id = list_id.filter(|id| agent.s.rpc.outgoing().contains_key(id));
                        out
                    }
                    14 => {
                        prompt_id = None;
                        list_id = None;
                        agent.step(Input::Exited {
                            code: Some(1),
                            stderr: b"gone",
                        })
                    }
                    15 => agent.step(Input::Fault {
                        reason: "random fault".into(),
                    }),
                    16 => match agent.s.rpc.waiting_permissions().first() {
                        Some(&entry) => agent.step(Input::Command(Command::Answer {
                            entry,
                            option: Some("allow".into()),
                        })),
                        None => agent.step(Input::Tick),
                    },
                    17 => agent.feed(&line(json!({"jsonrpc": "2.0", "method": "$/cancel_request",
                        "params": {"requestId": format!("p{}", rng.below(100))}}))),
                    18 => {
                        req += 1;
                        let out = agent.step(Input::Command(Command::List { req, cursor: None }));
                        if let Some(id) = sent(&out.stdin).first().and_then(|m| m["id"].as_u64()) {
                            list_id = Some(id);
                        }
                        out
                    }
                    _ => match list_id.take() {
                        Some(id) => agent.feed(&result(id, json!({"sessions": []}))),
                        None => agent.step(Input::Tick),
                    },
                };
                stored.saw(out);
                invariant(&agent.s);
                match rng.below(5) {
                    0 => stored = Stored::whole(&agent.s),
                    1 | 2 => stored.append(&agent.s),
                    _ => {}
                }
            }
            // A list out isn't checkpointed (`agents-session-list` flag
            // 4): it is answered before the comparison.
            if let Some(id) = list_id {
                stored.saw(agent.feed(&result(id, json!({"sessions": []}))));
                invariant(&agent.s);
            }
            stored.append(&agent.s);
            assert_eq!(stored.decode(agent.now), agent.s);
        }
    }

    // --- fuzz -----------------------------------------------------------

    #[test]
    fn fuzz_decoding_never_panics_on_random_or_mutated_checkpoints() {
        let mut rng = XorShift(0x5eed_ac01_c4ec_0001);
        let now = Instant::now();
        let valid = encode(&Agent::busy().s).expect("it fits");
        // A base and two segments, so mutations reach format 2's records.
        let segmented = {
            let mut agent = Agent::ready();
            let mut stored = Stored::whole(&agent.s);
            stored.saw(agent.prompt("first"));
            stored.saw(agent.feed(&chunk("Working")));
            stored.append(&agent.s);
            stored.saw(agent.feed(&permission(json!("p1"))));
            stored.append(&agent.s);
            stored.bytes
        };
        for round in 0..3000 {
            let valid = if round % 3 == 0 { &segmented } else { &valid };
            let bytes: Vec<u8> = match round % 4 {
                // Random bytes, sometimes behind a good header.
                0 => (0..rng.below(64)).map(|_| rng.next() as u8).collect(),
                1 => {
                    let mut bytes = valid[..PREFIX_LEN + rng.below(RECORD_HEADER_LEN)].to_vec();
                    bytes.extend((0..rng.below(256)).map(|_| rng.next() as u8));
                    bytes
                }
                // A valid checkpoint cut short or with bytes flipped.
                2 => valid[..rng.below(valid.len())].to_vec(),
                _ => {
                    let mut bytes = valid.to_vec();
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

    // --- tool content (`agents-tool-content` decision 8) -----------------

    fn tool_update(meta: Value, content: Value) -> Vec<u8> {
        update(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1",
            "content": content, "_meta": meta}),
        )
    }

    /// A session mid-turn with a diff, and an output whose last piece ended
    /// inside an escape sequence and a character.
    fn with_tool_content(stored: Option<&mut Stored>) -> Agent {
        let mut agent = Agent::ready();
        let mut seen = Vec::new();
        seen.push(agent.prompt("go"));
        seen.push(agent.feed(&update(
            json!({"sessionUpdate": "tool_call", "toolCallId": "c1",
            "title": "Edit", "kind": "edit",
            "content": [{"type": "diff", "path": "/w/a", "oldText": "1\n2\n", "newText": "1\nb\n"},
                {"type": "terminal", "terminalId": "t1"}]}),
        )));
        seen.push(agent.feed(&tool_update(
            json!({"terminal_info": {"terminal_id": "t1", "cwd": "/w"},
                "terminal_output_delta": {"terminal_id": "t1", "data": "line\n\u{1b}[3"}}),
            Value::Null,
        )));
        // Data is a JSON string, so no character arrives cut; the escape
        // sequence is what stays held between pieces.
        seen.push(agent.feed(&tool_update(
            json!({"terminal_output_delta": {"terminal_id": "t1", "data": "1mbold"}}),
            Value::Null,
        )));
        if let Some(stored) = stored {
            for out in seen {
                stored.saw(out);
            }
        }
        agent
    }

    #[test]
    fn a_session_with_tool_content_round_trips_whole_and_as_segments() {
        let agent = with_tool_content(None);
        let open = agent.s.outputs.get("t1").expect("open");
        assert_eq!(open.filter.line, "bold");
        let decoded = agent.round_trip();
        assert_eq!(decoded, agent.s);
        // A sequence cut between two pieces, restored, is still removed.
        let mut agent = with_tool_content(None);
        agent.feed(&tool_update(
            json!({"terminal_output_delta": {"terminal_id": "t1", "data": "\u{1b}[3"}}),
            Value::Null,
        ));
        assert_eq!(agent.s.outputs["t1"].filter.mode, Mode::Csi);
        let mut restored = Agent {
            s: agent.round_trip(),
            now: agent.now,
        };
        assert_eq!(restored.s, agent.s);
        restored.feed(&tool_update(
            json!({"terminal_output_delta": {"terminal_id": "t1", "data": "2mtail\n"},
                "terminal_exit": {"terminal_id": "t1", "exit_code": 0}}),
            Value::Null,
        ));
        let Some(Entry::Tool { content, .. }) = entries(&restored.s).into_iter().nth(1) else {
            panic!("no tool");
        };
        assert!(
            matches!(&content[1], ToolItem::Output(output) if output.text == "line\nboldtail\n" && output.cwd.as_deref() == Some("/w")),
            "{content:?}"
        );
        // As a base and segments, the last one mid-sequence.
        let base = Agent::ready();
        let mut stored = Stored::whole(&base.s);
        let mut agent = with_tool_content(Some(&mut stored));
        stored.append(&agent.s);
        let out = agent.feed(&tool_update(
            json!({"terminal_output_delta": {"terminal_id": "t1", "data": " more\u{1b}]0;ti"}}),
            Value::Null,
        ));
        stored.saw(out);
        stored.append(&agent.s);
        assert_eq!(stored.decode(agent.now), agent.s);
    }

    #[test]
    fn tool_output_is_merged_per_item_within_its_limit_keeping_the_later_exit_and_folds_the_same() {
        let output = |id, item, text: &str, exit| Event::ToolOutput {
            id: EntryId(id),
            item,
            text: text.into(),
            exit,
        };
        let done = Some(Exit::Code { code: 0 });
        assert_eq!(
            merged(&[
                output(1, 0, "a", None),
                output(1, 0, "b", done.clone()),
                output(1, 0, "c", None),
                output(1, 1, "x", None),
                output(1, 0, "d", None),
            ]),
            [
                output(1, 0, "ab", done.clone()),
                // Nothing follows an exit into its event.
                output(1, 0, "c", None),
                output(1, 1, "x", None),
                output(1, 0, "d", None),
            ]
        );
        let half = "y".repeat(ITEM_LIMIT / 2 + 1);
        assert_eq!(
            merged(&[output(1, 0, &half, None), output(1, 0, &half, None)]).len(),
            2,
            "a merge never passes ITEM_LIMIT"
        );
        // Streamed output makes a segment of a few events folding the same.
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("go"));
        for n in 0..200 {
            stored.saw(agent.feed(&tool_update(
                json!({"terminal_output_delta": {"terminal_id": "t1", "data": format!("{n}\n")}}),
                Value::Null,
            )));
        }
        let unsaved = stored.unsaved.len();
        stored.append(&agent.s);
        let mut rest = &stored.bytes[PREFIX_LEN..];
        next_record(&mut rest).expect("a base");
        let segment = next_record(&mut rest)
            .expect("a record")
            .expect("a segment");
        let state: Value = serde_json::from_slice(segment.state).expect("JSON");
        let written = state["events"].as_array().expect("events").len();
        assert!(written < unsaved / 10, "{written} of {unsaved}");
        assert_eq!(stored.decode(agent.now), agent.s);
    }

    #[test]
    fn an_open_output_naming_no_output_item_is_unreadable() {
        let agent = with_tool_content(None);
        let (state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        assert!(state["outputs"]["t1"].is_object(), "{state}");
        let mut no_entry = state.clone();
        no_entry["outputs"]["t1"]["entry"] = json!(99);
        let mut not_output = state.clone();
        // Item 0 is the diff.
        not_output["outputs"]["t1"]["item"] = json!(0);
        let mut user = state.clone();
        user["outputs"]["t1"]["entry"] = json!(1);
        user["outputs"]["t1"]["item"] = json!(1);
        for (bad, entry) in [(no_entry, 99), (not_output, 2), (user, 1)] {
            assert_eq!(
                decode(&rebuild(&bad, &partial), agent.now),
                Err(Unreadable::Entry {
                    name: "outputs",
                    entry: EntryId(entry)
                }),
                "{bad}"
            );
        }
        let mut bad_mode = state.clone();
        bad_mode["outputs"]["t1"]["mode"] = json!("teleport");
        let mut long_partial = state;
        long_partial["outputs"]["t1"]["partial"] = json!([1, 2, 3, 4]);
        for bad in [bad_mode, long_partial] {
            assert_eq!(
                decode(&rebuild(&bad, &partial), agent.now),
                Err(Unreadable::Field { name: "outputs" })
            );
        }
    }

    // --- commands and options (agents-commands) -------------------------

    /// Ready in `s1` with commands, and the options made from v1's modes
    /// (keys the reducer can't name) beside a config option.
    fn with_options() -> Agent {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&result(
            1,
            json!({"protocolVersion": 1,
            "agentCapabilities": {"loadSession": true, "sessionCapabilities": {"resume": {}}},
            "agentInfo": {"name": "fake", "version": "1.0"}}),
        ));
        agent.feed(&result(2, json!({"sessionId": "s1",
            "modes": {"currentModeId": "default", "availableModes": [
                {"id": "default", "name": "Default"}, {"id": "plan", "name": "Plan"}]},
            "configOptions": [{"id": "fast", "name": "Fast", "type": "boolean", "currentValue": false}]})));
        agent.feed(&update(json!({"sessionUpdate": "available_commands_update",
            "availableCommands": [{"name": "compact", "description": "Compacts", "input": {"hint": "how"}}]})));
        assert_eq!(agent.s.transcript.options().len(), 2);
        agent
    }

    #[test]
    fn options_from_modes_round_trip_as_a_base_and_with_segments_and_a_set_out_is_dropped() {
        let mut agent = with_options();
        let restored = agent.round_trip();
        assert_eq!(restored, agent.s);
        assert_eq!(restored.keys.len(), 2);
        assert_eq!(restored.modes.get("mode").map(String::as_str), Some("default"));
        // A restored session sets its options by their keys.
        let mut again = Agent {
            s: restored,
            now: agent.now,
        };
        let out = again.step(Input::Command(Command::SetOption {
            option: "mode".into(),
            value: brindle_remote_protocol::client::OptionValue::Choice {
                value: "plan".into(),
            },
        }));
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(sent(&out.stdin)[0]["method"], "session/set_mode");
        // With segments: a mode change and new commands as events.
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.feed(&update(
            json!({"sessionUpdate": "current_mode_update", "currentModeId": "plan"}),
        )));
        stored.saw(agent.feed(&update(json!({"sessionUpdate": "available_commands_update",
            "availableCommands": []}))));
        stored.append(&agent.s);
        let decoded = stored.decode(agent.now);
        assert_eq!(decoded, agent.s);
        assert_eq!(decoded.modes.get("mode").map(String::as_str), Some("plan"));
        assert!(decoded.transcript.commands().is_empty());
        // A set out at an encode isn't kept; its id is.
        let out = agent.step(Input::Command(Command::SetOption {
            option: "fast".into(),
            value: brindle_remote_protocol::client::OptionValue::Toggle { on: true },
        }));
        assert_eq!(out.reply, Some(Ok(())));
        assert!(agent.s.rpc.setting());
        let bytes = encode(&agent.s).expect("it fits");
        let adopted = adopt(&bytes, false, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Ok(()));
        assert!(!adopted.session.rpc.setting());
        assert_eq!(adopted.session.rpc.next_id(), agent.s.rpc.next_id());
        assert_eq!(adopted.session.transcript.options(), agent.s.transcript.options());
    }

    #[test]
    fn a_checkpoint_from_before_options_reads_with_both_lists_empty() {
        let agent = Agent::busy();
        let bytes = encode(&agent.s).expect("it fits");
        let (state, partial) = state_of(&bytes);
        assert!(state.get("option_keys").is_none() && state.get("modes").is_none());
        for restored in [decode(&bytes, agent.now), decode(&rebuild(&state, &partial), agent.now)] {
            let restored = restored.expect("it reads");
            assert!(restored.transcript.commands().is_empty());
            assert!(restored.transcript.options().is_empty());
            assert!(restored.keys.is_empty() && restored.modes.is_empty());
        }
        // Keys that aren't the held options' make it unreadable.
        let agent = with_options();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        state["option_keys"] = json!({"fast": {"config": "fast"}});
        assert_eq!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Field {
                name: "option_keys"
            })
        );
        state["option_keys"] = json!({"fast": "fast", "mode": "modes"});
        assert_eq!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Field {
                name: "option_keys"
            })
        );
        state["option_keys"] = json!({"fast": {"config": "fast"}, "mode": "modes"});
        state["modes"] = json!({"mode": 3});
        assert_eq!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Field { name: "modes" })
        );
    }
}
