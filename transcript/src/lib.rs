//! The agent transcript that Brindle and its terminal server share: the
//! events the core sends over client protocol 4 and the fold both sides
//! apply them with. Pure as a crate (fcis R4): it takes values and returns
//! values, and does no IO and reads no clock.
//!
//! - **Events** ([`Event`]) are Brindle's own, never ACP's: the core's
//!   reducer turns the agent's messages into them, applies each to its log
//!   and sends it; a client applies the same events to its copy. They travel
//!   as JSON objects ([`Event::to_value`], [`Event::from_value`]) inside
//!   client protocol 4's `TRANSCRIPT`, which `brindle-remote-protocol`
//!   carries without reading.
//! - **The fold** ([`Transcript::apply`]) is the one way a log changes. An
//!   event it can't apply is a [`FoldError`], never a panic. A log rebuilt
//!   from [`Transcript::replay`] equals the log it came from.
//! - **Forms are frozen per client protocol version** (`process R5`): the
//!   event kinds and the values of every closed enum inside an event are
//!   protocol 4's, pinned by `tests/fixtures/v4/`. Within protocol 4 an
//!   event may only gain optional fields, defaulted and omitted when unset;
//!   [`Event::from_value`] ignores fields it doesn't know, and an unknown
//!   kind or value is a [`DecodeError`].
//!
//! The encoding: every enum that carries data is an object tagged by a
//! field of its own (`event`, `type` for an [`Entry`], `turn`, `agent`,
//! `slot`, `hold`, `reason`, `outcome`), and every enum that doesn't is a
//! `snake_case` string. An optional field is left out when unset.

// railway R3, types R2: no `_ =>` on one of Brindle's own enums. railway R5,
// R9: a decoder never panics on bytes from outside, and non-test code has no
// `unwrap` or `expect`. Non-test code only.
#![cfg_attr(not(test), warn(clippy::wildcard_enum_match_arm))]
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

use std::collections::VecDeque;
use std::fmt;

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod fuzz;

/// The most text one event carries in a field that grows ([`Event::Added`]'s
/// text and [`Event::Appended`]): longer text is split, at a character
/// boundary, into more `Appended`s.
pub const TEXT_CHUNK: usize = 64 * 1024;

/// An entry's id: per session, from 1, never reused (`types R3`). Never an
/// index: entries before the first kept one may have been trimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryId(pub u64);

impl EntryId {
    /// The first id of a session.
    pub const FIRST: Self = Self(1);

    fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One entry of the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    /// A prompt the user sent.
    User { text: String },
    /// What the agent said.
    Agent { text: String },
    /// What the agent thought aloud.
    Thought { text: String },
    /// A tool call, by the agent's id for it. Its content isn't kept.
    Tool {
        call: String,
        title: String,
        kind: ToolKind,
        status: ToolStatus,
    },
    /// A permission request and, once answered, its outcome. `call` is the
    /// tool call it asks about, when the agent named one.
    Permission {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call: Option<String>,
        title: String,
        #[serde(default)]
        options: Vec<PermissionOption>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<PermissionOutcome>,
    },
    /// Something the server says about the session (a line that wasn't
    /// ACP, a stderr tail after a failed start, output lost in a restart).
    Notice { level: NoticeLevel, text: String },
}

impl Entry {
    /// The text that [`Event::Appended`] grows, for the kinds that have one.
    fn text_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::User { text } | Self::Agent { text } | Self::Thought { text } => Some(text),
            Self::Tool { .. } | Self::Permission { .. } | Self::Notice { .. } => None,
        }
    }

    /// The encoded length of the entry, as the log's bound counts it.
    fn encoded_len(&self) -> usize {
        serde_json::to_vec(self).map_or(0, |bytes| bytes.len())
    }
}

/// What a tool call does, as ACP names it; an agent's kind Brindle doesn't
/// know is [`ToolKind::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    Other,
}

/// Where a tool call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// How a notice reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// One answer a permission request offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    /// The agent's id for it, which an answer names.
    pub id: String,
    /// What the user is shown.
    pub name: String,
    pub kind: PermissionKind,
}

/// What choosing an option does, as ACP names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

/// How a permission request was answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PermissionOutcome {
    /// A client chose the option with this id.
    Selected { option: String },
    /// The turn was cancelled, or the agent withdrew the request.
    Cancelled,
}

/// The session's one queue slot (design decision 10).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "slot", rename_all = "snake_case")]
pub enum Slot {
    #[default]
    Empty,
    /// Queued: sent when the running turn ends with `end_turn`.
    Waiting { text: String },
    /// Not sent, and not sent on its own: the user sends it again or
    /// discards it.
    Held { text: String, reason: HoldReason },
}

/// Why a prompt in the slot is held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "hold", rename_all = "snake_case")]
pub enum HoldReason {
    /// The user cancelled the turn it waited for.
    Cancelled,
    /// The agent answered the turn's prompt with an error.
    Failed,
    /// The turn ended with a stop reason other than `end_turn` and
    /// `cancelled`.
    Stopped { stop: StopReason },
    /// The agent asked for a login.
    LoginRequired,
    /// The agent stopped being ready: ACP handling failed, or it exited.
    AgentStopped,
    /// A restart lost the end of the turn it waited for.
    OutputLost,
}

/// Where the session's turn is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "turn", rename_all = "snake_case")]
pub enum TurnState {
    /// No turn has run yet.
    #[default]
    Idle,
    Working,
    /// Waiting on a permission request.
    Blocked,
    /// A cancel was sent; the agent hasn't answered the prompt yet.
    Cancelling,
    Ended {
        stop: StopReason,
    },
    /// The agent answered the prompt with an error, or the turn was lost.
    Failed {
        message: String,
    },
}

/// Why a turn ended, as the agent said; a reason Brindle doesn't know keeps
/// its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    Other { name: String },
}

/// Where the agent is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "agent", rename_all = "snake_case")]
pub enum AgentState {
    /// Started; `initialize` and `session/new` not answered yet.
    #[default]
    Starting,
    /// Takes prompts. The name and version are what the agent gave.
    Ready {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    /// The agent needs a login through its own command line.
    LoginRequired { message: String },
    /// The agent can't be used: it speaks another ACP version, or handling
    /// its messages failed.
    Failed { message: String },
    /// The agent's process ended. `code` is `None` when a signal ended it;
    /// `stderr` is the end of its standard error.
    Exited {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<i32>,
        #[serde(default)]
        stderr: String,
    },
    /// The agent was lost with the holder that ran it, so neither its exit
    /// status nor its standard error is known; `message` says so.
    Lost { message: String },
}

/// The agent's ACP session (its conversation), once the agent gave one:
/// its id, the absolute directory it works in, and how it can be taken up
/// again after the agent ends (OpenSpec change `agents-session-recovery`,
/// design decision 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    /// The `sessionId` the agent gave.
    pub id: String,
    pub cwd: String,
    pub continuation: Continuation,
}

/// Which of ACP's `session/resume` and `session/load` the agent offered in
/// `initialize`: one closed value, so "offers load" and "offers resume"
/// can't disagree (`types R1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Continuation {
    ResumeAndLoad,
    Resume,
    Load,
    Neither,
}

impl Continuation {
    /// From `initialize`'s `agentCapabilities.loadSession` and whether it
    /// gave `sessionCapabilities.resume`.
    pub fn from_offers(load: bool, resume: bool) -> Self {
        match (load, resume) {
            (true, true) => Self::ResumeAndLoad,
            (false, true) => Self::Resume,
            (true, false) => Self::Load,
            (false, false) => Self::Neither,
        }
    }

    pub fn offers_load(self) -> bool {
        match self {
            Self::ResumeAndLoad | Self::Load => true,
            Self::Resume | Self::Neither => false,
        }
    }

    pub fn offers_resume(self) -> bool {
        match self {
            Self::ResumeAndLoad | Self::Resume => true,
            Self::Load | Self::Neither => false,
        }
    }

    /// Whether a conversation can be taken up again at all.
    pub fn continuable(self) -> bool {
        match self {
            Self::ResumeAndLoad | Self::Resume | Self::Load => true,
            Self::Neither => false,
        }
    }
}

/// A transcript event: one change to the log, the slot, the turn or the
/// agent's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// A new entry, with the next id.
    Added {
        id: EntryId,
        entry: Entry,
    },
    /// More text for a user, agent or thought entry.
    Appended {
        id: EntryId,
        text: String,
    },
    /// A tool call's fields that changed.
    ToolChanged {
        id: EntryId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<ToolKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ToolStatus>,
    },
    /// A permission request's outcome.
    Answered {
        id: EntryId,
        outcome: PermissionOutcome,
    },
    /// The whole queue slot, set as given.
    Queue {
        slot: Slot,
    },
    Turn {
        state: TurnState,
    },
    Agent {
        state: AgentState,
    },
    /// The agent's conversation: sent when the agent gives or takes up an
    /// ACP session, before its `Ready`. It stays through the agent's later
    /// states, and a later one replaces it.
    Conversation {
        conversation: Conversation,
    },
    /// The entries up to and including `through` were dropped; `dropped`
    /// counts them, or in a replay every entry dropped so far.
    Trimmed {
        through: EntryId,
        dropped: u64,
    },
}

impl Event {
    /// The event as the JSON object client protocol 4 carries.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// Reads an event a peer sent. Fields it doesn't know are ignored; a
    /// value that isn't an object, an unknown kind, an unknown value of one
    /// of the event's enums or a missing field is a [`DecodeError`].
    pub fn from_value(value: &serde_json::Value) -> Result<Self, DecodeError> {
        if !value.is_object() {
            return Err(DecodeError(format!(
                "a transcript event is a JSON object, not {}",
                json_kind(value)
            )));
        }
        Self::deserialize(value)
            .map_err(|error| DecodeError(format!("bad transcript event: {error}")))
    }
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// An event [`Event::from_value`] couldn't read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(String);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DecodeError {}

/// An event [`Transcript::apply`] couldn't apply. The log is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldError {
    /// An `Added` whose id isn't the next one.
    OutOfOrder { id: EntryId, expected: EntryId },
    /// An event for an entry the log doesn't have (never added, or trimmed).
    UnknownEntry { id: EntryId },
    /// An `Appended` for an entry with no text to grow.
    NotAppendable { id: EntryId },
    /// A `ToolChanged` for an entry that isn't a tool call.
    NotATool { id: EntryId },
    /// An `Answered` for an entry that isn't a permission request.
    NotAPermission { id: EntryId },
    /// An `Answered` for a permission request that already has an outcome.
    AlreadyAnswered { id: EntryId },
    /// A `Trimmed` past the last entry of a log that has entries.
    TrimPastEnd { through: EntryId, next: EntryId },
    /// A `Trimmed` that doesn't go past the last one.
    TrimBehind { through: EntryId, trimmed: EntryId },
}

impl fmt::Display for FoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfOrder { id, expected } => {
                write!(f, "entry {id} was added where entry {expected} was next")
            }
            Self::UnknownEntry { id } => write!(f, "there is no entry {id}"),
            Self::NotAppendable { id } => write!(f, "entry {id} has no text to append to"),
            Self::NotATool { id } => write!(f, "entry {id} isn't a tool call"),
            Self::NotAPermission { id } => write!(f, "entry {id} isn't a permission request"),
            Self::AlreadyAnswered { id } => {
                write!(f, "permission request {id} was already answered")
            }
            Self::TrimPastEnd { through, next } => {
                write!(
                    f,
                    "trimmed through entry {through}, but the next entry is {next}"
                )
            }
            Self::TrimBehind { through, trimmed } => write!(
                f,
                "trimmed through entry {through}, but entries through {trimmed} were already"
            ),
        }
    }
}

impl std::error::Error for FoldError {}

/// What the session list says an agent session is doing, in program
/// status's names (design decision 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Working,
    Blocked,
    Done,
    Error,
}

/// An entry the log keeps, with its encoded length.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Kept {
    id: EntryId,
    entry: Entry,
    size: usize,
}

/// The entries trimmed so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Trim {
    through: EntryId,
    dropped: u64,
}

/// An agent session's log: its entries in id order, the queue slot, the
/// turn and the agent's state. It changes only through [`Transcript::apply`],
/// so the core's log and a client's copy fold the same events the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    entries: VecDeque<Kept>,
    next: EntryId,
    slot: Slot,
    turn: TurnState,
    agent: AgentState,
    /// Set by [`Event::Conversation`]; `None` until the agent gave one.
    conversation: Option<Conversation>,
    trimmed: Option<Trim>,
    /// The kept entries' encoded length.
    size: usize,
}

impl Default for Transcript {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            next: EntryId::FIRST,
            slot: Slot::Empty,
            turn: TurnState::Idle,
            agent: AgentState::Starting,
            conversation: None,
            trimmed: None,
            size: 0,
        }
    }
}

impl Transcript {
    /// A new session's log: no entries, the slot empty, no turn, the agent
    /// starting.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one event, or says why it can't and leaves the log as it was.
    pub fn apply(&mut self, event: &Event) -> Result<(), FoldError> {
        match event {
            Event::Added { id, entry } => {
                if *id != self.next {
                    return Err(FoldError::OutOfOrder {
                        id: *id,
                        expected: self.next,
                    });
                }
                let size = entry.encoded_len();
                self.entries.push_back(Kept {
                    id: *id,
                    entry: entry.clone(),
                    size,
                });
                self.size += size;
                self.next = id.next();
            }
            Event::Appended { id, text } => {
                let kept = self.kept_mut(*id)?;
                let Some(own) = kept.entry.text_mut() else {
                    return Err(FoldError::NotAppendable { id: *id });
                };
                own.push_str(text);
                let grown = escaped_len(text);
                kept.size += grown;
                self.size += grown;
            }
            Event::ToolChanged {
                id,
                title,
                kind,
                status,
            } => {
                let kept = self.kept_mut(*id)?;
                let Entry::Tool {
                    title: own_title,
                    kind: own_kind,
                    status: own_status,
                    ..
                } = &mut kept.entry
                else {
                    return Err(FoldError::NotATool { id: *id });
                };
                if let Some(title) = title {
                    own_title.clone_from(title);
                }
                if let Some(kind) = kind {
                    *own_kind = *kind;
                }
                if let Some(status) = status {
                    *own_status = *status;
                }
                self.resize_entry(*id);
            }
            Event::Answered { id, outcome } => {
                let kept = self.kept_mut(*id)?;
                let Entry::Permission {
                    outcome: own_outcome,
                    ..
                } = &mut kept.entry
                else {
                    return Err(FoldError::NotAPermission { id: *id });
                };
                if own_outcome.is_some() {
                    return Err(FoldError::AlreadyAnswered { id: *id });
                }
                *own_outcome = Some(outcome.clone());
                self.resize_entry(*id);
            }
            Event::Queue { slot } => self.slot = slot.clone(),
            Event::Turn { state } => self.turn = state.clone(),
            Event::Agent { state } => self.agent = state.clone(),
            Event::Conversation { conversation } => {
                self.conversation = Some(conversation.clone());
            }
            Event::Trimmed { through, dropped } => self.trim(*through, *dropped)?,
        }
        Ok(())
    }

    fn trim(&mut self, through: EntryId, dropped: u64) -> Result<(), FoldError> {
        if let Some(trim) = self.trimmed
            && through <= trim.through
        {
            return Err(FoldError::TrimBehind {
                through,
                trimmed: trim.through,
            });
        }
        if through >= self.next && !self.entries.is_empty() {
            return Err(FoldError::TrimPastEnd {
                through,
                next: self.next,
            });
        }
        while let Some(front) = self.entries.front()
            && front.id <= through
        {
            self.size = self.size.saturating_sub(front.size);
            self.entries.pop_front();
        }
        if through >= self.next {
            self.next = through.next();
        }
        let before = self.trimmed.map_or(0, |trim| trim.dropped);
        self.trimmed = Some(Trim {
            through,
            dropped: before.saturating_add(dropped),
        });
        Ok(())
    }

    fn index(&self, id: EntryId) -> Result<usize, FoldError> {
        self.entries
            .binary_search_by_key(&id, |kept| kept.id)
            .map_err(|_| FoldError::UnknownEntry { id })
    }

    fn kept_mut(&mut self, id: EntryId) -> Result<&mut Kept, FoldError> {
        let index = self.index(id)?;
        self.entries
            .get_mut(index)
            .ok_or(FoldError::UnknownEntry { id })
    }

    /// Measures an entry again after a change that isn't an append.
    fn resize_entry(&mut self, id: EntryId) {
        if let Ok(index) = self.index(id)
            && let Some(kept) = self.entries.get_mut(index)
        {
            let size = kept.entry.encoded_len();
            self.size = self.size.saturating_sub(kept.size) + size;
            kept.size = size;
        }
    }

    /// The queue slot.
    pub fn slot(&self) -> &Slot {
        &self.slot
    }

    pub fn turn(&self) -> &TurnState {
        &self.turn
    }

    pub fn agent(&self) -> &AgentState {
        &self.agent
    }

    /// The agent's conversation, once it gave one; it stays after the
    /// agent ends.
    pub fn conversation(&self) -> Option<&Conversation> {
        self.conversation.as_ref()
    }

    /// The kept entries, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = (EntryId, &Entry)> {
        self.entries.iter().map(|kept| (kept.id, &kept.entry))
    }

    pub fn entry(&self, id: EntryId) -> Option<&Entry> {
        let index = self.index(id).ok()?;
        self.entries.get(index).map(|kept| &kept.entry)
    }

    /// The id the next entry gets.
    pub fn next_id(&self) -> EntryId {
        self.next
    }

    /// How many entries were trimmed in all.
    pub fn dropped(&self) -> u64 {
        self.trimmed.map_or(0, |trim| trim.dropped)
    }

    /// The kept entries' encoded length, which the log's bound counts.
    pub fn encoded_len(&self) -> usize {
        self.size
    }

    /// The events that rebuild this log in a fresh one: `Trimmed` when
    /// entries were dropped, each entry's `Added` with its text split into
    /// `Appended`s of at most [`TEXT_CHUNK`] bytes and a permission's
    /// outcome as its `Answered`, the slot's `Queue` when it isn't empty,
    /// the `Conversation` when there is one, then `Turn` and `Agent`.
    pub fn replay(&self) -> Vec<Event> {
        let mut events = Vec::with_capacity(self.entries.len() + 5);
        if let Some(trim) = self.trimmed {
            events.push(Event::Trimmed {
                through: trim.through,
                dropped: trim.dropped,
            });
        }
        for kept in &self.entries {
            replay_entry(kept.id, &kept.entry, &mut events);
        }
        match &self.slot {
            Slot::Empty => {}
            slot @ (Slot::Waiting { .. } | Slot::Held { .. }) => {
                events.push(Event::Queue { slot: slot.clone() })
            }
        }
        if let Some(conversation) = &self.conversation {
            events.push(Event::Conversation {
                conversation: conversation.clone(),
            });
        }
        events.push(Event::Turn {
            state: self.turn.clone(),
        });
        events.push(Event::Agent {
            state: self.agent.clone(),
        });
        events
    }

    /// What the session list shows for this log (design decision 15). The
    /// slot doesn't change it.
    pub fn activity(&self) -> Activity {
        match &self.agent {
            AgentState::Starting => Activity::Working,
            AgentState::LoginRequired { .. }
            | AgentState::Failed { .. }
            | AgentState::Exited { .. }
            | AgentState::Lost { .. } => Activity::Error,
            AgentState::Ready { .. } => match &self.turn {
                TurnState::Working | TurnState::Cancelling => Activity::Working,
                TurnState::Blocked => Activity::Blocked,
                TurnState::Idle => Activity::Idle,
                TurnState::Ended { stop } => match stop {
                    StopReason::EndTurn => Activity::Done,
                    StopReason::Cancelled => Activity::Idle,
                    StopReason::MaxTokens
                    | StopReason::MaxTurnRequests
                    | StopReason::Refusal
                    | StopReason::Other { .. } => Activity::Error,
                },
                TurnState::Failed { .. } => Activity::Error,
            },
        }
    }

    /// Drops the oldest entries whole until the kept ones' encoded length is
    /// at most `limit`, always keeping the newest, and returns the `Trimmed`
    /// it applied for them; `None` when nothing was dropped.
    pub fn trim_to(&mut self, limit: usize) -> Option<Event> {
        let mut size = self.size;
        let mut count: u64 = 0;
        let mut through = None;
        for kept in self
            .entries
            .iter()
            .take(self.entries.len().saturating_sub(1))
        {
            if size <= limit {
                break;
            }
            size = size.saturating_sub(kept.size);
            count += 1;
            through = Some(kept.id);
        }
        let event = Event::Trimmed {
            through: through?,
            dropped: count,
        };
        self.apply(&event).ok().map(|()| event)
    }
}

/// An entry's `Added`, its text's further `Appended`s and its outcome's
/// `Answered`.
fn replay_entry(id: EntryId, entry: &Entry, events: &mut Vec<Event>) {
    match entry {
        Entry::User { text } => replay_text(id, text, |text| Entry::User { text }, events),
        Entry::Agent { text } => replay_text(id, text, |text| Entry::Agent { text }, events),
        Entry::Thought { text } => replay_text(id, text, |text| Entry::Thought { text }, events),
        Entry::Permission {
            call,
            title,
            options,
            outcome,
        } => {
            events.push(Event::Added {
                id,
                entry: Entry::Permission {
                    call: call.clone(),
                    title: title.clone(),
                    options: options.clone(),
                    outcome: None,
                },
            });
            if let Some(outcome) = outcome {
                events.push(Event::Answered {
                    id,
                    outcome: outcome.clone(),
                });
            }
        }
        Entry::Tool { .. } | Entry::Notice { .. } => events.push(Event::Added {
            id,
            entry: entry.clone(),
        }),
    }
}

/// A text entry's `Added`, made by `entry` from its first piece, and an
/// `Appended` for each piece after it.
fn replay_text(id: EntryId, text: &str, entry: fn(String) -> Entry, events: &mut Vec<Event>) {
    let mut pieces = chunks(text);
    let first = pieces.next().unwrap_or_default().to_string();
    events.push(Event::Added {
        id,
        entry: entry(first),
    });
    events.extend(pieces.map(|piece| Event::Appended {
        id,
        text: piece.to_string(),
    }));
}

/// `text` in pieces of at most [`TEXT_CHUNK`] bytes, cut at character
/// boundaries; one empty piece for empty text.
pub fn chunks(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(text);
    std::iter::from_fn(move || {
        let left = rest?;
        if left.len() <= TEXT_CHUNK {
            rest = None;
            return Some(left);
        }
        let mut cut = TEXT_CHUNK;
        while !left.is_char_boundary(cut) {
            cut -= 1;
        }
        let (piece, after) = left.split_at(cut);
        rest = Some(after);
        Some(piece)
    })
}

/// How many bytes `text` takes inside a JSON string, as serde_json escapes
/// it: `"` and `\` take two, the control characters two (`\b`, `\t`, `\n`,
/// `\f`, `\r`) or six (`\u00XX`).
fn escaped_len(text: &str) -> usize {
    text.bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | 0x08 | b'\t' | b'\n' | 0x0c | b'\r' => 2,
            0x00..=0x1f => 6,
            _ => 1,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::{XorShift, random_transcript};

    fn added(id: u64, entry: Entry) -> Event {
        Event::Added {
            id: EntryId(id),
            entry,
        }
    }

    fn agent(text: &str) -> Entry {
        Entry::Agent { text: text.into() }
    }

    fn tool(call: &str) -> Entry {
        Entry::Tool {
            call: call.into(),
            title: "Read notes.md".into(),
            kind: ToolKind::Read,
            status: ToolStatus::Pending,
        }
    }

    fn permission() -> Entry {
        Entry::Permission {
            call: Some("call_1".into()),
            title: "Write notes.md".into(),
            options: vec![
                PermissionOption {
                    id: "allow-once".into(),
                    name: "Allow".into(),
                    kind: PermissionKind::AllowOnce,
                },
                PermissionOption {
                    id: "reject-once".into(),
                    name: "Reject".into(),
                    kind: PermissionKind::RejectOnce,
                },
            ],
            outcome: None,
        }
    }

    fn fold(events: &[Event]) -> Result<Transcript, FoldError> {
        let mut transcript = Transcript::new();
        for event in events {
            transcript.apply(event)?;
        }
        Ok(transcript)
    }

    #[test]
    fn chunks_append_to_one_entry() {
        let transcript = fold(&[
            added(1, agent("Hel")),
            Event::Appended {
                id: EntryId(1),
                text: "lo".into(),
            },
            Event::Appended {
                id: EntryId(1),
                text: "!".into(),
            },
        ])
        .unwrap();
        let entries: Vec<_> = transcript.entries().collect();
        assert_eq!(entries, [(EntryId(1), &agent("Hello!"))]);
        assert_eq!(transcript.next_id(), EntryId(2));
    }

    #[test]
    fn a_tool_changes_by_id_and_only_in_the_fields_given() {
        let mut transcript = fold(&[added(1, tool("call_1")), added(2, tool("call_2"))]).unwrap();
        transcript
            .apply(&Event::ToolChanged {
                id: EntryId(2),
                title: None,
                kind: None,
                status: Some(ToolStatus::Completed),
            })
            .unwrap();
        transcript
            .apply(&Event::ToolChanged {
                id: EntryId(1),
                title: Some("Edit notes.md".into()),
                kind: Some(ToolKind::Edit),
                status: None,
            })
            .unwrap();
        assert_eq!(
            transcript.entry(EntryId(1)),
            Some(&Entry::Tool {
                call: "call_1".into(),
                title: "Edit notes.md".into(),
                kind: ToolKind::Edit,
                status: ToolStatus::Pending,
            })
        );
        assert_eq!(
            transcript.entry(EntryId(2)),
            Some(&Entry::Tool {
                call: "call_2".into(),
                title: "Read notes.md".into(),
                kind: ToolKind::Read,
                status: ToolStatus::Completed,
            })
        );
    }

    #[test]
    fn an_answer_goes_only_on_a_permission_entry() {
        let outcome = PermissionOutcome::Selected {
            option: "allow-once".into(),
        };
        let mut transcript = fold(&[added(1, agent("hi")), added(2, permission())]).unwrap();
        let before = transcript.clone();
        assert_eq!(
            transcript.apply(&Event::Answered {
                id: EntryId(1),
                outcome: outcome.clone(),
            }),
            Err(FoldError::NotAPermission { id: EntryId(1) })
        );
        assert_eq!(transcript, before, "a refused event changes nothing");
        transcript
            .apply(&Event::Answered {
                id: EntryId(2),
                outcome: outcome.clone(),
            })
            .unwrap();
        let Some(Entry::Permission { outcome: kept, .. }) = transcript.entry(EntryId(2)) else {
            panic!("not a permission entry");
        };
        assert_eq!(kept, &Some(outcome));
    }

    #[test]
    fn queue_sets_the_slot_from_any_slot_and_never_fails() {
        let slots = [
            Slot::Empty,
            Slot::Waiting {
                text: "next".into(),
            },
            Slot::Held {
                text: "held".into(),
                reason: HoldReason::Stopped {
                    stop: StopReason::MaxTokens,
                },
            },
        ];
        for from in &slots {
            for to in &slots {
                let mut transcript = Transcript::new();
                transcript
                    .apply(&Event::Queue { slot: from.clone() })
                    .unwrap();
                transcript
                    .apply(&Event::Queue { slot: to.clone() })
                    .unwrap();
                assert_eq!(transcript.slot(), to);
            }
        }
    }

    #[test]
    fn every_fold_error_is_returned_and_changes_nothing() {
        let base = fold(&[
            added(1, agent("a")),
            added(2, tool("call_1")),
            added(3, permission()),
            Event::Answered {
                id: EntryId(3),
                outcome: PermissionOutcome::Cancelled,
            },
            added(
                4,
                Entry::Notice {
                    level: NoticeLevel::Info,
                    text: "n".into(),
                },
            ),
            Event::Trimmed {
                through: EntryId(1),
                dropped: 1,
            },
        ])
        .unwrap();
        let cases = [
            (
                added(9, agent("x")),
                FoldError::OutOfOrder {
                    id: EntryId(9),
                    expected: EntryId(5),
                },
            ),
            (
                added(4, agent("x")),
                FoldError::OutOfOrder {
                    id: EntryId(4),
                    expected: EntryId(5),
                },
            ),
            (
                Event::Appended {
                    id: EntryId(1),
                    text: "trimmed".into(),
                },
                FoldError::UnknownEntry { id: EntryId(1) },
            ),
            (
                Event::Appended {
                    id: EntryId(7),
                    text: "never added".into(),
                },
                FoldError::UnknownEntry { id: EntryId(7) },
            ),
            (
                Event::Appended {
                    id: EntryId(2),
                    text: "x".into(),
                },
                FoldError::NotAppendable { id: EntryId(2) },
            ),
            (
                Event::Appended {
                    id: EntryId(4),
                    text: "x".into(),
                },
                FoldError::NotAppendable { id: EntryId(4) },
            ),
            (
                Event::ToolChanged {
                    id: EntryId(3),
                    title: None,
                    kind: None,
                    status: Some(ToolStatus::Failed),
                },
                FoldError::NotATool { id: EntryId(3) },
            ),
            (
                Event::Answered {
                    id: EntryId(2),
                    outcome: PermissionOutcome::Cancelled,
                },
                FoldError::NotAPermission { id: EntryId(2) },
            ),
            (
                Event::Answered {
                    id: EntryId(3),
                    outcome: PermissionOutcome::Cancelled,
                },
                FoldError::AlreadyAnswered { id: EntryId(3) },
            ),
            (
                Event::Trimmed {
                    through: EntryId(5),
                    dropped: 4,
                },
                FoldError::TrimPastEnd {
                    through: EntryId(5),
                    next: EntryId(5),
                },
            ),
            (
                Event::Trimmed {
                    through: EntryId(1),
                    dropped: 1,
                },
                FoldError::TrimBehind {
                    through: EntryId(1),
                    trimmed: EntryId(1),
                },
            ),
        ];
        for (event, error) in cases {
            let mut transcript = base.clone();
            assert_eq!(transcript.apply(&event), Err(error.clone()), "{event:?}");
            assert_eq!(transcript, base, "{event:?} changed the log");
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn a_replay_folds_back_to_the_same_log() {
        let mut rng = XorShift::new(0x7a5c_0001);
        let mut slots = [0usize; 3];
        let mut conversations = [0usize; 2];
        let mut lost = 0usize;
        for _ in 0..400 {
            let transcript = random_transcript(&mut rng);
            slots[match transcript.slot() {
                Slot::Empty => 0,
                Slot::Waiting { .. } => 1,
                Slot::Held { .. } => 2,
            }] += 1;
            conversations[usize::from(transcript.conversation().is_some())] += 1;
            lost += usize::from(matches!(transcript.agent(), AgentState::Lost { .. }));
            let replay = transcript.replay();
            for event in &replay {
                if let Event::Added { entry, .. } = event
                    && let Entry::User { text } | Entry::Agent { text } | Entry::Thought { text } =
                        entry
                {
                    assert!(text.len() <= TEXT_CHUNK);
                }
                if let Event::Appended { text, .. } = event {
                    assert!(text.len() <= TEXT_CHUNK && !text.is_empty());
                }
            }
            assert_eq!(fold(&replay), Ok(transcript.clone()));
            // And through the wire's encoding.
            let decoded: Vec<Event> = replay
                .iter()
                .map(|event| Event::from_value(&event.to_value()).unwrap())
                .collect();
            assert_eq!(decoded, replay);
        }
        assert!(slots.iter().all(|&n| n > 0), "slots generated: {slots:?}");
        assert!(
            conversations.iter().all(|&n| n > 0),
            "logs without and with a conversation: {conversations:?}"
        );
        assert!(lost > 0, "no log ended with the agent lost");
    }

    fn conversation(id: &str, continuation: Continuation) -> Conversation {
        Conversation {
            id: id.into(),
            cwd: "/home/u/src".into(),
            continuation,
        }
    }

    #[test]
    fn a_conversation_event_sets_the_conversation_and_a_later_one_replaces_it() {
        let mut transcript = Transcript::new();
        assert_eq!(transcript.conversation(), None);
        let first = conversation("s1", Continuation::ResumeAndLoad);
        transcript
            .apply(&Event::Conversation {
                conversation: first.clone(),
            })
            .unwrap();
        assert_eq!(transcript.conversation(), Some(&first));
        // Any agent state may follow, and the conversation stays.
        for state in [
            ready(),
            AgentState::Exited {
                code: Some(1),
                stderr: String::new(),
            },
            AgentState::Lost {
                message: "its holder ended".into(),
            },
            AgentState::Starting,
        ] {
            transcript.apply(&Event::Agent { state }).unwrap();
            assert_eq!(transcript.conversation(), Some(&first));
        }
        let second = conversation("s1", Continuation::Load);
        transcript
            .apply(&Event::Conversation {
                conversation: second.clone(),
            })
            .unwrap();
        assert_eq!(transcript.conversation(), Some(&second));
        assert_eq!(transcript.next_id(), EntryId::FIRST, "no entry was added");
    }

    #[test]
    fn a_replay_carries_the_conversation_right_before_the_turn_and_the_agent() {
        let held = Slot::Held {
            text: "next".into(),
            reason: HoldReason::AgentStopped,
        };
        let kept = conversation("s1", Continuation::Resume);
        let lost = AgentState::Lost {
            message: "its holder ended".into(),
        };
        let transcript = fold(&[
            Event::Conversation {
                conversation: kept.clone(),
            },
            added(1, agent("hi")),
            Event::Queue { slot: held.clone() },
            Event::Agent {
                state: lost.clone(),
            },
        ])
        .unwrap();
        assert_eq!(
            transcript.replay(),
            [
                added(1, agent("hi")),
                Event::Queue { slot: held },
                Event::Conversation { conversation: kept },
                Event::Turn {
                    state: TurnState::Idle
                },
                Event::Agent { state: lost },
            ]
        );
        assert_eq!(fold(&transcript.replay()), Ok(transcript));
    }

    #[test]
    fn continuation_follows_what_initialize_offered() {
        let rows = [
            (true, true, Continuation::ResumeAndLoad),
            (false, true, Continuation::Resume),
            (true, false, Continuation::Load),
            (false, false, Continuation::Neither),
        ];
        for (load, resume, continuation) in rows {
            assert_eq!(Continuation::from_offers(load, resume), continuation);
            assert_eq!(continuation.offers_load(), load, "{continuation:?}");
            assert_eq!(continuation.offers_resume(), resume, "{continuation:?}");
            assert_eq!(continuation.continuable(), load || resume, "{continuation:?}");
        }
    }

    #[test]
    fn long_text_replays_in_chunks_cut_at_character_boundaries() {
        // Three-byte characters, so a cut at exactly 64 KiB would split one.
        let text = "€".repeat(TEXT_CHUNK / 3 * 2 + 5);
        let transcript = fold(&[added(1, agent(&text))]).unwrap();
        let replay = transcript.replay();
        let appended = replay
            .iter()
            .filter(|event| matches!(event, Event::Appended { .. }))
            .count();
        assert_eq!(appended, 2);
        assert_eq!(fold(&replay), Ok(transcript));
        let pieces: Vec<&str> = chunks(&text).collect();
        assert_eq!(pieces[0].len(), TEXT_CHUNK - 1, "cut before the character");
        assert_eq!(pieces.concat(), text);
        assert!(pieces.iter().all(|piece| piece.len() <= TEXT_CHUNK));
        assert_eq!(chunks("").collect::<Vec<_>>(), [""]);
    }

    #[test]
    fn a_replay_holds_the_trim_the_entries_the_slot_the_turn_and_the_agent() {
        let held = Slot::Held {
            text: "again".into(),
            reason: HoldReason::Failed,
        };
        let transcript = fold(&[
            added(1, agent("old")),
            added(2, permission()),
            Event::Answered {
                id: EntryId(2),
                outcome: PermissionOutcome::Selected {
                    option: "reject-once".into(),
                },
            },
            Event::Trimmed {
                through: EntryId(1),
                dropped: 1,
            },
            Event::Queue { slot: held.clone() },
            Event::Turn {
                state: TurnState::Failed {
                    message: "rate limited".into(),
                },
            },
        ])
        .unwrap();
        let Entry::Permission {
            call,
            title,
            options,
            ..
        } = permission()
        else {
            unreachable!()
        };
        assert_eq!(
            transcript.replay(),
            [
                Event::Trimmed {
                    through: EntryId(1),
                    dropped: 1
                },
                added(
                    2,
                    Entry::Permission {
                        call,
                        title,
                        options,
                        outcome: None
                    }
                ),
                Event::Answered {
                    id: EntryId(2),
                    outcome: PermissionOutcome::Selected {
                        option: "reject-once".into()
                    }
                },
                Event::Queue { slot: held },
                Event::Turn {
                    state: TurnState::Failed {
                        message: "rate limited".into()
                    }
                },
                Event::Agent {
                    state: AgentState::Starting
                },
            ]
        );
        // An empty slot isn't replayed.
        assert!(
            !Transcript::new()
                .replay()
                .iter()
                .any(|event| matches!(event, Event::Queue { .. }))
        );
    }

    fn with(agent: AgentState, turn: TurnState) -> Transcript {
        fold(&[Event::Agent { state: agent }, Event::Turn { state: turn }]).unwrap()
    }

    fn ready() -> AgentState {
        AgentState::Ready {
            name: Some("fake".into()),
            version: None,
        }
    }

    fn ended(stop: StopReason) -> TurnState {
        TurnState::Ended { stop }
    }

    #[test]
    fn activity_follows_the_table_for_every_row() {
        let failed = || TurnState::Failed {
            message: "m".into(),
        };
        let rows = [
            (AgentState::Starting, TurnState::Idle, Activity::Working),
            (ready(), TurnState::Working, Activity::Working),
            (ready(), TurnState::Cancelling, Activity::Working),
            (ready(), TurnState::Blocked, Activity::Blocked),
            (ready(), ended(StopReason::EndTurn), Activity::Done),
            (ready(), TurnState::Idle, Activity::Idle),
            (ready(), ended(StopReason::Cancelled), Activity::Idle),
            (ready(), ended(StopReason::MaxTokens), Activity::Error),
            (ready(), ended(StopReason::MaxTurnRequests), Activity::Error),
            (ready(), ended(StopReason::Refusal), Activity::Error),
            (
                ready(),
                ended(StopReason::Other {
                    name: "later".into(),
                }),
                Activity::Error,
            ),
            (ready(), failed(), Activity::Error),
            (
                AgentState::LoginRequired {
                    message: "log in".into(),
                },
                failed(),
                Activity::Error,
            ),
            (
                AgentState::Failed {
                    message: "m".into(),
                },
                TurnState::Working,
                Activity::Error,
            ),
            (
                AgentState::Exited {
                    code: Some(1),
                    stderr: String::new(),
                },
                ended(StopReason::EndTurn),
                Activity::Error,
            ),
            (
                AgentState::Lost {
                    message: "its holder ended".into(),
                },
                TurnState::Failed {
                    message: "the agent exited".into(),
                },
                Activity::Error,
            ),
        ];
        for (agent, turn, activity) in rows {
            let transcript = with(agent.clone(), turn.clone());
            assert_eq!(transcript.activity(), activity, "{agent:?}, {turn:?}");
        }
    }

    #[test]
    fn the_slot_doesnt_change_the_activity() {
        for slot in [
            Slot::Waiting { text: "w".into() },
            Slot::Held {
                text: "h".into(),
                reason: HoldReason::Cancelled,
            },
        ] {
            let mut transcript = with(ready(), ended(StopReason::EndTurn));
            transcript.apply(&Event::Queue { slot }).unwrap();
            assert_eq!(transcript.activity(), Activity::Done);
        }
    }

    #[test]
    fn trimming_keeps_the_newest_entries_and_its_event_drops_the_same_ones() {
        let events: Vec<Event> = (1..=10)
            .map(|id| added(id, agent(&"x".repeat(100))))
            .collect();
        let mut trimmed = fold(&events).unwrap();
        let mut copy = trimmed.clone();
        let one = trimmed
            .entries()
            .next()
            .map(|(_, entry)| entry.encoded_len())
            .unwrap();
        let event = trimmed.trim_to(one * 3).expect("over the limit");
        assert_eq!(
            event,
            Event::Trimmed {
                through: EntryId(7),
                dropped: 7
            }
        );
        let kept: Vec<EntryId> = trimmed.entries().map(|(id, _)| id).collect();
        assert_eq!(kept, [EntryId(8), EntryId(9), EntryId(10)]);
        assert!(trimmed.encoded_len() <= one * 3);
        assert_eq!(trimmed.dropped(), 7);
        copy.apply(&event).unwrap();
        assert_eq!(copy, trimmed);
        // Within the limit, nothing is dropped.
        assert_eq!(trimmed.trim_to(one * 3), None);
        // The newest entry stays even when it alone is over the limit.
        let event = trimmed.trim_to(0).unwrap();
        assert_eq!(
            event,
            Event::Trimmed {
                through: EntryId(9),
                dropped: 2
            }
        );
        assert_eq!(trimmed.entries().count(), 1);
        assert_eq!(trimmed.dropped(), 9);
        assert_eq!(trimmed.trim_to(0), None);
        assert_eq!(fold(&trimmed.replay()), Ok(trimmed));
    }

    #[test]
    fn the_encoded_length_follows_appends_and_changes() {
        let mut transcript = fold(&[
            added(1, agent("a\"b")),
            Event::Appended {
                id: EntryId(1),
                text: "\n\u{1}\\é".into(),
            },
            added(2, tool("call_1")),
            Event::ToolChanged {
                id: EntryId(2),
                title: Some("a longer title than before".into()),
                kind: None,
                status: Some(ToolStatus::InProgress),
            },
            added(3, permission()),
        ])
        .unwrap();
        transcript
            .apply(&Event::Answered {
                id: EntryId(3),
                outcome: PermissionOutcome::Cancelled,
            })
            .unwrap();
        let measured: usize = transcript
            .entries()
            .map(|(_, entry)| serde_json::to_vec(entry).unwrap().len())
            .sum();
        assert_eq!(transcript.encoded_len(), measured);
    }

    #[test]
    fn events_encode_as_tagged_objects() {
        let event = Event::Added {
            id: EntryId(3),
            entry: Entry::Tool {
                call: "call_1".into(),
                title: "Read".into(),
                kind: ToolKind::SwitchMode,
                status: ToolStatus::InProgress,
            },
        };
        assert_eq!(
            event.to_value(),
            serde_json::json!({
                "event": "added",
                "id": 3,
                "entry": {"type": "tool", "call": "call_1", "title": "Read", "kind": "switch_mode", "status": "in_progress"},
            })
        );
        assert_eq!(
            Event::Turn {
                state: ended(StopReason::Other {
                    name: "later".into()
                })
            }
            .to_value(),
            serde_json::json!({"event": "turn", "state": {"turn": "ended", "stop": {"reason": "other", "name": "later"}}})
        );
        // Unset optional fields are left out.
        assert_eq!(
            Event::ToolChanged {
                id: EntryId(1),
                title: None,
                kind: None,
                status: None
            }
            .to_value(),
            serde_json::json!({"event": "tool_changed", "id": 1})
        );
    }

    #[test]
    fn decoding_refuses_what_isnt_an_event_and_ignores_unknown_fields() {
        for value in [
            serde_json::json!(null),
            serde_json::json!([{"event": "turn", "state": {"turn": "idle"}}]),
            serde_json::json!("turn"),
            serde_json::json!({}),
            serde_json::json!({"event": "teleported"}),
            serde_json::json!({"event": "turn", "state": {"turn": "sleeping"}}),
            serde_json::json!({"event": "turn"}),
            serde_json::json!({"event": "added", "id": -1, "entry": {"type": "user", "text": ""}}),
        ] {
            let error = Event::from_value(&value).unwrap_err();
            assert!(!error.to_string().is_empty(), "{value}");
        }
        assert_eq!(
            Event::from_value(&serde_json::json!({
                "event": "queue",
                "later": [1, {"x": null}],
                "slot": {"slot": "waiting", "text": "next", "later": true},
            })),
            Ok(Event::Queue {
                slot: Slot::Waiting {
                    text: "next".into()
                }
            })
        );
    }
}
