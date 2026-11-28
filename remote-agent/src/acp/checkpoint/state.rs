use std::collections::BTreeMap;
use std::io::{self, Write};
use std::time::Instant;

use brindle_remote_protocol::serde_json::{self, Map, Value, json};
use brindle_transcript::{
    Continuation, Conversation, Entry, EntryId, Event, ITEM_LIMIT, LoginMethod, ReplayRef,
    TEXT_CHUNK, ToolItem, Transcript,
};

use crate::acp::dialect::{Dialect, Unsupported};
use crate::acp::envelope::Id;
use crate::acp::forms::OptionKey;
use crate::acp::lines::{Lines, Partial};
use crate::acp::plain::{self, Mode};
use crate::acp::reduce::{
    AfterLogin, AgentSession, BadRun, Greeting, Log, Login, OpenOutput, Phase, Replay, Run,
    Process, RunKind, Start, ToolBook, tool_index,
};
use crate::acp::rpc::{Purpose, Rpc};

use super::format::{Limited, Unreadable};

/// `events` with each run of `Appended`s to one entry merged into as few
/// as fit [`TEXT_CHUNK`] each: they fold to the same text. The events are
/// moved, not copied.
pub(super) fn merged(events: Vec<Event>) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    for event in events {
        if let Event::Appended { id, text } = &event
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
        } = &event
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
        out.push(event);
    }
    out
}

/// The partial line's bytes, all of them.
pub(super) fn partial_bytes(session: &AgentSession) -> &[u8] {
    match session.process.lines.partial() {
        Partial::Collecting(bytes) => bytes,
        Partial::TooLong { .. } | Partial::Resyncing { .. } => &[],
    }
}

/// What a record writes of the fields kept as changes (OpenSpec change
/// `agents-checkpoint-deltas`, decisions 3 and 4).
pub(super) struct Changes<'a> {
    /// `lines.from`: the partial line's bytes written start there; 0 when
    /// they are written whole, and then left out.
    pub(super) from: u64,
    /// The login methods, written when they are `Some`.
    pub(super) methods: Option<&'a [LoginMethod]>,
    /// The outputs: each one set or, `None`, closed; written when there is
    /// one.
    pub(super) outputs: Vec<(&'a str, Option<&'a OpenOutput>)>,
}

/// What follows the fields: the base's log or a segment's events.
pub(super) enum Body<'a> {
    Log(ReplayRef<'a>),
    Events(&'a [Event]),
}

/// A JSON object written a field at a time; the keys are this module's
/// own names, which need no escaping.
struct ObjectOut<'w, 'a> {
    out: &'w mut Limited<'a>,
    first: bool,
}

impl<'w, 'a> ObjectOut<'w, 'a> {
    fn start(out: &'w mut Limited<'a>) -> io::Result<Self> {
        out.write_all(b"{")?;
        Ok(Self { out, first: true })
    }

    /// Writes the key `name`, and gives the writer its value goes to.
    fn key(&mut self, name: &str) -> io::Result<&mut Limited<'a>> {
        if !self.first {
            self.out.write_all(b",")?;
        }
        self.first = false;
        self.out.write_all(b"\"")?;
        self.out.write_all(name.as_bytes())?;
        self.out.write_all(b"\":")?;
        Ok(self.out)
    }

    fn value(&mut self, name: &str, value: &Value) -> io::Result<()> {
        serde_json::to_writer(self.key(name)?, value).map_err(io::Error::from)
    }

    fn end(self) -> io::Result<()> {
        self.out.write_all(b"}")
    }
}

/// Writes `session`'s state into `out` as one JSON object: every field but
/// the log, those kept as changes as `changes` says, then `body`. The
/// fields come in a fixed order, the log or events last; the small ones
/// are built as values, and the log and the events stream from the
/// session.
pub(super) fn write_state(
    out: &mut Limited<'_>,
    session: &AgentSession,
    changes: &Changes<'_>,
    body: Body<'_>,
) -> io::Result<()> {
    let mut lines = match session.process.lines.partial() {
        Partial::Collecting(_) => json!({ "partial": "collecting" }),
        Partial::TooLong { len } => json!({ "partial": "too_long", "len": len }),
        Partial::Resyncing { len } => json!({ "partial": "resyncing", "len": len }),
    };
    if let Some(object) = lines.as_object_mut() {
        object.insert("covers".into(), session.process.lines.covers().into());
        if changes.from > 0 {
            object.insert("from".into(), changes.from.into());
        }
    }
    let mut state = ObjectOut::start(out)?;
    state.value("acp", &session.rpc.dialect().version().into())?;
    state.value("cwd", &session.cwd.as_str().into())?;
    state.value("version", &session.version.as_str().into())?;
    state.value("phase", &phase_value(&session.phase))?;
    state.value("rpc", &rpc_value(&session.rpc))?;
    state.value(
        "run",
        &session.process.run.map_or(Value::Null, |run| {
            json!({ "kind": run_kind_name(run.kind), "entry": run.entry.0 })
        }),
    )?;
    // The tool calls are the log's from here on (decision 2).
    state.value("tool_epoch", &session.process.book.tool_epoch.0.into())?;
    // The open outputs (`agents-tool-content` decision 8): a base's all of
    // them, a segment's those changed, left out when there are none.
    if !changes.outputs.is_empty() {
        let outputs: Map<String, Value> = changes
            .outputs
            .iter()
            .map(|(terminal, open)| {
                let open = open.map_or(Value::Null, open_output_value);
                ((*terminal).to_owned(), open)
            })
            .collect();
        state.value("outputs", &Value::Object(outputs))?;
    }
    state.value(
        "bad_lines",
        &session.process.bad.map_or(0, |bad| bad.lines).into(),
    )?;
    state.value("lines", &lines)?;
    if let Some(methods) = changes.methods {
        serde_json::to_writer(state.key("methods")?, methods).map_err(io::Error::from)?;
    }
    // Left out when false, so a state written before
    // `agents-session-list` reads as one whose agent lists nothing.
    if session.lists {
        state.value("lists", &json!(true))?;
    }
    if session.chose {
        state.value("chose", &json!(true))?;
    }
    // The options' keys and the modes last in effect (`agents-commands`,
    // decision 12), left out when empty.
    if !session.keys.is_empty() {
        let keys: Map<String, Value> = session
            .keys
            .iter()
            .map(|(id, key)| (id.clone(), key.to_value()))
            .collect();
        state.value("option_keys", &Value::Object(keys))?;
    }
    if !session.modes.is_empty() {
        state.value("modes", &json!(session.modes))?;
    }
    match body {
        Body::Log(log) => serde_json::to_writer(state.key("log")?, &log),
        Body::Events(events) => serde_json::to_writer(state.key("events")?, events),
    }
    .map_err(io::Error::from)?;
    state.end()
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

/// A state's bytes as its JSON object.
pub(super) fn state_object(state: &[u8]) -> Result<Map<String, Value>, Unreadable> {
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

pub(super) fn array<'a>(
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

/// The partial line a record left, as far as a chain's next record may
/// continue it: the tool epoch, the stream offset where the line started,
/// and how many of its bytes the chain holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HeldLine {
    epoch: EntryId,
    start: u64,
    len: u64,
}

/// What a chain of records leaves of the fields a segment writes as
/// changes (OpenSpec change `agents-checkpoint-deltas`, decisions 3, 4
/// and 6): the partial line's bytes, the login methods last written and
/// the outputs as patched. [`restore`] takes the rest from the last
/// record.
pub(super) struct Folded {
    partial: Vec<u8>,
    line: Option<HeldLine>,
    methods: Option<Value>,
    outputs: Map<String, Value>,
}

impl Folded {
    /// What the base `state`, with the line bytes `partial`, holds; its
    /// `methods` and `outputs` are taken out of it.
    pub(super) fn base(state: &mut Map<String, Value>, partial: &[u8]) -> Result<Self, Unreadable> {
        let mut folded = Self {
            partial: Vec::new(),
            line: None,
            methods: None,
            outputs: Map::new(),
        };
        folded.fold(state, partial, true)?;
        Ok(folded)
    }

    /// Folds a segment's `state` and line bytes `partial` on.
    pub(super) fn segment(
        &mut self,
        state: &mut Map<String, Value>,
        partial: &[u8],
    ) -> Result<(), Unreadable> {
        self.fold(state, partial, false)
    }

    fn fold(
        &mut self,
        state: &mut Map<String, Value>,
        partial: &[u8],
        base: bool,
    ) -> Result<(), Unreadable> {
        let epoch = EntryId(number(state, "tool_epoch")?);
        let lines = object(field(state, "lines")?, "lines")?;
        let covers = number(lines, "covers")?;
        let collecting = text(lines, "partial")? == "collecting";
        let from = match lines.get("from") {
            None => 0,
            Some(from) => from.as_u64().ok_or(Unreadable::Field { name: "lines" })?,
        };
        if !collecting && !partial.is_empty() {
            return Err(Unreadable::StrayBytes { len: partial.len() });
        }
        if from == 0 {
            self.partial.clear();
        } else {
            // The bytes continue the line the record before held, at the
            // same place in the same agent process (decision 3).
            let added = partial.len() as u64;
            let start = covers.saturating_sub(from.saturating_add(added));
            let continues = !base
                && collecting
                && self.line.is_some_and(|held| {
                    held.len == from && held.start == start && held.epoch == epoch
                });
            if !continues {
                return Err(Unreadable::PartialGap {
                    from,
                    held: self.line.map_or(0, |held| held.len),
                });
            }
        }
        self.partial.extend_from_slice(partial);
        let len = self.partial.len() as u64;
        // Where the encoder put it too (`line_mark`).
        let start = covers.saturating_sub(len);
        self.line = collecting.then_some(HeldLine { epoch, start, len });
        // Written when they changed; `[]` when emptied (decision 4).
        if let Some(methods) = state.remove("methods") {
            self.methods = Some(methods);
        }
        // Each terminal set or, `null`, closed, in order.
        if let Some(patch) = state.remove("outputs") {
            let Value::Object(patch) = patch else {
                return Err(Unreadable::Field { name: "outputs" });
            };
            for (terminal, open) in patch {
                if open.is_null() {
                    if self.outputs.remove(&terminal).is_none() {
                        return Err(Unreadable::Field { name: "outputs" });
                    }
                } else {
                    self.outputs.insert(terminal, open);
                }
            }
        }
        Ok(())
    }
}

/// The session the last record's `state` holds, with the log folded
/// already and what the chain `folded` leaves.
pub(super) fn restore(
    state: &Map<String, Value>,
    transcript: Transcript,
    folded: Folded,
    now: Instant,
) -> Result<AgentSession, Unreadable> {
    let acp = number(state, "acp")?;
    let dialect = Dialect::from_version(acp)
        .map_err(|Unsupported { version }| Unreadable::Acp { version })?;
    let cwd = text(state, "cwd")?.to_owned();
    let version = text(state, "version")?.to_owned();
    let phase = restore_phase(object(field(state, "phase")?, "phase")?)?;
    let mut rpc = restore_rpc(object(field(state, "rpc")?, "rpc")?)?;
    rpc.speak(dialect);
    let run = field(state, "run")?;
    let run = if run.is_null() {
        None
    } else {
        Some(restore_run(object(run, "run")?)?)
    };
    // The tool calls are the log's from the epoch on (decision 2).
    let tool_epoch = EntryId(number(state, "tool_epoch")?);
    let tools = tool_index(&transcript, tool_epoch);
    let mut outputs = BTreeMap::new();
    for (terminal, open) in &folded.outputs {
        outputs.insert(terminal.clone(), restore_open_output(open)?);
    }
    let bad_lines = number(state, "bad_lines")?;
    let lines = restore_lines(object(field(state, "lines")?, "lines")?, &folded.partial)?;
    let methods: Vec<LoginMethod> = match folded.methods {
        None => Vec::new(),
        Some(methods) => serde_json::from_value(methods)
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
    // Each tool call names a tool entry of the log by construction.
    debug_assert!(
        tools
            .values()
            .all(|entry| matches!(transcript.entry(*entry), Some(Entry::Tool { .. })))
    );
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
        process: Process {
            lines,
            run,
            bad: (bad_lines > 0).then_some(BadRun {
                lines: bad_lines,
                last: now,
            }),
            book: ToolBook {
                tools,
                outputs,
                ..ToolBook::starting_at(tool_epoch)
            },
        },
        phase,
        cwd,
        version,
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
pub(super) fn restore_log(events: &[Value]) -> Result<Transcript, Unreadable> {
    let mut transcript = Transcript::new();
    fold_log(&mut transcript, events)?;
    Ok(transcript)
}

/// Folds a record's events onto `transcript`.
pub(super) fn fold_log(transcript: &mut Transcript, events: &[Value]) -> Result<(), Unreadable> {
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

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::{self, json};
    use brindle_transcript::{AgentState, Exit, HoldReason, Slot};

    use crate::acp::reduce::{BAD_RUN_QUIET, Command, Input, step};

    use super::super::harness::*;
    use super::super::*;
    use super::*;

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
        assert!(agent.s.process.run.is_none(), "the tool call ended the run");
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
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
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
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
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
        let (state, _) = state_of(&encode(&agent.s).expect("it fits").bytes);
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
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
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
        let (state, _) = state_of(&encode(&agent.s).expect("it fits").bytes);
        assert_eq!(state["methods"][1]["kind"], json!("terminal"));
        assert_eq!(agent.round_trip(), agent.s);
        let (state, _) = state_of(&encode(&Agent::new().s).expect("it fits").bytes);
        assert!(state.get("methods").is_none(), "{state}");
        // Methods that don't read are unreadable.
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
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
        let (state, _) = state_of(&encode(&agent.s).expect("it fits").bytes);
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
        let (state, _) = state_of(&encode(&agent.s).expect("it fits").bytes);
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
        let (state, _) = state_of(&encode(&agent.s).expect("it fits").bytes);
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
        let (state, _) = state_of(&encode(&agent.s).expect("it fits").bytes);
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
        let bytes = encode(&agent.s).expect("it fits").bytes;
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
        let bytes = encode(&agent.s).expect("it fits").bytes;
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
        agent.s.process.run = None;
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
        assert!(agent.s.process.lines.covers() > 0);
        assert_eq!(restored.process.lines.covers(), agent.s.process.lines.covers());
    }

    #[test]
    fn a_partial_line_travels_as_raw_bytes_after_the_state() {
        let mut agent = Agent::ready();
        agent.feed(b"\xff\xfe not UTF-8");
        let bytes = encode(&agent.s).expect("it fits").bytes;
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
            agent.s.process.lines = Lines::restore(partial, 9_000_100);
            let bytes = encode(&agent.s).expect("it fits").bytes;
            assert_eq!(bytes[bytes.len() - 4..], [0, 0, 0, 0], "no line bytes follow");
            assert!(state_of(&bytes).1.is_empty());
            assert_eq!(decode(&bytes, agent.now).expect("it reads"), agent.s);
        }
    }

    #[test]
    fn the_header_is_the_magic_and_format_2_then_a_base_record() {
        let bytes = encode(&Agent::new().s).expect("it fits").bytes;
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
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
        state["later"] = json!({"added": true});
        let rebuilt = rebuild(&state, &partial);
        assert_eq!(decode(&rebuilt, agent.now).expect("it reads"), agent.s);
    }

    #[test]
    fn a_bad_line_run_keeps_its_count_and_writes_its_notice_a_second_after_adoption() {
        let mut agent = Agent::ready();
        agent.feed(b"hello\nworld\n");
        assert_eq!(agent.s.process.bad.map(|bad| bad.lines), Some(2));
        let later = agent.now + std::time::Duration::from_secs(60);
        let bytes = encode(&agent.s).expect("it fits").bytes;
        let adopted = adopt(&bytes, false, "/x", "0.4.0", later);
        assert_eq!(adopted.restored, Ok(()));
        assert_eq!(
            adopted.session.process.bad,
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
        let open = agent.s.process.book.outputs.get("t1").expect("open");
        assert_eq!(open.filter.line, "bold");
        let decoded = agent.round_trip();
        assert_eq!(decoded, agent.s);
        // A sequence cut between two pieces, restored, is still removed.
        let mut agent = with_tool_content(None);
        agent.feed(&tool_update(
            json!({"terminal_output_delta": {"terminal_id": "t1", "data": "\u{1b}[3"}}),
            Value::Null,
        ));
        assert_eq!(agent.s.process.book.outputs["t1"].filter.mode, Mode::Csi);
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
            merged(vec![
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
            merged(vec![output(1, 0, &half, None), output(1, 0, &half, None)]).len(),
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
        let (state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
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
        let bytes = encode(&agent.s).expect("it fits").bytes;
        let adopted = adopt(&bytes, false, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Ok(()));
        assert!(!adopted.session.rpc.setting());
        assert_eq!(adopted.session.rpc.next_id(), agent.s.rpc.next_id());
        assert_eq!(adopted.session.transcript.options(), agent.s.transcript.options());
    }

    #[test]
    fn a_checkpoint_from_before_options_reads_with_both_lists_empty() {
        let agent = Agent::busy();
        let bytes = encode(&agent.s).expect("it fits").bytes;
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
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
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
