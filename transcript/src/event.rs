use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Output;

/// The most text one event carries in a field that grows ([`Event::Added`]'s
/// text and [`Event::Appended`]): longer text is split, at a character
/// boundary, into more `Appended`s.
pub const TEXT_CHUNK: usize = 64 * 1024;

/// The most one event of protocol 4 encodes to ([`Event::encoded_len`];
/// OpenSpec change `agents-event-bounds`, design decision 1): a frame's
/// megabyte less room for any frame's envelope. It is the event's own
/// contract, so it lives here; the producer (the core's reducer) keeps every
/// event within it, and `brindle-remote-agent`'s `acp::limits` derives
/// that from the bounds and asserts it fits a frame with its envelope.
pub const MAX_EVENT: usize = 1024 * 1024 - 1024;

/// The most a path keeps, in bytes, cut at a character boundary: a
/// location's, a file change's and its source's, and an output's
/// directory.
pub const MAX_PATH: usize = 4 * 1024;

/// What one kept diff line adds to its text on the wire: its sign, two
/// quotes and a comma ([`ToolItem::wire_len`]).
pub const LINE_FRAMING: usize = 4;

/// What one hunk adds on the wire beside its lines:
/// `{"old_start":4294967295,"new_start":4294967295,"lines":[]},` is 59.
pub const HUNK_FRAMING: usize = 64;

/// The most an output's exit signal name keeps, and what an output's
/// measure keeps for its exit.
pub const SIGNAL_RESERVE: usize = 64;

/// What the log's measure counts for each entry beside its own encoding
/// ([`Transcript::encoded_len`]): the most a replay and a checkpoint's
/// tools map add around a small entry, so a stored log is at most twice
/// its measure (design decision 2).
pub const ENTRY_FRAMING: usize = 64;

/// An entry's id: per session, from 1, never reused (`types R3`). Never an
/// index: entries before the first kept one may have been trimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryId(pub u64);

impl EntryId {
    /// The first id of a session.
    pub const FIRST: Self = Self(1);

    pub(crate) fn next(self) -> Self {
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
    pub(crate) fn text_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::User { text } | Self::Agent { text } | Self::Thought { text } => Some(text),
            Self::Tool { .. } | Self::Permission { .. } | Self::Notice { .. } => None,
        }
    }

    /// The encoded length of the entry with its [`ENTRY_FRAMING`], as the
    /// log's bound counts it.
    pub(crate) fn measure(&self) -> usize {
        counted(self).saturating_add(ENTRY_FRAMING)
    }

    /// The entry's kind, as its encoding's `type` names it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::User { .. } => "user",
            Self::Agent { .. } => "agent",
            Self::Thought { .. } => "thought",
            Self::Tool { .. } => "tool",
            Self::Permission { .. } => "permission",
            Self::Notice { .. } => "notice",
        }
    }

    /// A tool entry's content, locations and raw values held to the tool
    /// content bounds (design decision 6): each item by [`ToolItem::bound`],
    /// the list by [`ContentBudget`] with what it drops added to `more`,
    /// at most [`MAX_LOCATIONS`] locations with paths of at most
    /// [`MAX_PATH`], and raw values of at most [`RAW_LIMIT`] ending with
    /// `…` when cut. The identity within the bounds, and idempotent; any
    /// other entry is left as it is.
    pub(crate) fn bound_tool(&mut self) {
        let Self::Tool {
            content,
            more,
            locations,
            input,
            result,
            ..
        } = self
        else {
            return;
        };
        let dropped = bound_content(content);
        *more = more.saturating_add(dropped);
        bound_locations(locations);
        bound_raw(input);
        bound_raw(result);
    }
}

/// `content` with each item bounded and the list kept within a
/// [`ContentBudget`]; returns how many items it dropped.
fn bound_content(content: &mut Vec<ToolItem>) -> u32 {
    let mut budget = ContentBudget::new();
    let mut kept = Vec::with_capacity(content.len().min(MAX_ITEMS));
    for mut item in content.drain(..) {
        item.bound();
        if budget.take(&item) {
            kept.push(item);
        }
    }
    *content = kept;
    budget.left_out()
}

fn bound_locations(locations: &mut Vec<Location>) {
    locations.truncate(MAX_LOCATIONS);
    for location in locations {
        cut_in_place(&mut location.path, MAX_PATH);
    }
}

/// A raw value past [`RAW_LIMIT`] cut to end with `…` within it, as the
/// reducer keeps one.
fn bound_raw(raw: &mut Option<String>) {
    if let Some(text) = raw
        && text.len() > RAW_LIMIT
    {
        cut_in_place(text, RAW_LIMIT - '…'.len_utf8());
        text.push('…');
    }
}

/// `text` cut to at most `limit` bytes at a character boundary; whether
/// it was cut.
fn cut_in_place(text: &mut String, limit: usize) -> bool {
    if text.len() <= limit {
        return false;
    }
    let mut cut = limit;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    true
}

/// How many bytes `value` encodes to, counted as serde_json writes them,
/// with nothing allocated; one that can't encode counts as `usize::MAX`.
fn counted<T: Serialize + ?Sized>(value: &T) -> usize {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => counter.0,
        Err(_) => usize::MAX,
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
/// a [`FileChange`]'s lines with their framing, an [`Output`]'s text;
/// OpenSpec change `agents-tool-content`, design decision 5). That one
/// event fits a frame however its items are grouped is not this bound's
/// doing but `acp::limits`' invariant I1 (`agents-event-bounds`).
pub const ITEM_LIMIT: usize = TEXT_CHUNK;

/// The most content items a tool call keeps; later ones are counted in its
/// `more`.
pub const MAX_ITEMS: usize = 32;

/// The most a tool call's items hold together, by [`ToolItem::wire_len`];
/// later items are counted in its `more` ([`ContentBudget`]).
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
    /// The item's text with its framing, as [`CALL_LIMIT`] counts it
    /// (`agents-event-bounds` decision 3): a text's bytes; a diff's paths,
    /// and each kept line's bytes with [`LINE_FRAMING`] and each hunk's
    /// [`HUNK_FRAMING`]; an output's [`ITEM_LIMIT`] (what its text may
    /// grow to, whatever it holds yet), its command and directory and
    /// [`SIGNAL_RESERVE`]. No `ToolOutput` changes it.
    pub fn wire_len(&self) -> usize {
        match self {
            Self::Text { text, .. } => text.len(),
            Self::Diff(change) => change.wire_len(),
            Self::Output(output) => ITEM_LIMIT
                .saturating_add(output.command.as_ref().map_or(0, String::len))
                .saturating_add(output.cwd.as_ref().map_or(0, String::len))
                .saturating_add(SIGNAL_RESERVE),
        }
    }

    /// The item held to its own bounds (design decision 6): a text cut at
    /// [`ITEM_LIMIT`] and marked `cut`; a diff's paths cut at [`MAX_PATH`]
    /// and its lines past [`ITEM_LIMIT`] (counted with their framing, as
    /// the reducer keeps them) dropped into `omitted`, a hunk left empty
    /// removed; an output's text kept to its suffix by its own rule, its
    /// directory cut at [`MAX_PATH`], its command at [`TEXT_CHUNK`] and its
    /// signal name at [`SIGNAL_RESERVE`]. The identity within the bounds,
    /// and idempotent.
    pub fn bound(&mut self) {
        match self {
            Self::Text { text, cut } => {
                if cut_in_place(text, ITEM_LIMIT) {
                    *cut = true;
                }
            }
            Self::Diff(change) => change.bound(),
            Self::Output(output) => {
                output.append("");
                if let Some(command) = &mut output.command {
                    cut_in_place(command, TEXT_CHUNK);
                }
                if let Some(cwd) = &mut output.cwd {
                    cut_in_place(cwd, MAX_PATH);
                }
                if let Some(exit) = &mut output.exit {
                    exit.bound();
                }
            }
        }
    }
}

/// The rule a tool call's items are kept by (`agents-event-bounds`
/// decision 3), which the reducer applies before it sends them and the fold
/// applies to what it is sent: at most [`MAX_ITEMS`] items whose
/// [`ToolItem::wire_len`]s add up to at most [`CALL_LIMIT`]. An item that
/// doesn't fit is left out and counted, and later ones are still tried.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentBudget {
    items: usize,
    len: usize,
    left_out: u32,
}

impl ContentBudget {
    pub fn new() -> Self {
        Self::default()
    }

    /// A budget that already holds `items`, each taken as it stands.
    pub fn holding<'a>(items: impl IntoIterator<Item = &'a ToolItem>) -> Self {
        let mut budget = Self::new();
        for item in items {
            budget.items += 1;
            budget.len = budget.len.saturating_add(item.wire_len());
        }
        budget
    }

    /// Whether `item` fits beside what is taken.
    pub fn fits(&self, item: &ToolItem) -> bool {
        self.items < MAX_ITEMS && self.len.saturating_add(item.wire_len()) <= CALL_LIMIT
    }

    /// Takes `item` when it fits, else counts it left out; whether it fit.
    pub fn take(&mut self, item: &ToolItem) -> bool {
        if self.fits(item) {
            self.items += 1;
            self.len = self.len.saturating_add(item.wire_len());
            true
        } else {
            self.left_out = self.left_out.saturating_add(1);
            false
        }
    }

    /// Counts one item left out without measuring it.
    pub fn leave_out(&mut self) {
        self.left_out = self.left_out.saturating_add(1);
    }

    /// How many items were left out.
    pub fn left_out(&self) -> u32 {
        self.left_out
    }
}

/// `items` in the pieces one `ToolItems` event each carries (design
/// decision 2): consecutive items whose [`ToolItem::wire_len`]s add up to
/// at most [`TEXT_CHUNK`], or one larger item alone. The reducer's live
/// split and a replay group by it, so both send the same pieces.
pub fn item_pieces(items: &[ToolItem]) -> Vec<&[ToolItem]> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut len = 0usize;
    for (at, item) in items.iter().enumerate() {
        let item_len = item.wire_len();
        if at > start && len.saturating_add(item_len) > TEXT_CHUNK {
            pieces.extend(items.get(start..at));
            start = at;
            len = 0;
        }
        len = len.saturating_add(item_len);
    }
    if start < items.len() {
        pieces.extend(items.get(start..));
    }
    pieces
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
    /// The change's paths, its op's name, and its hunks' [`Hunk::wire_len`]
    /// together.
    pub fn wire_len(&self) -> usize {
        let op = match &self.op {
            FileOp::Move { from } | FileOp::Copy { from } => from.len(),
            FileOp::Other { name } => name.len(),
            FileOp::Add | FileOp::Delete | FileOp::Modify => 0,
        };
        self.hunks
            .iter()
            .map(Hunk::wire_len)
            .fold(self.path.len().saturating_add(op), usize::saturating_add)
    }

    /// The change held to its bounds (design decision 6): paths and the
    /// op's name cut at [`MAX_PATH`]; hunks kept, as the reducer's diff
    /// keeps them, until one more line (with its [`LINE_FRAMING`], and
    /// [`HUNK_FRAMING`] with a hunk's first) would pass [`ITEM_LIMIT`],
    /// the rest counted in `omitted`; a hunk left without lines removed.
    fn bound(&mut self) {
        cut_in_place(&mut self.path, MAX_PATH);
        match &mut self.op {
            FileOp::Move { from } | FileOp::Copy { from } => {
                cut_in_place(from, MAX_PATH);
            }
            FileOp::Other { name } => {
                cut_in_place(name, MAX_PATH);
            }
            FileOp::Add | FileOp::Delete | FileOp::Modify => {}
        }
        let mut len = 0usize;
        let mut full = false;
        let mut dropped = 0usize;
        self.hunks.retain_mut(|hunk| {
            if full {
                dropped = dropped.saturating_add(hunk.lines.len());
                return false;
            }
            if hunk.lines.is_empty() {
                // An empty hunk the reducer never makes: counted whole.
                if len.saturating_add(HUNK_FRAMING) > ITEM_LIMIT {
                    full = true;
                    return false;
                }
                len += HUNK_FRAMING;
                return true;
            }
            let mut keep = hunk.lines.len();
            for (at, line) in hunk.lines.iter().enumerate() {
                let framing = if at == 0 {
                    LINE_FRAMING + HUNK_FRAMING
                } else {
                    LINE_FRAMING
                };
                let grown = len.saturating_add(line.text().len()).saturating_add(framing);
                if grown > ITEM_LIMIT {
                    full = true;
                    keep = at;
                    break;
                }
                len = grown;
            }
            dropped = dropped.saturating_add(hunk.lines.len() - keep);
            hunk.lines.truncate(keep);
            !hunk.lines.is_empty()
        });
        let dropped = u32::try_from(dropped).unwrap_or(u32::MAX);
        self.omitted = self.omitted.saturating_add(dropped);
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
/// and the new file (1-based), and its lines. One read from the wire starts
/// at 1 or later and numbers no line past `u32::MAX` on either side
/// (`agents-event-bounds` decision 7): any other is a [`DecodeError`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "HunkForm")]
pub struct Hunk {
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    /// Its lines' bytes, each with [`LINE_FRAMING`], and its
    /// [`HUNK_FRAMING`].
    pub fn wire_len(&self) -> usize {
        self.lines
            .iter()
            .map(|line| line.text().len().saturating_add(LINE_FRAMING))
            .fold(HUNK_FRAMING, usize::saturating_add)
    }
}

/// A [`Hunk`] as the wire has it, before its numbers are checked.
#[derive(Deserialize)]
struct HunkForm {
    old_start: u32,
    new_start: u32,
    lines: Vec<DiffLine>,
}

impl TryFrom<HunkForm> for Hunk {
    type Error = String;

    fn try_from(form: HunkForm) -> Result<Self, String> {
        let old = form
            .lines
            .iter()
            .filter(|line| matches!(line, DiffLine::Context(_) | DiffLine::Removed(_)))
            .count();
        let new = form
            .lines
            .iter()
            .filter(|line| matches!(line, DiffLine::Context(_) | DiffLine::Added(_)))
            .count();
        for (side, start, count) in [("old", form.old_start, old), ("new", form.new_start, new)] {
            if start == 0 {
                return Err(format!("a hunk's {side} side starts at line 0; lines count from 1"));
            }
            // The last line this side numbers: `start + count - 1`.
            let last = u64::from(start).saturating_add(count as u64).saturating_sub(1);
            if last > u64::from(u32::MAX) {
                return Err(format!(
                    "a hunk's {side} side starts at line {start} and numbers {count} lines, past line {}",
                    u32::MAX
                ));
            }
        }
        Ok(Self {
            old_start: form.old_start,
            new_start: form.new_start,
            lines: form.lines,
        })
    }
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

/// How a command ended: an exit code, or a signal by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "exit", rename_all = "snake_case")]
pub enum Exit {
    Code { code: i32 },
    Signal { name: String },
}

impl Exit {
    /// A signal's name cut at [`SIGNAL_RESERVE`].
    pub fn bound(&mut self) {
        match self {
            Self::Code { .. } => {}
            Self::Signal { name } => {
                cut_in_place(name, SIGNAL_RESERVE);
            }
        }
    }
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

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

pub(crate) fn is_zero_u64(value: &u64) -> bool {
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

/// The most commands [`Event::Commands`] carries (OpenSpec change
/// `agents-commands`, design decision 2); the reducer keeps the first ones.
pub const MAX_COMMANDS: usize = 256;

/// The most options [`Event::Options`] carries.
pub const MAX_OPTIONS: usize = 32;

/// The most choices one option's [`OptionControl::Select`] holds.
pub const MAX_CHOICES: usize = 256;

/// A slash command the agent offers (ACP's `available_commands_update`):
/// typed as `/NAME` at the start of a prompt, and sent as one. `hint` says
/// what to type after it, when the command takes input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// One of the session's options as the agent offers it (OpenSpec change
/// `agents-commands`, design decision 1): the mode, the model, a thinking
/// level or one of the agent's own. `id` is what a set names; ids are
/// unique within the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOption {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub category: OptionCategory,
    pub kind: OptionControl,
}

/// What an option is about; a category Brindle doesn't know keeps its name,
/// as [`StopReason::Other`] does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "category", rename_all = "snake_case")]
pub enum OptionCategory {
    Mode,
    Model,
    ModelConfig,
    ThoughtLevel,
    Other { name: String },
}

/// How an option is set: one of a list of choices, or on and off. A
/// `Select`'s `current` may name no choice (an agent does that while a list
/// loads); it is kept as sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case")]
pub enum OptionControl {
    Select {
        current: String,
        choices: Vec<OptionChoice>,
    },
    Toggle {
        on: bool,
    },
}

/// One choice of a [`OptionControl::Select`]: `value` is what a set sends,
/// `name` what the user is shown, `group` the name of the group the agent
/// put it in, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionChoice {
    pub value: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
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
    /// The agent's slash commands, the whole list, replacing the last
    /// (OpenSpec change `agents-commands`); an empty list clears them.
    Commands { commands: Vec<SlashCommand> },
    /// The session's options, the whole list, replacing the last; an empty
    /// list clears them.
    Options { options: Vec<SessionOption> },
}

impl Event {
    /// The event as the JSON object client protocol 4 carries.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// How many bytes the event encodes to, as [`MAX_EVENT`] counts it:
    /// serde_json's encoding counted as it is written, with nothing
    /// allocated.
    pub fn encoded_len(&self) -> usize {
        counted(self)
    }

    /// The event's kind, as its encoding's `event` names it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Added { .. } => "added",
            Self::Appended { .. } => "appended",
            Self::ToolChanged { .. } => "tool_changed",
            Self::ToolItems { .. } => "tool_items",
            Self::ToolOutput { .. } => "tool_output",
            Self::Answered { .. } => "answered",
            Self::Queue { .. } => "queue",
            Self::Turn { .. } => "turn",
            Self::Agent { .. } => "agent",
            Self::Conversation { .. } => "conversation",
            Self::Trimmed { .. } => "trimmed",
            Self::Window { .. } => "window",
            Self::Commands { .. } => "commands",
            Self::Options { .. } => "options",
        }
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

/// `text` with every control character but `\n`, `\t` and a `\r` before
/// `\n` (or at the end, where a chunk may have cut a `\r\n`) replaced by
/// U+FFFD (`agents-acp` decision 8). The core stores a prompt cleaned so,
/// and a client compares a prompt it sent with the log by the same rule
/// (OpenSpec change `agents-client-state-fixes`, decision 5).
pub fn clean(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let keep = match c {
            '\n' | '\t' => true,
            '\r' => chars.peek().is_none_or(|next| *next == '\n'),
            other => !other.is_control(),
        };
        cleaned.push(if keep { c } else { '\u{FFFD}' });
    }
    cleaned
}

/// How many bytes [`clean`] would make of `text`, without building it: a
/// control character it replaces takes U+FFFD's three.
pub fn cleaned_len(text: &str) -> usize {
    let mut len = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let keep = match c {
            '\n' | '\t' => true,
            '\r' => chars.peek().is_none_or(|next| *next == '\n'),
            other => !other.is_control(),
        };
        len += if keep { c.len_utf8() } else { '\u{FFFD}'.len_utf8() };
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::XorShift;
    use crate::harness::*;
    use crate::*;

    #[test]
    fn clean_replaces_control_characters_but_line_ends_and_tabs() {
        assert_eq!(clean("a\u{1b}b\u{7}c"), "a\u{fffd}b\u{fffd}c");
        assert_eq!(clean("tab\there\nend"), "tab\there\nend");
        assert_eq!(clean("crlf\r\nlone\rend\r"), "crlf\r\nlone\u{fffd}end\r");
        assert_eq!(clean("\u{0}\u{85}ünï 🦀"), "\u{fffd}\u{fffd}ünï 🦀");
        assert_eq!(clean(""), "");
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
    fn commands_and_options_encode_tagged_and_round_trip() {
        let commands = Event::Commands {
            commands: vec![compact()],
        };
        assert_eq!(
            commands.to_value(),
            serde_json::json!({"event": "commands", "commands": [
                {"name": "compact", "description": "Free up context", "hint": "<instructions>"}
            ]})
        );
        let options = Event::Options {
            options: vec![mode_option("ask"), fast_option()],
        };
        assert_eq!(
            options.to_value(),
            serde_json::json!({"event": "options", "options": [
                {"id": "mode", "name": "Mode", "category": {"category": "mode"},
                 "kind": {"control": "select", "current": "ask", "choices": [
                     {"value": "ask", "name": "Ask", "description": "Asks first"},
                     {"value": "code", "name": "Code", "group": "Edits"}
                 ]}},
                {"id": "fast", "name": "Fast mode", "description": "Speed",
                 "category": {"category": "other", "name": "speed"},
                 "kind": {"control": "toggle", "on": false}}
            ]})
        );
        for event in [
            commands,
            options,
            Event::Commands { commands: vec![] },
            Event::Options { options: vec![] },
        ] {
            assert_eq!(Event::from_value(&event.to_value()), Ok(event));
        }
        // A missing list is a decode error, as any missing field is.
        assert!(Event::from_value(&serde_json::json!({"event": "options"})).is_err());
        assert!(
            Event::from_value(&serde_json::json!({"event": "options", "options": [
                {"id": "x", "name": "X", "category": {"category": "later"},
                 "kind": {"control": "toggle", "on": true}}
            ]}))
            .is_err()
        );
    }

    /// `Event::encoded_len` equals `serde_json::to_vec(event).len()` for
    /// random events from `fuzz.rs`'s generator (the fixtures' are
    /// `tests/events.rs`'s).
    #[test]
    fn the_encoded_length_of_a_random_event_is_its_encodings() {
        let mut rng = XorShift::new(0xeb_0001);
        for _ in 0..2000 {
            let event = crate::fuzz::any_event(&mut rng);
            assert_eq!(event.encoded_len(), serde_json::to_vec(&event).unwrap().len());
        }
    }

    /// `escaped_len` of a cleaned string is at most twice its bytes, and
    /// `cleaned_len(t)` is `clean(t).len()`, for random strings with
    /// controls.
    #[test]
    fn cleaned_text_escapes_to_at_most_twice_its_bytes_and_cleaned_len_is_cleans() {
        let mut rng = XorShift::new(0xeb_0002);
        let chars = ['a', '"', '\\', '\n', '\t', '\r', '\u{0}', '\u{1b}', '\u{7f}', '\u{85}', 'é', '🦀'];
        for _ in 0..5000 {
            let text: String = (0..rng.below(40)).map(|_| chars[rng.below(chars.len())]).collect();
            let cleaned = clean(&text);
            assert_eq!(cleaned_len(&text), cleaned.len(), "{text:?}");
            assert!(escaped_len(&cleaned) <= 2 * cleaned.len(), "{cleaned:?}");
            assert_eq!(escaped_len(&cleaned) + 2, serde_json::to_vec(&cleaned).unwrap().len());
        }
    }

    /// `wire_len` of a diff counts 4 bytes a line and 64 a hunk, beside its
    /// paths.
    #[test]
    fn the_wire_length_of_a_diff_counts_its_lines_and_hunks_framing() {
        assert_eq!(lines_diff(&[], 0).wire_len(), "/b/c".len() + "/a".len());
        assert_eq!(lines_diff(&["", "", ""], 1).wire_len(), 6 + 3 * LINE_FRAMING + HUNK_FRAMING);
        assert_eq!(
            lines_diff(&["ab", "c"], 2).wire_len(),
            6 + 2 * (3 + 2 * LINE_FRAMING + HUNK_FRAMING)
        );
        // The framing is no less than what the encoding adds.
        let item = lines_diff(&["", ""], 3);
        let ToolItem::Diff(change) = &item else { unreachable!() };
        let bare = serde_json::to_vec(&FileChange { hunks: Vec::new(), ..change.clone() }).unwrap().len();
        assert!(serde_json::to_vec(change).unwrap().len() - bare <= item.wire_len() - 6);
    }

    /// `wire_len` of an output counts `ITEM_LIMIT`, its command and its
    /// directory, whatever its text holds.
    #[test]
    fn the_wire_length_of_an_output_is_its_limit_command_and_directory() {
        let empty = ToolItem::Output(Output::default());
        assert_eq!(empty.wire_len(), ITEM_LIMIT + SIGNAL_RESERVE);
        let full = ToolItem::Output(Output {
            command: Some("cargo test".into()),
            cwd: Some("/p".into()),
            text: "x".repeat(ITEM_LIMIT).into(),
            omitted: 9,
            partial: true,
            exit: Some(Exit::Signal { name: "KILL".into() }),
        });
        assert_eq!(full.wire_len(), ITEM_LIMIT + 10 + 2 + SIGNAL_RESERVE);
    }

    /// `ContentBudget` keeps at most 32 items whose `wire_len`s add up to at
    /// most `CALL_LIMIT`, trying later items after one that doesn't fit,
    /// and counts the rest.
    #[test]
    fn a_content_budget_keeps_what_fits_and_counts_the_rest() {
        let mut budget = ContentBudget::new();
        let small = text_item("s");
        for _ in 0..MAX_ITEMS {
            assert!(budget.take(&small));
        }
        assert!(!budget.take(&small));
        assert_eq!(budget.left_out(), 1);
        // Three outputs fill most of it; a fourth doesn't fit, a small text
        // after it still does.
        let mut budget = ContentBudget::new();
        let output = ToolItem::Output(Output::default());
        assert!(budget.take(&output) && budget.take(&output) && budget.take(&output));
        assert!(!budget.take(&output));
        assert!(budget.take(&small));
        assert!(!budget.take(&text_item(&"t".repeat(CALL_LIMIT))));
        assert_eq!(budget.left_out(), 2);
        assert!(ContentBudget::holding(&[output.clone(), output.clone(), output.clone()]).fits(&small));
        assert!(!ContentBudget::holding(&[output.clone(), output.clone(), output.clone()]).fits(&output));
    }

    /// A hunk value that starts at 0, or whose lines would be numbered past
    /// `u32::MAX`, is a decode error; the last line number itself decodes.
    #[test]
    fn a_hunk_numbered_from_0_or_past_the_last_line_is_a_decode_error() {
        let hunk = |old: u64, new: u64, lines: &[&str]| {
            serde_json::json!({"event": "tool_items", "id": 1, "from": 0, "items": [
                {"item": "diff", "op": "modify", "path": "/p",
                 "hunks": [{"old_start": old, "new_start": new, "lines": lines}]}]})
        };
        let max = u64::from(u32::MAX);
        for value in [
            hunk(0, 1, &[" a"]),
            hunk(1, 0, &["+a"]),
            hunk(max, max, &[" a", " b"]),
            hunk(max - 1, 1, &["-a", "-b", "-c"]),
            hunk(1, max, &["+a", "+b"]),
        ] {
            let error = Event::from_value(&value).expect_err("a bad hunk");
            assert!(error.to_string().contains("hunk"), "{error}");
        }
        for value in [
            hunk(max, max, &[" a"]),
            hunk(max, 1, &["+a", "+b"]),
            hunk(1, max, &["-a", "-b"]),
        ] {
            assert!(Event::from_value(&value).is_ok(), "{value}");
        }
    }
}
