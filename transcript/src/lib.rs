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
//! - **Pages** (OpenSpec change `agents-log-paging`): an attach sends
//!   [`Transcript::page`], the newest whole entries within a byte budget
//!   behind an [`Event::Window`], and a client fetches older ones with
//!   [`Transcript::fetch`] and folds them in front with
//!   [`Transcript::prepend`]. The laws, pinned by property tests: a page
//!   folds to the log's suffix from its window, live events keep the copy
//!   equal to the server's suffix, and fetched pages folded in front until
//!   nothing more remains give the whole log.
//! - **Forms are frozen per client protocol version** (`process R5`): the
//!   event kinds and the values of every closed enum inside an event are
//!   protocol 4's, pinned by `tests/fixtures/v4/`. Protocol 4's forms
//!   freeze with the release that first ships it; until then the `agents-*`
//!   changes amend them in place. Once shipped, an event may only gain
//!   optional fields, defaulted and omitted when unset;
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

use std::collections::{BTreeMap, VecDeque};
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
    /// A tool call, by the agent's id for it, with its content (OpenSpec
    /// change `agents-tool-content`): the items in the agent's order, the
    /// count of items dropped past [`MAX_ITEMS`] or [`CALL_LIMIT`], the
    /// files it works on, and its raw input and result as text. Each is
    /// left out when unset, so a tool call without content encodes as
    /// before content existed.
    Tool {
        call: String,
        title: String,
        kind: ToolKind,
        status: ToolStatus,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        content: Vec<ToolItem>,
        #[serde(default, skip_serializing_if = "is_zero_u32")]
        more: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        locations: Vec<Location>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<String>,
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
    /// A tool call entry with no content.
    pub fn tool(call: String, title: String, kind: ToolKind, status: ToolStatus) -> Self {
        Self::Tool {
            call,
            title,
            kind,
            status,
            content: Vec::new(),
            more: 0,
            locations: Vec::new(),
            input: None,
            result: None,
        }
    }

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

/// The most text one tool content item holds (a [`ToolItem::Text`]'s text,
/// a [`FileChange`]'s lines, an [`Output`]'s text; OpenSpec change
/// `agents-tool-content`, design decision 5). Equal to [`TEXT_CHUNK`], so
/// any one item fits one event.
pub const ITEM_LIMIT: usize = TEXT_CHUNK;

/// The most content items a tool call keeps; later ones are counted in its
/// `more`.
pub const MAX_ITEMS: usize = 32;

/// The most text a tool call's items hold together; later items are
/// counted in its `more`.
pub const CALL_LIMIT: usize = 256 * 1024;

/// The most text each of a tool call's raw input and raw result keeps.
pub const RAW_LIMIT: usize = 16 * 1024;

/// The most locations a tool call keeps.
pub const MAX_LOCATIONS: usize = 32;

/// One item of a tool call's content, in Brindle's own form: no ACP
/// version is named in it (design decision 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "item", rename_all = "snake_case")]
pub enum ToolItem {
    /// Markdown source; `cut` when its end was dropped at [`ITEM_LIMIT`].
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "is_false")]
        cut: bool,
    },
    /// One file's change.
    Diff(FileChange),
    /// One command's output.
    Output(Output),
}

impl ToolItem {
    /// The item's text, as [`ITEM_LIMIT`] and [`CALL_LIMIT`] count it.
    pub fn text_len(&self) -> usize {
        match self {
            Self::Text { text, .. } => text.len(),
            Self::Diff(change) => change.text_len(),
            Self::Output(output) => output.text.len(),
        }
    }
}

/// One file's change: its operation, its path and its changed lines as
/// hunks. A change with no hunks is valid (a binary file, a move with no
/// edit). `omitted` counts the lines not kept past [`ITEM_LIMIT`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    #[serde(flatten)]
    pub op: FileOp,
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hunks: Vec<Hunk>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub omitted: u32,
}

impl FileChange {
    /// The kept lines' text together.
    pub fn text_len(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .map(|line| line.text().len())
            .sum()
    }
}

/// What a file change does. Only a move or copy has a second path; an
/// operation a later ACP adds keeps its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FileOp {
    Add,
    Delete,
    Modify,
    Move { from: String },
    Copy { from: String },
    Other { name: String },
}

/// A run of changed lines with their context: where it starts in the old
/// and the new file (1-based), and its lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

/// One line of a hunk, without its line break. It encodes as one JSON
/// string whose first character is its sign: a space, `+` or `-`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    Context(String),
    Added(String),
    Removed(String),
}

impl DiffLine {
    /// The line's text, without its sign.
    pub fn text(&self) -> &str {
        match self {
            Self::Context(text) | Self::Added(text) | Self::Removed(text) => text,
        }
    }
}

impl Serialize for DiffLine {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (sign, text) = match self {
            Self::Context(text) => (' ', text),
            Self::Added(text) => ('+', text),
            Self::Removed(text) => ('-', text),
        };
        let mut line = String::with_capacity(text.len() + 1);
        line.push(sign);
        line.push_str(text);
        serializer.serialize_str(&line)
    }
}

impl<'de> Deserialize<'de> for DiffLine {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let line = String::deserialize(deserializer)?;
        let mut chars = line.chars();
        let sign = chars.next();
        let text = chars.as_str().to_string();
        match sign {
            Some(' ') => Ok(Self::Context(text)),
            Some('+') => Ok(Self::Added(text)),
            Some('-') => Ok(Self::Removed(text)),
            Some(other) => Err(serde::de::Error::custom(format!(
                "a diff line starts with a space, + or -, not {other:?}"
            ))),
            None => Err(serde::de::Error::custom("a diff line can't be empty")),
        }
    }
}

/// One command's output, as plain text. `omitted` counts the bytes dropped
/// from its start past [`ITEM_LIMIT`]; `partial` says the kept text starts
/// inside a line whose start was dropped, which the fold needs to keep
/// whole lines the same however the output was split.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub omitted: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub partial: bool,
    /// How the command ended; `None` while it runs, or when it never said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<Exit>,
}

impl Output {
    /// Appends `text` and keeps the longest suffix within [`ITEM_LIMIT`]
    /// (design decision 8): whole lines are dropped from the start, and a
    /// single line longer than the limit keeps its last bytes, cut at a
    /// character boundary. Appending `a` then `b` gives what appending
    /// `a + b` does. Returns the bytes dropped, for the entry's size.
    fn append(&mut self, text: &str) -> String {
        self.text.push_str(text);
        let cut = self.cut();
        if cut == 0 {
            return String::new();
        }
        let partial = self.text.as_bytes().get(cut - 1) != Some(&b'\n');
        let dropped: String = self.text.drain(..cut).collect();
        self.omitted = self.omitted.saturating_add(dropped.len() as u64);
        self.partial = partial;
        dropped
    }

    /// Where the kept text starts: the first line start at or after the
    /// limit's reach, or failing one the first character boundary there.
    /// The text's own start is a line start unless it is `partial`.
    fn cut(&self) -> usize {
        let text = &self.text;
        let low = text.len().saturating_sub(ITEM_LIMIT);
        if low == 0 && !self.partial {
            return 0;
        }
        if low > 0 && text.as_bytes().get(low - 1) == Some(&b'\n') {
            return low;
        }
        // The first character boundary from `low`: a line break is one
        // byte, so none is skipped.
        let boundary = (low..=text.len())
            .find(|&at| text.is_char_boundary(at))
            .unwrap_or(text.len());
        match text.get(boundary..).and_then(|rest| rest.find('\n')) {
            Some(at) if boundary + at + 1 < text.len() => boundary + at + 1,
            Some(_) | None => boundary,
        }
    }
}

/// How a command ended: an exit code, or a signal by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "exit", rename_all = "snake_case")]
pub enum Exit {
    Code { code: i32 },
    Signal { name: String },
}

/// A file a tool call works on, and its line when the agent gave one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// A field that a change event leaves out when unchanged, sends as `null`
/// when cleared and as its value when set.
mod change {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, T: Serialize>(
        value: &Option<Option<T>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(inner) => inner.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
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
    /// The agent needs a login: its message, and the login methods it
    /// advertised in `initialize`, in its order (none encodes as before
    /// methods existed).
    LoginRequired {
        message: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        methods: Vec<LoginMethod>,
    },
    /// A login the server runs for a client (ACP's `authenticate`) is out:
    /// the method's id and name, so a client attaching mid-login can say
    /// what runs.
    LoggingIn { method: String, name: String },
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
    /// Initialized, with no conversation yet: the session waits for a
    /// client to choose one to continue, or a new one (OpenSpec change
    /// `agents-session-list`, decision 1). `message` is the agent's refusal
    /// of the last choice, when it refused one (flag 5); none encodes as
    /// `{"agent":"choosing"}`.
    Choosing {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

/// A login method an agent advertised (OpenSpec change `agents-login`,
/// design decision 2). On the wire it is `{id, name, description?, kind,
/// args?, env?}`; only a [`LoginKind::Terminal`] method has `args` and `env`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginMethod {
    /// The agent's id for it, which a login names.
    pub id: String,
    /// What the user is shown.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub kind: LoginKind,
}

/// How a login method runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoginKind {
    /// The agent logs in itself when asked (ACP's `authenticate`), which
    /// the server sends on a client's request.
    Agent,
    /// The client runs the agent's own program with `args` appended and
    /// `env` laid over its environment, and restarts the agent once it
    /// exits with status 0.
    Terminal {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    },
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
    /// The conversation's title, once the agent gave one (ACP's
    /// `session_info_update`; OpenSpec change `agents-session-title`).
    /// Never empty: no title is `None`. Left out when unset, so an
    /// untitled conversation encodes as before titles existed (protocol 4,
    /// amended in place while unreleased).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
    /// A tool call's fields that changed. `locations` replaces the list
    /// whole; `input` and `result` are left out when unchanged, `null` when
    /// cleared.
    ToolChanged {
        id: EntryId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<ToolKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ToolStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locations: Option<Vec<Location>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "change")]
        input: Option<Option<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "change")]
        result: Option<Option<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        more: Option<u32>,
    },
    /// A tool call's content from item `from` on is replaced by `items`: a
    /// replaced list is `from: 0`, an appended item `from` the length, and
    /// `from: 0` with no items clears it.
    ToolItems {
        id: EntryId,
        from: u32,
        #[serde(default)]
        items: Vec<ToolItem>,
    },
    /// More text for a tool call's output item, and how its command ended
    /// once it did. The fold keeps the text's longest suffix within
    /// [`ITEM_LIMIT`].
    ToolOutput {
        id: EntryId,
        item: u32,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit: Option<Exit>,
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
    /// ACP session, before its `Ready`, and again when the agent retitles
    /// it. It stays through the agent's later states, and a later one
    /// replaces it.
    Conversation {
        conversation: Conversation,
    },
    /// The entries up to and including `through` were dropped; `dropped`
    /// counts them, or in a replay every entry dropped so far.
    Trimmed {
        through: EntryId,
        dropped: u64,
    },
    /// Only in a page ([`Transcript::page`]), after any `Trimmed` and
    /// before the first `Added`, never live: the entries from `from` on
    /// follow, and `earlier` kept entries before `from` exist on the server
    /// and can be fetched (OpenSpec change `agents-log-paging`, design
    /// decision 3). `Trimmed` still means gone.
    Window {
        from: EntryId,
        earlier: u64,
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
    /// A `ToolChanged`, `ToolItems` or `ToolOutput` for an entry that isn't
    /// a tool call.
    NotATool { id: EntryId },
    /// A `ToolItems` whose `from` is past the end of the content.
    ItemsOutOfRange { id: EntryId, from: u32, len: u32 },
    /// A `ToolOutput` for an item that is missing or isn't an output.
    NotAnOutput { id: EntryId, item: u32 },
    /// An `Answered` for an entry that isn't a permission request.
    NotAPermission { id: EntryId },
    /// An `Answered` for a permission request that already has an outcome.
    AlreadyAnswered { id: EntryId },
    /// A `Trimmed` past the last entry of a log that has entries.
    TrimPastEnd { through: EntryId, next: EntryId },
    /// A `Trimmed` that doesn't go past the last one.
    TrimBehind { through: EntryId, trimmed: EntryId },
    /// A `Window` on a transcript that already holds entries.
    WindowAfterEntries,
    /// A `Window` that starts before the next entry's id.
    WindowBehind { from: EntryId, next: EntryId },
    /// A `Window` with no earlier entries: a page that holds every entry
    /// has no window.
    WindowEmpty { from: EntryId },
    /// A fetched page that doesn't end right before the held entries.
    PageGap { last: EntryId, from: EntryId },
    /// A fetched page whose entries and `more` don't fit the window: `more`
    /// with no entries, no `more` while earlier entries would remain, or
    /// more entries than the window has.
    WindowMismatch {
        earlier: u64,
        fetched: u64,
        more: bool,
    },
    /// A fetched page with entries for a transcript that holds every entry.
    NoWindow,
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
            Self::ItemsOutOfRange { id, from, len } => write!(
                f,
                "tool call {id}'s content was replaced from item {from}, but it has {len}"
            ),
            Self::NotAnOutput { id, item } => {
                write!(f, "tool call {id} has no output item {item}")
            }
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
            Self::WindowAfterEntries => {
                f.write_str("a page's window came after entries were already held")
            }
            Self::WindowBehind { from, next } => write!(
                f,
                "a page's window starts at entry {from}, but the next entry is {next}"
            ),
            Self::WindowEmpty { from } => {
                write!(f, "a page's window at entry {from} has no earlier entries")
            }
            Self::PageGap { last, from } => write!(
                f,
                "a fetched page ends at entry {last}, but the held entries start at {from}"
            ),
            Self::WindowMismatch {
                earlier,
                fetched,
                more,
            } => {
                let more = if *more {
                    "more to fetch"
                } else {
                    "nothing more to fetch"
                };
                write!(
                    f,
                    "a fetched page of {fetched} entries with {more} doesn't fit the {earlier} earlier entries"
                )
            }
            Self::NoWindow => {
                f.write_str("a fetched page came for a transcript that holds every entry")
            }
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

/// The most a page or a fetched page holds, in entries' encoded length
/// (OpenSpec change `agents-log-paging`, design decision 3).
pub const PAGE_BYTES: usize = 256 * 1024;

/// What a paged transcript doesn't hold: the entries before `from`, of
/// which `earlier` (never 0) are kept on the server and can be fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub from: EntryId,
    pub earlier: u64,
}

/// A fetch's answer ([`Transcript::fetch`]): the entries' events, in id
/// order, and whether kept entries remain before the first of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub events: Vec<Event>,
    pub more: bool,
}

/// An agent session's log: its entries in id order, the queue slot, the
/// turn and the agent's state. It changes only through [`Transcript::apply`]
/// and, on a client's copy, [`Transcript::prepend`], so the core's log and
/// a client's copy fold the same events the same way.
///
/// A client's copy may hold only a suffix of the log (OpenSpec change
/// `agents-log-paging`, design decision 4): a page's [`Event::Window`]
/// sets a [`Window`], events for entries below it are ignored (a later
/// fetch brings them current), a `Trimmed` below it lowers its count, and
/// fetched pages folded in front with [`Transcript::prepend`] lower it
/// until it goes. The server's log never has a window.
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
    /// Set by [`Event::Window`]; `None` when every kept entry is held.
    window: Option<Window>,
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
            window: None,
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
        if let Some(id) = entry_change(event)
            && self.window.is_some_and(|window| id < window.from)
        {
            // Not held: a later fetch brings the entry as it is then.
            return Ok(());
        }
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
                locations,
                input,
                result,
                more,
            } => {
                let kept = self.kept_mut(*id)?;
                let Entry::Tool {
                    title: own_title,
                    kind: own_kind,
                    status: own_status,
                    locations: own_locations,
                    input: own_input,
                    result: own_result,
                    more: own_more,
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
                if let Some(locations) = locations {
                    own_locations.clone_from(locations);
                }
                if let Some(input) = input {
                    own_input.clone_from(input);
                }
                if let Some(result) = result {
                    own_result.clone_from(result);
                }
                if let Some(more) = more {
                    *own_more = *more;
                }
                self.resize_entry(*id);
            }
            Event::ToolItems { id, from, items } => {
                let kept = self.kept_mut(*id)?;
                let Entry::Tool { content, .. } = &mut kept.entry else {
                    return Err(FoldError::NotATool { id: *id });
                };
                let len = u32::try_from(content.len()).unwrap_or(u32::MAX);
                if *from > len {
                    return Err(FoldError::ItemsOutOfRange {
                        id: *id,
                        from: *from,
                        len,
                    });
                }
                content.truncate(*from as usize);
                content.extend(items.iter().cloned());
                self.resize_entry(*id);
            }
            Event::ToolOutput {
                id,
                item,
                text,
                exit,
            } => {
                let kept = self.kept_mut(*id)?;
                let Entry::Tool { content, .. } = &mut kept.entry else {
                    return Err(FoldError::NotATool { id: *id });
                };
                let Some(ToolItem::Output(output)) = content.get_mut(*item as usize) else {
                    return Err(FoldError::NotAnOutput { id: *id, item: *item });
                };
                let before = output_fields_len(output);
                let dropped = output.append(text);
                if let Some(exit) = exit {
                    output.exit = Some(exit.clone());
                }
                let after = output_fields_len(output);
                // Grown by the text and shrunk by what was dropped, with no
                // re-encode per piece; `omitted`, `partial` and `exit` are
                // measured alone.
                let old = kept.size;
                let size = (old + escaped_len(text) + after)
                    .saturating_sub(escaped_len(&dropped) + before);
                kept.size = size;
                self.size = self.size.saturating_sub(old) + size;
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
            Event::Window { from, earlier } => {
                if !self.entries.is_empty() {
                    return Err(FoldError::WindowAfterEntries);
                }
                if *from < self.next {
                    return Err(FoldError::WindowBehind {
                        from: *from,
                        next: self.next,
                    });
                }
                if *earlier == 0 {
                    return Err(FoldError::WindowEmpty { from: *from });
                }
                self.next = *from;
                self.window = Some(Window {
                    from: *from,
                    earlier: *earlier,
                });
            }
        }
        Ok(())
    }

    fn trim(&mut self, through: EntryId, dropped: u64) -> Result<(), FoldError> {
        self.trim_entries(through, dropped)?;
        if let Some(window) = self.window {
            // Below the window, the trim takes entries the server kept and
            // this copy never held: fewer remain to fetch. At or past it,
            // the held entries were dropped through it, so nothing earlier
            // is left.
            let earlier = if through < window.from {
                window
                    .earlier
                    .min(window.from.0.saturating_sub(through.0).saturating_sub(1))
            } else {
                0
            };
            self.window = (earlier > 0).then_some(Window {
                from: window.from,
                earlier,
            });
        }
        Ok(())
    }

    fn trim_entries(&mut self, through: EntryId, dropped: u64) -> Result<(), FoldError> {
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

    /// What this copy doesn't hold, when it was paged; the server's log
    /// never has a window.
    pub fn window(&self) -> Option<Window> {
        self.window
    }

    /// The events that rebuild this log in a fresh one: `Trimmed` when
    /// entries were dropped, `Window` when this copy holds only a suffix,
    /// each entry's `Added` with its text split into `Appended`s of at most
    /// [`TEXT_CHUNK`] bytes and a permission's outcome as its `Answered`,
    /// the slot's `Queue` when it isn't empty, the `Conversation` when there
    /// is one, then `Turn` and `Agent`.
    pub fn replay(&self) -> Vec<Event> {
        self.replay_from(0)
    }

    /// [`Transcript::replay`] cut at a start entry (design decision 3): the
    /// newest whole entries whose encoded lengths add up to at most
    /// `budget`, always the newest one, reaching back to the oldest pending
    /// permission and to the tool call each pending permission names (the
    /// newest tool entry with that call before it; OpenSpec change
    /// `agents-tool-content`, design decision 15); then a `Window` naming
    /// how many kept entries precede them. Everything that isn't an entry
    /// (the slot, the conversation, the turn, the agent's state) is in
    /// every page. A log that fits the budget pages as its `replay()`.
    pub fn page(&self, budget: usize) -> Vec<Event> {
        let mut start = self.entries.len() - self.newest_within(self.entries.len(), budget);
        for (index, kept) in self.entries.iter().enumerate() {
            let Entry::Permission {
                outcome: None,
                call,
                ..
            } = &kept.entry
            else {
                continue;
            };
            start = start.min(index);
            if let Some(call) = call
                && let Some(tool) = self.entries.range(..index).rposition(|kept| {
                    matches!(&kept.entry, Entry::Tool { call: own, .. } if own == call)
                })
            {
                start = start.min(tool);
            }
        }
        self.replay_from(start)
    }

    /// The newest whole kept entries with ids below `before` whose encoded
    /// lengths add up to at most `budget` (at least one when any exists),
    /// each as it stands now, in id order; `more` says whether kept
    /// entries remain before the first of them. A fetch carries entries
    /// only: the state after them is already the client's.
    pub fn fetch(&self, before: EntryId, budget: usize) -> Fetched {
        let end = self.entries.partition_point(|kept| kept.id < before);
        let start = end - self.newest_within(end, budget);
        let mut events = Vec::new();
        for kept in self.entries.range(start..end) {
            replay_entry(kept.id, &kept.entry, &mut events);
        }
        Fetched {
            events,
            more: start > 0 || self.window.is_some(),
        }
    }

    /// How many of the kept entries before index `end`, newest first, fit
    /// in `budget`; at least one when there is one.
    fn newest_within(&self, end: usize, budget: usize) -> usize {
        let mut total = 0usize;
        let mut taken = 0usize;
        for kept in self.entries.range(..end).rev() {
            total = total.saturating_add(kept.size);
            if taken > 0 && total > budget {
                break;
            }
            taken += 1;
        }
        taken
    }

    /// Folds a fetched page in front of the held entries (design decision
    /// 4): its entries must end right before the window's `from`; they go
    /// in front, the window's count drops by theirs, and the window goes
    /// when `more` is false. An empty page with `more` false on a copy
    /// with no window is the answer to a fetch a trim overtook, and changes
    /// nothing. A refused page changes nothing.
    pub fn prepend(&mut self, events: &[Event], more: bool) -> Result<(), FoldError> {
        let Some(first) = events.iter().find_map(|event| match event {
            Event::Added { id, .. } => Some(*id),
            Event::Appended { .. }
            | Event::ToolChanged { .. }
            | Event::ToolItems { .. }
            | Event::ToolOutput { .. }
            | Event::Answered { .. }
            | Event::Queue { .. }
            | Event::Turn { .. }
            | Event::Agent { .. }
            | Event::Conversation { .. }
            | Event::Trimmed { .. }
            | Event::Window { .. } => None,
        }) else {
            return match self.window {
                None if !more => Ok(()),
                window => Err(FoldError::WindowMismatch {
                    earlier: window.map_or(0, |window| window.earlier),
                    fetched: 0,
                    more,
                }),
            };
        };
        let Some(window) = self.window else {
            return Err(FoldError::NoWindow);
        };
        let mut page = Self {
            next: first,
            ..Self::default()
        };
        for event in events {
            page.apply(event)?;
        }
        let last = page.entries.back().map_or(first, |kept| kept.id);
        if page.window.is_some() || last.next() != window.from {
            return Err(FoldError::PageGap {
                last,
                from: window.from,
            });
        }
        let fetched = page.entries.len() as u64;
        let earlier = window.earlier.checked_sub(fetched);
        let earlier = match (earlier, more) {
            (Some(earlier), true) if earlier > 0 => Some(earlier),
            (Some(0), false) => None,
            (Some(_) | None, true | false) => {
                return Err(FoldError::WindowMismatch {
                    earlier: window.earlier,
                    fetched,
                    more,
                });
            }
        };
        let from = page.entries.front().map_or(window.from, |kept| kept.id);
        self.size += page.size;
        while let Some(kept) = page.entries.pop_back() {
            self.entries.push_front(kept);
        }
        self.window = earlier.map(|earlier| Window { from, earlier });
        Ok(())
    }

    /// This log with the entries before `from` taken out into a window:
    /// what a page from `from` folds to (design decision 4's first law).
    #[cfg(test)]
    pub(crate) fn suffix(&self, from: EntryId) -> Self {
        let mut suffix = self.clone();
        let mut taken = 0u64;
        while let Some(front) = suffix.entries.front()
            && front.id < from
        {
            suffix.size -= front.size;
            suffix.entries.pop_front();
            taken += 1;
        }
        let earlier = taken + self.window.map_or(0, |window| window.earlier);
        if earlier > 0 {
            let from = suffix.entries.front().map_or(suffix.next, |kept| kept.id);
            suffix.window = Some(Window { from, earlier });
        }
        suffix
    }

    fn replay_from(&self, start: usize) -> Vec<Event> {
        let mut events = Vec::with_capacity(self.entries.len() - start + 6);
        if let Some(trim) = self.trimmed {
            events.push(Event::Trimmed {
                through: trim.through,
                dropped: trim.dropped,
            });
        }
        let earlier = start as u64 + self.window.map_or(0, |window| window.earlier);
        if earlier > 0 {
            let from = self.entries.get(start).map_or(self.next, |kept| kept.id);
            events.push(Event::Window { from, earlier });
        }
        for kept in self.entries.range(start..) {
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
            AgentState::Starting | AgentState::LoggingIn { .. } => Activity::Working,
            // Nothing runs on the user's behalf while a conversation is
            // chosen (`agents-session-list`'s "A choosing zone waits
            // quietly").
            AgentState::Choosing { .. } => Activity::Idle,
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

/// The encoded length of an output's fields that `ToolOutput` changes other
/// than its text: `omitted`, `partial` and `exit`, each with its comma,
/// none when unset (every one follows the item's tag and its text).
fn output_fields_len(output: &Output) -> usize {
    let omitted = if output.omitted == 0 {
        0
    } else {
        ",\"omitted\":".len() + output.omitted.to_string().len()
    };
    let partial = if output.partial {
        ",\"partial\":true".len()
    } else {
        0
    };
    let exit = output.exit.as_ref().map_or(0, |exit| {
        ",\"exit\":".len() + serde_json::to_vec(exit).map_or(0, |bytes| bytes.len())
    });
    omitted + partial + exit
}

/// The entry a change to an existing entry names: the events a paged copy
/// ignores below its window.
fn entry_change(event: &Event) -> Option<EntryId> {
    match event {
        Event::Appended { id, .. }
        | Event::ToolChanged { id, .. }
        | Event::ToolItems { id, .. }
        | Event::ToolOutput { id, .. }
        | Event::Answered { id, .. } => Some(*id),
        Event::Added { .. }
        | Event::Queue { .. }
        | Event::Turn { .. }
        | Event::Agent { .. }
        | Event::Conversation { .. }
        | Event::Trimmed { .. }
        | Event::Window { .. } => None,
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
        Entry::Tool {
            call,
            title,
            kind,
            status,
            content,
            more,
            locations,
            input,
            result,
        } => {
            // The entry without its content, then one item per event, so
            // any one event stays within a frame.
            events.push(Event::Added {
                id,
                entry: Entry::Tool {
                    call: call.clone(),
                    title: title.clone(),
                    kind: *kind,
                    status: *status,
                    content: Vec::new(),
                    more: *more,
                    locations: locations.clone(),
                    input: input.clone(),
                    result: result.clone(),
                },
            });
            events.extend(content.iter().enumerate().map(|(index, item)| {
                Event::ToolItems {
                    id,
                    from: u32::try_from(index).unwrap_or(u32::MAX),
                    items: vec![item.clone()],
                }
            }));
        }
        Entry::Notice { .. } => events.push(Event::Added {
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
        Entry::tool(
            call.into(),
            "Read notes.md".into(),
            ToolKind::Read,
            ToolStatus::Pending,
        )
    }

    /// A `ToolChanged` of the title, kind and status alone.
    fn changed(
        id: u64,
        title: Option<&str>,
        kind: Option<ToolKind>,
        status: Option<ToolStatus>,
    ) -> Event {
        Event::ToolChanged {
            id: EntryId(id),
            title: title.map(Into::into),
            kind,
            status,
            locations: None,
            input: None,
            result: None,
            more: None,
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
            .apply(&changed(2, None, None, Some(ToolStatus::Completed)))
            .unwrap();
        transcript
            .apply(&changed(1, Some("Edit notes.md"), Some(ToolKind::Edit), None))
            .unwrap();
        assert_eq!(
            transcript.entry(EntryId(1)),
            Some(&Entry::tool(
                "call_1".into(),
                "Edit notes.md".into(),
                ToolKind::Edit,
                ToolStatus::Pending,
            ))
        );
        assert_eq!(
            transcript.entry(EntryId(2)),
            Some(&Entry::tool(
                "call_2".into(),
                "Read notes.md".into(),
                ToolKind::Read,
                ToolStatus::Completed,
            ))
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
                changed(3, None, None, Some(ToolStatus::Failed)),
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
        let mut conversations = [0usize; 3];
        let mut lost = 0usize;
        for _ in 0..400 {
            let transcript = random_transcript(&mut rng);
            slots[match transcript.slot() {
                Slot::Empty => 0,
                Slot::Waiting { .. } => 1,
                Slot::Held { .. } => 2,
            }] += 1;
            conversations[match transcript.conversation() {
                None => 0,
                Some(Conversation { title: None, .. }) => 1,
                Some(Conversation { title: Some(_), .. }) => 2,
            }] += 1;
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
            "logs without a conversation, with an untitled one and with a titled one: \
             {conversations:?}"
        );
        assert!(lost > 0, "no log ended with the agent lost");
    }

    fn conversation(id: &str, continuation: Continuation) -> Conversation {
        Conversation {
            id: id.into(),
            cwd: "/home/u/src".into(),
            continuation,
            title: None,
        }
    }

    #[test]
    fn a_titled_conversation_replaces_an_untitled_one_and_a_later_one_clears_it() {
        let untitled = conversation("s1", Continuation::Resume);
        let titled = Conversation {
            title: Some("Hello.txt file creation".into()),
            ..untitled.clone()
        };
        let mut transcript = Transcript::new();
        for (next, title) in [
            (&untitled, None),
            (&titled, Some("Hello.txt file creation")),
            (&untitled, None),
        ] {
            transcript
                .apply(&Event::Conversation {
                    conversation: next.clone(),
                })
                .unwrap();
            assert_eq!(
                transcript.conversation().and_then(|c| c.title.as_deref()),
                title
            );
        }
        // A replay carries the title.
        transcript
            .apply(&Event::Conversation {
                conversation: titled.clone(),
            })
            .unwrap();
        assert!(transcript.replay().contains(&Event::Conversation {
            conversation: titled
        }));
        assert_eq!(fold(&transcript.replay()), Ok(transcript));
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
            (AgentState::Choosing { message: None }, TurnState::Idle, Activity::Idle),
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
                    methods: Vec::new(),
                },
                failed(),
                Activity::Error,
            ),
            (
                AgentState::LoginRequired {
                    message: "log in".into(),
                    methods: vec![agent_method("chat-gpt", "ChatGPT")],
                },
                TurnState::Idle,
                Activity::Error,
            ),
            (
                AgentState::LoggingIn {
                    method: "chat-gpt".into(),
                    name: "ChatGPT".into(),
                },
                TurnState::Idle,
                Activity::Working,
            ),
            (
                AgentState::LoggingIn {
                    method: "chat-gpt".into(),
                    name: "ChatGPT".into(),
                },
                failed(),
                Activity::Working,
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
            changed(
                2,
                Some("a longer title than before"),
                None,
                Some(ToolStatus::InProgress),
            ),
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
            entry: Entry::tool(
                "call_1".into(),
                "Read".into(),
                ToolKind::SwitchMode,
                ToolStatus::InProgress,
            ),
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
            changed(1, None, None, None).to_value(),
            serde_json::json!({"event": "tool_changed", "id": 1})
        );
    }

    fn agent_method(id: &str, name: &str) -> LoginMethod {
        LoginMethod {
            id: id.into(),
            name: name.into(),
            description: None,
            kind: LoginKind::Agent,
        }
    }

    fn terminal_method() -> LoginMethod {
        LoginMethod {
            id: "cli-login".into(),
            name: "Log in from the terminal".into(),
            description: Some("Paste a code".into()),
            kind: LoginKind::Terminal {
                args: vec!["login".into()],
                env: BTreeMap::from([("LOGIN_MODE".into(), "device".into())]),
            },
        }
    }

    #[test]
    fn a_login_demand_without_methods_encodes_as_before_methods_existed() {
        let state = AgentState::LoginRequired {
            message: "run agent login".into(),
            methods: Vec::new(),
        };
        assert_eq!(
            Event::Agent { state }.to_value(),
            serde_json::json!({"event": "agent", "state": {"agent": "login_required", "message": "run agent login"}})
        );
    }

    #[test]
    fn login_methods_encode_flat_with_their_kind_and_only_a_terminals_args_and_env() {
        let state = AgentState::LoginRequired {
            message: "m".into(),
            methods: vec![agent_method("chat-gpt", "ChatGPT"), terminal_method()],
        };
        let value = Event::Agent { state }.to_value();
        assert_eq!(
            value["state"]["methods"],
            serde_json::json!([
                {"id": "chat-gpt", "name": "ChatGPT", "kind": "agent"},
                {"id": "cli-login", "name": "Log in from the terminal", "description": "Paste a code",
                 "kind": "terminal", "args": ["login"], "env": {"LOGIN_MODE": "device"}},
            ])
        );
        // A terminal method with no args and no env leaves both out.
        let bare = LoginMethod {
            kind: LoginKind::Terminal {
                args: Vec::new(),
                env: BTreeMap::new(),
            },
            ..agent_method("t", "T")
        };
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            serde_json::json!({"id": "t", "name": "T", "kind": "terminal"})
        );
        // An agent method's stray args and env are not its own: they're
        // ignored, as any unknown field is.
        let stray = serde_json::json!({"event": "agent", "state": {"agent": "login_required", "message": "m",
            "methods": [{"id": "a", "name": "A", "kind": "agent", "args": ["x"], "env": {"K": "v"}}]}});
        assert_eq!(
            Event::from_value(&stray),
            Ok(Event::Agent {
                state: AgentState::LoginRequired {
                    message: "m".into(),
                    methods: vec![agent_method("a", "A")],
                }
            })
        );
    }

    #[test]
    fn a_login_method_without_its_id_name_or_kind_or_of_an_unknown_kind_is_a_decode_error() {
        for method in [
            serde_json::json!({"name": "A", "kind": "agent"}),
            serde_json::json!({"id": "a", "kind": "agent"}),
            serde_json::json!({"id": "a", "name": "A"}),
            serde_json::json!({"id": "a", "name": "A", "kind": "env_var"}),
            serde_json::json!({"id": "a", "name": "A", "kind": "terminal", "args": "login"}),
            serde_json::json!({"id": "a", "name": "A", "kind": "terminal", "env": {"K": 1}}),
        ] {
            let value = serde_json::json!({"event": "agent", "state": {"agent": "login_required", "message": "m", "methods": [method]}});
            assert!(Event::from_value(&value).is_err(), "{value}");
        }
        for state in [
            serde_json::json!({"agent": "logging_in", "method": "a"}),
            serde_json::json!({"agent": "logging_in", "name": "A"}),
        ] {
            let value = serde_json::json!({"event": "agent", "state": state});
            assert!(Event::from_value(&value).is_err(), "{value}");
        }
    }

    #[test]
    fn the_login_states_fold_and_replay_as_the_agents_state() {
        let required = AgentState::LoginRequired {
            message: "log in".into(),
            methods: vec![agent_method("chat-gpt", "ChatGPT"), terminal_method()],
        };
        let logging_in = AgentState::LoggingIn {
            method: "chat-gpt".into(),
            name: "ChatGPT".into(),
        };
        let mut transcript = with(required.clone(), TurnState::Idle);
        assert_eq!(transcript.agent(), &required);
        assert_eq!(fold(&transcript.replay()), Ok(transcript.clone()));
        transcript
            .apply(&Event::Agent {
                state: logging_in.clone(),
            })
            .unwrap();
        assert_eq!(transcript.agent(), &logging_in);
        assert_eq!(transcript.replay().last(), Some(&Event::Agent { state: logging_in }));
        assert_eq!(fold(&transcript.replay()), Ok(transcript));
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

    // Paging (OpenSpec change `agents-log-paging`, design decisions 3, 4).

    /// A log of `count` agent entries of about `size` bytes each, then a
    /// ready agent.
    fn long(count: u64, size: usize) -> Transcript {
        let mut events: Vec<Event> = (1..=count)
            .map(|id| added(id, agent(&"x".repeat(size))))
            .collect();
        events.push(Event::Agent { state: ready() });
        fold(&events).unwrap()
    }

    fn window(from: u64, earlier: u64) -> Event {
        Event::Window {
            from: EntryId(from),
            earlier,
        }
    }

    fn appended(id: u64, text: &str) -> Event {
        Event::Appended {
            id: EntryId(id),
            text: text.into(),
        }
    }

    /// A copy paged from entry 5 of `long(8, 10)`.
    fn paged() -> Transcript {
        let server = long(8, 10);
        let copy = fold(&server.replay_from(4)).unwrap();
        assert_eq!(
            copy.window(),
            Some(Window {
                from: EntryId(5),
                earlier: 4
            })
        );
        copy
    }

    #[test]
    fn a_window_goes_only_on_an_empty_transcript_at_or_past_the_next_id() {
        let mut copy = fold(&[window(5, 4)]).unwrap();
        assert_eq!(copy.next_id(), EntryId(5));
        assert_eq!(
            copy.window(),
            Some(Window {
                from: EntryId(5),
                earlier: 4
            })
        );
        copy.apply(&added(5, agent("hi"))).unwrap();
        let before = copy.clone();
        assert_eq!(
            copy.apply(&window(9, 1)),
            Err(FoldError::WindowAfterEntries)
        );
        assert_eq!(copy, before, "a refused event changes nothing");

        let mut trimmed = fold(&[Event::Trimmed {
            through: EntryId(6),
            dropped: 6,
        }])
        .unwrap();
        let before = trimmed.clone();
        assert_eq!(
            trimmed.apply(&window(5, 1)),
            Err(FoldError::WindowBehind {
                from: EntryId(5),
                next: EntryId(7)
            })
        );
        assert_eq!(trimmed, before, "a refused event changes nothing");
        trimmed.apply(&window(7, 3)).unwrap();
        // A window with nothing earlier is refused.
        let mut empty = Transcript::new();
        assert_eq!(
            empty.apply(&window(3, 0)),
            Err(FoldError::WindowEmpty { from: EntryId(3) })
        );
        assert_eq!(empty, Transcript::new());
    }

    #[test]
    fn changes_below_the_window_are_ignored_and_the_rest_fold_as_before() {
        let mut copy = paged();
        let before = copy.clone();
        for event in [
            appended(2, "more"),
            changed(1, None, None, Some(ToolStatus::Completed)),
            Event::Answered {
                id: EntryId(4),
                outcome: PermissionOutcome::Cancelled,
            },
        ] {
            assert_eq!(copy.apply(&event), Ok(()), "{event:?}");
            assert_eq!(copy, before, "{event:?} changed nothing");
        }
        // An entry from the window on is held.
        copy.apply(&appended(5, "!")).unwrap();
        assert_eq!(copy.entry(EntryId(5)), Some(&agent(&format!("{}!", "x".repeat(10)))));
        // An `Added` must still be the next id.
        assert_eq!(
            copy.apply(&added(10, agent("late"))),
            Err(FoldError::OutOfOrder {
                id: EntryId(10),
                expected: EntryId(9)
            })
        );
        // State events aren't entries: unchanged.
        copy.apply(&Event::Queue {
            slot: Slot::Waiting { text: "next".into() },
        })
        .unwrap();
        copy.apply(&Event::Turn { state: TurnState::Working }).unwrap();
        copy.apply(&Event::Conversation {
            conversation: conversation("s1", Continuation::Resume),
        })
        .unwrap();
        assert_eq!(copy.turn(), &TurnState::Working);
        assert!(copy.window().is_some());
    }

    #[test]
    fn a_trim_below_the_window_lowers_its_count_and_one_at_or_past_it_drops_it() {
        let mut copy = paged();
        copy.apply(&Event::Trimmed {
            through: EntryId(2),
            dropped: 2,
        })
        .unwrap();
        assert_eq!(
            copy.window(),
            Some(Window {
                from: EntryId(5),
                earlier: 2
            })
        );
        assert_eq!(copy.entries().count(), 4, "no held entry was dropped");
        copy.apply(&Event::Trimmed {
            through: EntryId(4),
            dropped: 2,
        })
        .unwrap();
        assert_eq!(copy.window(), None, "nothing earlier is left");
        assert_eq!(copy.dropped(), 4);

        let mut copy = paged();
        copy.apply(&Event::Trimmed {
            through: EntryId(6),
            dropped: 6,
        })
        .unwrap();
        assert_eq!(copy.window(), None);
        assert_eq!(
            copy.entries().map(|(id, _)| id).collect::<Vec<_>>(),
            [EntryId(7), EntryId(8)]
        );
    }

    #[test]
    fn a_log_that_fits_pages_as_its_replay() {
        let server = long(8, 10);
        assert_eq!(server.page(PAGE_BYTES), server.replay());
        assert_eq!(server.page(server.encoded_len()), server.replay());
    }

    #[test]
    fn a_long_log_pages_its_newest_whole_entries_within_the_budget_then_its_state() {
        let mut server = long(10, 100);
        let held = Slot::Held {
            text: "again".into(),
            reason: HoldReason::Failed,
        };
        let titled = Conversation {
            title: Some("Hello.txt file creation".into()),
            ..conversation("s1", Continuation::Resume)
        };
        for event in [
            Event::Trimmed {
                through: EntryId(2),
                dropped: 2,
            },
            Event::Queue { slot: held.clone() },
            Event::Conversation {
                conversation: titled.clone(),
            },
            Event::Turn {
                state: TurnState::Working,
            },
        ] {
            server.apply(&event).unwrap();
        }
        let size = server.entries.back().unwrap().size;
        let page = server.page(size * 3);
        assert_eq!(
            page,
            [
                Event::Trimmed {
                    through: EntryId(2),
                    dropped: 2
                },
                window(8, 5),
                added(8, agent(&"x".repeat(100))),
                added(9, agent(&"x".repeat(100))),
                added(10, agent(&"x".repeat(100))),
                Event::Queue { slot: held },
                Event::Conversation {
                    conversation: titled
                },
                Event::Turn {
                    state: TurnState::Working
                },
                Event::Agent { state: ready() },
            ]
        );
        assert_eq!(fold(&page), Ok(server.suffix(EntryId(8))));
        // A budget one byte short takes one entry fewer.
        assert_eq!(fold(&server.page(size * 3 - 1)).unwrap().entries().count(), 2);
    }

    #[test]
    fn a_budget_of_nothing_gives_the_newest_entry_and_a_huge_one_goes_alone() {
        let server = long(4, 10);
        let page = fold(&server.page(0)).unwrap();
        assert_eq!(
            page.entries().map(|(id, _)| id).collect::<Vec<_>>(),
            [EntryId(4)]
        );
        let mut server = long(4, 10);
        server.apply(&added(5, agent(&"€".repeat(350_000)))).unwrap();
        let page = server.page(PAGE_BYTES);
        let copy = fold(&page).unwrap();
        assert_eq!(
            copy.entries().map(|(id, _)| id).collect::<Vec<_>>(),
            [EntryId(5)]
        );
        assert_eq!(copy.window().map(|window| window.earlier), Some(4));
        assert!(page.iter().filter(|event| matches!(event, Event::Appended { .. })).count() > 10);
    }

    #[test]
    fn a_pending_permission_far_back_pulls_the_page_down_to_it() {
        let mut server = long(2, 10);
        server.apply(&added(3, permission())).unwrap();
        for id in 4..40 {
            server.apply(&added(id, agent(&"y".repeat(64 * 1024)))).unwrap();
        }
        let copy = fold(&server.page(PAGE_BYTES)).unwrap();
        assert_eq!(
            copy.window(),
            Some(Window {
                from: EntryId(3),
                earlier: 2
            })
        );
        assert_eq!(copy, server.suffix(EntryId(3)));
    }

    #[test]
    fn a_page_carries_a_login_demand_with_its_methods() {
        let mut server = long(20, 100);
        let demand = AgentState::LoginRequired {
            message: "log in first".into(),
            methods: vec![agent_method("oauth", "Log in"), terminal_method()],
        };
        server
            .apply(&Event::Agent {
                state: demand.clone(),
            })
            .unwrap();
        let page = server.page(300);
        assert_eq!(page.last(), Some(&Event::Agent { state: demand }));
        assert!(matches!(page[0], Event::Window { .. }));
    }

    #[test]
    fn a_fetch_takes_whole_entries_below_before_as_they_stand_now() {
        let mut server = long(3, 10);
        server.apply(&added(4, tool("call_1"))).unwrap();
        server.apply(&added(5, permission())).unwrap();
        for id in 6..=8 {
            server.apply(&added(id, agent("z"))).unwrap();
        }
        server
            .apply(&changed(4, None, None, Some(ToolStatus::Completed)))
            .unwrap();
        server
            .apply(&Event::Answered {
                id: EntryId(5),
                outcome: PermissionOutcome::Cancelled,
            })
            .unwrap();
        let fetched = server.fetch(EntryId(6), PAGE_BYTES);
        assert!(!fetched.more);
        let mut expected = Vec::new();
        for (id, entry) in server.entries().take(5) {
            replay_entry(id, entry, &mut expected);
        }
        assert_eq!(fetched.events, expected);
        assert!(
            fetched.events.iter().all(|event| entry_change(event).is_some()
                || matches!(event, Event::Added { .. })),
            "entries only"
        );
        // A budget that takes two: more remain.
        let size = server.entries.get(4).unwrap().size + server.entries.get(3).unwrap().size;
        let fetched = server.fetch(EntryId(6), size);
        assert!(fetched.more);
        assert_eq!(fold_page(&fetched.events).first(), Some(&EntryId(4)));
        // At or below the first kept entry: nothing.
        assert_eq!(
            server.fetch(EntryId(1), PAGE_BYTES),
            Fetched {
                events: Vec::new(),
                more: false
            }
        );
        assert_eq!(server.fetch(EntryId(2), 0).events, [added(1, agent(&"x".repeat(10)))]);
    }

    /// The ids of a fetched page's `Added`s.
    fn fold_page(events: &[Event]) -> Vec<EntryId> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Added { id, .. } => Some(*id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_page_folded_in_front_lowers_the_count_and_the_last_drops_the_window() {
        let server = long(8, 10);
        let mut copy = paged();
        let first = server.fetch(EntryId(5), server.entries[0].size * 2);
        assert!(first.more);
        copy.prepend(&first.events, first.more).unwrap();
        assert_eq!(
            copy.window(),
            Some(Window {
                from: EntryId(3),
                earlier: 2
            })
        );
        assert_eq!(copy, server.suffix(EntryId(3)));
        let rest = server.fetch(EntryId(3), PAGE_BYTES);
        assert!(!rest.more);
        copy.prepend(&rest.events, rest.more).unwrap();
        assert_eq!(copy, server);
    }

    #[test]
    fn an_empty_page_without_more_on_a_copy_with_no_window_changes_nothing() {
        let mut copy = long(3, 10);
        let before = copy.clone();
        assert_eq!(copy.prepend(&[], false), Ok(()));
        assert_eq!(copy, before);
    }

    #[test]
    fn a_page_that_doesnt_fit_the_window_is_refused_and_changes_nothing() {
        let server = long(8, 10);
        let gap = server.fetch(EntryId(4), PAGE_BYTES);
        let whole = server.fetch(EntryId(5), PAGE_BYTES);
        let part = server.fetch(EntryId(5), 0);
        let cases: [(Transcript, &[Event], bool, FoldError); 5] = [
            (
                paged(),
                &gap.events,
                false,
                FoldError::PageGap {
                    last: EntryId(3),
                    from: EntryId(5),
                },
            ),
            (long(3, 10), &whole.events, false, FoldError::NoWindow),
            (
                paged(),
                &part.events,
                false,
                FoldError::WindowMismatch {
                    earlier: 4,
                    fetched: 1,
                    more: false,
                },
            ),
            (
                paged(),
                &[],
                true,
                FoldError::WindowMismatch {
                    earlier: 4,
                    fetched: 0,
                    more: true,
                },
            ),
            (
                paged(),
                &whole.events,
                true,
                FoldError::WindowMismatch {
                    earlier: 4,
                    fetched: 4,
                    more: true,
                },
            ),
        ];
        for (mut copy, events, more, error) in cases {
            let before = copy.clone();
            assert_eq!(copy.prepend(events, more), Err(error.clone()));
            assert_eq!(copy, before, "{error}");
            assert!(!error.to_string().is_empty());
        }
        // An empty page without more while earlier entries remain.
        let mut copy = paged();
        assert!(matches!(
            copy.prepend(&[], false),
            Err(FoldError::WindowMismatch { fetched: 0, .. })
        ));
    }

    #[test]
    fn a_paged_copy_replays_with_its_window() {
        let copy = paged();
        assert_eq!(fold(&copy.replay()), Ok(copy));
    }

    #[test]
    fn the_new_fold_errors_read_as_what_went_wrong() {
        for (error, text) in [
            (
                FoldError::WindowAfterEntries,
                "a page's window came after entries were already held",
            ),
            (
                FoldError::WindowBehind {
                    from: EntryId(3),
                    next: EntryId(5),
                },
                "a page's window starts at entry 3, but the next entry is 5",
            ),
            (
                FoldError::PageGap {
                    last: EntryId(3),
                    from: EntryId(5),
                },
                "a fetched page ends at entry 3, but the held entries start at 5",
            ),
            (
                FoldError::WindowMismatch {
                    earlier: 4,
                    fetched: 1,
                    more: false,
                },
                "a fetched page of 1 entries with nothing more to fetch doesn't fit the 4 earlier entries",
            ),
            (
                FoldError::WindowEmpty { from: EntryId(3) },
                "a page's window at entry 3 has no earlier entries",
            ),
            (
                FoldError::NoWindow,
                "a fetched page came for a transcript that holds every entry",
            ),
        ] {
            assert_eq!(error.to_string(), text);
        }
    }

    // Tool content (OpenSpec change `agents-tool-content`).

    fn text_item(text: &str) -> ToolItem {
        ToolItem::Text {
            text: text.into(),
            cut: false,
        }
    }

    fn output_item(command: Option<&str>) -> ToolItem {
        ToolItem::Output(Output {
            command: command.map(Into::into),
            ..Output::default()
        })
    }

    fn diff_item() -> ToolItem {
        ToolItem::Diff(FileChange {
            op: FileOp::Modify,
            path: "/p/main.rs".into(),
            hunks: vec![Hunk {
                old_start: 10,
                new_start: 10,
                lines: vec![
                    DiffLine::Context("fn main() {".into()),
                    DiffLine::Removed("    old();".into()),
                    DiffLine::Added("    new();".into()),
                ],
            }],
            omitted: 0,
        })
    }

    /// `tool(call)` with this content and raw input.
    fn tool_with(call: &str, items: Vec<ToolItem>, raw_input: Option<&str>) -> Entry {
        let mut entry = tool(call);
        if let Entry::Tool { content, input, .. } = &mut entry {
            *content = items;
            *input = raw_input.map(Into::into);
        }
        entry
    }

    fn items(id: u64, from: u32, items: Vec<ToolItem>) -> Event {
        Event::ToolItems {
            id: EntryId(id),
            from,
            items,
        }
    }

    fn output(id: u64, item: u32, text: &str, exit: Option<Exit>) -> Event {
        Event::ToolOutput {
            id: EntryId(id),
            item,
            text: text.into(),
            exit,
        }
    }

    fn content_of(transcript: &Transcript, id: u64) -> Vec<ToolItem> {
        match transcript.entry(EntryId(id)) {
            Some(Entry::Tool { content, .. }) => content.clone(),
            other => panic!("entry {id} is {other:?}"),
        }
    }

    fn measured(transcript: &Transcript) -> usize {
        transcript
            .entries()
            .map(|(_, entry)| serde_json::to_vec(entry).unwrap().len())
            .sum()
    }

    #[test]
    fn each_tool_content_form_round_trips() {
        let entry = Entry::Tool {
            call: "c1".into(),
            title: "Edit main.rs".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Completed,
            content: vec![
                text_item("done"),
                ToolItem::Text {
                    text: "cut".into(),
                    cut: true,
                },
                diff_item(),
                ToolItem::Diff(FileChange {
                    op: FileOp::Move { from: "/p/a".into() },
                    path: "/p/b".into(),
                    hunks: Vec::new(),
                    omitted: 3,
                }),
                ToolItem::Diff(FileChange {
                    op: FileOp::Other { name: "chmod".into() },
                    path: "/p/c".into(),
                    hunks: Vec::new(),
                    omitted: 0,
                }),
                ToolItem::Output(Output {
                    command: Some("cargo test".into()),
                    cwd: Some("/p".into()),
                    text: "ok\n".into(),
                    omitted: 12,
                    partial: true,
                    exit: Some(Exit::Code { code: 0 }),
                }),
                ToolItem::Output(Output {
                    exit: Some(Exit::Signal { name: "KILL".into() }),
                    ..Output::default()
                }),
            ],
            more: 2,
            locations: vec![Location {
                path: "/p/main.rs".into(),
                line: Some(10),
            }],
            input: Some("{\"path\":\"/p\"}".into()),
            result: Some("ok".into()),
        };
        let events = [
            added(1, entry),
            items(1, 0, vec![diff_item()]),
            output(1, 2, "more\n", Some(Exit::Code { code: 1 })),
            Event::ToolChanged {
                id: EntryId(1),
                title: None,
                kind: None,
                status: None,
                locations: Some(Vec::new()),
                input: Some(None),
                result: Some(Some("r".into())),
                more: Some(0),
            },
        ];
        for event in events {
            let value = event.to_value();
            assert_eq!(Event::from_value(&value), Ok(event.clone()), "{value}");
        }
    }

    #[test]
    fn a_diff_line_is_one_string_with_its_sign_first() {
        let value = serde_json::to_value(diff_item()).unwrap();
        assert_eq!(
            value["lines"],
            serde_json::Value::Null,
            "the lines are in the hunks"
        );
        assert_eq!(
            value["hunks"][0]["lines"],
            serde_json::json!([" fn main() {", "-    old();", "+    new();"])
        );
        assert_eq!(value["item"], "diff");
        assert_eq!(value["op"], "modify");
        for bad in ["x", "", "*a"] {
            let event = serde_json::json!({
                "event": "tool_items", "id": 1, "from": 0,
                "items": [{"item": "diff", "op": "add", "path": "/p",
                           "hunks": [{"old_start": 1, "new_start": 1, "lines": [bad]}]}],
            });
            let error = Event::from_value(&event).unwrap_err();
            assert!(error.to_string().contains("diff line"), "{bad:?}: {error}");
        }
        // A sign alone is an empty line.
        let line: DiffLine = serde_json::from_value(serde_json::json!("+")).unwrap();
        assert_eq!(line, DiffLine::Added(String::new()));
    }

    #[test]
    fn a_tool_call_without_content_encodes_as_before_content_existed() {
        let value = added(4, tool("call_1")).to_value();
        assert_eq!(
            value,
            serde_json::json!({"event": "added", "id": 4, "entry": {
                "type": "tool", "call": "call_1", "title": "Read notes.md",
                "kind": "read", "status": "pending"}})
        );
        assert_eq!(
            changed(4, None, None, Some(ToolStatus::Completed)).to_value(),
            serde_json::json!({"event": "tool_changed", "id": 4, "status": "completed"})
        );
    }

    #[test]
    fn decoding_refuses_bad_tool_content_and_ignores_unknown_fields_in_it() {
        for value in [
            serde_json::json!({"event": "tool_items", "id": 1, "from": 0,
                               "items": [{"item": "picture"}]}),
            serde_json::json!({"event": "tool_items", "id": 1, "from": -1, "items": []}),
            serde_json::json!({"event": "tool_items", "id": 1, "from": 0,
                               "items": [{"item": "diff", "op": "teleport", "path": "/p"}]}),
            serde_json::json!({"event": "tool_items", "id": 1, "from": 0,
                               "items": [{"item": "diff", "op": "move", "path": "/p"}]}),
            serde_json::json!({"event": "tool_output", "id": 1, "item": 0}),
            serde_json::json!({"event": "tool_output", "id": 1, "item": 0, "text": "",
                               "exit": {"exit": "eaten"}}),
            serde_json::json!({"event": "tool_changed", "id": 1, "locations": [{"line": 3}]}),
        ] {
            let error = Event::from_value(&value).unwrap_err();
            assert!(!error.to_string().is_empty(), "{value}");
        }
        assert_eq!(
            Event::from_value(&serde_json::json!({
                "event": "tool_output", "id": 1, "item": 0, "text": "a", "later": 1,
                "exit": {"exit": "code", "code": 2, "later": true},
            })),
            Ok(output(1, 0, "a", Some(Exit::Code { code: 2 })))
        );
    }

    #[test]
    fn tool_items_replace_from_their_index_and_refuse_past_the_end() {
        let mut transcript = fold(&[added(1, tool("c1")), added(2, agent("a"))]).unwrap();
        transcript
            .apply(&items(1, 0, vec![text_item("a"), text_item("b")]))
            .unwrap();
        assert_eq!(content_of(&transcript, 1), [text_item("a"), text_item("b")]);
        // Appended at the length.
        transcript.apply(&items(1, 2, vec![diff_item()])).unwrap();
        assert_eq!(content_of(&transcript, 1).len(), 3);
        // Replaced from 1.
        transcript.apply(&items(1, 1, vec![text_item("c")])).unwrap();
        assert_eq!(content_of(&transcript, 1), [text_item("a"), text_item("c")]);
        assert_eq!(transcript.encoded_len(), measured(&transcript));
        // Cleared.
        transcript.apply(&items(1, 0, Vec::new())).unwrap();
        assert_eq!(content_of(&transcript, 1), []);
        assert_eq!(transcript.encoded_len(), measured(&transcript));

        transcript.apply(&items(1, 0, vec![output_item(None)])).unwrap();
        let base = transcript.clone();
        for (event, error) in [
            (
                items(1, 2, vec![text_item("x")]),
                FoldError::ItemsOutOfRange {
                    id: EntryId(1),
                    from: 2,
                    len: 1,
                },
            ),
            (
                items(2, 0, Vec::new()),
                FoldError::NotATool { id: EntryId(2) },
            ),
            (
                output(2, 0, "x", None),
                FoldError::NotATool { id: EntryId(2) },
            ),
            (
                output(1, 1, "x", None),
                FoldError::NotAnOutput {
                    id: EntryId(1),
                    item: 1,
                },
            ),
            (items(9, 0, Vec::new()), FoldError::UnknownEntry { id: EntryId(9) }),
        ] {
            let mut copy = base.clone();
            assert_eq!(copy.apply(&event), Err(error.clone()), "{event:?}");
            assert_eq!(copy, base, "{event:?} changed the log");
            assert!(!error.to_string().is_empty());
        }
        let mut copy = fold(&[added(1, tool("c1"))]).unwrap();
        copy.apply(&items(1, 0, vec![text_item("t")])).unwrap();
        let base = copy.clone();
        assert_eq!(
            copy.apply(&output(1, 0, "x", None)),
            Err(FoldError::NotAnOutput {
                id: EntryId(1),
                item: 0
            })
        );
        assert_eq!(copy, base);
    }

    #[test]
    fn tool_changed_sets_clears_and_keeps_the_new_fields() {
        let mut transcript = fold(&[added(1, tool("c1"))]).unwrap();
        let location = Location {
            path: "/p".into(),
            line: None,
        };
        transcript
            .apply(&Event::ToolChanged {
                id: EntryId(1),
                title: None,
                kind: None,
                status: None,
                locations: Some(vec![location.clone()]),
                input: Some(Some("in".into())),
                result: Some(Some("out".into())),
                more: Some(4),
            })
            .unwrap();
        transcript
            .apply(&Event::ToolChanged {
                id: EntryId(1),
                title: None,
                kind: None,
                status: None,
                locations: None,
                input: Some(None),
                result: None,
                more: None,
            })
            .unwrap();
        let Some(Entry::Tool {
            locations,
            input,
            result,
            more,
            ..
        }) = transcript.entry(EntryId(1))
        else {
            panic!()
        };
        assert_eq!(
            (locations, input, result, more),
            (&vec![location], &None, &Some("out".into()), &4)
        );
        assert_eq!(transcript.encoded_len(), measured(&transcript));
    }

    #[test]
    fn output_appends_and_keeps_whole_lines_within_the_limit() {
        let mut transcript = fold(&[
            added(1, tool("c1")),
            items(1, 0, vec![text_item("t"), output_item(Some("seq"))]),
        ])
        .unwrap();
        transcript.apply(&output(1, 1, "1\n2\n", None)).unwrap();
        transcript
            .apply(&output(1, 1, "3\n", Some(Exit::Code { code: 0 })))
            .unwrap();
        let ToolItem::Output(kept) = &content_of(&transcript, 1)[1] else {
            panic!()
        };
        assert_eq!(
            (kept.text.as_str(), kept.omitted, kept.partial, &kept.exit),
            ("1\n2\n3\n", 0, false, &Some(Exit::Code { code: 0 }))
        );
        assert_eq!(transcript.encoded_len(), measured(&transcript));

        // Past the limit: whole lines go from the start.
        let line = format!("{}\n", "x".repeat(99));
        let mut transcript = fold(&[added(1, tool("c1")), items(1, 0, vec![output_item(None)])])
            .unwrap();
        let lines = ITEM_LIMIT / 100 + 10;
        transcript
            .apply(&output(1, 0, &line.repeat(lines), None))
            .unwrap();
        let ToolItem::Output(kept) = &content_of(&transcript, 1)[0] else {
            panic!()
        };
        assert_eq!(kept.text, line.repeat(ITEM_LIMIT / 100));
        assert_eq!(kept.omitted, (line.len() * (lines - ITEM_LIMIT / 100)) as u64);
        assert!(!kept.partial);
        assert_eq!(transcript.encoded_len(), measured(&transcript));

        // A single line past the limit keeps its last bytes, at a
        // character boundary.
        let mut transcript = fold(&[added(1, tool("c1")), items(1, 0, vec![output_item(None)])])
            .unwrap();
        let long = format!("a{}", "é".repeat(ITEM_LIMIT));
        transcript.apply(&output(1, 0, &long, None)).unwrap();
        let ToolItem::Output(kept) = &content_of(&transcript, 1)[0] else {
            panic!()
        };
        assert_eq!(kept.text, "é".repeat(ITEM_LIMIT / 2));
        assert_eq!(kept.omitted as usize, long.len() - ITEM_LIMIT);
        assert!(kept.partial);
        assert_eq!(transcript.encoded_len(), measured(&transcript));
        // Its line ends, and the next one drops what was left of it.
        transcript.apply(&output(1, 0, "\nnext", None)).unwrap();
        let ToolItem::Output(kept) = &content_of(&transcript, 1)[0] else {
            panic!()
        };
        assert_eq!(kept.text, "next");
        assert!(!kept.partial);
        assert_eq!(transcript.encoded_len(), measured(&transcript));
    }

    /// Folding `ToolOutput(a)` then `ToolOutput(b)` equals folding
    /// `ToolOutput(a + b)` (design decision 8), which the checkpoint's
    /// merge relies on: random texts, split at random points.
    #[test]
    fn output_appended_in_pieces_folds_as_appended_whole() {
        const PIECES: &[&str] = &["a", "bc", "\n", "é", "🦀", "\"", "\\", "\t", "line\n"];
        let mut rng = XorShift::new(0x7e57_00a1);
        for round in 0..1000 {
            // Some texts reach past the limit, with long lines or none.
            let target = if round % 3 == 0 {
                ITEM_LIMIT + rng.below(2 * ITEM_LIMIT)
            } else {
                rng.below(300)
            };
            let newlines = rng.below(4) > 0;
            let mut text = String::new();
            while text.len() < target {
                let piece = PIECES[rng.below(PIECES.len())];
                if piece.contains('\n') && !newlines {
                    continue;
                }
                text.push_str(&piece.repeat(1 + rng.below(if target > 300 { 200 } else { 2 })));
                // Long runs without a line break.
                if rng.below(50) == 0 {
                    text.push_str(&"z".repeat(rng.below(ITEM_LIMIT + 64)));
                }
            }
            let start = [added(1, tool("c1")), items(1, 0, vec![output_item(None)])];
            let whole = {
                let mut transcript = fold(&start).unwrap();
                transcript.apply(&output(1, 0, &text, None)).unwrap();
                transcript
            };
            let mut pieces = fold(&start).unwrap();
            let mut rest = text.as_str();
            while !rest.is_empty() {
                let mut cut = 1 + rng.below(rest.len());
                while !rest.is_char_boundary(cut) {
                    cut += 1;
                }
                let (piece, after) = rest.split_at(cut);
                pieces.apply(&output(1, 0, piece, None)).unwrap();
                rest = after;
            }
            assert_eq!(pieces, whole, "round {round}");
            assert_eq!(whole.encoded_len(), measured(&whole));
        }
    }

    #[test]
    fn a_tool_entry_replays_without_content_then_one_item_per_event() {
        let entry = tool_with(
            "c1",
            vec![text_item("a"), diff_item(), output_item(Some("ls"))],
            Some("in"),
        );
        let transcript = fold(&[added(1, entry)]).unwrap();
        let replay = transcript.replay();
        assert_eq!(
            replay[..4],
            [
                added(1, tool_with("c1", Vec::new(), Some("in"))),
                items(1, 0, vec![text_item("a")]),
                items(1, 1, vec![diff_item()]),
                items(1, 2, vec![output_item(Some("ls"))]),
            ]
        );
        assert_eq!(fold(&replay), Ok(transcript));
    }

    #[test]
    fn the_log_trims_large_tool_entries_whole() {
        let big = ToolItem::Text {
            text: "t".repeat(ITEM_LIMIT),
            cut: true,
        };
        let mut events = Vec::new();
        for id in 1..=40u64 {
            events.push(added(id, tool("c1")));
            events.push(items(id, 0, vec![big.clone(), big.clone(), big.clone()]));
        }
        let mut transcript = fold(&events).unwrap();
        let entry = transcript.encoded_len() / 40;
        assert!(entry > 3 * ITEM_LIMIT);
        let trimmed = transcript.trim_to(12 * entry).unwrap();
        assert_eq!(
            trimmed,
            Event::Trimmed {
                through: EntryId(28),
                dropped: 28
            }
        );
        assert_eq!(transcript.entries().count(), 12);
        assert_eq!(transcript.encoded_len(), measured(&transcript));
        assert_eq!(fold(&transcript.replay()), Ok(transcript));
    }

    #[test]
    fn a_paged_copy_ignores_tool_content_below_its_window() {
        let mut server = fold(&[added(1, tool("c1")), items(1, 0, vec![output_item(None)])])
            .unwrap();
        for id in 2..=8 {
            server.apply(&added(id, agent(&"x".repeat(10)))).unwrap();
        }
        let mut copy = fold(&server.replay_from(4)).unwrap();
        let before = copy.clone();
        for event in [items(1, 0, vec![text_item("t")]), output(1, 0, "x", None)] {
            assert_eq!(copy.apply(&event), Ok(()), "{event:?}");
            assert_eq!(copy, before, "{event:?} changed nothing");
        }
    }

    /// A log of a tool call `call`, entries of `filler` bytes, then a
    /// permission for `call`, answered or not.
    fn permission_after(call: &str, filler: u64, outcome: Option<PermissionOutcome>) -> Transcript {
        let mut events = vec![
            added(1, agent("before")),
            added(2, tool_with(call, vec![diff_item()], None)),
        ];
        let chunk = 64 * 1024;
        let count = filler / chunk as u64;
        for id in 3..3 + count {
            events.push(added(id, agent(&"y".repeat(chunk))));
        }
        // `permission()` names call_1.
        let asked = 3 + count;
        events.push(added(asked, permission()));
        if let Some(outcome) = outcome {
            events.push(Event::Answered {
                id: EntryId(asked),
                outcome,
            });
        }
        events.push(added(asked + 1, agent("after")));
        fold(&events).unwrap()
    }

    #[test]
    fn a_waiting_permissions_tool_call_far_back_starts_the_page() {
        let server = permission_after("call_1", 2 * 1024 * 1024, None);
        let copy = fold(&server.page(PAGE_BYTES)).unwrap();
        assert_eq!(copy.window().map(|window| window.from), Some(EntryId(2)));
        assert_eq!(copy, server.suffix(EntryId(2)));
        assert_eq!(content_of(&copy, 2), [diff_item()]);
    }

    #[test]
    fn an_answered_permissions_tool_call_doesnt_widen_the_page() {
        let server = permission_after(
            "call_1",
            2 * 1024 * 1024,
            Some(PermissionOutcome::Cancelled),
        );
        let plain = fold(&server.page(PAGE_BYTES)).unwrap();
        assert!(plain.window().unwrap().from > EntryId(30));
    }

    #[test]
    fn a_permission_naming_a_call_the_log_doesnt_hold_pages_as_before() {
        let server = permission_after("call_9", 2 * 1024 * 1024, None);
        let copy = fold(&server.page(PAGE_BYTES)).unwrap();
        // The budget's newest entries, the permission among them, as before:
        // not the tool call of another id.
        let permission = server
            .entries()
            .find(|(_, entry)| matches!(entry, Entry::Permission { .. }))
            .map(|(id, _)| id)
            .unwrap();
        let from = copy.window().unwrap().from;
        assert!(from > EntryId(30) && from <= permission, "{from}");
    }
}
