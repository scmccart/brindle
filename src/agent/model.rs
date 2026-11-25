//! The chat model of an agent zone (design.md decision 8): what the zone
//! shows and decides, on the UI thread, from the transcript the reader
//! thread folds ([`crate::agent::shared`]) and the session's life events.
//!
//! GPUI-free: plain values in, plain values out (`fcis R1`, `R5`); the
//! blocks hold `SharedString`s so the view draws them without copying.
//! The view owns the prompt input's text: nothing here writes it, apart
//! from a launch's first prompt left as a draft when it can't be sent
//! ([`First::Draft`]) and a refused send's text put back
//! ([`Replied::SetDraft`]); nothing from the queue slot ever reaches it.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use brindle_ui::{Inline, RichBlock};
use gpui::SharedString;

use crate::FirstPrompt;
use crate::agent::markdown::{self, Part};
use crate::agent::shared::{Changes, Dirty};
use crate::agent::transcript::{
    Activity, AgentState, Entry, EntryId, HoldReason, NoticeLevel, PermissionKind, PermissionOption,
    PermissionOutcome, Slot, StopReason, ToolKind, ToolStatus, Transcript, TurnState, status_of,
};
use crate::terminal::links::Plan;
use crate::terminal::status::State;

/// The client protocol version that carries agent sessions.
pub const AGENT_PROTOCOL: u32 = 4;

/// A request's id on the session's connection (the client protocol's
/// `req`), which its `OK` or `ERROR` names.
pub type RequestId = u64;

/// How long a user message's block grows before it is cut at the next
/// line break, so a long pasted prompt lays out in pieces. Agent messages
/// are cut at their top-level Markdown blocks instead.
pub const BLOCK_LIMIT: usize = 4 * 1024;

/// Why an agent session never started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartFailure {
    /// No terminal server could be reached or started: the cause.
    NoServer(String),
    /// The endpoint's terminal server speaks client protocols below 4.
    TooOld { endpoint: String },
    /// The server refused the spawn: the command and the server's error
    /// verbatim (on Windows a batch file's hint included).
    Refused { command: String, cause: String },
}

/// For logs and the client's error chain; the zone words it with
/// [`failure_text`].
impl std::fmt::Display for StartFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoServer(cause) => write!(f, "no terminal server: {cause}"),
            Self::TooOld { endpoint } => f.write_str(&too_old_text(endpoint)),
            Self::Refused { command, cause } => write!(f, "the agent {command} didn't start: {cause}"),
        }
    }
}

impl std::error::Error for StartFailure {}

/// Whether a connection that agreed on client `protocol` can start an
/// agent session on `endpoint`: only protocol 4 and up carry
/// `SPAWN_AGENT`, so an older server is refused before anything is sent.
pub fn agent_start(protocol: u32, endpoint: &str) -> Result<(), StartFailure> {
    if protocol >= AGENT_PROTOCOL {
        Ok(())
    } else {
        Err(StartFailure::TooOld { endpoint: endpoint.to_string() })
    }
}

/// The words for a server too old for agent sessions.
pub fn too_old_text(endpoint: &str) -> String {
    format!("the terminal server on {endpoint} is too old for agent sessions; it will update once its terminals have ended")
}

/// Where the zone's agent is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPhase {
    /// The session is being started or attached; nothing folded yet.
    Connecting,
    /// The session never started.
    NotStarted(StartFailure),
    /// The agent's state, as the last `Agent` event folded says.
    Live(AgentState),
    /// The session can't be got back.
    Lost { reason: String },
    /// Brindle can't follow the session any more; it was detached.
    Broken { reason: String },
}

/// The connection to the session's endpoint, beside the phase: a reconnect
/// keeps the conversation shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Up,
    Reconnecting,
}

/// Whether the session still runs on its server, beside the phase: it
/// ends on the session's exit (an agent that exited, or one hung up for
/// its ACP version, whose last state stays `Failed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLife {
    Open,
    Ended,
}

/// One of a permission request's buttons, keyed by its entry and `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionButton {
    pub id: SharedString,
    pub name: SharedString,
    pub kind: PermissionKind,
}

/// What a block draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockBody {
    /// A part of a user message, plain ([`parts`]).
    UserText { text: SharedString },
    /// One top-level Markdown block of an agent message.
    AgentPart { block: Arc<RichBlock> },
    /// A thought, whole: all its Markdown blocks.
    Thought { blocks: Arc<[Arc<RichBlock>]>, open: bool },
    Tool { kind: ToolKind, title: SharedString, status: ToolStatus },
    /// The buttons in the agent's order, allow kinds first.
    Permission { title: SharedString, options: Vec<PermissionButton>, outcome: Option<PermissionOutcome> },
    Notice { level: NoticeLevel, text: SharedString },
}

/// One item of the conversation's list: an entry, or one part of a message
/// (`part` counts from 0; `start` is its first byte in the entry's text;
/// `last` is the entry's last part, where its bubble ends).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub entry: EntryId,
    pub part: usize,
    pub start: usize,
    pub last: bool,
    pub body: BlockBody,
}

/// A change to the conversation's list, as the view's list state takes
/// it: the items in `range` replaced by `count` new ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub range: Range<usize>,
    pub count: usize,
}

/// The byte ranges of `text` its parts cover, from `from` (the start of a
/// part) on: a part ends at a blank line, or, once it is past
/// [`BLOCK_LIMIT`], at the next line break. Line breaks between parts
/// belong to none, so a part never starts with one. A cut point depends
/// only on the text before it, so text appended to a message changes only
/// its last part and adds new ones. A message with no text but line breaks
/// is one empty part.
pub fn parts(text: &str, from: usize) -> Vec<Range<usize>> {
    let from = from.min(text.len());
    let mut parts = Vec::new();
    let mut start = from;
    loop {
        start += text[start..].bytes().take_while(|b| *b == b'\n').count();
        if start >= text.len() {
            break;
        }
        let rest = &text[start..];
        let paragraph = rest.find("\n\n").map(|p| start + p);
        let long = if rest.len() > BLOCK_LIMIT {
            let mut cut = start + BLOCK_LIMIT;
            while !text.is_char_boundary(cut) {
                cut += 1;
            }
            text[cut..].find('\n').map(|q| cut + q)
        } else {
            None
        };
        let end = match (paragraph, long) {
            (Some(p), Some(q)) => p.min(q),
            (Some(cut), None) | (None, Some(cut)) => cut,
            (None, None) => text.len(),
        };
        parts.push(start..end);
        start = end;
    }
    if parts.is_empty() {
        parts.push(from..from);
    }
    parts
}

fn is_allow(kind: PermissionKind) -> bool {
    match kind {
        PermissionKind::AllowOnce | PermissionKind::AllowAlways => true,
        PermissionKind::RejectOnce | PermissionKind::RejectAlways => false,
    }
}

/// A permission request's buttons: the agent's order, allow kinds first.
fn buttons(options: &[PermissionOption]) -> Vec<PermissionButton> {
    let allow = options.iter().filter(|o| is_allow(o.kind));
    let reject = options.iter().filter(|o| !is_allow(o.kind));
    allow
        .chain(reject)
        .map(|o| PermissionButton { id: o.id.clone().into(), name: o.name.clone().into(), kind: o.kind })
        .collect()
}

/// The blocks of user message `id`, from its part `part` starting at byte
/// `from` of its text (0 and 0 for the whole message).
fn user_blocks(id: EntryId, text: &str, part: usize, from: usize) -> Vec<Block> {
    let ranges = parts(text, from);
    let count = ranges.len();
    ranges
        .into_iter()
        .enumerate()
        .map(|(i, range)| Block {
            entry: id,
            part: part + i,
            start: range.start,
            last: i + 1 == count,
            body: BlockBody::UserText { text: text.get(range).unwrap_or_default().to_string().into() },
        })
        .collect()
}

/// The blocks an entry is drawn as: one per part of a user message
/// ([`parts`]) or per rendered part of an agent message (`rendered`), else
/// one; a thought holds its `rendered` blocks and is drawn open when
/// `thought_open`.
fn blocks_of(id: EntryId, entry: &Entry, rendered: &[Part], thought_open: bool) -> Vec<Block> {
    let one = |body| vec![Block { entry: id, part: 0, start: 0, last: true, body }];
    match entry {
        Entry::User { text } => user_blocks(id, text, 0, 0),
        Entry::Agent { .. } => rendered
            .iter()
            .enumerate()
            .map(|(i, part)| Block {
                entry: id,
                part: i,
                start: part.range.start,
                last: i + 1 == rendered.len(),
                body: BlockBody::AgentPart { block: part.block.clone() },
            })
            .collect(),
        Entry::Thought { .. } => {
            let blocks = rendered.iter().map(|part| part.block.clone()).collect();
            one(BlockBody::Thought { blocks, open: thought_open })
        }
        Entry::Tool { title, kind, status, .. } => {
            one(BlockBody::Tool { kind: *kind, title: title.clone().into(), status: *status })
        }
        Entry::Permission { title, options, outcome, .. } => one(BlockBody::Permission {
            title: title.clone().into(),
            options: buttons(options),
            outcome: outcome.clone(),
        }),
        Entry::Notice { level, text } => one(BlockBody::Notice { level: *level, text: text.clone().into() }),
    }
}

/// An agent message's or thought's text rendered ([`markdown::render`]),
/// and whether the renderer panicked, when its text is shown as one plain
/// paragraph instead; `None` for the entries that aren't Markdown.
fn render_entry(entry: &Entry) -> Option<(Vec<Part>, bool)> {
    let text = match entry {
        Entry::Agent { text } | Entry::Thought { text } => text,
        Entry::User { .. } | Entry::Tool { .. } | Entry::Permission { .. } | Entry::Notice { .. } => return None,
    };
    Some(match std::panic::catch_unwind(|| markdown::render(text)) {
        Ok(parts) => (parts, false),
        Err(_) => {
            let block = RichBlock::Paragraph(Inline::plain(text.clone()));
            (vec![Part { range: 0..text.len(), block: Arc::new(block) }], true)
        }
    })
}

/// Whether a value may be resent from the queue row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resend {
    Available,
    /// Disabled, with the tooltip saying why.
    Unavailable(SharedString),
}

/// The row above the prompt input: the queue slot as the zone shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueRow {
    None,
    /// Queued: sent when the running turn ends normally.
    Waiting { text: SharedString },
    /// Not sent, with why in words, and whether send-again works.
    Held { text: SharedString, reason: HoldReason, why: SharedString, resend: Resend },
}

/// The name a stop reason has in the dump and the hold's words.
pub fn stop_word(stop: &StopReason) -> &str {
    match stop {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Refusal => "refusal",
        StopReason::Cancelled => "cancelled",
        StopReason::Other { name } => name,
    }
}

/// Why a prompt is held, in words ("not sent: …").
pub fn hold_words(reason: &HoldReason) -> String {
    match reason {
        HoldReason::Cancelled => "you stopped the turn".into(),
        HoldReason::Failed => "the turn failed".into(),
        HoldReason::Stopped { stop } => format!("the turn ended: {}", stop_word(stop)),
        HoldReason::LoginRequired => "the agent needs a login".into(),
        HoldReason::AgentStopped => "the agent stopped".into(),
        HoldReason::OutputLost => "output was lost while the terminal server restarted".into(),
    }
}

/// The dump's word for a hold reason.
fn hold_word(reason: &HoldReason) -> String {
    match reason {
        HoldReason::Cancelled => "cancelled".into(),
        HoldReason::Failed => "failed".into(),
        HoldReason::Stopped { stop } => format!("stopped:{}", stop_word(stop)),
        HoldReason::LoginRequired => "login-required".into(),
        HoldReason::AgentStopped => "agent-stopped".into(),
        HoldReason::OutputLost => "output-lost".into(),
    }
}

const EXITED: &str = "the agent has exited";

/// Whether a held prompt can be sent again, for the phase and the
/// session's life: only to a ready agent whose session is open.
fn resend_of(phase: &AgentPhase, life: SessionLife) -> Resend {
    let why = match life {
        SessionLife::Ended => EXITED,
        SessionLife::Open => match phase {
            AgentPhase::Live(AgentState::Ready { .. }) => return Resend::Available,
            AgentPhase::Live(AgentState::Starting) => "the agent is starting",
            AgentPhase::Live(AgentState::LoginRequired { .. }) => {
                "the agent needs a login; open the profile again after logging in"
            }
            AgentPhase::Live(AgentState::Failed { .. }) => "the agent failed",
            AgentPhase::Live(AgentState::Exited { .. }) => EXITED,
            AgentPhase::Connecting | AgentPhase::NotStarted(_) => "the agent isn't ready",
            AgentPhase::Lost { .. } => "the agent session was lost",
            AgentPhase::Broken { .. } => "Brindle can't show this agent session any more",
        },
    };
    Resend::Unavailable(why.into())
}

/// The queue row for `slot` in `phase` with the session's `life`.
pub fn queue_row(slot: &Slot, phase: &AgentPhase, life: SessionLife) -> QueueRow {
    match slot {
        Slot::Empty => QueueRow::None,
        Slot::Waiting { text } => QueueRow::Waiting { text: text.clone().into() },
        Slot::Held { text, reason } => QueueRow::Held {
            text: text.clone().into(),
            reason: reason.clone(),
            why: hold_words(reason).into(),
            resend: resend_of(phase, life),
        },
    }
}

/// What a zone that can't take prompts says above its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub text: String,
    /// Drawn as a destructive alert (else the default one).
    pub destructive: bool,
    /// The agent's own words (its message, or the end of its standard
    /// error), shown under the text.
    pub detail: Option<String>,
    /// A command to show on a line of its own in the monospace font (a
    /// Windows batch file's `cmd /c …` form).
    pub mono: Option<String>,
}

/// What the alert names: the agent (its name, else its profile's), the
/// profile's command and the endpoint.
#[derive(Debug, Clone, Copy)]
pub struct Names<'a> {
    pub agent: &'a str,
    pub command: &'a str,
    pub endpoint: &'a str,
}

/// The `cmd /c …` form a Windows batch file's spawn error suggests, from
/// the server's words (`… run it as \`cmd /c npx …\``).
fn batch_form(cause: &str) -> Option<String> {
    let rest = &cause[cause.find("`cmd /c ")? + 1..];
    Some(rest[..rest.find('`')?].to_string())
}

/// The alert for `phase` and the session's `life`, or none while the zone
/// works (design.md decision 13): the action, then the cause. The two
/// `Failed` rows differ only by `life`: ended, it is a version mismatch's,
/// never worded as an exit.
pub fn failure_text(phase: &AgentPhase, life: SessionLife, names: Names<'_>) -> Option<Alert> {
    let alert = |text: String, destructive, detail: Option<&str>| Alert {
        text,
        destructive,
        detail: detail.map(str::to_string).filter(|d| !d.is_empty()),
        mono: None,
    };
    let agent = names.agent;
    match phase {
        AgentPhase::Connecting => None,
        AgentPhase::NotStarted(StartFailure::NoServer(cause)) => {
            Some(alert(format!("Brindle could not start the agent {}: {cause}", names.command), true, None))
        }
        AgentPhase::NotStarted(StartFailure::Refused { command, cause }) => Some(Alert {
            mono: batch_form(cause),
            ..alert(format!("Brindle could not start the agent {command}: {cause}"), true, None)
        }),
        AgentPhase::NotStarted(StartFailure::TooOld { endpoint }) => Some(alert(too_old_text(endpoint), true, None)),
        AgentPhase::Live(AgentState::Starting | AgentState::Ready { .. }) => match life {
            SessionLife::Open => None,
            SessionLife::Ended => {
                Some(alert(format!("the agent session on {} has ended", names.endpoint), true, None))
            }
        },
        AgentPhase::Live(AgentState::LoginRequired { message }) => Some(alert(
            format!(
                "{agent} needs you to log in. Log in through the agent's own command line, then open the profile again."
            ),
            false,
            Some(message),
        )),
        AgentPhase::Live(AgentState::Failed { message }) => match life {
            SessionLife::Open => {
                Some(alert(format!("Brindle's terminal server failed handling {agent}: {message}"), true, None))
            }
            SessionLife::Ended => Some(alert(format!("{agent} can't be used: {message}"), true, None)),
        },
        AgentPhase::Live(AgentState::Exited { code, stderr }) => {
            let text = match code {
                Some(code) => format!("{agent} exited with status {code}"),
                None => format!("{agent} was ended by a signal"),
            };
            Some(alert(text, true, Some(stderr)))
        }
        AgentPhase::Lost { reason } => {
            Some(alert(format!("the agent session on {} was lost: {reason}", names.endpoint), true, None))
        }
        AgentPhase::Broken { reason } => Some(alert(
            "Brindle can't show this agent session any more; attach it again from Attach Session…".into(),
            true,
            Some(reason),
        )),
    }
}

/// Why the prompt input takes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisabledWhy {
    Connecting,
    NotStarted,
    Starting,
    LoginRequired,
    Failed,
    Exited,
    Lost,
    Broken,
    Reconnecting,
}

impl DisabledWhy {
    /// The prompt input's reason line.
    pub fn text(self) -> &'static str {
        match self {
            Self::Connecting => "connecting to the terminal server",
            Self::NotStarted => "the agent didn't start",
            Self::Starting => "the agent is starting",
            Self::LoginRequired => "the agent needs a login; log in through its own command line, then open the profile again",
            Self::Failed => "the agent can't be used",
            Self::Exited => EXITED,
            Self::Lost => "the agent session was lost",
            Self::Broken => "Brindle can't show this agent session any more",
            Self::Reconnecting => "reconnecting to the terminal server",
        }
    }
}

/// What the prompt input's submit button is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptStatus {
    /// Send (disabled by the component while the text is blank).
    Ready,
    /// A prompt is on its way.
    Submitted,
    /// A turn runs: the button stops it.
    Streaming,
    Disabled(DisabledWhy),
}

impl PromptStatus {
    fn word(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Submitted => "submitted",
            Self::Streaming => "streaming",
            Self::Disabled(_) => "disabled",
        }
    }
}

/// Why enter sends nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// An IME composition is open: enter commits it.
    Composing,
    Blank,
    NotReady(DisabledWhy),
    /// A prompt is on its way.
    InFlight,
    /// A turn runs and the slot holds a prompt.
    AlreadyQueued,
}

/// The words for a prompt kept because one is already queued.
pub const ALREADY_QUEUED: &str = "a prompt is already queued";

/// The refusal shown when a request couldn't be sent at all
/// ([`ChatModel::unsent`]).
pub const UNSENT: &str = "the agent session isn't connected; nothing was sent";

/// What enter does with the prompt input's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submit {
    /// Send this as a prompt (the server sends it, or queues it during a
    /// turn); the input keeps it until the server accepts it.
    Send(String),
    Nothing(Why),
}

/// A request the zone sent, kept until its reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    Prompt,
    /// The launch's first prompt, put back as the draft if refused.
    FirstPrompt(String),
    Resend,
    Unqueue,
    Answer(EntryId),
}

/// What a reply does to the prompt input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replied {
    /// A prompt was accepted: the input empties.
    ClearDraft,
    /// The first prompt was refused: it becomes the draft.
    SetDraft(String),
    /// The input is left as it is.
    Nothing,
}

/// What the queue row's remove (or discard) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unqueue {
    /// Send `AGENT_UNQUEUE`; the row goes when the server's `Queue` comes.
    Send,
    /// The session has ended or was lost: the row was cleared here, and
    /// nothing is sent.
    Cleared,
    /// Nothing to remove, or nothing can be sent now.
    Nothing,
}

/// What a launch's first prompt does now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum First {
    /// The agent is ready: send it, once.
    Send(String),
    /// It can't be sent: it becomes the prompt input's draft, unsent.
    Draft(String),
}

/// A desktop notification an agent zone raises (decision 10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentNotice {
    pub title: String,
    pub body: String,
}

/// What a change to the model asks of the view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Absorbed {
    /// For the conversation's list, in order.
    pub splices: Vec<Splice>,
    /// The shown status, when it changed.
    pub status: Option<Option<State>>,
    pub notice: Option<AgentNotice>,
    pub title_changed: bool,
    pub queue_changed: bool,
    /// The launch's first prompt, when it is sent or left as a draft now.
    pub first: Option<First>,
}

/// What a change compares against.
struct Before {
    status: Option<State>,
    title: String,
    queue: QueueRow,
}

/// An agent zone's model.
#[derive(Debug, Clone)]
pub struct ChatModel {
    /// The profile's name, the title until the agent names itself.
    profile: String,
    /// The agent's own name, from the last `Ready` that gave one.
    agent_name: Option<String>,
    phase: AgentPhase,
    link: Link,
    life: SessionLife,
    turn: TurnState,
    activity: Activity,
    slot: Slot,
    queue: QueueRow,
    /// The zone's copy of the kept entries, in id order.
    entries: Vec<(EntryId, Entry)>,
    /// The rendered parts of the agent messages and thoughts in `entries`
    /// (`types R10`): refreshed when one is appended to or replaced,
    /// rebuilt on a reset, dropped with a trimmed entry.
    rendered: BTreeMap<EntryId, Vec<Part>>,
    /// The renderer panicked once (logged once).
    render_failed: bool,
    blocks: Vec<Block>,
    /// Thoughts the user opened or closed.
    toggled: BTreeMap<EntryId, bool>,
    /// The thought drawn open on its own: the latest entry while a turn
    /// runs.
    auto_open: Option<EntryId>,
    first: Option<FirstPrompt>,
    pending: BTreeMap<RequestId, Sent>,
    refusal: Option<String>,
    /// The user typed after a `done` or `error` arrived.
    seen: bool,
    /// A restore's note, shown above the conversation.
    note: Option<String>,
}

fn running(turn: &TurnState) -> bool {
    match turn {
        TurnState::Working | TurnState::Blocked | TurnState::Cancelling => true,
        TurnState::Idle | TurnState::Ended { .. } | TurnState::Failed { .. } => false,
    }
}

impl ChatModel {
    /// A zone of `profile`'s agent, with a launch's `first` prompt (only a
    /// launch has one: a restored or attached zone never does).
    pub fn new(profile: impl Into<String>, first: Option<FirstPrompt>) -> Self {
        Self {
            profile: profile.into(),
            agent_name: None,
            phase: AgentPhase::Connecting,
            link: Link::Up,
            life: SessionLife::Open,
            turn: TurnState::Idle,
            activity: Activity::Idle,
            slot: Slot::Empty,
            queue: QueueRow::None,
            entries: Vec::new(),
            rendered: BTreeMap::new(),
            render_failed: false,
            blocks: Vec::new(),
            toggled: BTreeMap::new(),
            auto_open: None,
            first,
            pending: BTreeMap::new(),
            refusal: None,
            seen: false,
            note: None,
        }
    }

    /// A restore's note (a recorded profile gone from the config).
    pub fn set_note(&mut self, note: impl Into<String>) {
        self.note = Some(note.into());
    }

    /// The profile was renamed.
    pub fn set_profile_name(&mut self, profile: impl Into<String>) {
        self.profile = profile.into();
    }

    #[cfg(test)]
    pub fn phase(&self) -> &AgentPhase {
        &self.phase
    }

    pub fn link(&self) -> Link {
        self.link
    }

    #[cfg(test)]
    pub fn life(&self) -> SessionLife {
        self.life
    }

    #[cfg(test)]
    pub fn turn(&self) -> &TurnState {
        &self.turn
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn queue(&self) -> &QueueRow {
        &self.queue
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The refusal shown beside the prompt input.
    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    /// The zone's title: the agent's name from the last `Ready` that gave
    /// one (kept after it exits or fails), else the profile's.
    pub fn title(&self) -> String {
        self.agent_name.clone().unwrap_or_else(|| self.profile.clone())
    }

    /// The status the zone shows (decision 10): its activity's, while the
    /// phase is live; none outside it; a `done` or `error` the user has
    /// seen by typing is dropped until the activity changes.
    pub fn shown_status(&self) -> Option<State> {
        match &self.phase {
            AgentPhase::Live(_) => {}
            AgentPhase::Connecting | AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => {
                return None;
            }
        }
        let state = status_of(self.activity);
        match state {
            State::Done | State::Error if self.seen => None,
            State::Done | State::Error | State::Idle | State::Working | State::Blocked => Some(state),
        }
    }

    /// The user typed into the prompt input: a `done` or `error` showing
    /// is seen. The new shown status, when it changed.
    pub fn seen(&mut self) -> Option<Option<State>> {
        let before = self.shown_status();
        match status_of(self.activity) {
            State::Done | State::Error => self.seen = true,
            State::Idle | State::Working | State::Blocked => {}
        }
        let after = self.shown_status();
        (after != before).then_some(after)
    }

    fn before(&self) -> Before {
        Before { status: self.shown_status(), title: self.title(), queue: self.queue.clone() }
    }

    /// Folds what changed in the shared transcript into the zone's copy.
    pub fn absorb(&mut self, transcript: &Transcript, changes: Changes) -> Absorbed {
        let before = self.before();
        let mut splices = Vec::new();
        if changes.reset {
            self.entries = transcript.entries().map(|(id, entry)| (id, entry.clone())).collect();
            self.auto_open = None;
            self.rendered.clear();
            let renders: Vec<_> = self.entries.iter().map(|(id, entry)| (*id, render_entry(entry))).collect();
            for (id, render) in renders {
                self.keep_render(id, render);
            }
            let old = self.blocks.len();
            self.blocks = self.entries.iter().flat_map(|(id, entry)| self.draw(*id, entry, false)).collect();
            splices.push(Splice { range: 0..old, count: self.blocks.len() });
        } else {
            if let Some(through) = changes.trimmed_through {
                self.entries.retain(|(id, _)| *id > through);
                self.rendered.retain(|id, _| *id > through);
                let gone = self.blocks.partition_point(|block| block.entry <= through);
                self.blocks.drain(..gone);
                if gone > 0 {
                    splices.push(Splice { range: 0..gone, count: 0 });
                }
            }
            for (id, dirty) in changes.changed {
                splices.extend(self.update_entry(transcript, id, dirty));
            }
            if let Some(from) = changes.added_from {
                let at = self.blocks.len();
                for (id, entry) in transcript.entries().filter(|(id, _)| *id >= from) {
                    if self.entries.last().is_some_and(|(last, _)| *last >= id) {
                        continue;
                    }
                    self.entries.push((id, entry.clone()));
                    self.keep_render(id, render_entry(entry));
                    let fresh = self.draw(id, entry, false);
                    self.blocks.extend(fresh);
                }
                if self.blocks.len() > at {
                    splices.push(Splice { range: at..at, count: self.blocks.len() - at });
                }
            }
        }
        if changes.reset || changes.turn_or_agent || changes.added_from.is_some() {
            self.turn = transcript.turn().clone();
            let activity = transcript.activity();
            if activity != self.activity {
                self.seen = false;
            }
            self.activity = activity;
            match &self.phase {
                AgentPhase::Connecting | AgentPhase::Live(_) => self.phase = AgentPhase::Live(transcript.agent().clone()),
                AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => {}
            }
            if let AgentPhase::Live(AgentState::Ready { name: Some(name), .. }) = &self.phase {
                self.agent_name = Some(name.clone());
            }
        }
        if changes.reset || changes.slot_changed {
            self.slot = transcript.slot().clone();
        }
        splices.extend(self.update_thoughts(changes.reset));
        self.finish(before, splices, changes.from_replay)
    }

    /// Copies entry `id` again (its tail only when text was appended) and
    /// redraws its blocks from the first that changed.
    fn update_entry(&mut self, transcript: &Transcript, id: EntryId, dirty: Dirty) -> Option<Splice> {
        let source = transcript.entry(id)?;
        let index = self.entries.binary_search_by_key(&id, |(own, _)| *own).ok()?;
        let own = &mut self.entries[index].1;
        let appended = match (dirty, &mut *own, source) {
            (
                Dirty::Appended { from_byte },
                Entry::User { text: own } | Entry::Agent { text: own } | Entry::Thought { text: own },
                Entry::User { text } | Entry::Agent { text } | Entry::Thought { text },
            ) if own.len() == from_byte && text.is_char_boundary(from_byte) => {
                own.push_str(&text[from_byte..]);
                true
            }
            (Dirty::Appended { .. } | Dirty::Replaced, _, _) => false,
        };
        if !appended {
            *own = source.clone();
        }
        // An agent message or thought is rendered again whole; the parts
        // that didn't change keep their cached values.
        let render = render_entry(&self.entries[index].1).map(|(fresh, panicked)| {
            let old = self.rendered.get(&id).map_or(&[][..], Vec::as_slice);
            let same = old.iter().zip(&fresh).take_while(|(old, fresh)| old == fresh).count();
            let mut parts = old[..same].to_vec();
            parts.extend(fresh.into_iter().skip(same));
            (parts, panicked)
        });
        self.keep_render(id, render);
        let range = self.entry_blocks(id);
        let open = self.thought_open(id);
        let entry = &self.entries[index].1;
        let (range, fresh) = match entry {
            // A user message's earlier parts don't change: only its last
            // and new ones are drawn again.
            Entry::User { text } if !range.is_empty() => {
                let last = &self.blocks[range.end - 1];
                (range.end - 1..range.end, user_blocks(id, text, last.part, last.start))
            }
            // An agent message's items from the first that differs: a
            // changed part, or the end of its bubble moving.
            Entry::Agent { .. } => {
                let fresh = self.draw(id, entry, open);
                let same = self.blocks[range.clone()].iter().zip(&fresh).take_while(|(old, new)| old == new).count();
                if same == range.len() && same == fresh.len() {
                    return None;
                }
                (range.start + same..range.end, fresh[same..].to_vec())
            }
            Entry::User { .. }
            | Entry::Thought { .. }
            | Entry::Tool { .. }
            | Entry::Permission { .. }
            | Entry::Notice { .. } => (range, self.draw(id, entry, open)),
        };
        let count = fresh.len();
        self.blocks.splice(range.clone(), fresh);
        Some(Splice { range, count })
    }

    /// Keeps entry `id`'s rendered parts (`None`: it has none), logging the
    /// renderer's first panic.
    fn keep_render(&mut self, id: EntryId, render: Option<(Vec<Part>, bool)>) {
        match render {
            Some((parts, panicked)) => {
                if panicked && !self.render_failed {
                    self.render_failed = true;
                    log::warn!("the Markdown renderer panicked on agent entry {id}; showing its text plain");
                }
                self.rendered.insert(id, parts);
            }
            None => {
                self.rendered.remove(&id);
            }
        }
    }

    /// The blocks entry `id` is drawn as, from its cached parts.
    fn draw(&self, id: EntryId, entry: &Entry, thought_open: bool) -> Vec<Block> {
        blocks_of(id, entry, self.rendered.get(&id).map_or(&[][..], Vec::as_slice), thought_open)
    }

    /// The blocks of entry `id`.
    fn entry_blocks(&self, id: EntryId) -> Range<usize> {
        self.blocks.partition_point(|b| b.entry < id)..self.blocks.partition_point(|b| b.entry <= id)
    }

    fn thought_open(&self, id: EntryId) -> bool {
        self.toggled.get(&id).copied().unwrap_or(self.auto_open == Some(id))
    }

    /// The thought open on its own: the latest entry, while a turn runs.
    fn wanted_auto_open(&self) -> Option<EntryId> {
        match self.entries.last() {
            Some((id, Entry::Thought { .. })) if running(&self.turn) => Some(*id),
            Some(_) | None => None,
        }
    }

    /// Redraws the thoughts whose openness changed (`all`: every one).
    fn update_thoughts(&mut self, all: bool) -> Vec<Splice> {
        let before = self.auto_open;
        self.auto_open = self.wanted_auto_open();
        let ids: Vec<EntryId> = if all {
            self.entries.iter().filter(|(_, e)| matches!(e, Entry::Thought { .. })).map(|(id, _)| *id).collect()
        } else {
            [before, self.auto_open].into_iter().flatten().collect()
        };
        ids.into_iter().filter_map(|id| self.redraw_thought(id)).collect()
    }

    /// Draws thought `id` open or closed as [`Self::thought_open`] says,
    /// when that isn't how it is drawn.
    fn redraw_thought(&mut self, id: EntryId) -> Option<Splice> {
        let open = self.thought_open(id);
        let range = self.entry_blocks(id);
        let block = self.blocks.get_mut(range.start).filter(|_| range.len() == 1)?;
        let BlockBody::Thought { open: drawn, .. } = &mut block.body else { return None };
        if *drawn == open {
            return None;
        }
        *drawn = open;
        Some(Splice { range, count: 1 })
    }

    /// The thought trigger: opens or closes thought `id`.
    pub fn toggle_thought(&mut self, id: EntryId) -> Vec<Splice> {
        let open = !self.thought_open(id);
        self.toggled.insert(id, open);
        self.redraw_thought(id).into_iter().collect()
    }

    /// `toggle_agent_thoughts`: opens every thought when one is closed,
    /// else closes them all.
    pub fn toggle_thoughts(&mut self) -> Vec<Splice> {
        let ids: Vec<EntryId> =
            self.entries.iter().filter(|(_, e)| matches!(e, Entry::Thought { .. })).map(|(id, _)| *id).collect();
        let open = ids.iter().any(|id| !self.thought_open(*id));
        ids.into_iter()
            .filter_map(|id| {
                self.toggled.insert(id, open);
                self.redraw_thought(id)
            })
            .collect()
    }

    /// The session's start failed.
    pub fn not_started(&mut self, failure: StartFailure) -> Absorbed {
        self.life_event(|model| model.phase = AgentPhase::NotStarted(failure))
    }

    /// The session ended on its server (`EXIT`): the agent state shown
    /// stays the last folded.
    pub fn ended(&mut self) -> Absorbed {
        self.life_event(|model| model.life = SessionLife::Ended)
    }

    /// The session can't be got back.
    pub fn lost(&mut self, reason: impl Into<String>) -> Absorbed {
        let reason = reason.into();
        self.life_event(|model| model.phase = AgentPhase::Lost { reason })
    }

    /// Brindle can't follow the session any more.
    pub fn broken(&mut self, reason: impl Into<String>) -> Absorbed {
        let reason = reason.into();
        self.life_event(|model| model.phase = AgentPhase::Broken { reason })
    }

    /// The link to the endpoint went down or came back.
    pub fn set_link(&mut self, link: Link) -> Absorbed {
        self.life_event(|model| model.link = link)
    }

    fn life_event(&mut self, change: impl FnOnce(&mut Self)) -> Absorbed {
        let before = self.before();
        change(self);
        self.finish(before, Vec::new(), false)
    }

    fn finish(&mut self, before: Before, splices: Vec<Splice>, from_replay: bool) -> Absorbed {
        self.queue = queue_row(&self.slot, &self.phase, self.life);
        let status = self.shown_status();
        let changed = status != before.status;
        Absorbed {
            splices,
            status: changed.then_some(status),
            notice: if changed { self.notice(before.status, status, from_replay) } else { None },
            title_changed: self.title() != before.title,
            queue_changed: self.queue != before.queue,
            first: self.settle_first(from_replay),
        }
    }

    /// The launch's first prompt: sent on the first live `Ready`, never
    /// from a replay; left as the draft on a failure, or when a replay is
    /// what shows the agent ready.
    fn settle_first(&mut self, from_replay: bool) -> Option<First> {
        let first = self.first.as_ref()?;
        let draft = || Some(First::Draft(first.text().to_string()));
        let now = match (&self.phase, self.life) {
            (_, SessionLife::Ended) => draft(),
            (AgentPhase::Live(AgentState::Ready { .. }), SessionLife::Open) => match (from_replay, self.link) {
                (true, Link::Up | Link::Reconnecting) => draft(),
                (false, Link::Up) => Some(First::Send(first.text().to_string())),
                (false, Link::Reconnecting) => None,
            },
            (AgentPhase::Connecting | AgentPhase::Live(AgentState::Starting), SessionLife::Open) => None,
            (
                AgentPhase::NotStarted(_)
                | AgentPhase::Live(AgentState::LoginRequired { .. } | AgentState::Failed { .. } | AgentState::Exited { .. })
                | AgentPhase::Lost { .. }
                | AgentPhase::Broken { .. },
                SessionLife::Open,
            ) => draft(),
        };
        if now.is_some() {
            self.first = None;
        }
        now
    }

    /// What the prompt input's button is, and whether it takes prompts.
    pub fn prompt_status(&self) -> PromptStatus {
        let disabled = |why| PromptStatus::Disabled(why);
        match &self.phase {
            AgentPhase::Connecting => disabled(DisabledWhy::Connecting),
            AgentPhase::NotStarted(_) => disabled(DisabledWhy::NotStarted),
            AgentPhase::Lost { .. } => disabled(DisabledWhy::Lost),
            AgentPhase::Broken { .. } => disabled(DisabledWhy::Broken),
            AgentPhase::Live(AgentState::Starting) => disabled(DisabledWhy::Starting),
            AgentPhase::Live(AgentState::LoginRequired { .. }) => disabled(DisabledWhy::LoginRequired),
            AgentPhase::Live(AgentState::Failed { .. }) => disabled(DisabledWhy::Failed),
            AgentPhase::Live(AgentState::Exited { .. }) => disabled(DisabledWhy::Exited),
            AgentPhase::Live(AgentState::Ready { .. }) => match (self.life, self.link) {
                (SessionLife::Ended, _) => disabled(DisabledWhy::Exited),
                (SessionLife::Open, Link::Reconnecting) => disabled(DisabledWhy::Reconnecting),
                (SessionLife::Open, Link::Up) if self.in_flight() => PromptStatus::Submitted,
                (SessionLife::Open, Link::Up) if running(&self.turn) => PromptStatus::Streaming,
                (SessionLife::Open, Link::Up) => PromptStatus::Ready,
            },
        }
    }

    fn in_flight(&self) -> bool {
        self.pending.values().any(|sent| matches!(sent, Sent::Prompt | Sent::FirstPrompt(_)))
    }

    /// Enter in the prompt input with `draft`, `composing` when an IME
    /// composition is open. During a turn with the slot empty it sends (the
    /// server queues it); while a turn runs with the slot taken it keeps
    /// the text with the reason; with no turn, a held prompt doesn't block.
    pub fn submit(&mut self, draft: &str, composing: bool) -> Submit {
        let why = if composing {
            Some(Why::Composing)
        } else if draft.trim().is_empty() {
            Some(Why::Blank)
        } else {
            match self.prompt_status() {
                PromptStatus::Disabled(why) => Some(Why::NotReady(why)),
                PromptStatus::Submitted => Some(Why::InFlight),
                PromptStatus::Streaming => match &self.slot {
                    Slot::Empty => None,
                    Slot::Waiting { .. } | Slot::Held { .. } => Some(Why::AlreadyQueued),
                },
                PromptStatus::Ready => None,
            }
        };
        match why {
            Some(why) => {
                if why == Why::AlreadyQueued {
                    self.refusal = Some(ALREADY_QUEUED.into());
                }
                Submit::Nothing(why)
            }
            None => {
                self.refusal = None;
                Submit::Send(draft.to_string())
            }
        }
    }

    /// A request went out as `request`.
    pub fn sent(&mut self, request: RequestId, sent: Sent) {
        self.pending.insert(request, sent);
    }

    /// The server answered `request`: a prompt accepted empties the input;
    /// a refusal is shown beside it, and a refused first prompt becomes
    /// the draft.
    pub fn replied(&mut self, request: RequestId, outcome: Result<(), String>) -> Replied {
        let Some(sent) = self.pending.remove(&request) else { return Replied::Nothing };
        self.answered(sent, outcome)
    }

    /// A request [`Self::submit`] or the queue's or a permission's
    /// decision asked for couldn't go (the session isn't connected, or the
    /// link fell between the decision and the send): answered as a refusal
    /// with [`UNSENT`], so a first prompt becomes the draft.
    pub fn unsent(&mut self, sent: Sent) -> Replied {
        self.answered(sent, Err(UNSENT.into()))
    }

    /// The terminal server's word about the session that answers no request
    /// (a refused cancel): shown beside the prompt input, as a refusal.
    pub fn server_note(&mut self, text: impl Into<String>) {
        self.refusal = Some(text.into());
    }

    /// Whether the zone's session is one a quit leaves for the next launch
    /// to attach (design.md decision 5's `zone_records`): not once it has
    /// ended, nor when it never started, was lost or can't be followed.
    pub fn keeps_session(&self) -> bool {
        let phase = match &self.phase {
            AgentPhase::Connecting | AgentPhase::Live(_) => true,
            AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => false,
        };
        phase && self.life == SessionLife::Open
    }

    fn answered(&mut self, sent: Sent, outcome: Result<(), String>) -> Replied {
        match (sent, outcome) {
            (Sent::Prompt, Ok(())) => {
                self.refusal = None;
                Replied::ClearDraft
            }
            (Sent::FirstPrompt(_) | Sent::Resend | Sent::Unqueue | Sent::Answer(_), Ok(())) => Replied::Nothing,
            (Sent::FirstPrompt(text), Err(why)) => {
                self.refusal = Some(why);
                Replied::SetDraft(text)
            }
            (Sent::Prompt | Sent::Resend | Sent::Unqueue | Sent::Answer(_), Err(why)) => {
                self.refusal = Some(why);
                Replied::Nothing
            }
        }
    }

    /// Whether anything can be sent to the session now.
    fn can_send(&self) -> bool {
        let live = match &self.phase {
            AgentPhase::Live(_) => true,
            AgentPhase::Connecting | AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => false,
        };
        live && self.life == SessionLife::Open && self.link == Link::Up
    }

    /// The queue row's remove, or a held prompt's discard
    /// (`agent_unqueue`). After the session ended or was lost the row is
    /// only cleared here.
    pub fn unqueue(&mut self) -> Unqueue {
        match &self.slot {
            Slot::Empty => return Unqueue::Nothing,
            Slot::Waiting { .. } | Slot::Held { .. } => {}
        }
        let gone = self.life == SessionLife::Ended || matches!(self.phase, AgentPhase::Lost { .. });
        if gone {
            self.slot = Slot::Empty;
            self.queue = QueueRow::None;
            Unqueue::Cleared
        } else if self.can_send() {
            Unqueue::Send
        } else {
            Unqueue::Nothing
        }
    }

    /// Whether the held prompt can be sent again now (`agent_resend`).
    pub fn resend(&self) -> bool {
        matches!(&self.queue, QueueRow::Held { resend: Resend::Available, .. }) && self.can_send()
    }

    /// Whether a stop (escape, the stop button, `agent_cancel`) has a turn
    /// to stop.
    pub fn cancel(&self) -> bool {
        let turn = match &self.turn {
            TurnState::Working | TurnState::Blocked => true,
            TurnState::Idle | TurnState::Cancelling | TurnState::Ended { .. } | TurnState::Failed { .. } => false,
        };
        turn && self.can_send()
    }

    /// Whether choosing `option` answers permission request `entry`: it
    /// must be pending and offer that option.
    pub fn answer(&self, entry: EntryId, option: &str) -> bool {
        let pending = self.entries.binary_search_by_key(&entry, |(id, _)| *id).ok().is_some_and(|index| {
            match &self.entries[index].1 {
                Entry::Permission { options, outcome: None, .. } => options.iter().any(|o| o.id == option),
                Entry::Permission { outcome: Some(_), .. }
                | Entry::User { .. }
                | Entry::Agent { .. }
                | Entry::Thought { .. }
                | Entry::Tool { .. }
                | Entry::Notice { .. } => false,
            }
        });
        pending && self.can_send()
    }

    /// The pending permission requests' buttons, in order, for the zone's
    /// focus stops.
    pub fn pending_permissions(&self) -> Vec<(EntryId, Vec<SharedString>)> {
        self.blocks
            .iter()
            .filter_map(|block| match &block.body {
                BlockBody::Permission { options, outcome: None, .. } => {
                    Some((block.entry, options.iter().map(|o| o.id.clone()).collect()))
                }
                BlockBody::Permission { outcome: Some(_), .. }
                | BlockBody::UserText { .. }
                | BlockBody::AgentPart { .. }
                | BlockBody::Thought { .. }
                | BlockBody::Tool { .. }
                | BlockBody::Notice { .. } => None,
            })
            .collect()
    }

    /// `copy_last_agent_message`: the latest agent message's text.
    pub fn last_agent_message(&self) -> Option<&str> {
        self.entries.iter().rev().find_map(|(_, entry)| match entry {
            Entry::Agent { text } => Some(text.as_str()),
            Entry::User { .. } | Entry::Thought { .. } | Entry::Tool { .. } | Entry::Permission { .. } | Entry::Notice { .. } => {
                None
            }
        })
    }

    /// `copy_last_code_block`: the last code block's text, at any depth,
    /// of the latest agent message that has one (thoughts don't count).
    // The action that uses it comes with the agent view's rich text
    // (`agents-markdown` task 4.2).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn last_code_block(&self) -> Option<&str> {
        self.entries.iter().rev().filter(|(_, entry)| matches!(entry, Entry::Agent { .. })).find_map(|(id, _)| {
            let parts = self.rendered.get(id)?;
            parts.iter().rev().find_map(|part| markdown::last_code(&part.block)).map(|text| text.as_ref())
        })
    }

    /// An entry's whole text, for a message's copy button.
    pub fn message_text(&self, id: EntryId) -> Option<&str> {
        let index = self.entries.binary_search_by_key(&id, |(own, _)| *own).ok()?;
        match &self.entries[index].1 {
            Entry::Agent { text } | Entry::User { text } => Some(text),
            Entry::Thought { .. } | Entry::Tool { .. } | Entry::Permission { .. } | Entry::Notice { .. } => None,
        }
    }

    /// `copy_queued_prompt`: the slot's text, waiting or held.
    pub fn queued_text(&self) -> Option<&str> {
        match &self.slot {
            Slot::Empty => None,
            Slot::Waiting { text } | Slot::Held { text, .. } => Some(text),
        }
    }

    /// The reconnecting banner over the conversation.
    pub fn banner(&self, endpoint: &str) -> Option<String> {
        match self.link {
            Link::Up => None,
            Link::Reconnecting => Some(format!("reconnecting to {endpoint}…")),
        }
    }

    /// The alert for the zone's phase, if any ([`failure_text`]).
    pub fn alert(&self, command: &str, endpoint: &str) -> Option<Alert> {
        let agent = self.title();
        failure_text(&self.phase, self.life, Names { agent: &agent, command, endpoint })
    }

    /// The words an `error` notification gives.
    fn error_text(&self) -> String {
        match &self.phase {
            AgentPhase::Live(AgentState::LoginRequired { message } | AgentState::Failed { message }) => message.clone(),
            AgentPhase::Live(AgentState::Exited { code: Some(code), .. }) => format!("exited with status {code}"),
            AgentPhase::Live(AgentState::Exited { code: None, .. }) => "ended by a signal".into(),
            AgentPhase::Live(AgentState::Starting | AgentState::Ready { .. }) => match &self.turn {
                TurnState::Failed { message } => message.clone(),
                TurnState::Ended { stop } => format!("the turn ended: {}", stop_word(stop)),
                TurnState::Idle | TurnState::Working | TurnState::Blocked | TurnState::Cancelling => "error".into(),
            },
            AgentPhase::Connecting | AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => {
                "error".into()
            }
        }
    }

    /// The latest pending permission request's title.
    fn pending_title(&self) -> Option<&str> {
        self.entries.iter().rev().find_map(|(_, entry)| match entry {
            Entry::Permission { title, outcome: None, .. } => Some(title.as_str()),
            Entry::Permission { outcome: Some(_), .. }
            | Entry::User { .. }
            | Entry::Agent { .. }
            | Entry::Thought { .. }
            | Entry::Tool { .. }
            | Entry::Notice { .. } => None,
        })
    }

    /// The notification for a shown status going from `prev` to `next`:
    /// on becoming `blocked`, `done` or `error` from another, never while
    /// replaying. Titled with the agent's name (else the profile's); the
    /// body is the permission's title, `Done`, or the failure's words.
    pub fn notice(&self, prev: Option<State>, next: Option<State>, replaying: bool) -> Option<AgentNotice> {
        if replaying || prev == next {
            return None;
        }
        let body = match next? {
            State::Blocked => self.pending_title().unwrap_or("Waiting for permission").to_string(),
            State::Done => "Done".to_string(),
            State::Error => self.error_text(),
            State::Idle | State::Working => return None,
        };
        Some(AgentNotice { title: self.title(), body })
    }

    /// The dump's lines for this zone (decision 15), zone `zone`, with the
    /// conversation `at_end` or away from it; each link's action from what
    /// `plan` says of its target (`links::plan_here` in Brindle).
    pub fn describe(&self, zone: u64, at_end: bool, plan: &dyn Fn(&str) -> Plan) -> String {
        self.lines(Some(zone), Some(at_end), plan)
    }

    /// What `--wait-for` matches in an agent zone: the dump's lines,
    /// without the zone's id and the scroll position.
    pub fn wait_text(&self, plan: &dyn Fn(&str) -> Plan) -> String {
        self.lines(None, None, plan)
    }

    /// The `--- block` line of each of entry `id`'s rendered parts, in
    /// order, each followed by a `--- link` line per link in it.
    fn part_lines(&self, id: EntryId, plan: &dyn Fn(&str) -> Plan) -> String {
        let mut out = String::new();
        for (index, part) in self.rendered.get(&id).into_iter().flatten().enumerate() {
            let (kind, text) = (markdown::kind_word(&part.block), markdown::shown_text(&part.block));
            out.push_str(&format!("--- block entry={id} part={index} kind={kind} {text:?}\n"));
            for (link, text) in markdown::block_links(&part.block) {
                let action = markdown::link_action(&link.target, plan(&link.target));
                out.push_str(&format!(
                    "--- link entry={id} part={index} target={:?} text={text:?} action={}\n",
                    link.target.as_ref(),
                    markdown::action_word(&action)
                ));
            }
        }
        out
    }

    fn lines(&self, zone: Option<u64>, at_end: Option<bool>, plan: &dyn Fn(&str) -> Plan) -> String {
        let mut out = String::from("--- agent");
        if let Some(zone) = zone {
            out.push_str(&format!(" zone={zone}"));
        }
        out.push_str(&format!(" state={}", state_word(&self.phase)));
        match &self.phase {
            AgentPhase::Live(AgentState::LoginRequired { message } | AgentState::Failed { message }) => {
                out.push_str(&format!(" message={message:?}"));
            }
            // The failure as logged, the server's cause verbatim (on
            // Windows a batch file's `cmd /c` form included).
            AgentPhase::NotStarted(failure) => out.push_str(&format!(" message={:?}", failure.to_string())),
            AgentPhase::Live(AgentState::Starting | AgentState::Ready { .. } | AgentState::Exited { .. })
            | AgentPhase::Connecting
            | AgentPhase::Lost { .. }
            | AgentPhase::Broken { .. } => {}
        }
        if let AgentPhase::Live(AgentState::Ready { name: Some(name), .. }) = &self.phase {
            out.push_str(&format!(" name={name:?}"));
        }
        let live = matches!(self.phase, AgentPhase::Live(_));
        let activity = if live { status_of(self.activity).name() } else { "none" };
        let status = self.shown_status().map_or("none", State::name);
        let link = match self.link {
            Link::Up => "up",
            Link::Reconnecting => "reconnecting",
        };
        let session = match self.life {
            SessionLife::Open => "open",
            SessionLife::Ended => "ended",
        };
        out.push_str(&format!(
            " turn={} activity={activity} status={status} link={link} session={session}\n",
            turn_word(&self.turn)
        ));
        if let Some(note) = &self.note {
            out.push_str(&format!("--- note {note:?}\n"));
        }
        for (id, entry) in &self.entries {
            out.push_str(&format!("--- entry {id} {}\n", self.entry_line(*id, entry)));
            out.push_str(&self.part_lines(*id, plan));
        }
        match &self.queue {
            QueueRow::None => {}
            QueueRow::Waiting { text } => out.push_str(&format!("--- queued waiting {:?}\n", text.as_ref())),
            QueueRow::Held { text, reason, resend, .. } => {
                let resend = match resend {
                    Resend::Available => "yes",
                    Resend::Unavailable(_) => "no",
                };
                out.push_str(&format!("--- queued held reason={} resend={resend} {:?}\n", hold_word(reason), text.as_ref()));
            }
        }
        out.push_str(&format!("--- conversation entries={}", self.entries.len()));
        if let Some(at_end) = at_end {
            out.push_str(if at_end { " at=end" } else { " at=away" });
        }
        out.push_str(&format!("\n--- prompt status={}", self.prompt_status().word()));
        if let Some(refusal) = &self.refusal {
            out.push_str(&format!(" refusal={refusal:?}"));
        }
        out.push('\n');
        out
    }

    fn entry_line(&self, id: EntryId, entry: &Entry) -> String {
        match entry {
            Entry::User { text } => format!("user {text:?}"),
            Entry::Agent { text } => format!("agent {text:?}"),
            Entry::Thought { text } => format!("thought open={} {text:?}", self.thought_open(id)),
            Entry::Tool { call, title, kind, status } => {
                format!("tool call={call:?} kind={} status={} {title:?}", tool_kind_word(*kind), tool_status_word(*status))
            }
            Entry::Permission { call, title, options, outcome } => {
                let call = call.as_ref().map(|call| format!(" call={call:?}")).unwrap_or_default();
                let options: Vec<String> =
                    buttons(options).iter().map(|o| format!("{}:{:?}", o.id, o.name.as_ref())).collect();
                let outcome = match outcome {
                    None => "pending".to_string(),
                    Some(PermissionOutcome::Selected { option }) => format!("selected:{option}"),
                    Some(PermissionOutcome::Cancelled) => "cancelled".to_string(),
                };
                format!("permission{call} options={} outcome={outcome} {title:?}", options.join("|"))
            }
            Entry::Notice { level, text } => {
                let level = match level {
                    NoticeLevel::Info => "info",
                    NoticeLevel::Warning => "warn",
                    NoticeLevel::Error => "error",
                };
                format!("notice level={level} {text:?}")
            }
        }
    }
}

fn state_word(phase: &AgentPhase) -> String {
    match phase {
        AgentPhase::Connecting => "connecting".into(),
        AgentPhase::NotStarted(_) => "not-started".into(),
        AgentPhase::Live(AgentState::Starting) => "starting".into(),
        AgentPhase::Live(AgentState::Ready { .. }) => "ready".into(),
        AgentPhase::Live(AgentState::LoginRequired { .. }) => "login-required".into(),
        AgentPhase::Live(AgentState::Failed { .. }) => "failed".into(),
        AgentPhase::Live(AgentState::Exited { code: Some(code), .. }) => format!("exited:{code}"),
        AgentPhase::Live(AgentState::Exited { code: None, .. }) => "exited:signal".into(),
        AgentPhase::Lost { .. } => "lost".into(),
        AgentPhase::Broken { .. } => "broken".into(),
    }
}

fn turn_word(turn: &TurnState) -> String {
    match turn {
        TurnState::Idle => "idle".into(),
        TurnState::Working => "working".into(),
        TurnState::Blocked => "blocked".into(),
        TurnState::Cancelling => "cancelling".into(),
        TurnState::Ended { stop } => format!("ended:{}", stop_word(stop)),
        TurnState::Failed { .. } => "failed".into(),
    }
}

fn tool_kind_word(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "read",
        ToolKind::Edit => "edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "execute",
        ToolKind::Think => "think",
        ToolKind::Fetch => "fetch",
        ToolKind::SwitchMode => "switch_mode",
        ToolKind::Other => "other",
    }
}

fn tool_status_word(status: ToolStatus) -> &'static str {
    match status {
        ToolStatus::Pending => "pending",
        ToolStatus::InProgress => "in_progress",
        ToolStatus::Completed => "completed",
        ToolStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::shared::AgentShared;
    use crate::agent::transcript::Event;

    /// A zone's model fed through the fold, as the view feeds it.
    struct Zone {
        shared: AgentShared,
        model: ChatModel,
    }

    impl Zone {
        fn new(first: Option<&str>) -> Self {
            let first = first.map(|text| FirstPrompt::new(text.to_string()).unwrap());
            Self { shared: AgentShared::new(), model: ChatModel::new("Fake", first) }
        }

        fn feed(&mut self, events: &[Event]) -> Absorbed {
            let values: Vec<_> = events.iter().map(Event::to_value).collect();
            self.shared.apply(&values).unwrap();
            self.absorb()
        }

        fn absorb(&mut self) -> Absorbed {
            let changes = self.shared.take_changes();
            self.model.absorb(self.shared.transcript(), changes)
        }

        fn replay(&mut self, events: &[Event]) -> Absorbed {
            self.shared.replay_start();
            let values: Vec<_> = events.iter().map(Event::to_value).collect();
            self.shared.apply(&values).unwrap();
            self.shared.replay_end();
            self.absorb()
        }

        /// A started zone: the agent ready, named "Fake Agent".
        fn ready() -> Self {
            let mut zone = Self::new(None);
            zone.feed(&[ready()]);
            zone
        }
    }

    fn ready() -> Event {
        Event::Agent { state: AgentState::Ready { name: Some("Fake Agent".into()), version: None } }
    }

    fn agent_state(state: AgentState) -> Event {
        Event::Agent { state }
    }

    fn turn(state: TurnState) -> Event {
        Event::Turn { state }
    }

    fn queue(slot: Slot) -> Event {
        Event::Queue { slot }
    }

    fn added(id: u64, entry: Entry) -> Event {
        Event::Added { id: EntryId(id), entry }
    }

    fn user(text: &str) -> Entry {
        Entry::User { text: text.into() }
    }

    fn agent(text: &str) -> Entry {
        Entry::Agent { text: text.into() }
    }

    fn thought(text: &str) -> Entry {
        Entry::Thought { text: text.into() }
    }

    fn option(id: &str, name: &str, kind: PermissionKind) -> PermissionOption {
        PermissionOption { id: id.into(), name: name.into(), kind }
    }

    fn permission(title: &str) -> Entry {
        Entry::Permission {
            call: Some("call_2".into()),
            title: title.into(),
            options: vec![
                option("reject-once", "Reject", PermissionKind::RejectOnce),
                option("allow-once", "Allow once", PermissionKind::AllowOnce),
            ],
            outcome: None,
        }
    }

    fn waiting() -> Slot {
        Slot::Waiting { text: "next".into() }
    }

    fn held(reason: HoldReason) -> Slot {
        Slot::Held { text: "next".into(), reason }
    }

    fn texts(model: &ChatModel) -> Vec<String> {
        model
            .blocks()
            .iter()
            .map(|block| match &block.body {
                BlockBody::UserText { text } | BlockBody::Notice { text, .. } => text.to_string(),
                BlockBody::AgentPart { block } => markdown::shown_text(block),
                BlockBody::Thought { blocks, .. } => {
                    blocks.iter().map(|block| markdown::shown_text(block)).collect::<Vec<_>>().join("\n\n")
                }
                BlockBody::Tool { title, .. } | BlockBody::Permission { title, .. } => title.to_string(),
            })
            .collect()
    }

    /// The plan Brindle would make on a Linux machine named `thishost`.
    fn linux(uri: &str) -> Plan {
        crate::terminal::links::plan(uri, "thishost", false)
    }

    /// An entry's blocks, rendered as the model renders them.
    fn drawn(id: EntryId, entry: &Entry, open: bool) -> Vec<Block> {
        let rendered = render_entry(entry).map(|(parts, _)| parts).unwrap_or_default();
        blocks_of(id, entry, &rendered, open)
    }

    const EVERY_HOLD: [HoldReason; 9] = [
        HoldReason::Cancelled,
        HoldReason::Failed,
        HoldReason::Stopped { stop: StopReason::MaxTokens },
        HoldReason::Stopped { stop: StopReason::MaxTurnRequests },
        HoldReason::Stopped { stop: StopReason::Refusal },
        HoldReason::Stopped { stop: StopReason::Cancelled },
        HoldReason::LoginRequired,
        HoldReason::AgentStopped,
        HoldReason::OutputLost,
    ];

    fn every_phase() -> Vec<AgentPhase> {
        vec![
            AgentPhase::Connecting,
            AgentPhase::NotStarted(StartFailure::NoServer("no helper".into())),
            AgentPhase::NotStarted(StartFailure::TooOld { endpoint: "A8".into() }),
            AgentPhase::NotStarted(StartFailure::Refused { command: "fake".into(), cause: "not found".into() }),
            AgentPhase::Live(AgentState::Starting),
            AgentPhase::Live(AgentState::Ready { name: None, version: None }),
            AgentPhase::Live(AgentState::LoginRequired { message: "log in".into() }),
            AgentPhase::Live(AgentState::Failed { message: "bad".into() }),
            AgentPhase::Live(AgentState::Exited { code: Some(1), stderr: "boom".into() }),
            AgentPhase::Lost { reason: "gone".into() },
            AgentPhase::Broken { reason: "bad event".into() },
        ]
    }

    #[test]
    fn agent_sessions_need_client_protocol_4() {
        assert_eq!(agent_start(4, "this machine"), Ok(()));
        assert_eq!(agent_start(5, "this machine"), Ok(()));
        for old in [1, 2, 3] {
            assert_eq!(agent_start(old, "A8"), Err(StartFailure::TooOld { endpoint: "A8".into() }), "protocol {old}");
        }
        assert_eq!(
            too_old_text("A8"),
            "the terminal server on A8 is too old for agent sessions; it will update once its terminals have ended"
        );
    }

    /// Decision 8: a message splits at blank lines and, past 4 KiB, at the
    /// next line break; appending changes only the last part.
    #[test]
    fn a_message_splits_at_blank_lines_and_past_4_kib() {
        let text = "one\nline\n\n\ntwo\n\nthree";
        let cut: Vec<&str> = parts(text, 0).into_iter().map(|r| &text[r]).collect();
        assert_eq!(cut, ["one\nline", "two", "three"]);
        let empty = Range { start: 0, end: 0 };
        assert_eq!(parts("", 0), vec![empty.clone()]);
        assert_eq!(parts("\n\n", 0), vec![empty]);

        let long = format!("{}\nrest\nmore", "x".repeat(BLOCK_LIMIT + 10));
        let cut: Vec<&str> = parts(&long, 0).into_iter().map(|r| &long[r]).collect();
        assert_eq!(cut, ["x".repeat(BLOCK_LIMIT + 10).as_str(), "rest\nmore"]);
        let short = format!("{}\nrest", "x".repeat(BLOCK_LIMIT - 10));
        assert_eq!(parts(&short, 0).len(), 1, "under the limit a line break doesn't cut");

        // Every prefix's parts but its last are the whole text's.
        let whole = format!("a\n\nb{}\nc\n\n\nd", "é".repeat(BLOCK_LIMIT));
        let full = parts(&whole, 0);
        for end in (0..=whole.len()).filter(|end| whole.is_char_boundary(*end)) {
            let prefix = parts(&whole[..end], 0);
            assert_eq!(prefix[..prefix.len() - 1], full[..prefix.len() - 1], "prefix of {end} bytes");
        }
    }

    /// Decision 8: each entry kind's block, a permission's allow kinds
    /// first, keyed by entry and option id.
    #[test]
    fn each_entry_is_drawn_as_its_blocks() {
        let blocks = drawn(EntryId(5), &permission("Run cargo test?"), false);
        let [Block { entry: EntryId(5), part: 0, last: true, body: BlockBody::Permission { options, outcome: None, .. }, .. }] =
            blocks.as_slice()
        else {
            panic!("{blocks:?}")
        };
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_ref()).collect();
        assert_eq!(ids, ["allow-once", "reject-once"]);

        let message = drawn(EntryId(2), &agent("a\n\nb"), false);
        assert_eq!(message.iter().map(|b| (b.part, b.start, b.last)).collect::<Vec<_>>(), [(0, 0, false), (1, 3, true)]);
        assert!(matches!(&message[0].body, BlockBody::AgentPart { .. }));
        let message = drawn(EntryId(2), &user("a\n\nb"), false);
        assert!(matches!(&message[1].body, BlockBody::UserText { text } if text.as_ref() == "b"));
        assert_eq!(drawn(EntryId(1), &user(""), false).len(), 1, "an empty message is one block");
        assert_eq!(drawn(EntryId(1), &agent(""), false).len(), 1, "an empty message is one block");
        let tool = Entry::Tool { call: "c".into(), title: "Read".into(), kind: ToolKind::Read, status: ToolStatus::Pending };
        assert!(matches!(drawn(EntryId(3), &tool, false)[0].body, BlockBody::Tool { kind: ToolKind::Read, .. }));
        assert!(matches!(drawn(EntryId(4), &thought("t"), true)[0].body, BlockBody::Thought { open: true, .. }));
    }

    /// Decision 8: new entries splice in at the end, a streaming message
    /// redraws only its last block, a trim drops blocks and a replay starts
    /// the list over.
    #[test]
    fn absorbing_changes_splices_only_what_changed() {
        let mut zone = Zone::ready();
        let absorbed = zone.feed(&[added(1, user("hi")), added(2, agent("Hel"))]);
        assert_eq!(absorbed.splices, [Splice { range: 0..0, count: 2 }]);

        let absorbed = zone.feed(&[Event::Appended { id: EntryId(2), text: "lo\n\nthere".into() }]);
        assert_eq!(absorbed.splices, [Splice { range: 1..2, count: 2 }]);
        assert_eq!(texts(&zone.model), ["hi", "Hello", "there"]);
        let absorbed = zone.feed(&[Event::Appended { id: EntryId(2), text: " you".into() }]);
        assert_eq!(absorbed.splices, [Splice { range: 2..3, count: 1 }], "only the last part");
        assert_eq!(texts(&zone.model), ["hi", "Hello", "there you"]);

        let absorbed = zone.feed(&[
            added(3, permission("Run?")),
            Event::Answered { id: EntryId(3), outcome: PermissionOutcome::Cancelled },
        ]);
        assert_eq!(absorbed.splices, [Splice { range: 3..3, count: 1 }], "a new entry's change is in its copy");
        let absorbed = zone.feed(&[Event::Trimmed { through: EntryId(1), dropped: 1 }]);
        assert_eq!(absorbed.splices, [Splice { range: 0..1, count: 0 }]);
        assert_eq!(texts(&zone.model), ["Hello", "there you", "Run?"]);

        let absorbed = zone.replay(&[added(1, user("again")), ready()]);
        assert_eq!(absorbed.splices, [Splice { range: 0..3, count: 1 }]);
        assert_eq!(texts(&zone.model), ["again"]);
    }

    /// Decision 3: the first prompt is sent once, on the first live `Ready`.
    #[test]
    fn the_first_prompt_is_sent_once_on_the_first_live_ready() {
        let mut zone = Zone::new(Some("run the tests"));
        assert_eq!(zone.feed(&[agent_state(AgentState::Starting)]).first, None, "not yet");
        assert_eq!(zone.feed(&[ready()]).first, Some(First::Send("run the tests".into())));
        assert_eq!(zone.feed(&[ready()]).first, None, "once");
        assert_eq!(zone.model.ended().first, None);

        let mut zone = Zone::new(Some("run the tests"));
        zone.model.set_link(Link::Reconnecting);
        assert_eq!(zone.feed(&[ready()]).first, None, "held back while reconnecting");
        assert_eq!(zone.model.set_link(Link::Up).first, Some(First::Send("run the tests".into())));

        let mut zone = Zone::new(None);
        assert_eq!(zone.feed(&[ready()]).first, None, "a restored or attached zone has none");
    }

    /// Decision 3: a replay never sends it; every failure leaves it as the
    /// draft.
    #[test]
    fn the_first_prompt_becomes_the_draft_on_a_failure_or_a_replay() {
        let draft = Some(First::Draft("run the tests".into()));
        let mut zone = Zone::new(Some("run the tests"));
        assert_eq!(zone.replay(&[ready()]).first, draft, "never sent from a replay");

        let failed = [
            AgentState::LoginRequired { message: "log in".into() },
            AgentState::Failed { message: "bad".into() },
            AgentState::Exited { code: Some(1), stderr: String::new() },
        ];
        for state in failed {
            let mut zone = Zone::new(Some("run the tests"));
            assert_eq!(zone.feed(&[agent_state(state.clone())]).first, draft, "{state:?}");
            assert_eq!(zone.feed(&[ready()]).first, None, "and never sent later");
        }
        let mut zone = Zone::new(Some("run the tests"));
        assert_eq!(zone.model.not_started(StartFailure::NoServer("no helper".into())).first, draft);
        let mut zone = Zone::new(Some("run the tests"));
        assert_eq!(zone.model.lost("gone").first, draft);
        let mut zone = Zone::new(Some("run the tests"));
        assert_eq!(zone.model.broken("bad event").first, draft);
        let mut zone = Zone::new(Some("run the tests"));
        zone.feed(&[agent_state(AgentState::Starting)]);
        assert_eq!(zone.model.ended().first, draft);

        // A refused send leaves it as the draft, with the refusal.
        let mut zone = Zone::new(Some("run the tests"));
        zone.feed(&[ready()]);
        zone.model.sent(7, Sent::FirstPrompt("run the tests".into()));
        assert_eq!(zone.model.prompt_status(), PromptStatus::Submitted);
        assert_eq!(zone.model.replied(7, Err("the agent isn't ready".into())), Replied::SetDraft("run the tests".into()));
        assert_eq!(zone.model.refusal(), Some("the agent isn't ready"));
    }

    /// Decisions 8 and 9: every slot and hold reason with each phase, and
    /// the session's life.
    #[test]
    fn every_slot_and_hold_reason_with_each_phase() {
        let words = [
            "you stopped the turn",
            "the turn failed",
            "the turn ended: max_tokens",
            "the turn ended: max_turn_requests",
            "the turn ended: refusal",
            "the turn ended: cancelled",
            "the agent needs a login",
            "the agent stopped",
            "output was lost while the terminal server restarted",
        ];
        for (reason, words) in EVERY_HOLD.iter().zip(words) {
            assert_eq!(hold_words(reason), words);
        }
        for phase in every_phase() {
            for life in [SessionLife::Open, SessionLife::Ended] {
                assert_eq!(queue_row(&Slot::Empty, &phase, life), QueueRow::None);
                assert_eq!(queue_row(&waiting(), &phase, life), QueueRow::Waiting { text: "next".into() });
                for reason in &EVERY_HOLD {
                    let QueueRow::Held { text, reason: shown, why, resend } = queue_row(&held(reason.clone()), &phase, life)
                    else {
                        panic!("{phase:?} {reason:?}")
                    };
                    assert_eq!((text.as_ref(), &shown, why.to_string()), ("next", reason, hold_words(reason)));
                    let ready = matches!(phase, AgentPhase::Live(AgentState::Ready { .. }));
                    let expected = match (ready, life) {
                        (true, SessionLife::Open) => Resend::Available,
                        (_, SessionLife::Ended) => Resend::Unavailable("the agent has exited".into()),
                        (false, SessionLife::Open) => resend_of(&phase, life),
                    };
                    assert_eq!(resend, expected, "{phase:?} {life:?}");
                    assert_ne!(resend == Resend::Available, !ready || life == SessionLife::Ended);
                }
            }
        }
        let login = AgentPhase::Live(AgentState::LoginRequired { message: "log in".into() });
        assert_eq!(
            resend_of(&login, SessionLife::Open),
            Resend::Unavailable("the agent needs a login; open the profile again after logging in".into())
        );
        assert_eq!(resend_of(&AgentPhase::Live(AgentState::Starting), SessionLife::Open), Resend::Unavailable("the agent is starting".into()));
        assert_eq!(
            resend_of(&AgentPhase::Live(AgentState::Failed { message: "x".into() }), SessionLife::Open),
            Resend::Unavailable("the agent failed".into())
        );
    }

    /// Decision 9: a new prompt beside a held one is sent with no turn and
    /// kept during one; a waiting one blocks it during a turn too.
    #[test]
    fn a_new_prompt_beside_the_slot() {
        let mut zone = Zone::ready();
        zone.feed(&[queue(held(HoldReason::Cancelled))]);
        assert_eq!(zone.model.submit("again", false), Submit::Send("again".into()), "no turn: held doesn't block");
        zone.feed(&[turn(TurnState::Working)]);
        assert_eq!(zone.model.prompt_status(), PromptStatus::Streaming);
        assert_eq!(zone.model.submit("again", false), Submit::Nothing(Why::AlreadyQueued));
        assert_eq!(zone.model.refusal(), Some(ALREADY_QUEUED));
        zone.feed(&[queue(waiting())]);
        assert_eq!(zone.model.submit("again", false), Submit::Nothing(Why::AlreadyQueued));
        zone.feed(&[queue(Slot::Empty)]);
        assert_eq!(zone.model.submit("again", false), Submit::Send("again".into()), "the server queues it");
        assert_eq!(zone.model.refusal(), None);

        assert_eq!(zone.model.submit("again", true), Submit::Nothing(Why::Composing));
        assert_eq!(zone.model.submit(" \n", false), Submit::Nothing(Why::Blank));
        zone.model.sent(1, Sent::Prompt);
        assert_eq!(zone.model.submit("again", false), Submit::Nothing(Why::InFlight));
        assert_eq!(zone.model.replied(1, Ok(())), Replied::ClearDraft);
        zone.model.sent(2, Sent::Prompt);
        assert_eq!(zone.model.replied(2, Err("no".into())), Replied::Nothing, "a refused prompt keeps the draft");
        assert_eq!(zone.model.refusal(), Some("no"));
        assert_eq!(zone.model.replied(2, Ok(())), Replied::Nothing, "answered once");
        zone.model.set_link(Link::Reconnecting);
        assert_eq!(zone.model.submit("again", false), Submit::Nothing(Why::NotReady(DisabledWhy::Reconnecting)));
        assert_eq!(Zone::new(None).model.submit("hi", false), Submit::Nothing(Why::NotReady(DisabledWhy::Connecting)));
    }

    /// Decision 9: no slot change touches the prompt input's draft.
    #[test]
    fn no_slot_change_alters_the_draft() {
        let mut zone = Zone::new(Some("first"));
        assert_eq!(zone.feed(&[ready()]).first, Some(First::Send("first".into())));
        let mut slots = vec![Slot::Empty, waiting()];
        slots.extend(EVERY_HOLD.iter().cloned().map(held));
        for slot in slots.iter().chain(slots.iter().rev()) {
            let absorbed = zone.feed(&[queue(slot.clone())]);
            assert_eq!(absorbed.first, None, "{slot:?}");
            assert!(absorbed.splices.is_empty() && absorbed.status.is_none(), "{slot:?}");
        }
        zone.model.sent(3, Sent::Unqueue);
        zone.model.sent(4, Sent::Resend);
        assert_eq!(zone.model.replied(3, Ok(())), Replied::Nothing);
        assert_eq!(zone.model.replied(4, Err("no prompt is held".into())), Replied::Nothing);
    }

    /// Decision 9: unqueue and resend send requests while the session is
    /// open, and after it ended (or was lost) only clear the row here.
    #[test]
    fn unqueue_and_resend() {
        let mut zone = Zone::ready();
        assert_eq!(zone.model.unqueue(), Unqueue::Nothing);
        assert!(!zone.model.resend());
        zone.feed(&[queue(waiting())]);
        assert_eq!(zone.model.unqueue(), Unqueue::Send);
        assert!(!zone.model.resend(), "only a held prompt is sent again");
        let absorbed = zone.feed(&[queue(held(HoldReason::Failed))]);
        assert!(absorbed.queue_changed);
        assert!(zone.model.resend());
        assert_eq!(zone.model.unqueue(), Unqueue::Send);
        assert_eq!(zone.model.queued_text(), Some("next"));

        zone.feed(&[agent_state(AgentState::Exited { code: Some(0), stderr: String::new() })]);
        zone.model.ended();
        let QueueRow::Held { resend, .. } = zone.model.queue() else { panic!("{:?}", zone.model.queue()) };
        assert_eq!(resend, &Resend::Unavailable("the agent has exited".into()), "the zone keeps its copy");
        assert!(!zone.model.resend());
        assert_eq!(zone.model.unqueue(), Unqueue::Cleared);
        assert_eq!(zone.model.queue(), &QueueRow::None);
        assert_eq!(zone.model.queued_text(), None);

        let mut zone = Zone::ready();
        zone.feed(&[queue(held(HoldReason::AgentStopped))]);
        zone.model.lost("gone");
        assert_eq!(zone.model.unqueue(), Unqueue::Cleared);
        let mut zone = Zone::ready();
        zone.feed(&[queue(waiting())]);
        zone.model.set_link(Link::Reconnecting);
        assert_eq!(zone.model.unqueue(), Unqueue::Nothing, "nothing is sent while reconnecting");
    }

    /// Decision 9 and `agents-sessions` decision 17: a login demanded by a
    /// prompt.
    #[test]
    fn a_login_demanded_by_a_prompt() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("hi")), turn(TurnState::Working), added(2, agent("Sure")), queue(waiting())]);
        let message = "Authentication required";
        let absorbed = zone.feed(&[
            queue(held(HoldReason::LoginRequired)),
            turn(TurnState::Failed { message: message.into() }),
            agent_state(AgentState::LoginRequired { message: message.into() }),
        ]);
        assert_eq!(absorbed.status, Some(Some(State::Error)));
        assert_eq!(zone.model.turn(), &TurnState::Failed { message: message.into() });
        let QueueRow::Held { why, resend, .. } = zone.model.queue() else { panic!("{:?}", zone.model.queue()) };
        assert_eq!(why.as_ref(), "the agent needs a login");
        assert_eq!(
            resend,
            &Resend::Unavailable("the agent needs a login; open the profile again after logging in".into())
        );
        assert!(!zone.model.resend());
        assert_eq!(zone.model.unqueue(), Unqueue::Send, "discard still works");
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::LoginRequired));
        assert_eq!(
            DisabledWhy::LoginRequired.text(),
            "the agent needs a login; log in through its own command line, then open the profile again"
        );
        assert_eq!(texts(&zone.model), ["hi", "Sure"], "earlier entries kept");
        let alert = zone.model.alert("fake-acp", "this machine").unwrap();
        assert_eq!(
            alert.text,
            "Fake Agent needs you to log in. Log in through the agent's own command line, then open the profile again."
        );
        assert_eq!((alert.destructive, alert.detail.as_deref()), (false, Some(message)));
        assert!(zone.model.describe(1, true, &linux).contains("--- queued held reason=login-required resend=no \"next\"\n"));
    }

    /// Decision 13: a version mismatch's `Failed`, then the session's exit,
    /// stays `Live(Failed)` and is never worded as an exit.
    #[test]
    fn a_version_mismatch_stays_failed_after_the_session_ends() {
        let mut zone = Zone::new(None);
        let message = "the agent supports ACP versions 2; Brindle speaks 1";
        zone.feed(&[
            agent_state(AgentState::Failed { message: message.into() }),
            added(1, Entry::Notice { level: NoticeLevel::Warning, text: "stderr tail".into() }),
        ]);
        zone.model.ended();
        assert_eq!(zone.model.phase(), &AgentPhase::Live(AgentState::Failed { message: message.into() }));
        assert_eq!(zone.model.life(), SessionLife::Ended);
        let alert = zone.model.alert("fake-acp", "this machine").unwrap();
        assert_eq!(alert.text, format!("Fake can't be used: {message}"));
        assert!(!alert.text.contains("exit"));
        let dump = zone.model.describe(1, true, &linux);
        assert!(dump.starts_with(&format!("--- agent zone=1 state=failed message={message:?} turn=idle")), "{dump}");
        assert!(dump.contains(" session=ended\n"), "{dump}");
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::Failed));
    }

    /// Decision 13: each failure's alert, the action then the cause.
    #[test]
    fn each_failure_has_its_alert() {
        let names = Names { agent: "Fake Agent", command: "fake-acp", endpoint: "A8" };
        let text = |phase: AgentPhase, life| failure_text(&phase, life, names).map(|alert| alert.text);
        assert_eq!(text(AgentPhase::Connecting, SessionLife::Open), None);
        assert_eq!(text(AgentPhase::Live(AgentState::Ready { name: None, version: None }), SessionLife::Open), None);
        assert_eq!(text(AgentPhase::Live(AgentState::Starting), SessionLife::Open), None);
        assert_eq!(
            text(AgentPhase::NotStarted(StartFailure::NoServer("no helper".into())), SessionLife::Open).unwrap(),
            "Brindle could not start the agent fake-acp: no helper"
        );
        assert_eq!(
            text(AgentPhase::NotStarted(StartFailure::TooOld { endpoint: "A8".into() }), SessionLife::Open).unwrap(),
            too_old_text("A8")
        );
        assert_eq!(
            text(AgentPhase::Live(AgentState::Failed { message: "bad".into() }), SessionLife::Open).unwrap(),
            "Brindle's terminal server failed handling Fake Agent: bad"
        );
        let exited = failure_text(
            &AgentPhase::Live(AgentState::Exited { code: Some(3), stderr: "boom".into() }),
            SessionLife::Ended,
            names,
        )
        .unwrap();
        assert_eq!((exited.text.as_str(), exited.detail.as_deref()), ("Fake Agent exited with status 3", Some("boom")));
        assert_eq!(
            text(AgentPhase::Live(AgentState::Exited { code: None, stderr: String::new() }), SessionLife::Ended).unwrap(),
            "Fake Agent was ended by a signal"
        );
        assert_eq!(
            text(AgentPhase::Lost { reason: "the holder crashed".into() }, SessionLife::Open).unwrap(),
            "the agent session on A8 was lost: the holder crashed"
        );
        assert_eq!(
            text(AgentPhase::Broken { reason: "x".into() }, SessionLife::Open).unwrap(),
            "Brindle can't show this agent session any more; attach it again from Attach Session…"
        );
        assert_eq!(
            text(AgentPhase::Live(AgentState::Ready { name: None, version: None }), SessionLife::Ended).unwrap(),
            "the agent session on A8 has ended"
        );

        let cause = "npx is a batch file (C:/npx.cmd); run it as `cmd /c npx acp`";
        let refused = StartFailure::Refused { command: "npx".into(), cause: cause.into() };
        let alert = failure_text(&AgentPhase::NotStarted(refused), SessionLife::Open, names).unwrap();
        assert_eq!(alert.text, format!("Brindle could not start the agent npx: {cause}"));
        assert_eq!(alert.mono.as_deref(), Some("cmd /c npx acp"));
        assert!(alert.destructive);
        let plain = StartFailure::Refused { command: "fake".into(), cause: "not found".into() };
        assert_eq!(failure_text(&AgentPhase::NotStarted(plain), SessionLife::Open, names).unwrap().mono, None);
    }

    /// Decision 8: a thought is open while it is the latest entry and the
    /// turn runs, closed once another follows, unless the user toggled it.
    #[test]
    fn thoughts_open_while_latest_and_streaming() {
        let mut zone = Zone::ready();
        zone.feed(&[turn(TurnState::Working), added(1, thought("hmm"))]);
        assert!(matches!(zone.model.blocks()[0].body, BlockBody::Thought { open: true, .. }));
        let absorbed = zone.feed(&[added(2, agent("answer"))]);
        assert!(matches!(zone.model.blocks()[0].body, BlockBody::Thought { open: false, .. }));
        assert!(absorbed.splices.contains(&Splice { range: 0..1, count: 1 }));

        assert_eq!(zone.model.toggle_thought(EntryId(1)), [Splice { range: 0..1, count: 1 }]);
        assert!(matches!(zone.model.blocks()[0].body, BlockBody::Thought { open: true, .. }));
        zone.feed(&[added(3, thought("more"))]);
        assert!(matches!(zone.model.blocks()[2].body, BlockBody::Thought { open: true, .. }));
        zone.feed(&[turn(TurnState::Ended { stop: StopReason::EndTurn })]);
        assert!(matches!(zone.model.blocks()[2].body, BlockBody::Thought { open: false, .. }), "closed once the turn ends");
        assert!(matches!(zone.model.blocks()[0].body, BlockBody::Thought { open: true, .. }), "toggled stays");

        assert_eq!(zone.model.toggle_thoughts().len(), 1, "one was closed: all open");
        assert_eq!(zone.model.toggle_thoughts().len(), 2, "all open: all close");
        assert!(zone.model.blocks().iter().all(|b| !matches!(b.body, BlockBody::Thought { open: true, .. })));

        let mut idle = Zone::ready();
        idle.feed(&[added(1, thought("hmm"))]);
        assert!(matches!(idle.model.blocks()[0].body, BlockBody::Thought { open: false, .. }), "no turn running");
    }

    /// Decision 8: an answer names a pending request's option; an answered
    /// request takes none.
    #[test]
    fn permissions_are_answered_once() {
        let mut zone = Zone::ready();
        zone.feed(&[turn(TurnState::Blocked), added(1, permission("Run cargo test?"))]);
        assert!(zone.model.answer(EntryId(1), "allow-once"));
        assert!(!zone.model.answer(EntryId(1), "allow-always"), "not offered");
        assert!(!zone.model.answer(EntryId(9), "allow-once"), "no such entry");
        assert_eq!(zone.model.pending_permissions(), [(EntryId(1), vec!["allow-once".into(), "reject-once".into()])]);
        zone.model.sent(1, Sent::Answer(EntryId(1)));
        assert_eq!(zone.model.replied(1, Ok(())), Replied::Nothing, "the outcome comes as an event");
        zone.feed(&[Event::Answered { id: EntryId(1), outcome: PermissionOutcome::Selected { option: "allow-once".into() } }]);
        assert!(!zone.model.answer(EntryId(1), "allow-once"), "answered");
        assert!(zone.model.pending_permissions().is_empty());
        assert!(zone.model.describe(1, true, &linux).contains("outcome=selected:allow-once"));
    }

    /// Decision 10: the shown status is the activity's while live, with
    /// `done` and `error` dropped once the user types, until it changes.
    #[test]
    fn the_shown_status_and_the_seen_rule() {
        let mut zone = Zone::new(None);
        assert_eq!(zone.model.shown_status(), None, "connecting");
        assert_eq!(zone.feed(&[agent_state(AgentState::Starting)]).status, Some(Some(State::Working)));
        zone.feed(&[ready()]);
        assert_eq!(zone.model.shown_status(), Some(State::Idle));
        assert_eq!(zone.model.seen(), None, "idle isn't dropped");
        zone.feed(&[turn(TurnState::Working)]);
        let absorbed = zone.feed(&[turn(TurnState::Ended { stop: StopReason::EndTurn })]);
        assert_eq!(absorbed.status, Some(Some(State::Done)));
        assert_eq!(zone.model.seen(), Some(None));
        assert_eq!(zone.model.seen(), None);
        assert_eq!(zone.feed(&[added(1, agent("more"))]).status, None, "the activity didn't change");
        assert_eq!(zone.feed(&[turn(TurnState::Working)]).status, Some(Some(State::Working)));
        assert_eq!(zone.feed(&[turn(TurnState::Ended { stop: StopReason::EndTurn })]).status, Some(Some(State::Done)));
        assert_eq!(zone.model.lost("gone").status, Some(None), "no status outside live");
    }

    /// Decision 10: notices on becoming blocked, done or error, never from
    /// a replay, titled with the agent's name, else the profile's.
    #[test]
    fn notices_on_blocked_done_and_error() {
        let mut zone = Zone::ready();
        zone.feed(&[turn(TurnState::Working)]);
        let absorbed = zone.feed(&[turn(TurnState::Blocked), added(1, permission("Run cargo test?"))]);
        assert_eq!(absorbed.notice, Some(AgentNotice { title: "Fake Agent".into(), body: "Run cargo test?".into() }));
        let absorbed = zone.feed(&[turn(TurnState::Ended { stop: StopReason::EndTurn })]);
        assert_eq!(absorbed.notice, Some(AgentNotice { title: "Fake Agent".into(), body: "Done".into() }));
        let absorbed = zone.feed(&[turn(TurnState::Failed { message: "rate limited".into() })]);
        assert_eq!(absorbed.notice, Some(AgentNotice { title: "Fake Agent".into(), body: "rate limited".into() }));
        assert_eq!(zone.feed(&[turn(TurnState::Working)]).notice, None, "working raises none");

        let mut unnamed = Zone::new(None);
        let absorbed = unnamed.feed(&[agent_state(AgentState::Failed { message: "bad".into() })]);
        assert_eq!(absorbed.notice, Some(AgentNotice { title: "Fake".into(), body: "bad".into() }));
        assert_eq!(unnamed.model.notice(Some(State::Working), Some(State::Error), true), None, "replaying");

        let mut replayed = Zone::new(None);
        let absorbed = replayed.replay(&[ready(), turn(TurnState::Ended { stop: StopReason::EndTurn })]);
        assert_eq!(absorbed.status, Some(Some(State::Done)));
        assert_eq!(absorbed.notice, None, "a replay raises nothing");
    }

    /// Decision 8: the title is the agent's name from `Ready`, kept after
    /// it exits, else the profile's.
    #[test]
    fn the_title_is_the_agents_name_else_the_profiles() {
        let mut zone = Zone::new(None);
        assert_eq!(zone.model.title(), "Fake");
        assert!(zone.feed(&[ready()]).title_changed);
        assert_eq!(zone.model.title(), "Fake Agent");
        zone.feed(&[agent_state(AgentState::Exited { code: Some(0), stderr: String::new() })]);
        assert_eq!(zone.model.title(), "Fake Agent");
        let mut renamed = Zone::new(None);
        renamed.model.set_profile_name("Renamed");
        assert_eq!(renamed.model.title(), "Renamed");
    }

    /// Decision 15: the dump's lines, and what `--wait-for` matches.
    #[test]
    fn the_dump_describes_the_zone() {
        let mut zone = Zone::ready();
        zone.feed(&[
            added(1, user("hi")),
            turn(TurnState::Working),
            added(2, agent("Hello!")),
            added(3, thought("Considering")),
            added(4, Entry::Tool { call: "call_1".into(), title: "Read main.rs".into(), kind: ToolKind::Read, status: ToolStatus::Pending }),
            Event::ToolChanged { id: EntryId(4), title: None, kind: None, status: Some(ToolStatus::Completed) },
            added(5, permission("Run cargo test?")),
            added(6, Entry::Notice { level: NoticeLevel::Warning, text: "1 line was not an ACP message".into() }),
            queue(waiting()),
        ]);
        let expected = "\
--- agent zone=1 state=ready name=\"Fake Agent\" turn=working activity=working status=working link=up session=open
--- entry 1 user \"hi\"
--- entry 2 agent \"Hello!\"
--- block entry=2 part=0 kind=paragraph \"Hello!\"
--- entry 3 thought open=false \"Considering\"
--- block entry=3 part=0 kind=paragraph \"Considering\"
--- entry 4 tool call=\"call_1\" kind=read status=completed \"Read main.rs\"
--- entry 5 permission call=\"call_2\" options=allow-once:\"Allow once\"|reject-once:\"Reject\" outcome=pending \"Run cargo test?\"
--- entry 6 notice level=warn \"1 line was not an ACP message\"
--- queued waiting \"next\"
--- conversation entries=6 at=end
--- prompt status=streaming
";
        assert_eq!(zone.model.describe(1, true, &linux), expected);
        let wait = zone.model.wait_text(&linux);
        assert!(wait.starts_with("--- agent state=ready name=\"Fake Agent\" turn=working"), "{wait}");
        assert!(wait.contains("--- conversation entries=6\n"), "{wait}");

        let mut zone = Zone::new(None);
        zone.model.set_note("the profile \"Old\" is gone; this tab uses \"Fake\"");
        zone.model.not_started(StartFailure::TooOld { endpoint: "A8".into() });
        zone.model.submit("x", false);
        let dump = zone.model.describe(2, false, &linux);
        assert_eq!(
            dump,
            "--- agent zone=2 state=not-started message=\"the terminal server on A8 is too old for agent sessions; \
             it will update once its terminals have ended\" turn=idle activity=none status=none link=up session=open\n\
             --- note \"the profile \\\"Old\\\" is gone; this tab uses \\\"Fake\\\"\"\n\
             --- conversation entries=0 at=away\n\
             --- prompt status=disabled\n"
        );
        assert_eq!(zone.model.note(), Some("the profile \"Old\" is gone; this tab uses \"Fake\""));
        let mut held = Zone::ready();
        held.feed(&[queue(held_slot())]);
        held.model.sent(1, Sent::Prompt);
        held.model.replied(1, Err("the agent isn't ready".into()));
        let dump = held.model.describe(1, true, &linux);
        assert!(dump.contains("--- queued held reason=stopped:max_tokens resend=yes \"next\"\n"), "{dump}");
        assert!(dump.ends_with("--- prompt status=ready refusal=\"the agent isn't ready\"\n"), "{dump}");
        assert_eq!(held.model.banner("A8"), None);
        held.model.set_link(Link::Reconnecting);
        assert_eq!(held.model.link(), Link::Reconnecting);
        assert_eq!(held.model.banner("A8").as_deref(), Some("reconnecting to A8…"));
        assert!(held.model.describe(1, true, &linux).contains("link=reconnecting"));
    }

    fn held_slot() -> Slot {
        held(HoldReason::Stopped { stop: StopReason::MaxTokens })
    }

    /// The copy actions read the zone's own copy.
    #[test]
    fn the_copy_actions_read_the_zones_copy() {
        let mut zone = Zone::ready();
        assert_eq!(zone.model.last_agent_message(), None);
        zone.feed(&[added(1, user("hi")), added(2, agent("one")), added(3, agent("two")), added(4, thought("t"))]);
        assert_eq!(zone.model.last_agent_message(), Some("two"));
        assert_eq!(zone.model.message_text(EntryId(1)), Some("hi"));
        assert_eq!(zone.model.message_text(EntryId(4)), None);
        assert!(!zone.model.cancel(), "no turn");
        zone.feed(&[turn(TurnState::Working)]);
        assert!(zone.model.cancel());
        zone.feed(&[turn(TurnState::Cancelling)]);
        assert!(!zone.model.cancel(), "already cancelling");
    }

    /// A request that couldn't go is answered as a refusal: a prompt's
    /// draft stays, a first prompt becomes the draft; a server's note is
    /// shown the same way.
    #[test]
    fn an_unsent_request_is_refused_and_a_first_prompt_becomes_the_draft() {
        let mut zone = Zone::ready();
        assert_eq!(zone.model.unsent(Sent::Prompt), Replied::Nothing);
        assert_eq!(zone.model.refusal(), Some(UNSENT));
        assert_eq!(zone.model.unsent(Sent::FirstPrompt("go".into())), Replied::SetDraft("go".into()));
        assert_eq!(zone.model.unsent(Sent::Resend), Replied::Nothing);
        zone.model.server_note("the terminal server: no turn is running");
        assert_eq!(zone.model.refusal(), Some("the terminal server: no turn is running"));
        assert_eq!(zone.model.prompt_status(), PromptStatus::Ready, "nothing is in flight");
    }

    /// Only a session that is open (or still connecting) is left for the
    /// next launch.
    #[test]
    fn a_zone_keeps_its_session_only_while_it_is_open() {
        let mut zone = Zone::new(None);
        assert!(zone.model.keeps_session(), "connecting");
        zone.feed(&[ready()]);
        assert!(zone.model.keeps_session());
        zone.model.ended();
        assert!(!zone.model.keeps_session(), "ended");
        let mut lost = Zone::ready();
        lost.model.lost("gone");
        assert!(!lost.model.keeps_session());
        let mut broken = Zone::ready();
        broken.model.broken("bad event");
        assert!(!broken.model.keeps_session());
        let mut never = Zone::new(None);
        never.model.not_started(StartFailure::NoServer("no helper".into()));
        assert!(!never.model.keeps_session());
    }

    fn appended(id: u64, text: &str) -> Event {
        Event::Appended { id: EntryId(id), text: text.into() }
    }

    /// Entry `id`'s cached parts.
    fn parts_of(model: &ChatModel, id: u64) -> Vec<Part> {
        model.rendered.get(&EntryId(id)).cloned().unwrap_or_default()
    }

    /// The model's blocks are what a fresh model fed the same entries
    /// whole would draw, and its cache holds exactly the agent messages and
    /// thoughts, each rendered from its whole text.
    fn in_step(model: &ChatModel) {
        let expected: Vec<Block> =
            model.entries.iter().flat_map(|(id, entry)| drawn(*id, entry, model.thought_open(*id))).collect();
        assert_eq!(model.blocks, expected);
        let markdown: Vec<EntryId> = model
            .entries
            .iter()
            .filter(|(_, entry)| matches!(entry, Entry::Agent { .. } | Entry::Thought { .. }))
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(model.rendered.keys().copied().collect::<Vec<_>>(), markdown);
        for (id, entry) in &model.entries {
            if let Entry::Agent { text } | Entry::Thought { text } = entry {
                assert_eq!(model.rendered[id], markdown::render(text), "entry {id}");
            }
        }
    }

    const STREAMED: &str = "Intro with [a link](https://example.com) and **bold**.\n\n\
                            ```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\
                            - one\n- [x] two\n\n> quoted\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\nSee [d] — é.\n\n[d]: https://d.example\n";

    /// Decision 5: whatever the chunks, a streamed message ends with the
    /// parts of its whole text.
    #[test]
    fn streaming_at_every_split_point_ends_with_the_whole_texts_parts() {
        let whole = STREAMED;
        for split in (1..whole.len()).filter(|at| whole.is_char_boundary(*at)) {
            let mut zone = Zone::ready();
            zone.feed(&[added(1, user("hi")), added(2, agent(&whole[..split]))]);
            zone.feed(&[appended(2, &whole[split..])]);
            assert_eq!(parts_of(&zone.model, 2), markdown::render(whole), "split at {split}");
            in_step(&zone.model);
        }
        // Character by character.
        let mut zone = Zone::ready();
        zone.feed(&[added(1, agent(""))]);
        for c in whole.chars() {
            zone.feed(&[appended(1, &c.to_string())]);
        }
        in_step(&zone.model);
        assert_eq!(parts_of(&zone.model, 1), markdown::render(whole));
    }

    /// Decision 5: an unclosed fence is code to the end; its closing fence
    /// and the text after it add a part, the parts above unchanged.
    #[test]
    fn an_unclosed_fence_is_code_and_its_close_adds_a_part() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, agent("Intro\n\n```rust\nfn a() {}\n"))]);
        let absorbed = zone.feed(&[appended(1, "fn b() {}\n")]);
        assert_eq!(texts(&zone.model), ["Intro", "fn a() {}\nfn b() {}\n"]);
        assert_eq!(markdown::kind_word(&parts_of(&zone.model, 1)[1].block), "code:rust");
        assert_eq!(absorbed.splices, [Splice { range: 1..2, count: 1 }], "the paragraph above stays");
        let intro = parts_of(&zone.model, 1)[0].block.clone();
        let absorbed = zone.feed(&[appended(1, "```\nAfter")]);
        assert_eq!(texts(&zone.model), ["Intro", "fn a() {}\nfn b() {}\n", "After"]);
        assert_eq!(absorbed.splices, [Splice { range: 1..2, count: 2 }], "the code's bubble no longer ends there");
        assert!(Arc::ptr_eq(&intro, &parts_of(&zone.model, 1)[0].block));
        in_step(&zone.model);
    }

    /// Decision 5: a reference definition that arrives late changes the
    /// earlier part that uses it, which is spliced from there.
    #[test]
    fn a_late_reference_definition_splices_from_the_part_it_changes() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("q")), added(2, agent("Lead.\n\nSee [the docs][d].\n\n"))]);
        assert_eq!(texts(&zone.model), ["q", "Lead.", "See [the docs][d]."]);
        let absorbed = zone.feed(&[appended(2, "[d]: https://example.com")]);
        assert_eq!(texts(&zone.model), ["q", "Lead.", "See the docs."], "the definition isn't shown");
        assert_eq!(absorbed.splices, [Splice { range: 2..3, count: 1 }]);
        let RichBlock::Paragraph(inline) = &*parts_of(&zone.model, 2)[1].block else { panic!() };
        assert_eq!(inline.links()[0].target.as_ref(), "https://example.com");
        in_step(&zone.model);
    }

    /// User messages keep step 1's plain parts, cut at blank lines and past
    /// 4 KiB, and only their last part is drawn again.
    #[test]
    fn user_messages_split_as_before() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("**a**\n\nb"))]);
        assert_eq!(texts(&zone.model), ["**a**", "b"], "plain, asterisks kept");
        assert!(zone.model.rendered.is_empty());
        let absorbed = zone.feed(&[appended(1, "c\n\nd")]);
        assert_eq!(texts(&zone.model), ["**a**", "bc", "d"]);
        assert_eq!(absorbed.splices, [Splice { range: 1..2, count: 2 }]);
        let long = format!("{}\nrest", "x".repeat(BLOCK_LIMIT + 10));
        zone.feed(&[added(2, user(&long))]);
        assert_eq!(zone.model.blocks().len(), 5);
    }

    /// Decision 5: a thought is one item holding all its blocks, rendered
    /// again whole as it grows.
    #[test]
    fn a_thought_renders_as_one_block() {
        let mut zone = Zone::ready();
        zone.feed(&[turn(TurnState::Working), added(1, thought("Plan:\n\n- a"))]);
        let absorbed = zone.feed(&[appended(1, "\n- b\n\nDone.")]);
        assert_eq!(absorbed.splices, [Splice { range: 0..1, count: 1 }]);
        let [Block { body: BlockBody::Thought { blocks, open: true }, .. }] = zone.model.blocks() else {
            panic!("{:?}", zone.model.blocks())
        };
        let kinds: Vec<String> = blocks.iter().map(|block| markdown::kind_word(block)).collect();
        assert_eq!(kinds, ["paragraph", "list", "paragraph"]);
        assert_eq!(markdown::shown_text(&blocks[1]), "- a\n- b");
        in_step(&zone.model);
    }

    /// Decision 5: trims and resets keep the cache in step with the
    /// entries.
    #[test]
    fn trims_and_resets_keep_the_cache_in_step() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, agent("one")), added(2, thought("t")), added(3, user("u")), added(4, agent("four"))]);
        in_step(&zone.model);
        zone.feed(&[Event::Trimmed { through: EntryId(2), dropped: 2 }]);
        assert_eq!(zone.model.rendered.keys().copied().collect::<Vec<_>>(), [EntryId(4)]);
        in_step(&zone.model);
        zone.replay(&[added(1, agent("again *x*")), added(2, thought("hm")), ready()]);
        assert_eq!(zone.model.rendered.keys().copied().collect::<Vec<_>>(), [EntryId(1), EntryId(2)]);
        assert_eq!(texts(&zone.model), ["again x", "hm"]);
        in_step(&zone.model);
    }

    /// Decision 4: a renderer panic shows the source as one plain
    /// paragraph (the walk can't panic, so the guard is checked directly).
    #[test]
    fn the_renderer_runs_guarded() {
        let (parts, panicked) = render_entry(&agent("*a*")).unwrap();
        assert!(!panicked);
        assert_eq!(parts, markdown::render("*a*"));
        assert_eq!(render_entry(&user("*a*")), None);
        let mut model = ChatModel::new("Fake", None);
        let plain = vec![Part { range: 0..3, block: Arc::new(RichBlock::Paragraph(Inline::plain("*a*"))) }];
        model.keep_render(EntryId(1), Some((plain.clone(), true)));
        assert!(model.render_failed);
        assert_eq!(model.rendered[&EntryId(1)], plain);
    }

    /// A Markdown message of at least 256 KiB: prose, lists and a long
    /// fence.
    fn long_markdown() -> String {
        let mut text = String::new();
        let mut section = 0;
        while text.len() < 256 * 1024 {
            section += 1;
            text.push_str(&format!(
                "## Section {section}\n\nSome prose with **bold**, a [link](https://example.com/{section}) and `code`,\n\
                 wrapped onto a second line.\n\n- item one\n- item two\n  - nested {section}\n\n"
            ));
            if section == 20 {
                text.push_str("```rust\n");
                for line in 0..2_000 {
                    text.push_str(&format!("let x{line} = {line};\n"));
                }
                text.push_str("```\n\n");
            }
        }
        text
    }

    /// Decision 6's guard (`testing R8`: the rule, not a time): streaming a
    /// 256 KiB message in 64-byte appends ends with the whole text's parts;
    /// each absorb splices from the first item whose part changed (or
    /// whose bubble's end moved) and touches no other entry; and the parts
    /// it didn't change, and every other entry's, keep their cached values.
    #[test]
    fn streaming_a_long_message_re_renders_only_what_changed() {
        let whole = long_markdown();
        let mut zone = Zone::ready();
        zone.feed(&[added(1, thought("before")), added(2, agent("")), added(3, user("after"))]);
        let thought_before = parts_of(&zone.model, 1);
        let mut at = 0;
        while at < whole.len() {
            let mut end = (at + 64).min(whole.len());
            while !whole.is_char_boundary(end) {
                end += 1;
            }
            let old = parts_of(&zone.model, 2);
            let items = zone.model.entry_blocks(EntryId(2));
            let absorbed = zone.feed(&[appended(2, &whole[at..end])]);
            let new = parts_of(&zone.model, 2);
            // The first item that must change: a part that differs, or the
            // last one when the count changed (its bubble's end moved).
            let differs = old.iter().zip(&new).take_while(|(old, new)| old == new).count();
            let first = if old.len() == new.len() { differs } else { differs.min(old.len() - 1) };
            match absorbed.splices.as_slice() {
                [] => assert_eq!(old, new, "nothing changed at {at}"),
                [splice] => {
                    assert_eq!(splice.range.start, items.start + first, "at {at}");
                    assert_eq!(splice.range.end, items.end, "at {at}: only this entry's items");
                    assert_eq!(splice.count, new.len() - first, "at {at}");
                }
                more => panic!("at {at}: {more:?}"),
            }
            for (old, new) in old.iter().zip(&new).take(differs) {
                assert!(Arc::ptr_eq(&old.block, &new.block), "at {at}: an unchanged part was replaced");
            }
            at = end;
        }
        assert_eq!(parts_of(&zone.model, 2), markdown::render(&whole));
        let after = parts_of(&zone.model, 1);
        assert!(thought_before.iter().zip(&after).all(|(a, b)| Arc::ptr_eq(&a.block, &b.block)), "another entry kept");

        // An absorb that changes nothing leaves the cache and the list alone.
        let before = parts_of(&zone.model, 2);
        let absorbed = zone.feed(&[appended(2, ""), turn(TurnState::Working)]);
        assert!(absorbed.splices.is_empty(), "{:?}", absorbed.splices);
        let kept = parts_of(&zone.model, 2);
        assert!(before.len() == kept.len() && before.iter().zip(&kept).all(|(a, b)| Arc::ptr_eq(&a.block, &b.block)));
        in_step(&zone.model);
    }

    /// Decision 11: block and link lines follow each agent message's and
    /// thought's entry line; user messages have none.
    #[test]
    fn the_dump_shows_each_part_and_link() {
        let mut zone = Zone::ready();
        zone.feed(&[
            added(1, user("**hi**")),
            added(2, agent("Try:\n\n```rust\nfn main() {}\n```\nSee [docs](https://example.com) or <mailto:a@example.com> [x](javascript:void).")),
            added(3, thought("- [ ] check `x`\n- see https://t.example")),
        ]);
        let dump = zone.model.describe(1, true, &linux);
        let expected = "\
--- entry 1 user \"**hi**\"
--- entry 2 agent \"Try:\\n\\n```rust\\nfn main() {}\\n```\\nSee [docs](https://example.com) or <mailto:a@example.com> [x](javascript:void).\"
--- block entry=2 part=0 kind=paragraph \"Try:\"
--- block entry=2 part=1 kind=code:rust \"fn main() {}\\n\"
--- block entry=2 part=2 kind=paragraph \"See docs or mailto:a@example.com x.\"
--- link entry=2 part=2 target=\"https://example.com\" text=\"docs\" action=open
--- link entry=2 part=2 target=\"mailto:a@example.com\" text=\"mailto:a@example.com\" action=confirm
--- link entry=2 part=2 target=\"javascript:void\" text=\"x\" action=refuse:scheme
--- entry 3 thought open=false \"- [ ] check `x`\\n- see https://t.example\"
--- block entry=3 part=0 kind=list \"- [ ] check x\\n- see https://t.example\"
--- link entry=3 part=0 target=\"https://t.example\" text=\"https://t.example\" action=open
--- conversation entries=3 at=end
";
        assert!(dump.contains(expected), "{dump}");
        assert!(!dump.contains("--- block entry=1"), "{dump}");
        let wait = zone.model.wait_text(&linux);
        assert!(wait.contains("--- block entry=2 part=1 kind=code:rust \"fn main() {}\\n\"\n"), "{wait}");
        assert!(wait.contains("--- link entry=2 part=2 target=\"https://example.com\" text=\"docs\" action=open\n"), "{wait}");
        // The plan is the caller's: Windows refuses a file link with no drive.
        zone.feed(&[added(4, agent("[f](file:///etc/hosts)"))]);
        let windows = |uri: &str| crate::terminal::links::plan(uri, "thishost", true);
        assert!(zone.model.wait_text(&windows).contains("action=refuse:no-drive\n"));
    }

    /// `copy_last_code_block`: the last code block of the latest agent
    /// message that has one, nested ones included, thoughts skipped.
    #[test]
    fn the_last_code_block() {
        let mut zone = Zone::ready();
        assert_eq!(zone.model.last_code_block(), None);
        zone.feed(&[added(1, agent("no code")), added(2, user("```\nuser\n```"))]);
        assert_eq!(zone.model.last_code_block(), None, "no agent code");
        zone.feed(&[added(3, agent("```\na\n```\n\n```\nb\n```"))]);
        assert_eq!(zone.model.last_code_block(), Some("b\n"));
        zone.feed(&[added(4, agent("- item\n\n  ```\n  nested\n  ```")), added(5, thought("```\nthought\n```"))]);
        assert_eq!(zone.model.last_code_block(), Some("nested\n"), "a thought doesn't count");
        zone.feed(&[added(6, agent("plain again"))]);
        assert_eq!(zone.model.last_code_block(), Some("nested\n"), "the latest message that has one");
    }
}
