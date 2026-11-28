//! The ACP reducer (design decisions 4, 7, 8, 10, 13, 14 and 17 of the
//! OpenSpec change `agents-sessions`): [`step`] takes one [`Input`] for an
//! agent session (the agent's stdout, a client's command, a holder event)
//! and returns what the core does about it as data ([`Output`]): the
//! transcript events to send, the bytes to write to the agent's stdin, the
//! reply to the client's request, whether to checkpoint and whether to hang
//! the agent up. The core's handlers gather, call `step` and apply; they
//! decide nothing (`fcis R1`, `R9`).
//!
//! - **The log changes only here.** [`AgentSession`] holds the
//!   [`Transcript`], and `step` applies every event it returns before
//!   returning it, so the core sends [`Output::events`] as they are. A
//!   [`FoldError`] from its own events is a bug in this reducer: the step
//!   stops and the session's ACP handling is marked failed (decision 13).
//! - **Lines are split here.** The session holds the [`Lines`] the agent's
//!   stdout goes through, so [`Input::Stdout`] takes the holder's bytes as
//!   they come and [`AgentSession::lines`] gives the checkpoint its `covers`.
//! - **Text from the agent is cleaned** ([`clean`]): control characters
//!   other than `\n`, `\t` and the `\r` of a `\r\n` become U+FFFD, so no
//!   client renders an escape sequence an agent wrote. Text fields are
//!   bounded at [`TEXT_CHUNK`]; growing text is split into `Appended`s.
//! - **Time is an argument.** A run of lines that aren't ACP messages is
//!   one notice with its count, written when a message follows, at any
//!   other input, or at an [`Input::Tick`] once [`BAD_RUN_QUIET`] has passed
//!   with no more; [`Output::wake`] says when to tick.
//! - **A conversation can be continued** (OpenSpec change
//!   `agents-session-recovery`). The transcript names it
//!   ([`Event::Conversation`]) once `session/new`, `session/resume` or
//!   `session/load` succeeds. An ended agent ([`Phase::Exited`], or
//!   [`Phase::Lost`] with its holder) whose conversation can be continued
//!   ([`AgentSession::continuable`]) is spawned again by [`Input::Restart`]
//!   into the same log, and a session the server no longer had is started
//!   by [`AgentSession::continuing`] into a fresh one. After `initialize`,
//!   [`choose`] picks `session/resume` or `session/load` from what the new
//!   agent offers; a kept log's load replay is muted (counted, not
//!   applied), a fresh log's is shown. No continue falls back to
//!   `session/new`.
//! - **The agent names the conversation** (OpenSpec change
//!   `agents-session-title`). A `session_info_update`'s title, made one line
//!   and bounded by [`session_title`], is the conversation's title: a new
//!   [`Event::Conversation`] when it changes, applied (and not counted)
//!   during a muted load too. [`AgentSession`]'s `ready` keeps it for the
//!   same ACP session, so a restart that continues it keeps the title, and
//!   a new ACP session starts without one.
//!
//! A whole pure module (`fcis` R2, R3; `PURE_MODULES`).

use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, Instant};

use brindle_remote_protocol::RequestId;
use brindle_remote_protocol::client::OptionValue;
use brindle_remote_protocol::serde_json::Value;
use brindle_transcript::{
    AgentState, Conversation, Entry, EntryId, Event, LoginMethod, NoticeLevel, Slot, Transcript,
    TurnState, chunks, clean,
};
use brindle_transcript::{HoldReason, MAX_EVENT};

use super::envelope::{self, Malformed, Message, RpcError};
use super::forms::{
    self, Content, NotificationKind, OptionKey, Patch, Refusal, Request, RequestKind, Update,
};
use super::limits::ENTRY_LIMIT;
use super::lines::{Lines, Piece};
use super::rpc::{Purpose, Rpc};

use text::bound;
use tools::placeholder;

mod lifecycle;
mod list;
mod login;
mod options;
mod permission;
mod text;
mod tools;
mod turn;

#[cfg(test)]
mod harness;

pub use lifecycle::{Greeting, Log, Phase, Replay, Start};
pub use list::ListAnswer;
pub use login::{AfterLogin, Login};
pub use text::session_title;
pub use tools::{MAX_OPEN_OUTPUTS, OpenOutput};

#[cfg(test)]
pub(super) use permission::pending_permissions;

// The log's bounds and the prompt's live with every other bound, in
// `acp::limits`; re-exported here, where their readers have found them.
pub use super::limits::{LOG_LIMIT, LOG_TRIM_TO, MAX_PROMPT};

/// How long a run of lines that aren't ACP messages may go without another
/// before its notice is written.
pub const BAD_RUN_QUIET: Duration = Duration::from_secs(1);

/// The notice after a muted replay of `updates` updates.
fn muted_notice(updates: u64) -> String {
    let plural = if updates == 1 { "" } else { "s" };
    format!("conversation continued; the agent's replay of {updates} update{plural} wasn't shown")
}

/// The kind of text entry a run of chunks grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    User,
    Agent,
    Thought,
}

/// The chunk run in progress: chunks of `kind` append to `entry` (decision
/// 7); a chunk of another kind, any other new entry or a turn's end ends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub kind: RunKind,
    pub entry: EntryId,
}

/// A run of lines that weren't ACP messages, not yet written as a notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadRun {
    pub lines: u64,
    /// When the last of them came.
    pub last: Instant,
}

/// One agent session's ACP state: its log, its JSON-RPC bookkeeping, its
/// stdout's line in progress and the rest of what `step` keeps. The
/// checkpoint (`acp::checkpoint`) encodes it whole.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentSession {
    pub(super) transcript: Transcript,
    pub(super) rpc: Rpc,
    pub(super) lines: Lines,
    pub(super) phase: Phase,
    /// The absolute directory `session/new` is given.
    pub(super) cwd: String,
    /// Brindle's version, for `initialize`'s `clientInfo`.
    pub(super) version: String,
    pub(super) run: Option<Run>,
    /// The tool entries by the agent's call id; trimmed entries leave it.
    pub(super) tools: BTreeMap<String, EntryId>,
    /// The outputs still appended to, by the agent's terminal id
    /// (`agents-tool-content` decision 6); at most [`MAX_OPEN_OUTPUTS`].
    pub(super) outputs: BTreeMap<String, OpenOutput>,
    pub(super) bad: Option<BadRun>,
    /// The login methods the agent's `initialize` advertised, in its order
    /// (`agents-login` decision 3), sent with every `LoginRequired`.
    pub(super) methods: Vec<LoginMethod>,
    /// Whether the agent's `initialize` offered a list of its
    /// conversations (`agents-session-list` decision 2).
    pub(super) lists: bool,
    /// Whether the continue out was chosen in [`Phase::Choosing`]: its
    /// refusal returns there, with the agent's message (decision 11).
    pub(super) chose: bool,
    /// How each held option is set, by its id (OpenSpec change
    /// `agents-commands`, decision 3): opaque here, compared whole and
    /// handed back to the dialect. Emptied with the options.
    pub(super) keys: BTreeMap<String, OptionKey>,
    /// The current value of each mode option as last in effect, by its id,
    /// kept across a stop (decision 12), so a run whose first list is in
    /// another mode says so.
    pub(super) modes: BTreeMap<String, String>,
}

impl AgentSession {
    /// A session whose agent is being spawned in `cwd` (already absolute,
    /// decision 4), by Brindle at `version`.
    pub fn new(cwd: String, version: String) -> Self {
        Self::starting(Start::New, cwd, version)
    }

    /// A new session whose agent is spawned to continue `conversation`,
    /// in the conversation's own directory, into a fresh log (a
    /// `SPAWN_AGENT` naming a conversation the server no longer had).
    pub fn continuing(conversation: Conversation, version: String) -> Self {
        let cwd = conversation.cwd.clone();
        let start = Start::Continue {
            conversation,
            log: Log::Fresh,
        };
        Self::starting(start, cwd, version)
    }

    /// A new session whose agent, once initialized in `cwd`, waits for a
    /// client to choose a conversation (`SPAWN_AGENT`'s `start: "choose"`).
    pub fn choosing(cwd: String, version: String) -> Self {
        Self::starting(Start::Choose, cwd, version)
    }

    fn starting(start: Start, cwd: String, version: String) -> Self {
        Self {
            transcript: Transcript::new(),
            rpc: Rpc::new(),
            lines: Lines::new(),
            phase: Phase::Spawning { start },
            cwd,
            version,
            run: None,
            tools: BTreeMap::new(),
            outputs: BTreeMap::new(),
            bad: None,
            methods: Vec::new(),
            lists: false,
            chose: false,
            keys: BTreeMap::new(),
            modes: BTreeMap::new(),
        }
    }

    pub fn transcript(&self) -> &Transcript {
        &self.transcript
    }

    /// Whether the agent has ended (exited, or lost with its holder) and
    /// its conversation can be continued: the agent gave an ACP session,
    /// offered `session/load` or `session/resume`, and didn't fail its
    /// start (a failed start, a continue's included, stays `Failed`).
    pub fn continuable(&self) -> bool {
        let ended = self.has_ended();
        let agent_ended = match self.transcript.agent() {
            AgentState::Exited { .. } | AgentState::Lost { .. } => true,
            AgentState::Starting
            | AgentState::Ready { .. }
            | AgentState::LoginRequired { .. }
            | AgentState::LoggingIn { .. }
            | AgentState::Failed { .. }
            | AgentState::Choosing { .. } => false,
        };
        ended
            && agent_ended
            && self
                .transcript
                .conversation()
                .is_some_and(|conversation| conversation.continuation.continuable())
    }

    /// Whether the agent's end (its exit, or its loss with its holder) has
    /// been stepped: a replay of the log then ends with it (OpenSpec change
    /// `agents-process-lifecycle`, decision 7).
    pub fn has_ended(&self) -> bool {
        match self.phase {
            Phase::Exited | Phase::Lost => true,
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Ready { .. }
            | Phase::Choosing { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken => false,
        }
    }

    /// Whether the agent runs and waits for a login (`agents-login`): an
    /// `AGENT_RESTART` is taken then, as well as once it has ended.
    pub fn needs_login(&self) -> bool {
        matches!(self.phase, Phase::LoginRequired(_))
    }

    /// The stdout line in progress and the bytes fed, for the checkpoint.
    pub fn lines(&self) -> &Lines {
        &self.lines
    }

    // What the tests look at; the core sees a session through its
    // transcript and its lines.

    #[cfg(test)]
    pub fn rpc(&self) -> &Rpc {
        &self.rpc
    }

    #[cfg(test)]
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    #[cfg(test)]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }
}

/// What happened to an agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input<'a> {
    /// The holder started the agent (`SPAWNED`): `initialize` goes out.
    Spawned,
    /// Bytes of the agent's stdout, as the holder sent them.
    Stdout(&'a [u8]),
    /// The holder's tail was truncated in an adoption (decision 11): the
    /// bytes fed next follow a gap.
    OutputLost,
    /// The agent's process ended (`EXITED`), with the stderr tail the
    /// holder sent before it (`STDERR_TAIL`, at most 16 KiB).
    Exited { code: Option<i32>, stderr: &'a [u8] },
    /// The agent's holder ended while the core ran, and the agent with it:
    /// as [`Input::Exited`], but the agent is `Lost` with `message`.
    Lost { message: &'a str },
    /// `AGENT_RESTART`: the agent ended and is spawned again to continue
    /// its conversation in this log. Refused unless
    /// [`AgentSession::continuable`], or the agent needs a login
    /// ([`Phase::LoginRequired`]): then it is hung up first
    /// ([`Phase::Restarting`]), and stepped again once it has exited.
    Restart,
    /// A command from the client that has the session attached.
    Command(Command),
    /// Handling this session failed in the core (a panic caught around
    /// `step`, decision 13): its ACP handling stops, with this reason.
    Fault { reason: String },
    /// Time passed (at the [`Output::wake`] asked for).
    Tick,
}

/// A client's command for an agent session (client protocol 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `AGENT_PROMPT`.
    Prompt { text: String },
    /// `AGENT_UNQUEUE`: discard the slot's prompt, waiting or held.
    Unqueue,
    /// `AGENT_RESEND`: send the held prompt again.
    Resend,
    /// `AGENT_CANCEL`.
    Cancel,
    /// `AGENT_PERMISSION`: the chosen option's id, or `None` for cancelled.
    Answer {
        entry: EntryId,
        option: Option<String>,
    },
    /// `AGENT_LOGIN`: log in with the agent's method `method`
    /// (`agents-login` decision 4).
    Login { method: String },
    /// `AGENT_LIST` `req`: a page of the agent's conversations after
    /// `cursor`, answered in [`Output::listed`] (`agents-session-list`).
    List {
        req: RequestId,
        cursor: Option<String>,
    },
    /// `AGENT_CHOOSE`: continue `conversation` in the session's directory,
    /// or start a new one with none.
    Choose { conversation: Option<String> },
    /// `AGENT_SET_OPTION`: set the session option `option` to `value`
    /// (OpenSpec change `agents-commands`, decision 4).
    SetOption { option: String, value: OptionValue },
}

/// Why a client's command was refused; its text is the `ERROR`'s, which
/// the core prefixes with the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    NotReady,
    LoginRequired,
    /// The agent can't be used: its start failed or its handling did.
    Failed {
        message: String,
    },
    Exited,
    /// The agent was lost with its holder.
    Lost,
    /// An `AGENT_RESTART` for an agent that is still running or starting.
    NotEnded,
    /// An `AGENT_RESTART` for an agent whose conversation can't be
    /// continued (none, neither offered, or a failed start).
    CannotContinue,
    AlreadyQueued,
    TooLong {
        len: usize,
    },
    NothingQueued,
    NoneHeld,
    NoSuchRequest {
        entry: EntryId,
    },
    AlreadyAnswered {
        entry: EntryId,
    },
    NoSuchOption {
        entry: EntryId,
        option: String,
    },
    /// An `AGENT_LOGIN` while the agent doesn't need a login.
    NoLoginNeeded,
    /// An `AGENT_LOGIN` while a login runs.
    AlreadyLoggingIn,
    /// An `AGENT_LOGIN` naming a method the agent didn't advertise.
    NoSuchMethod {
        method: String,
    },
    /// An `AGENT_LOGIN` naming a terminal method, which the client runs.
    RunByClient {
        method: String,
    },
    /// A prompt or resend while a login runs.
    LoggingIn,
    /// An `AGENT_CHOOSE` while the agent isn't choosing.
    NotChoosing,
    /// An `AGENT_LIST` of an agent that offers no list.
    NotOffered,
    /// An `AGENT_LIST` while another is awaited.
    ListBusy,
    /// An `AGENT_LIST` or `AGENT_CHOOSE` naming a cursor or conversation
    /// past its bound, in bytes.
    TooLongToList { what: &'static str, len: usize },
    /// An `AGENT_SET_OPTION` naming no option the agent offers.
    NoSuchSessionOption { id: String },
    /// An `AGENT_SET_OPTION` with a choice the option doesn't list, or the
    /// wrong kind of value (`value` as text).
    ChoiceNotOffered { id: String, value: String },
    /// An `AGENT_SET_OPTION` while an earlier set awaits its answer.
    OptionBusy,
    /// The agent is started again after its login: what was out to the
    /// one hung up is closed (OpenSpec change `agents-process-lifecycle`,
    /// decision 1).
    Restarted,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotReady => f.write_str("the agent isn't ready"),
            Self::LoginRequired => f.write_str("the agent needs a login"),
            Self::Failed { message } => write!(f, "the agent can't take prompts: {message}"),
            Self::Exited => f.write_str("the agent has exited"),
            Self::Lost => f.write_str("the agent was lost with its holder"),
            Self::NotEnded => f.write_str("the agent hasn't ended"),
            Self::CannotContinue => f.write_str("the agent can't continue this conversation"),
            Self::AlreadyQueued => f.write_str("a prompt is already queued"),
            Self::TooLong { len } => write!(
                f,
                "the prompt is {len} bytes, over the limit of {} KiB",
                MAX_PROMPT / 1024
            ),
            Self::NothingQueued => f.write_str("nothing is queued"),
            Self::NoneHeld => f.write_str("no prompt is held"),
            Self::NoSuchRequest { entry } => {
                write!(f, "entry {entry} isn't a permission request")
            }
            Self::AlreadyAnswered { entry } => {
                write!(f, "permission request {entry} was already answered")
            }
            Self::NoSuchOption { entry, option } => {
                write!(
                    f,
                    "permission request {entry} doesn't offer option {option:?}"
                )
            }
            Self::NoLoginNeeded => f.write_str("the agent isn't waiting for a login"),
            Self::AlreadyLoggingIn => f.write_str("a login is already running"),
            Self::NoSuchMethod { method } => {
                write!(f, "the agent offers no login method {method:?}")
            }
            Self::RunByClient { method } => write!(
                f,
                "login method {method:?} runs in a terminal, which Brindle opens"
            ),
            Self::LoggingIn => f.write_str("the agent is logging in"),
            Self::NotChoosing => f.write_str("the agent isn't choosing"),
            Self::NotOffered => f.write_str(brindle_remote_protocol::client::LIST_NOT_OFFERED),
            Self::ListBusy => f.write_str("a list of conversations is already being answered"),
            Self::TooLongToList { what, len } => write!(f, "the {what} is {len} bytes, over the limit"),
            Self::NoSuchSessionOption { id } => write!(f, "the agent offers no option {id:?}"),
            Self::ChoiceNotOffered { id, value } => {
                write!(f, "option {id:?} offers no choice {value:?}")
            }
            Self::OptionBusy => {
                f.write_str("an earlier change of an option hasn't been answered yet")
            }
            Self::Restarted => f.write_str("the agent is started again after its login"),
        }
    }
}

/// Why the core checkpoints now (decision 11's moments that the reducer
/// sees; `policy::agent_checkpoint_due` decides with the rest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moment {
    /// The agent became ready or needs a login: the session id is known.
    Started,
    /// A request is about to be written ([`Output::stdin`]): its id, and a
    /// prompt's user entry, reach the holder ahead of its bytes.
    BeforeRequest,
    TurnEnded,
    QueueChanged,
    /// A permission request arrived, or was answered.
    Permission,
    /// The agent stopped being usable: it failed or exited.
    Stopped,
    /// A login's state changed: it succeeded, failed or was cancelled
    /// (`agents-login` decision 15).
    Login,
    /// The server answered one of the agent's requests itself: the line it
    /// read reaches the holder, as consumed, ahead of the answer's bytes
    /// (`agents-process-lifecycle` decision 8).
    Answered,
}

/// What the core does after a step, in this order: send `events` to the
/// attached client, checkpoint when `checkpoint` says so, then write
/// `stdin` to the agent, then hang it up when `hang_up` says so.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// Already applied to the session's transcript.
    pub events: Vec<Event>,
    pub stdin: Vec<u8>,
    /// The answer to a client's command or an [`Input::Restart`]; `None`
    /// for other inputs and for a cancel, which is never refused here.
    pub reply: Option<Result<(), Refused>>,
    pub checkpoint: Option<Moment>,
    /// Close the agent's stdin, then end it after the grace (decision 2).
    pub hang_up: bool,
    /// When to step [`Input::Tick`].
    pub wake: Option<Instant>,
    /// The answer to a client's list (`agents-session-list`), when the
    /// agent answered it or it can no longer be.
    pub listed: Option<ListAnswer>,
}

/// Steps `session` with one input.
pub fn step(session: &mut AgentSession, input: Input<'_>, now: Instant) -> Output {
    run(session, input, now, MAX_EVENT)
}

/// [`step`] with the most one event may encode to as an argument, which
/// the tests lower to reach the emit guard (`testing R7`: no environment).
fn run(session: &mut AgentSession, input: Input<'_>, now: Instant, event_limit: usize) -> Output {
    let mut step = Step {
        s: session,
        out: Output::default(),
        fault: None,
        now,
        event_limit,
    };
    match input {
        Input::Spawned => step.spawned(),
        Input::Stdout(bytes) => step.stdout(bytes),
        Input::OutputLost => {
            step.flush_bad();
            step.output_lost();
        }
        Input::Exited { code, stderr } => {
            step.flush_bad();
            step.exited(code, stderr);
        }
        Input::Lost { message } => {
            step.flush_bad();
            step.lost(message);
        }
        Input::Restart => {
            step.flush_bad();
            let reply = step.restart();
            step.out.reply = Some(reply);
        }
        Input::Command(command) => {
            step.flush_bad();
            step.command(command);
        }
        Input::Fault { reason } => {
            step.flush_bad();
            step.fail(&reason);
        }
        Input::Tick => step.tick(),
    }
    step.finish()
}

struct Step<'s> {
    s: &'s mut AgentSession,
    out: Output,
    /// A fold error from this step's own events, or one too large to
    /// send: the rest is skipped.
    fault: Option<String>,
    now: Instant,
    /// The most one event may encode to: [`MAX_EVENT`], lower in tests.
    event_limit: usize,
}

impl Step<'_> {
    fn finish(mut self) -> Output {
        if let Some(reason) = self.fault.take() {
            // Decision 2 of `agents-process-lifecycle`: a faulted step
            // writes nothing it asked for; `fail`'s answers are all the
            // agent gets.
            self.out.stdin.clear();
            self.fail(&reason);
        }
        self.trim();
        self.out.wake = self.s.bad.map(|bad| bad.last + BAD_RUN_QUIET);
        self.out
    }

    // --- emitting -------------------------------------------------------

    /// Applies `event` and keeps it for the client; after a fold error,
    /// does nothing. An event past the step's `event_limit` (the bounds in
    /// `acp::limits` rule one out: I1) isn't applied, and fails the
    /// session saying what it was (`agents-event-bounds` decision 4).
    fn emit(&mut self, event: Event) {
        if self.fault.is_some() {
            return;
        }
        let len = event.encoded_len();
        if len > self.event_limit {
            self.fault = Some(oversized(&event, len, self.event_limit));
            return;
        }
        match self.s.transcript.apply(&event) {
            Ok(()) => self.out.events.push(event),
            Err(error) => self.fault = Some(error.to_string()),
        }
    }

    fn mark(&mut self, moment: Moment) {
        self.out.checkpoint = match self.out.checkpoint {
            Some(Moment::BeforeRequest) => Some(Moment::BeforeRequest),
            Some(
                Moment::Started
                | Moment::TurnEnded
                | Moment::QueueChanged
                | Moment::Permission
                | Moment::Stopped
                | Moment::Login
                | Moment::Answered,
            )
            | None => Some(moment),
        };
    }

    /// Adds an entry that isn't text, ending the chunk run.
    fn add(&mut self, entry: Entry) -> EntryId {
        let id = self.s.transcript.next_id();
        self.s.run = None;
        self.emit(Event::Added { id, entry });
        id
    }

    /// Adds a text entry of `kind` holding `text` (continued in more
    /// entries past [`ENTRY_LIMIT`]), and makes the last the chunk run.
    /// The first entry's id.
    fn add_text(&mut self, kind: RunKind, text: &str) -> EntryId {
        let id = self.s.transcript.next_id();
        self.s.run = None;
        self.text_run(kind, text);
        id
    }

    /// Streams `text` into the chunk run of `kind` (`agents-event-bounds`
    /// decision 2, A-1): appended to the run's entry while it holds less
    /// than [`ENTRY_LIMIT`], and continued past it in a new entry of the
    /// same kind, cut at the last line break in the room when there is
    /// one, else at a character boundary. With no run of `kind`, it starts
    /// one, even for no text. So no entry outgrows a trim's low mark, and
    /// the log stays within [`LOG_LIMIT`] however long a reply.
    fn text_run(&mut self, kind: RunKind, text: &str) {
        let mut rest = text;
        loop {
            let open = self
                .s
                .run
                .filter(|run| run.kind == kind)
                // A run at an entry without text (a bug) appends to it
                // anyway, so the fold refuses it and the handling fails.
                .and_then(|run| {
                    let entry = self.s.transcript.entry(run.entry)?;
                    Some((run.entry, text_len(entry).unwrap_or(0)))
                });
            match open {
                Some((entry, held)) => {
                    if rest.is_empty() {
                        return;
                    }
                    let (now, later) = within_room(rest, ENTRY_LIMIT.saturating_sub(held));
                    for piece in chunks(now).filter(|piece| !piece.is_empty()) {
                        self.emit(Event::Appended {
                            id: entry,
                            text: piece.to_owned(),
                        });
                    }
                    rest = later;
                    if !rest.is_empty() {
                        // Full: the rest goes in a new entry.
                        self.s.run = None;
                    }
                }
                None => {
                    // `rest` shrinks each time round unless it is empty,
                    // when this is the last, so the loop ends whatever
                    // `emit` did.
                    let (now, later) = within_room(rest, ENTRY_LIMIT);
                    self.new_text_entry(kind, now);
                    rest = later;
                    if rest.is_empty() {
                        return;
                    }
                }
            }
        }
    }

    /// A new text entry of `kind` holding `text`, split into `Appended`s,
    /// made the chunk run.
    fn new_text_entry(&mut self, kind: RunKind, text: &str) {
        let id = self.s.transcript.next_id();
        let mut pieces = chunks(text);
        let first = pieces.next().unwrap_or_default().to_owned();
        let entry = match kind {
            RunKind::User => Entry::User { text: first },
            RunKind::Agent => Entry::Agent { text: first },
            RunKind::Thought => Entry::Thought { text: first },
        };
        self.emit(Event::Added { id, entry });
        for piece in pieces {
            self.emit(Event::Appended {
                id,
                text: piece.to_owned(),
            });
        }
        self.s.run = Some(Run { kind, entry: id });
    }

    fn notice(&mut self, level: NoticeLevel, text: String) {
        self.add(Entry::Notice {
            level,
            text: bound(text),
        });
    }

    fn turn(&mut self, state: TurnState) {
        self.emit(Event::Turn { state });
    }

    fn agent(&mut self, state: AgentState) {
        self.emit(Event::Agent { state });
    }

    /// Holds a waiting prompt with `reason`; an empty or held slot stays.
    fn hold_waiting(&mut self, reason: HoldReason) {
        if let Slot::Waiting { text } = self.s.transcript.slot() {
            let text = text.clone();
            self.emit(Event::Queue {
                slot: Slot::Held { text, reason },
            });
            self.mark(Moment::QueueChanged);
        }
    }

    // --- reading the state ----------------------------------------------

    fn running(&self) -> bool {
        running(self.s.transcript.turn())
    }

    fn prompt_awaited(&self) -> bool {
        self.s
            .rpc
            .outgoing()
            .values()
            .any(|purpose| *purpose == Purpose::Prompt)
    }

    /// The ACP session id when prompts can be sent, else why not.
    fn readiness(&self) -> Result<String, Refused> {
        match &self.s.phase {
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Choosing { .. }
            | Phase::Restarting { .. } => Err(Refused::NotReady),
            Phase::Ready { session } => Ok(session.clone()),
            Phase::LoginRequired(_) => Err(Refused::LoginRequired),
            Phase::LoggingIn { .. } => Err(Refused::LoggingIn),
            Phase::Refused | Phase::Broken => Err(Refused::Failed {
                message: self.failure(),
            }),
            Phase::Exited => Err(Refused::Exited),
            Phase::Lost => Err(Refused::Lost),
        }
    }

    fn failure(&self) -> String {
        match self.s.transcript.agent() {
            AgentState::Failed { message } => message.clone(),
            AgentState::Starting
            | AgentState::Ready { .. }
            | AgentState::LoginRequired { .. }
            | AgentState::LoggingIn { .. }
            | AgentState::Exited { .. }
            | AgentState::Lost { .. }
            | AgentState::Choosing { .. } => "ACP handling failed".to_owned(),
        }
    }

    /// Whether the agent's lines are still read: not after a failed start,
    /// a failed handling, an exit or a lost holder, nor from an agent hung
    /// up to be started again.
    fn reading(&self) -> bool {
        match self.s.phase {
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Ready { .. }
            | Phase::Choosing { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. } => true,
            Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => false,
        }
    }

    /// Whether the agent's process has ended, by an exit or with its
    /// holder.
    fn ended(&self) -> bool {
        matches!(self.s.phase, Phase::Exited | Phase::Lost)
    }

    fn stdout(&mut self, bytes: &[u8]) {
        for piece in self.s.lines.feed(bytes) {
            if !self.reading() {
                continue;
            }
            match piece {
                Piece::Line(line) => self.line(&line),
                Piece::TooLong { len } => {
                    self.flush_bad();
                    self.notice(
                        NoticeLevel::Warning,
                        format!(
                            "a line of {len} bytes from the agent was over the {} MiB limit and was dropped",
                            super::lines::MAX_LINE / (1024 * 1024)
                        ),
                    );
                }
                // `OutputLost` wrote the notice for the gap.
                Piece::Lost { .. } => {}
            }
        }
    }

    fn output_lost(&mut self) {
        self.s.lines.cut_front();
        if self.ended() {
            return;
        }
        self.notice(
            NoticeLevel::Warning,
            "output from the agent was lost while the terminal server restarted".to_owned(),
        );
        let lost = "output was lost while the terminal server restarted";
        if self.s.rpc.forget(Purpose::Prompt) {
            self.hold_waiting(HoldReason::OutputLost);
            self.s.run = None;
            self.turn(TurnState::Failed {
                message: lost.to_owned(),
            });
            self.mark(Moment::TurnEnded);
        }
        // A login's answer lost with the tail: the user chooses again
        // (`agents-login` decision 4).
        if let Phase::LoggingIn { login, .. } = &self.s.phase
            && self.s.rpc.forget(Purpose::Authenticate)
        {
            let login = Login {
                message: "the login's answer was lost while the terminal server restarted"
                    .to_owned(),
                ..login.clone()
            };
            self.needs_login(login);
            self.mark(Moment::Login);
        }
        let starting = match self.s.phase {
            Phase::Initializing { .. } => self.s.rpc.forget(Purpose::Initialize),
            Phase::Creating { .. } => self.s.rpc.forget(Purpose::NewSession),
            Phase::Resuming { .. } => self.s.rpc.forget(Purpose::Resume),
            Phase::Loading { .. } => self.s.rpc.forget(Purpose::Load),
            Phase::Spawning { .. }
            | Phase::Ready { .. }
            | Phase::Choosing { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => false,
        };
        if starting {
            let message = match self.s.phase {
                Phase::Resuming { .. } | Phase::Loading { .. } => {
                    format!("the agent couldn't continue the conversation: {lost}")
                }
                Phase::Spawning { .. }
                | Phase::Initializing { .. }
                | Phase::Creating { .. }
                | Phase::Ready { .. }
                | Phase::Choosing { .. }
                | Phase::LoginRequired(_)
                | Phase::LoggingIn { .. }
                | Phase::Restarting { .. }
                | Phase::Refused
                | Phase::Broken
                | Phase::Exited
                | Phase::Lost => format!("the agent couldn't start: {lost}"),
            };
            self.refuse(message);
        }
    }

    fn tick(&mut self) {
        if let Some(bad) = self.s.bad
            && self.now >= bad.last + BAD_RUN_QUIET
        {
            self.flush_bad();
        }
    }

    // --- lines from the agent -------------------------------------------

    fn bad_line(&mut self) {
        let lines = self.s.bad.map_or(0, |bad| bad.lines);
        self.s.bad = Some(BadRun {
            lines: lines.saturating_add(1),
            last: self.now,
        });
    }

    /// Writes the run of bad lines as one notice.
    fn flush_bad(&mut self) {
        if let Some(bad) = self.s.bad.take() {
            let text = if bad.lines == 1 {
                "a line from the agent wasn't an ACP message".to_owned()
            } else {
                format!("{} lines from the agent weren't ACP messages", bad.lines)
            };
            self.notice(NoticeLevel::Warning, text);
        }
    }

    fn line(&mut self, line: &[u8]) {
        match envelope::decode(line) {
            Ok(message) => self.message(message),
            // An empty line is no message and no fault.
            Err(Malformed::Blank) => {}
            Err(Malformed::InvalidRequest) => {
                self.bad_line();
                self.out
                    .stdin
                    .extend(self.s.rpc.dialect().encode(Request::Error {
                        id: None,
                        refusal: Refusal::InvalidRequest,
                    }));
                self.mark(Moment::Answered);
            }
            // A batch is skipped and counted, never answered.
            Err(
                Malformed::NotJson
                | Malformed::Batch
                | Malformed::NotObject
                | Malformed::NotJsonRpc
                | Malformed::BadShape,
            ) => self.bad_line(),
        }
    }

    fn message(&mut self, message: Message) {
        match message {
            Message::Request { id, method, params } => {
                self.flush_bad();
                match self.s.rpc.dialect().request_kind(&method) {
                    RequestKind::Permission => self.permission(id, &params),
                    RequestKind::Unoffered => {
                        self.out
                            .stdin
                            .extend(self.s.rpc.dialect().encode(Request::Error {
                                id: Some(&id),
                                refusal: Refusal::MethodNotFound,
                            }));
                        self.mark(Moment::Answered);
                    }
                }
            }
            Message::Notification { method, params } => {
                match self.s.rpc.dialect().notification_kind(&method) {
                    NotificationKind::SessionUpdate => {
                        match self.s.rpc.dialect().session_update(&params) {
                            Ok(update) => {
                                self.flush_bad();
                                self.update(update);
                            }
                            // A muted replay counts even an update Brindle can't
                            // read: it would show nothing either way.
                            Err(_) if self.mute() => {}
                            Err(_) => self.bad_line(),
                        }
                    }
                    NotificationKind::CancelRequest => {
                        match self.s.rpc.dialect().cancel_request(&params) {
                            Ok(id) => {
                                self.flush_bad();
                                self.withdraw(&id);
                            }
                            Err(_) => self.bad_line(),
                        }
                    }
                    NotificationKind::Other => self.flush_bad(),
                }
            }
            Message::Response { id, result } => {
                self.flush_bad();
                if let Some(purpose) = self.s.rpc.answered(&id) {
                    self.response(purpose, Ok(&result));
                }
            }
            Message::Error { id, error } => {
                self.flush_bad();
                if let Some(id) = id
                    && let Some(purpose) = self.s.rpc.answered(&id)
                {
                    self.response(purpose, Err(&error));
                }
            }
        }
    }

    /// Counts an update of a muted replay, saying whether it was one.
    fn mute(&mut self) -> bool {
        match &mut self.s.phase {
            Phase::Loading {
                replay: Replay::Muted { updates },
                ..
            } => {
                *updates = updates.saturating_add(1);
                true
            }
            Phase::Loading {
                replay: Replay::Shown,
                ..
            }
            | Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Ready { .. }
            | Phase::Choosing { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => false,
        }
    }

    fn update(&mut self, update: forms::SessionUpdate) {
        // Only the conversation's own updates count once it is known.
        let own = match &self.s.phase {
            Phase::Ready { session: id }
            | Phase::Resuming {
                conversation: id, ..
            }
            | Phase::Loading {
                conversation: id, ..
            } => *id == update.session_id,
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Choosing { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => true,
        };
        if !own {
            return;
        }
        // A title is the conversation's state, not its log: a muted load's
        // replay applies it and doesn't count it, so the agent's latest
        // title is the one kept.
        if let Update::SessionInfo { title } = update.update {
            return self.retitle(&update.session_id, title);
        }
        // So are the commands and options (`agents-commands`, decision
        // 12): applied, muted or shown, and never counted.
        let update = match update.update {
            Update::Commands {
                commands,
                unreadable,
            } => return self.offer_commands(commands, unreadable),
            Update::Options(offered) => {
                let merged = self.s.rpc.dialect().merge_options(&self.held_options(), offered);
                return self.offer_options(merged, self.comparing());
            }
            Update::Current { key, value } => return self.set_current(&key, value),
            update @ (Update::UserChunk(_)
            | Update::AgentChunk(_)
            | Update::ThoughtChunk(_)
            | Update::ToolCall(_)
            | Update::ToolChanged(_)
            | Update::SessionInfo { .. }
            | Update::Ignored { .. }) => update,
        };
        if self.mute() {
            return;
        }
        match update {
            Update::UserChunk(content) => self.chunk(RunKind::User, content),
            Update::AgentChunk(content) => self.chunk(RunKind::Agent, content),
            Update::ThoughtChunk(content) => self.chunk(RunKind::Thought, content),
            Update::ToolCall(call) => self.tool_call(call),
            Update::ToolChanged(change) => self.tool_change(change),
            // Applied above.
            Update::SessionInfo { .. }
            | Update::Commands { .. }
            | Update::Options(_)
            | Update::Current { .. }
            | Update::Ignored { .. } => {}
        }
    }

    /// A `session_info_update`'s title for ACP session `session`: the
    /// conversation again with the new title when it differs, else nothing.
    /// Before the agent gave the session (no conversation in the log, or
    /// another session's), it names nothing Brindle knows.
    fn retitle(&mut self, session: &str, title: Patch<String>) {
        let title = match title {
            Patch::Keep => return,
            Patch::Clear => None,
            Patch::Set(raw) => session_title(&raw),
        };
        let Some(current) = self.s.transcript.conversation() else {
            return;
        };
        if current.id != session || current.title == title {
            return;
        }
        let conversation = Conversation {
            title,
            ..current.clone()
        };
        self.emit(Event::Conversation { conversation });
    }

    fn chunk(&mut self, kind: RunKind, content: Content) {
        let text = match content {
            Content::Text(text) => clean(&text),
            Content::Other(kind) => placeholder(kind).to_owned(),
        };
        if text.is_empty() {
            return;
        }
        self.text_run(kind, &text);
    }

    // --- answers to the core's requests ---------------------------------

    fn response(&mut self, purpose: Purpose, result: Result<&Value, &RpcError>) {
        match purpose {
            Purpose::Initialize => self.initialized(result),
            Purpose::NewSession => self.created(result),
            Purpose::Prompt => self.prompted(result),
            Purpose::Resume | Purpose::Load => self.continued(result),
            Purpose::Authenticate => self.authenticated(result),
            Purpose::List { req } => self.listed(req, result),
            Purpose::SetOption { id, value } => self.option_set(&id, &value, result),
        }
    }

    // --- client commands ------------------------------------------------

    fn command(&mut self, command: Command) {
        let reply = match command {
            Command::Prompt { text } => self.prompt(&text),
            Command::Unqueue => self.unqueue(),
            Command::Resend => self.resend(),
            Command::Cancel => {
                self.cancel();
                return;
            }
            Command::Answer { entry, option } => self.answer(entry, option),
            Command::Login { method } => self.login(method),
            Command::List { req, cursor } => self.list(req, cursor),
            Command::Choose { conversation } => self.choose_conversation(conversation),
            Command::SetOption { option, value } => self.set_option(option, value),
        };
        self.out.reply = Some(reply);
    }

    // --- the log's bound ------------------------------------------------

    /// Once the log passes [`LOG_LIMIT`] (decision 14), drops the oldest
    /// entries down to [`LOG_TRIM_TO`], and what pointed at them: the tool
    /// ids, the chunk run, and a permission request still waiting, which is
    /// answered `cancelled`.
    fn trim(&mut self) {
        if self.s.transcript.encoded_len() <= LOG_LIMIT {
            return;
        }
        let Some(event) = self.s.transcript.trim_to(LOG_TRIM_TO) else {
            return;
        };
        let through = match &event {
            Event::Trimmed { through, .. } => *through,
            Event::Added { .. }
            | Event::Appended { .. }
            | Event::ToolChanged { .. }
            | Event::ToolItems { .. }
            | Event::ToolOutput { .. }
            | Event::Answered { .. }
            | Event::Queue { .. }
            | Event::Turn { .. }
            | Event::Agent { .. }
            | Event::Conversation { .. }
            | Event::Commands { .. }
            | Event::Options { .. }
            | Event::Window { .. } => return,
        };
        self.out.events.push(event);
        self.s.tools.retain(|_, entry| *entry > through);
        self.s.outputs.retain(|_, open| open.entry > through);
        if self.s.run.is_some_and(|run| run.entry <= through) {
            self.s.run = None;
        }
        let dropped: Vec<EntryId> = self
            .s
            .rpc
            .permissions()
            .keys()
            .copied()
            .filter(|entry| *entry <= through)
            .collect();
        for entry in &dropped {
            if let Some(answer) = self.s.rpc.answer_permission(*entry, None) {
                self.out.stdin.extend(answer);
            }
        }
        if !dropped.is_empty() {
            self.notice(
                NoticeLevel::Warning,
                format!(
                    "{} waiting permission request(s) left the log and were answered cancelled",
                    dropped.len()
                ),
            );
            self.unblock();
            self.mark(Moment::Permission);
        }
    }
}

fn running(turn: &TurnState) -> bool {
    match turn {
        TurnState::Working | TurnState::Blocked | TurnState::Cancelling => true,
        TurnState::Idle | TurnState::Ended { .. } | TurnState::Failed { .. } => false,
    }
}

/// Why an event of `len` bytes, past `limit`, wasn't sent: its kind (an
/// added one's entry kind too) and the entry it is for, or the session's.
fn oversized(event: &Event, len: usize, limit: usize) -> String {
    let kind = event.kind();
    let entry = match event {
        Event::Added { id, entry } => Some((*id, Some(entry.kind()))),
        Event::Appended { id, .. }
        | Event::ToolChanged { id, .. }
        | Event::ToolItems { id, .. }
        | Event::ToolOutput { id, .. }
        | Event::Answered { id, .. } => Some((*id, None)),
        Event::Queue { .. }
        | Event::Turn { .. }
        | Event::Agent { .. }
        | Event::Conversation { .. }
        | Event::Trimmed { .. }
        | Event::Window { .. }
        | Event::Commands { .. }
        | Event::Options { .. } => None,
    };
    let rest = format!("is {len} bytes; one transcript event may be at most {limit}");
    match entry {
        Some((id, entry_kind)) => {
            let article = if kind.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
            let entry_kind = entry_kind.map(|kind| format!(" ({kind})")).unwrap_or_default();
            format!("{article} {kind}{entry_kind} event for entry {id} {rest}")
        }
        None => format!("the session's {kind} event {rest}"),
    }
}

/// The bytes a user, agent or thought entry's text holds; `None` for any
/// other entry.
fn text_len(entry: &Entry) -> Option<usize> {
    match entry {
        Entry::User { text } | Entry::Agent { text } | Entry::Thought { text } => Some(text.len()),
        Entry::Tool { .. } | Entry::Permission { .. } | Entry::Notice { .. } => None,
    }
}

/// `text` split where an entry with `room` bytes left fills: all of it
/// when it fits; else up to the last line break within `room` when there
/// is one, or the last character boundary within it.
fn within_room(text: &str, room: usize) -> (&str, &str) {
    if text.len() <= room {
        return (text, "");
    }
    let mut cut = room;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let cut = text[..cut].rfind('\n').map_or(cut, |at| at + 1);
    text.split_at(cut)
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::{self, json};
    use brindle_transcript::{
        Activity, CALL_LIMIT, Continuation, DiffLine, Exit, FileChange, FileOp, MAX_ITEMS,
        MAX_LOCATIONS, MAX_PATH, OptionCategory, OptionControl, Output as ToolOutputItem,
        PermissionOption, PermissionOutcome, RAW_LIMIT, SessionOption, StopReason, TEXT_CHUNK,
        ToolItem, ToolKind, ToolStatus,
    };

    use super::super::envelope::Id;
    use super::super::forms::AUTH_REQUIRED;
    use super::super::limits::{FIELD_LIMIT, MAX_PERMISSION_NAME, MAX_PERMISSION_OPTIONS};
    use super::harness::*;
    use super::options::typeable;
    use super::text::{MAX_SESSION_TITLE, cut_at, raw_text};
    use super::*;

    // --- chunk runs -----------------------------------------------------

    #[test]
    fn a_chunk_of_another_kind_ends_the_run() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&chunk("agent_thought_chunk", "hmm"));
        agent.feed(&said("yes"));
        agent.feed(&chunk("agent_thought_chunk", "more"));
        let kinds: Vec<&str> = agent.entries().iter().map(entry_kind).collect();
        assert_eq!(kinds, ["user", "thought", "agent", "thought"]);
    }

    #[test]
    fn a_tool_call_ends_the_run() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&said("a"));
        agent.feed(&tool_call("call_1", "Write", "pending"));
        agent.feed(&said("b"));
        let kinds: Vec<&str> = agent.entries().iter().map(entry_kind).collect();
        assert_eq!(kinds, ["user", "agent", "tool", "agent"]);
    }

    #[test]
    fn a_turns_end_ends_the_run() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.feed(&said("a"));
        agent.end(id, "end_turn");
        agent.feed(&said("b"));
        let kinds: Vec<&str> = agent.entries().iter().map(entry_kind).collect();
        assert_eq!(kinds, ["user", "agent", "agent"]);
    }

    #[test]
    fn a_user_prompt_is_no_run_the_agents_user_chunks_extend() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&chunk("user_message_chunk", "echo"));
        let kinds: Vec<&str> = agent.entries().iter().map(entry_kind).collect();
        assert_eq!(kinds, ["user", "user"]);
    }

    #[test]
    fn a_chunk_that_isnt_text_shows_a_placeholder_in_the_run() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&said("look: "));
        agent.feed(&update(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "image", "data": "AA==", "mimeType": "image/png"}})));
        assert_eq!(
            agent.last_entry().1,
            Entry::Agent {
                text: "look: [image]".into()
            }
        );
    }

    #[test]
    fn a_chunk_over_64_kib_is_split_into_appends() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let out = agent.feed(&said(&"é".repeat(TEXT_CHUNK)));
        assert_eq!(shape(&out.events), ["added agent", "appended"]);
        for event in &out.events {
            if let Event::Appended { text, .. }
            | Event::Added {
                entry: Entry::Agent { text },
                ..
            } = event
            {
                assert!(text.len() <= TEXT_CHUNK);
            }
        }
    }

    // --- messages the core doesn't take ---------------------------------

    #[test]
    fn a_request_for_a_method_not_offered_is_answered_method_not_found() {
        let mut agent = Agent::ready();
        let out = agent.feed(r#"{"jsonrpc":"2.0","id":4,"method":"fs/read_text_file","params":{"path":"/etc/passwd"}}"#);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":4,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}\n"
        );
        assert!(out.events.is_empty());
    }

    #[test]
    fn an_unknown_notification_is_ignored() {
        let mut agent = Agent::ready();
        let out = agent.feed(r#"{"jsonrpc":"2.0","method":"_vendor/ping","params":{}}"#);
        assert_eq!(out, Output::default());
    }

    #[test]
    fn an_unknown_update_kind_changes_nothing_and_the_next_chunk_is_added() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let out = agent.feed(&update(
            json!({"sessionUpdate": "brand_new_kind", "payload": {"x": 1}}),
        ));
        assert!(out.events.is_empty());
        agent.feed(&said("hi"));
        assert_eq!(agent.last_entry().1, Entry::Agent { text: "hi".into() });
        assert!(agent.notices().is_empty());
    }


    // --- bad lines ------------------------------------------------------

    #[test]
    fn a_run_of_bad_lines_is_one_notice_with_its_count_when_a_message_follows() {
        let mut agent = Agent::ready();
        agent.turn("one");
        for line in ["hello world", "[1,2]", "42", r#"{"no":"jsonrpc"}"#] {
            assert!(agent.feed(line).events.is_empty());
        }
        let out = agent.feed(&said("hi"));
        assert_eq!(shape(&out.events), ["added notice", "added agent"]);
        assert_eq!(
            agent.notices(),
            ["4 lines from the agent weren't ACP messages"]
        );
        assert_eq!(out.wake, None);
    }

    #[test]
    fn one_bad_line_says_a_line_wasnt_an_acp_message() {
        let mut agent = Agent::ready();
        agent.feed("hello world");
        agent.feed(&said("hi"));
        assert_eq!(
            agent.notices(),
            ["a line from the agent wasn't an ACP message"]
        );
    }

    #[test]
    fn a_run_of_bad_lines_is_written_once_a_quiet_second_has_passed() {
        let mut agent = Agent::ready();
        let out = agent.feed("hello world");
        assert_eq!(out.wake, Some(agent.now + BAD_RUN_QUIET));
        agent.now += BAD_RUN_QUIET / 2;
        assert!(agent.step(Input::Tick).events.is_empty());
        agent.now += BAD_RUN_QUIET;
        let out = agent.step(Input::Tick);
        assert_eq!(shape(&out.events), ["added notice"]);
        assert_eq!(out.wake, None);
    }

    #[test]
    fn a_blank_line_is_no_message_and_isnt_counted() {
        let mut agent = Agent::ready();
        let out = agent.step(Input::Stdout(b"\n   \r\n"));
        assert_eq!(out, Output::default());
    }

    #[test]
    fn a_batch_is_counted_and_not_answered() {
        let mut agent = Agent::ready();
        let out = agent.feed(r#"[{"jsonrpc":"2.0","id":1,"method":"fs/read_text_file"}]"#);
        assert!(out.stdin.is_empty());
        assert!(out.wake.is_some());
    }

    #[test]
    fn a_request_with_an_unusable_id_is_answered_invalid_request_and_counted() {
        let mut agent = Agent::ready();
        let out = agent.feed(r#"{"jsonrpc":"2.0","id":null,"method":"fs/read_text_file"}"#);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32600,\"message\":\"Invalid Request\"}}\n"
        );
        assert!(out.wake.is_some());
    }

    /// C-5 (`agents-process-lifecycle` decision 8): the line read reaches
    /// the holder, as consumed, ahead of the server's own answer, so a new
    /// core doesn't read it again and answer twice.
    #[test]
    fn an_unoffered_request_is_checkpointed_before_its_answer() {
        let mut agent = Agent::ready();
        let out = agent.feed(r#"{"jsonrpc":"2.0","id":4,"method":"fs/read_text_file","params":{"path":"/etc/passwd"}}"#);
        assert_eq!(out.checkpoint, Some(Moment::Answered));
        assert!(String::from_utf8_lossy(&out.stdin).contains("-32601"));
    }

    #[test]
    fn an_invalid_request_is_checkpointed_before_its_answer() {
        let mut agent = Agent::ready();
        let out = agent.feed(r#"{"jsonrpc":"2.0","id":null,"method":"fs/read_text_file"}"#);
        assert_eq!(out.checkpoint, Some(Moment::Answered));
        assert!(String::from_utf8_lossy(&out.stdin).contains("-32600"));
    }

    #[test]
    fn params_of_the_wrong_shape_count_as_a_bad_line() {
        let mut agent = Agent::ready();
        agent.feed(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1"}}"#);
        agent.feed(&said("hi"));
        assert_eq!(
            agent.notices(),
            ["a line from the agent wasn't an ACP message"]
        );
    }

    #[test]
    fn a_line_over_8_mib_is_dropped_with_a_notice() {
        let mut agent = Agent::ready();
        let mut line = vec![b'x'; super::super::lines::MAX_LINE + 1];
        line.push(b'\n');
        let out = agent.step(Input::Stdout(&line));
        assert_eq!(shape(&out.events), ["added notice"]);
        assert!(agent.notices()[0].contains("8 MiB"));
    }

    #[test]
    fn control_characters_become_u_fffd_but_tab_newline_and_crlf_stay() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&said("a\u{1b}[2Jb\tc\r\nd\re\u{9b}f\u{7f}é"));
        assert_eq!(
            agent.last_entry().1,
            Entry::Agent {
                text: "a\u{fffd}[2Jb\tc\r\nd\u{fffd}e\u{fffd}f\u{fffd}é".into()
            }
        );
    }

    #[test]
    fn a_crlf_cut_between_chunks_is_kept() {
        assert_eq!(clean("a\r"), "a\r");
        assert_eq!(clean("\nb"), "\nb");
        assert_eq!(clean("a\rb"), "a\u{fffd}b");
    }

    #[test]
    fn text_from_the_agent_passes_unchanged_through_the_pipes() {
        let mut agent = Agent::ready();
        agent.turn("one");
        // The bytes arrive cut inside a UTF-8 sequence.
        let line = format!("{}\n", said("tab\there\r\nend ünïcödé 🦀"));
        let (front, back) = line
            .as_bytes()
            .split_at(line.find('🦀').expect("a crab") + 2);
        agent.step(Input::Stdout(front));
        agent.step(Input::Stdout(back));
        assert_eq!(
            agent.last_entry().1,
            Entry::Agent {
                text: "tab\there\r\nend ünïcödé 🦀".into()
            }
        );
    }

    #[test]
    fn a_users_prompt_is_cleaned_before_it_is_logged_and_sent() {
        let mut agent = Agent::ready();
        agent.prompt("a\u{1b}b");
        assert_eq!(
            agent.entries()[0],
            Entry::User {
                text: "a\u{fffd}b".into()
            }
        );
        assert_eq!(
            agent.last("session/prompt")["params"]["prompt"][0]["text"],
            "a\u{fffd}b"
        );
    }


    // --- the log's bound (decision 14) ----------------------------------

    /// Feeds alternating agent and thought chunks of 1 MiB, each its own
    /// entry, until the log is past its bound.
    fn fill(agent: &mut Agent) -> Vec<Output> {
        let text = "x".repeat(1024 * 1024);
        (0..18)
            .map(|n| {
                let kind = if n % 2 == 0 {
                    "agent_message_chunk"
                } else {
                    "agent_thought_chunk"
                };
                agent.feed(&chunk(kind, &text))
            })
            .collect()
    }

    #[test]
    fn the_log_is_trimmed_past_16_mib_keeping_the_newest_entries() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let outs = fill(&mut agent);
        // One trim for the crossing, down to the low mark, not one per
        // entry past the limit.
        let trims: Vec<usize> = outs
            .iter()
            .enumerate()
            .filter(|(_, out)| {
                out.events
                    .iter()
                    .any(|event| matches!(event, Event::Trimmed { .. }))
            })
            .map(|(n, _)| n)
            .collect();
        assert_eq!(trims.len(), 1, "{trims:?}");
        let mut at_trim = Agent::ready();
        at_trim.turn("one");
        let text = "x".repeat(1024 * 1024);
        for n in 0..=trims[0] {
            let kind = if n % 2 == 0 {
                "agent_message_chunk"
            } else {
                "agent_thought_chunk"
            };
            at_trim.feed(&chunk(kind, &text));
        }
        assert!(at_trim.s.transcript.encoded_len() <= LOG_TRIM_TO);
        assert!(at_trim.s.transcript.encoded_len() > LOG_TRIM_TO - 2 * 1024 * 1024);
        assert!(agent.s.transcript.encoded_len() <= LOG_LIMIT);
        assert!(agent.s.transcript.dropped() > 0);
        let replay = agent.s.transcript.replay();
        assert!(
            matches!(replay[0], Event::Trimmed { .. }),
            "a replay starts with it"
        );
        assert_eq!(
            agent.s.transcript.next_id(),
            EntryId(20),
            "the newest entries stay"
        );
    }

    #[test]
    fn trimming_answers_a_waiting_permission_it_drops_and_forgets_its_tool_ids() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Write", "pending"));
        agent.ask(json!(0), &ALLOW_REJECT);
        let outs = fill(&mut agent);
        let answered: Vec<u8> = outs.iter().flat_map(|out| out.stdin.clone()).collect();
        assert_eq!(
            answered,
            b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"outcome\":{\"outcome\":\"cancelled\"}}}\n"
        );
        assert!(agent.s.rpc().permissions().is_empty());
        assert!(agent.s.tools.is_empty());
        assert_eq!(*agent.turn_state(), TurnState::Working);
    }

    /// The client's copy and a checkpoint's segments fold what the reducer
    /// emits, so the log changes only through `Output.events`
    /// (`agents-log-paging` decision 6). `Agent::step` checks the client's
    /// fold after every step; this folds from a base taken mid-session
    /// across every kind of change: a turn, a tool call and a permission,
    /// a retitle, a login demanded, an exit and a restart, and a muted load.
    #[test]
    fn the_log_changes_only_through_the_events_a_step_emits() {
        let mut agent = Agent::ready_offering(both());
        agent.turn("zero");
        let mut copy = Transcript::new();
        for event in agent.s.transcript().replay() {
            copy.apply(&event).expect("the replay folds");
        }
        let from = agent.events.len();
        let id = agent.prompt_id();
        agent.feed(&tool_call("call_1", "Write", "pending"));
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        agent.cmd(Command::Answer {
            entry,
            option: Some("allow".into()),
        });
        agent.feed(&tool_update("call_1", "completed"));
        agent.feed(&said("Done."));
        agent.end(id, "end_turn");
        agent.feed(&update(
            json!({"sessionUpdate": "session_info_update", "title": "A title"}),
        ));
        let id = agent.turn("one");
        agent.prompt("two");
        agent.feed(&error(id, AUTH_REQUIRED, "Authentication required"));
        assert!(matches!(agent.agent_state(), AgentState::LoginRequired { .. }));
        agent.exit();
        let id = agent.restart();
        agent.feed(&initialized(id, load_only()));
        agent.feed(&said("replayed"));
        agent.feed(&permission(json!(7), "call_2", &ALLOW_REJECT));
        let load = agent.last("session/load")["id"].as_u64().expect("an id");
        agent.feed(&result(load, json!({})));
        assert!(
            agent
                .notices()
                .iter()
                .any(|notice| notice.contains("wasn't shown")),
            "{:?}",
            agent.notices()
        );
        for event in &agent.events[from..] {
            copy.apply(event).expect("each event folds");
        }
        assert_eq!(&copy, agent.s.transcript());
    }

    // --- activity (decision 15's table, row by row) ---------------------

    #[test]
    fn a_starting_agent_is_working() {
        let mut agent = Agent::new();
        assert_eq!(agent.activity(), Activity::Working);
        agent.step(Input::Spawned);
        assert_eq!(agent.activity(), Activity::Working);
    }

    #[test]
    fn a_running_or_cancelling_turn_is_working() {
        let mut agent = Agent::ready();
        agent.turn("one");
        assert_eq!(agent.activity(), Activity::Working);
        agent.cmd(Command::Cancel);
        assert_eq!(agent.activity(), Activity::Working);
    }

    #[test]
    fn a_waiting_permission_request_is_blocked() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.ask(json!(0), &ALLOW_REJECT);
        assert_eq!(agent.activity(), Activity::Blocked);
    }

    #[test]
    fn a_turn_ended_with_end_turn_is_done() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.end(id, "end_turn");
        assert_eq!(agent.activity(), Activity::Done);
    }

    #[test]
    fn a_new_session_or_a_cancelled_turn_is_idle() {
        let mut agent = Agent::ready();
        assert_eq!(agent.activity(), Activity::Idle);
        let id = agent.turn("one");
        agent.cmd(Command::Cancel);
        agent.end(id, "cancelled");
        assert_eq!(agent.activity(), Activity::Idle);
    }

    #[test]
    fn another_stop_a_failed_turn_a_login_a_failure_or_an_exit_is_an_error() {
        for stop in ["max_tokens", "max_turn_requests", "refusal", "paused"] {
            let mut agent = Agent::ready();
            let id = agent.turn("one");
            agent.end(id, stop);
            assert_eq!(agent.activity(), Activity::Error, "{stop}");
        }
        let mut failed = Agent::ready();
        let id = failed.turn("one");
        failed.feed(&error(id, -1, "no"));
        assert_eq!(failed.activity(), Activity::Error);

        let mut login = Agent::new();
        login.step(Input::Spawned);
        login.feed(&result(1, json!({"protocolVersion": 1})));
        login.feed(&error(2, -32000, "log in"));
        assert_eq!(login.activity(), Activity::Error);

        let mut broken = Agent::ready();
        broken.step(Input::Fault { reason: "x".into() });
        assert_eq!(broken.activity(), Activity::Error);

        let mut exited = Agent::ready();
        exited.step(Input::Exited {
            code: Some(0),
            stderr: b"",
        });
        assert_eq!(exited.activity(), Activity::Error);
    }

    #[test]
    fn the_slot_never_changes_the_activity() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        let before = agent.activity();
        agent.prompt("two");
        assert_eq!(agent.activity(), before);
        agent.end(id, "max_tokens");
        let before = agent.activity();
        agent.cmd(Command::Unqueue);
        assert_eq!(agent.activity(), before);
    }

    // --- the captured sessions ------------------------------------------

    const CAPTURES: [(&str, &[u8]); 14] = [
        (
            "antigravity-1.5.16-reject",
            include_bytes!("../../../tests/fixtures/acp/antigravity-1.5.16-reject.log"),
        ),
        (
            "antigravity-1.5.16-trusted-workspace",
            include_bytes!("../../../tests/fixtures/acp/antigravity-1.5.16-trusted-workspace.log"),
        ),
        (
            "claude-agent-acp-0.112.0-auto",
            include_bytes!("../../../tests/fixtures/acp/claude-agent-acp-0.112.0-auto.log"),
        ),
        (
            "claude-agent-acp-0.112.0-haiku-reject",
            include_bytes!("../../../tests/fixtures/acp/claude-agent-acp-0.112.0-haiku-reject.log"),
        ),
        (
            "claude-agent-acp-0.112.0-reject",
            include_bytes!("../../../tests/fixtures/acp/claude-agent-acp-0.112.0-reject.log"),
        ),
        (
            "claude-code-acp-0.16.2-reject",
            include_bytes!("../../../tests/fixtures/acp/claude-code-acp-0.16.2-reject.log"),
        ),
        (
            "codex-acp-2.12.0-reject",
            include_bytes!("../../../tests/fixtures/acp/codex-acp-2.12.0-reject.log"),
        ),
        (
            "copilot-1.0.115-reject",
            include_bytes!("../../../tests/fixtures/acp/copilot-1.0.115-reject.log"),
        ),
        // `agents-tool-content` task 1.1: an allowed edit of an existing
        // file and an allowed `seq 1 4`, with today's `initialize` and once
        // more offering `clientCapabilities.terminal`.
        (
            "antigravity-1.5.16-edit-run",
            include_bytes!("../../../tests/fixtures/acp/antigravity-1.5.16-edit-run.log"),
        ),
        (
            "antigravity-1.5.16-edit-run-terminal",
            include_bytes!("../../../tests/fixtures/acp/antigravity-1.5.16-edit-run-terminal.log"),
        ),
        (
            "claude-agent-acp-0.112.0-edit-run",
            include_bytes!("../../../tests/fixtures/acp/claude-agent-acp-0.112.0-edit-run.log"),
        ),
        (
            "claude-agent-acp-0.112.0-edit-run-terminal",
            include_bytes!(
                "../../../tests/fixtures/acp/claude-agent-acp-0.112.0-edit-run-terminal.log"
            ),
        ),
        (
            "codex-acp-2.12.0-edit-run",
            include_bytes!("../../../tests/fixtures/acp/codex-acp-2.12.0-edit-run.log"),
        ),
        (
            "codex-acp-2.12.0-edit-run-terminal",
            include_bytes!("../../../tests/fixtures/acp/codex-acp-2.12.0-edit-run-terminal.log"),
        ),
    ];

    /// Hand-written lines with values no agent sent yet, for a session
    /// already ready in `sess-1`.
    const HANDWRITTEN: &[u8] =
        include_bytes!("../../../tests/fixtures/acp/handwritten-unknown-values.log");

    const GEMINI: &[u8] =
        include_bytes!("../../../tests/fixtures/acp/gemini-0.71.1-login-on-stdout.log");

    /// Plays a captured session through the reducer: the agent's lines are
    /// fed (the answers to the core's requests renumbered to the ids the
    /// core gave, and those to requests Brindle doesn't send skipped), the
    /// capture's prompts are sent as commands (`after_prompt` runs after
    /// each), and its permission answers are given by the option's kind,
    /// checking the line written against the captured one.
    fn replay(log: &[u8], after_prompt: impl FnMut(&mut Agent)) -> Agent {
        let mut agent = Agent::in_dir("/home/user/scratch");
        agent.step(Input::Spawned);
        replay_into(agent, log, after_prompt)
    }

    fn replay_into(
        mut agent: Agent,
        log: &[u8],
        mut after_prompt: impl FnMut(&mut Agent),
    ) -> Agent {
        let mut ours: Vec<(Value, u64)> = Vec::new();
        let mut skipped: Vec<Value> = Vec::new();
        for line in log.split(|b| *b == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if let Some(sent) = line.strip_prefix(b"c>a\t") {
                let sent: Value = serde_json::from_slice(sent).expect("a client line");
                let id = sent["id"].clone();
                match sent["method"].as_str() {
                    Some(method @ ("initialize" | "session/new")) => {
                        let line = agent.last(method).clone();
                        if line["id"] == id {
                            // The captures before `agents-login` sent no
                            // `auth.terminal`; rpc's tests pin the
                            // capabilities against the later capture.
                            let (mut ours_line, mut sent) = (line.clone(), sent.clone());
                            if method == "initialize" {
                                ours_line["params"]["clientCapabilities"] = Value::Null;
                                sent["params"]["clientCapabilities"] = Value::Null;
                            }
                            assert_eq!(ours_line, sent, "the core's {method}");
                        }
                        ours.push((id, line["id"].as_u64().expect("an id")));
                    }
                    Some("session/prompt") => {
                        let text = sent["params"]["prompt"][0]["text"].as_str().expect("text");
                        assert_eq!(agent.prompt(text).reply, Some(Ok(())));
                        ours.push((id, agent.prompt_id()));
                        after_prompt(&mut agent);
                    }
                    Some(_) => skipped.push(id),
                    None => {
                        let wire = Id::parse(&id).expect("an id");
                        let entry = *agent
                            .s
                            .rpc()
                            .permissions()
                            .iter()
                            .find(|(_, waiting)| **waiting == wire)
                            .expect("a waiting request")
                            .0;
                        let Some(Entry::Permission { options, .. }) =
                            agent.s.transcript.entry(entry)
                        else {
                            panic!("no permission entry");
                        };
                        let captured = sent["result"]["outcome"]["optionId"]
                            .as_str()
                            .expect("an option");
                        let kind = options
                            .iter()
                            .find(|o| o.id == captured)
                            .expect("offered")
                            .kind;
                        let chosen = options
                            .iter()
                            .find(|o| o.kind == kind)
                            .expect("a kind")
                            .id
                            .clone();
                        let out = agent.cmd(Command::Answer {
                            entry,
                            option: Some(chosen),
                        });
                        assert_eq!(out.reply, Some(Ok(())));
                        assert_eq!(
                            out.stdin,
                            [&serde_json::to_vec(&sent).expect("json")[..], b"\n"].concat()
                        );
                    }
                }
            } else if let Some(from_agent) = line.strip_prefix(b"a>c\t") {
                let mut bytes = from_agent.to_vec();
                if let Ok(mut message) = serde_json::from_slice::<Value>(from_agent)
                    && message.get("method").is_none()
                    && let Some(id) = message.get("id").cloned()
                {
                    if skipped.contains(&id) {
                        continue;
                    }
                    if let Some((_, own)) = ours.iter().find(|(captured, _)| *captured == id) {
                        message["id"] = Value::from(*own);
                    }
                    bytes = message.to_string().into_bytes();
                }
                bytes.push(b'\n');
                agent.step(Input::Stdout(&bytes));
            }
        }
        agent
    }

    #[test]
    fn every_captured_session_reduces_with_no_fault_and_the_clients_fold_agreeing() {
        for (name, log) in CAPTURES {
            let agent = replay(log, |_| {});
            assert!(
                matches!(agent.s.phase(), Phase::Ready { .. }),
                "{name}: {:?}",
                agent.s.phase()
            );
            assert!(
                matches!(agent.turn_state(), TurnState::Ended { .. }),
                "{name}: {:?}",
                agent.turn_state()
            );
            assert!(agent.s.rpc().outgoing().is_empty(), "{name}");
        }
    }

    /// Each captured agent's options and commands (`agents-commands` task
    /// 2.5): the config options in the agent's order with their
    /// categories, Claude's `fast` and Copilot's `allow_all` as categories
    /// of their own, claude-code-acp's modes and model list as one option
    /// each, and never a second mode or model option for the agents that
    /// send both.
    #[test]
    fn every_captured_session_holds_its_agents_options_and_commands() {
        let category = |option: &SessionOption| match &option.category {
            OptionCategory::Mode => "mode".to_owned(),
            OptionCategory::Model => "model".to_owned(),
            OptionCategory::ModelConfig => "model_config".to_owned(),
            OptionCategory::ThoughtLevel => "thought_level".to_owned(),
            OptionCategory::Other { name } => format!("other:{name}"),
        };
        for (name, log) in CAPTURES {
            let agent = replay(log, |_| {});
            let options: Vec<(String, String)> = agent
                .s
                .transcript
                .options()
                .iter()
                .map(|option| (option.id.clone(), category(option)))
                .collect();
            let options: Vec<(&str, &str)> =
                options.iter().map(|(id, category)| (id.as_str(), category.as_str())).collect();
            let expected: &[(&str, &str)] = match name.split('-').next() {
                Some("antigravity") => {
                    &[("model", "model"), ("thought_level", "thought_level"), ("mode", "mode")]
                }
                Some("claude") if name.starts_with("claude-agent") => &[
                    ("model", "model"),
                    ("effort", "thought_level"),
                    ("fast", "other:speed"),
                    ("mode", "mode"),
                ],
                Some("claude") => &[("mode", "mode"), ("model", "model")],
                Some("codex") => &[
                    ("model", "model"),
                    ("reasoning_effort", "thought_level"),
                    ("mode", "mode"),
                    ("collaboration_mode", "other:collaboration_mode"),
                ],
                Some("copilot") => {
                    &[("mode", "mode"), ("model", "model"), ("allow_all", "other:permissions")]
                }
                other => panic!("{name}: {other:?}"),
            };
            assert_eq!(options, expected, "{name}");
            assert_eq!(agent.s.keys.len(), options.len(), "{name}");
            for option in agent.s.transcript.options() {
                let OptionControl::Select { choices, .. } = &option.kind else {
                    panic!("{name}: {} isn't a select", option.id);
                };
                assert!(choices.len() >= 2, "{name}: {}", option.id);
            }
            if name.starts_with("claude-agent") {
                let fast = &agent.s.transcript.options()[2];
                assert!(
                    matches!(&fast.kind, OptionControl::Select { choices, .. } if choices.len() == 2),
                    "{name}"
                );
            }
            // claude-code-acp's are made from its modes and model list (the
            // dialect's own keys, which the reducer never names); every
            // other agent's are config options.
            let config = agent
                .s
                .keys
                .values()
                .filter(|key| matches!(key, OptionKey::Config { .. }))
                .count();
            if name.starts_with("claude-code") {
                assert_eq!(config, 0, "{name}");
                assert_ne!(agent.s.keys["mode"], agent.s.keys["model"], "{name}");
            } else {
                assert_eq!(config, options.len(), "{name}");
            }
            // Every agent offered commands, all typeable.
            let commands = agent.s.transcript.commands();
            assert!(!commands.is_empty(), "{name}");
            assert!(commands.iter().all(|command| typeable(&command.name)), "{name}");
            assert!(agent.notices().iter().all(|text| !text.contains("are kept")), "{name}");
        }
    }

    /// What each captured agent is called and what it named its
    /// conversation (`agents-session-title`): Codex named one only in the
    /// later captures, with its prompt.
    #[test]
    fn every_captured_session_names_its_agent_readably_and_keeps_its_title() {
        for (name, log) in CAPTURES {
            let agent = replay(log, |_| {});
            let edit_run = name.contains("-edit-run");
            let (readable, title) = match name.split('-').next() {
                Some("antigravity") if edit_run => {
                    ("Antigravity ACP Server", Some("Edit Notes File And Run Seq"))
                }
                Some("claude") if edit_run => ("Claude Agent", Some("Notes.txt apple to banana edit")),
                Some("codex") if edit_run => (
                    "Codex",
                    Some("In the current working directory there is a file named notes.txt."),
                ),
                Some("antigravity") => ("Antigravity ACP Server", Some("Create Hello.txt File")),
                Some("claude") if name.starts_with("claude-agent") => {
                    ("Claude Agent", Some("Hello.txt file creation"))
                }
                Some("claude") => ("Claude Code", None),
                Some("codex") => ("Codex", None),
                Some("copilot") => ("Copilot", Some("Please create a file named hello.txt")),
                other => panic!("{name}: {other:?}"),
            };
            assert!(
                matches!(agent.agent_state(), AgentState::Ready { name: Some(n), .. } if n == readable),
                "{name}: {:?}",
                agent.agent_state()
            );
            let kept = title_of(&agent);
            match title {
                None => assert_eq!(kept, None, "{name}"),
                Some(title) => {
                    let kept = kept.unwrap_or_else(|| panic!("{name}: no title"));
                    assert!(kept.starts_with(title), "{name}: {kept}");
                    assert!(kept.chars().count() <= MAX_SESSION_TITLE, "{name}");
                    assert!(!kept.contains('\n'), "{name}");
                }
            }
        }
    }

    #[test]
    fn copilots_captured_turn_merges_its_thoughts_and_answers_the_rejection_by_kind() {
        let agent = replay(CAPTURES[7].1, |_| {});
        let kinds: Vec<&str> = agent.entries().iter().map(entry_kind).collect();
        assert_eq!(kinds, ["user", "thought", "tool", "permission", "agent"]);
        assert!(
            matches!(&agent.entries()[3], Entry::Permission { outcome: Some(PermissionOutcome::Selected { option }), .. } if option == "reject_once")
        );
        assert!(matches!(
            &agent.entries()[2],
            Entry::Tool {
                status: ToolStatus::Failed,
                ..
            }
        ));
        assert_eq!(*agent.turn_state(), ended(StopReason::EndTurn));
        assert_eq!(agent.activity(), Activity::Done);
    }

    #[test]
    fn codexs_cancelled_ending_after_a_rejection_holds_a_waiting_prompt_as_a_stop_not_a_cancel() {
        let agent = replay(CAPTURES[6].1, |agent| {
            assert_eq!(agent.prompt("and then?").reply, Some(Ok(())));
        });
        assert_eq!(*agent.turn_state(), ended(StopReason::Cancelled));
        assert_eq!(agent.activity(), Activity::Idle);
        assert_eq!(
            *agent.slot(),
            held(
                "and then?",
                HoldReason::Stopped {
                    stop: StopReason::Cancelled
                }
            )
        );
        assert_eq!(
            agent
                .sent_methods()
                .iter()
                .filter(|m| **m == "session/cancel")
                .count(),
            0
        );
    }

    /// What each captured agent's tool content becomes
    /// (`agents-tool-content` task 3.7): every entry bounded, nothing
    /// unreadable, and each agent's edit, new file, input and output.
    #[test]
    fn every_captured_tool_call_keeps_its_content_bounded() {
        for (name, log) in CAPTURES {
            let agent = replay(log, |_| {});
            assert!(
                !agent
                    .notices()
                    .iter()
                    .any(|text| text.contains("couldn't be read")),
                "{name}: {:?}",
                agent.notices()
            );
            for entry in agent.entries() {
                if let Entry::Tool {
                    content,
                    input,
                    result,
                    locations,
                    more,
                    ..
                } = &entry
                {
                    let len: usize = content.iter().map(ToolItem::wire_len).sum();
                    assert!(content.len() <= MAX_ITEMS && len <= CALL_LIMIT, "{name}");
                    assert_eq!(*more, 0, "{name}");
                    assert!(locations.len() <= MAX_LOCATIONS, "{name}");
                    for raw in [input, result].into_iter().flatten() {
                        assert!(raw.len() <= RAW_LIMIT, "{name}");
                    }
                }
            }
        }
    }

    fn tools(agent: &Agent) -> Vec<Entry> {
        agent
            .entries()
            .into_iter()
            .filter(|entry| matches!(entry, Entry::Tool { .. }))
            .collect()
    }

    /// The tool items events each tool entry was sent.
    fn item_events(agent: &Agent, entry: EntryId) -> usize {
        agent
            .events
            .iter()
            .filter(|event| matches!(event, Event::ToolItems { id, .. } if *id == entry))
            .count()
    }

    fn capture_named(name: &str) -> &'static [u8] {
        CAPTURES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, log)| *log)
            .unwrap_or_else(|| panic!("no capture {name}"))
    }

    const NOTES_EDIT: [&str; 6] = [
        " first line",
        " second line",
        "-an apple a day",
        "+an banana a day",
        " fourth line",
        " fifth line",
    ];

    fn diff_lines(item: &ToolItem) -> Vec<String> {
        match item {
            ToolItem::Diff(change) => change
                .hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .map(|line| match line {
                    DiffLine::Context(text) => format!(" {text}"),
                    DiffLine::Removed(text) => format!("-{text}"),
                    DiffLine::Added(text) => format!("+{text}"),
                })
                .collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn claudes_and_antigravitys_captured_edits_are_one_file_change_each() {
        for name in [
            "claude-agent-acp-0.112.0-edit-run",
            "claude-agent-acp-0.112.0-edit-run-terminal",
            "antigravity-1.5.16-edit-run",
            "antigravity-1.5.16-edit-run-terminal",
        ] {
            let agent = replay(capture_named(name), |_| {});
            let edit = tools(&agent)
                .into_iter()
                .find(|entry| {
                    matches!(
                        entry,
                        Entry::Tool {
                            kind: ToolKind::Edit,
                            ..
                        }
                    )
                })
                .unwrap_or_else(|| panic!("{name}: no edit"));
            let [item] = &content_of(&edit)[..] else {
                panic!("{name}: {edit:?}");
            };
            assert!(
                matches!(item, ToolItem::Diff(FileChange { op: FileOp::Modify, path, .. }) if path == "/home/user/scratch/notes.txt"),
                "{name}"
            );
            assert_eq!(diff_lines(item), NOTES_EDIT, "{name}");
            let (id, _) = agent
                .s
                .transcript()
                .entries()
                .find(|(_, entry)| **entry == edit)
                .expect("held");
            // Each sends its fragment first (Claude in an update and again in
            // the request, which sends nothing more, Antigravity in the
            // request), then the whole file once the edit is made.
            assert_eq!(item_events(&agent, id), 2, "{name}");
        }
    }

    #[test]
    fn new_files_rejections_raw_input_and_output_read_as_each_agent_sent_them() {
        // Claude's `-auto` new file, and Antigravity's, whose content came
        // only in the permission request.
        for name in ["claude-agent-acp-0.112.0-auto", "antigravity-1.5.16-reject"] {
            let agent = replay(capture_named(name), |_| {});
            let [tool] = &tools(&agent)[..] else {
                panic!("{name}");
            };
            let [item] = &content_of(tool)[..] else {
                panic!("{name}: {tool:?}");
            };
            assert!(
                matches!(
                    item,
                    ToolItem::Diff(FileChange {
                        op: FileOp::Add,
                        ..
                    })
                ),
                "{name}"
            );
            assert_eq!(diff_lines(item), ["+hi"], "{name}");
        }
        // Copilot's patch is in its raw input, under its own title.
        let agent = replay(capture_named("copilot-1.0.115-reject"), |_| {});
        let [
            Entry::Tool {
                title,
                input: Some(input),
                result: Some(result),
                ..
            },
        ] = &tools(&agent)[..]
        else {
            panic!("{:?}", tools(&agent));
        };
        assert_eq!(title, "apply_patch");
        assert!(
            input.contains("\"fileName\": \"/home/user/scratch/hello.txt\""),
            "{input}"
        );
        assert!(result.contains("rejected"), "{result}");
        // Codex's commands: `pwd`'s output and exit, the rejected command's
        // output with none, and `seq 1 4`.
        let agent = replay(capture_named("codex-acp-2.12.0-reject"), |_| {});
        let outputs: Vec<Vec<ToolItem>> = tools(&agent).iter().map(content_of).collect();
        assert_eq!(
            outputs,
            [
                vec![ToolItem::Output(ToolOutputItem {
                    cwd: Some("/home/user/scratch".into()),
                    text: "/home/user/scratch\n".into(),
                    exit: Some(Exit::Code { code: 0 }),
                    ..ToolOutputItem::default()
                })],
                vec![ToolItem::Output(ToolOutputItem {
                    cwd: Some("/home/user/scratch".into()),
                    ..ToolOutputItem::default()
                })],
            ]
        );
        for name in [
            "codex-acp-2.12.0-edit-run",
            "codex-acp-2.12.0-edit-run-terminal",
        ] {
            let agent = replay(capture_named(name), |_| {});
            let last = tools(&agent).pop().expect("a tool");
            assert_eq!(
                content_of(&last),
                [ToolItem::Output(ToolOutputItem {
                    cwd: Some("/home/user/scratch".into()),
                    text: "1\n2\n3\n4\n".into(),
                    exit: Some(Exit::Code { code: 0 }),
                    ..ToolOutputItem::default()
                })],
                "{name}"
            );
        }
        // Codex's file read sends output for a terminal nothing opened
        // (flag 16): it is the read's own.
        let agent = replay(capture_named("codex-acp-2.12.0-edit-run"), |_| {});
        let read = tools(&agent).remove(0);
        assert!(
            matches!(&content_of(&read)[..], [ToolItem::Output(ToolOutputItem { text, .. })] if text == "an apple a day\n"),
            "{read:?}"
        );
    }

    #[test]
    fn antigravitys_string_request_id_is_answered_verbatim_and_its_tool_status_goes_back() {
        // `replay` checks the answer's bytes against the captured `"id":"1"`.
        let agent = replay(CAPTURES[0].1, |_| {});
        assert!(matches!(
            &agent.entries()[1],
            Entry::Tool {
                status: ToolStatus::Failed,
                ..
            }
        ));
        assert!(agent.events.iter().any(|event| matches!(
            event,
            Event::ToolChanged {
                status: Some(ToolStatus::Pending),
                ..
            }
        )));
    }

    #[test]
    fn the_handwritten_unknown_values_are_confined_to_their_items() {
        let agent = replay_into(Agent::ready_in("sess-1"), HANDWRITTEN, |_| {});
        // The unknown option kind is left out with a notice, an unknown
        // stop reason keeps its name.
        assert!(
            agent
                .notices()
                .iter()
                .any(|n| n.contains("allow_for_session"))
        );
        assert!(agent.events.iter().any(|event| matches!(event, Event::Turn { state: TurnState::Ended { stop: StopReason::Other { name } } } if name == "paused_for_review")));
    }

    #[test]
    fn geminis_login_prompt_on_stdout_is_one_notice_of_bad_lines_and_the_agent_stays_starting() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        for line in GEMINI.split(|b| *b == b'\n') {
            if let Some(from_agent) = line.strip_prefix(b"a>c\t") {
                agent.step(Input::Stdout(&[from_agent, b"\n"].concat()));
            }
        }
        agent.now += BAD_RUN_QUIET;
        agent.step(Input::Tick);
        assert_eq!(
            agent.notices(),
            ["6 lines from the agent weren't ACP messages"]
        );
        assert_eq!(*agent.agent_state(), AgentState::Starting);
    }

    // --- the bounds in sum (OpenSpec change `agents-event-bounds`) ------

    fn assert_events_fit(out: &Output) {
        for event in &out.events {
            let len = event.to_value().to_string().len();
            assert!(
                len <= brindle_transcript::MAX_EVENT,
                "an event of {len} bytes: {}",
                shape(std::slice::from_ref(event))[0]
            );
        }
    }

    /// A 300,000-line empty diff makes no event past `MAX_EVENT` (A-2,
    /// task 1.4).
    #[test]
    fn a_300000_line_empty_diff_makes_no_event_past_the_limit() {
        let mut agent = Agent::ready();
        agent.turn("write");
        let new = "\n".repeat(300_000);
        let out = agent.feed(&update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Write",
            "content": [{"type": "diff", "path": "/p/empty.txt", "oldText": null, "newText": new}]})));
        assert_events_fit(&out);
    }

    /// A permission with 20 options named with 64 KiB each makes no event
    /// past `MAX_EVENT` (A-2, task 1.4).
    #[test]
    fn a_permission_of_20_long_named_options_makes_no_event_past_the_limit() {
        let mut agent = Agent::ready();
        agent.turn("ask");
        let name = "n".repeat(64 * 1024);
        let options: Vec<Value> = (0..20)
            .map(|n| json!({"optionId": format!("o{n}"), "name": name, "kind": "allow_once"}))
            .collect();
        let out = agent.feed(
            &json!({"jsonrpc": "2.0", "id": 90, "method": "session/request_permission",
                "params": {"sessionId": "s1", "toolCall": {"toolCallId": "c1", "title": "Write"}, "options": options}})
            .to_string(),
        );
        assert_events_fit(&out);
    }

    /// A 20 MiB thought streamed in 64 KiB chunks leaves the log within
    /// `LOG_LIMIT` after every step (A-1, tasks 1.4 and 4.3), and the
    /// session's checkpoint encodes.
    #[test]
    fn a_20_mib_thought_leaves_the_log_within_its_limit_after_every_step() {
        let mut agent = Agent::ready();
        agent.turn("think");
        let piece = "t".repeat(64 * 1024);
        for _ in 0..(20 * 16) {
            agent.feed(&chunk("agent_thought_chunk", &piece));
            let len = agent.s.transcript.encoded_len();
            assert!(len <= LOG_LIMIT, "a log of {len} bytes");
        }
        assert!(super::super::checkpoint::encode(&agent.s).is_ok());
    }

    /// The texts of the entries after the first (the prompt).
    fn replies(agent: &Agent) -> Vec<String> {
        agent
            .entries()
            .into_iter()
            .skip(1)
            .map(|entry| match entry {
                Entry::User { text } | Entry::Agent { text } | Entry::Thought { text } => text,
                other => panic!("not text: {other:?}"),
            })
            .collect()
    }

    /// Lines of 100 bytes, `len` bytes in all.
    fn lined(len: usize) -> String {
        let line = format!("{}\n", "r".repeat(99));
        line.repeat(len.div_ceil(100))[..len].to_owned()
    }

    /// A 2.5 MiB reply streamed in 32 KiB chunks becomes three agent
    /// entries of at most `ENTRY_LIMIT`, whose texts joined are the reply,
    /// each cut at a line break when one lies in the room (task 4.3). A
    /// chunk that fits whole is kept whole: 32 of 32 KiB fill an entry
    /// exactly, mid-line, and the next starts a new entry; chunks of
    /// 30,000 bytes cross the limit, and the crossing one is cut at its
    /// last line break in the room.
    #[test]
    fn a_long_reply_continues_in_new_entries_cut_at_line_breaks() {
        let reply = lined(5 * 512 * 1024);
        for size in [32 * 1024, 30_000] {
            let mut agent = Agent::ready();
            agent.turn("go");
            for piece in reply.as_bytes().chunks(size) {
                agent.feed(&said(std::str::from_utf8(piece).expect("ASCII")));
            }
            let texts = replies(&agent);
            assert_eq!(texts.len(), 3, "{size}");
            assert!(agent.entries()[1..].iter().all(|entry| matches!(entry, Entry::Agent { .. })));
            assert!(texts.iter().all(|text| text.len() <= ENTRY_LIMIT), "{size}");
            assert_eq!(texts.concat(), reply, "{size}");
            if size == 30_000 {
                assert!(texts[..2].iter().all(|text| text.ends_with('\n')));
            } else {
                assert_eq!(texts[0].len(), ENTRY_LIMIT);
            }
        }
        // Without a line break in the room, at a character boundary.
        let mut agent = Agent::ready();
        agent.turn("go");
        let flat = "é".repeat(ENTRY_LIMIT / 2 + 10);
        for piece in flat.as_bytes().chunks(32 * 1024) {
            agent.feed(&said(std::str::from_utf8(piece).expect("even cuts of é")));
        }
        let texts = replies(&agent);
        assert_eq!(texts.len(), 2);
        assert_eq!(texts[0].len(), ENTRY_LIMIT);
        assert_eq!(texts.concat(), flat);
    }

    /// A reply of exactly `ENTRY_LIMIT` stays one entry, as before.
    #[test]
    fn a_1_mib_reply_stays_one_entry() {
        let mut agent = Agent::ready();
        agent.turn("go");
        let reply = lined(ENTRY_LIMIT);
        for piece in reply.as_bytes().chunks(32 * 1024) {
            agent.feed(&said(std::str::from_utf8(piece).expect("ASCII")));
        }
        assert_eq!(replies(&agent), [reply]);
    }

    /// A permission request of `options`, as `(id, name, kind)`.
    fn asking(options: &[(String, String, &str)]) -> String {
        let options: Vec<Value> = options
            .iter()
            .map(|(id, name, kind)| json!({"optionId": id, "name": name, "kind": kind}))
            .collect();
        json!({"jsonrpc": "2.0", "id": 90, "method": "session/request_permission",
            "params": {"sessionId": "s1", "toolCall": {"toolCallId": "c1", "title": "Write"}, "options": options}})
        .to_string()
    }

    fn permission_options(agent: &Agent) -> Vec<PermissionOption> {
        agent
            .entries()
            .into_iter()
            .find_map(|entry| match entry {
                Entry::Permission { options, .. } => Some(options),
                _ => None,
            })
            .expect("a permission entry")
    }

    /// A permission with 20 options of 64 KiB names keeps 16, names cut at
    /// 4 KiB, and a notice says 4 were left out (task 4.4).
    #[test]
    fn a_permission_keeps_16_options_their_names_cut() {
        let mut agent = Agent::ready();
        agent.turn("ask");
        let name = "n".repeat(64 * 1024);
        let options: Vec<(String, String, &str)> =
            (0..20).map(|n| (format!("o{n}"), name.clone(), "allow_once")).collect();
        agent.feed(&asking(&options));
        let kept = permission_options(&agent);
        assert_eq!(kept.len(), MAX_PERMISSION_OPTIONS);
        assert_eq!(kept[15].id, "o15");
        assert!(kept.iter().all(|option| option.name.len() == MAX_PERMISSION_NAME));
        assert!(
            agent.notices().iter().any(|text| text.contains("4 of its options were left out")),
            "{:?}",
            agent.notices()
        );
    }

    /// An option whose id is 2 KiB, empty or holds `\u{1b}` is left out,
    /// and a request left with none is answered `cancelled` (task 4.4).
    #[test]
    fn an_option_whose_id_cant_be_sent_back_is_left_out() {
        let mut agent = Agent::ready();
        agent.turn("ask");
        let bad = |id: String| (id, "Allow".to_owned(), "allow_once");
        agent.feed(&asking(&[
            bad("i".repeat(2048)),
            bad(String::new()),
            bad("a\u{1b}b".into()),
            bad("ok".into()),
        ]));
        let kept = permission_options(&agent);
        assert_eq!(kept.iter().map(|option| option.id.as_str()).collect::<Vec<_>>(), ["ok"]);
        let mut agent = Agent::ready();
        agent.turn("ask");
        agent.feed(&asking(&[bad("i".repeat(2048)), bad(String::new())]));
        let answer = agent
            .sent
            .iter()
            .rev()
            .find(|line| line["id"] == 90)
            .expect("the request answered");
        assert_eq!(answer["result"]["outcome"]["outcome"], "cancelled");
    }

    /// A terminal id of 2 KiB is ignored: its output isn't kept (task 4.4).
    #[test]
    fn a_terminal_id_past_its_bound_is_ignored() {
        let mut agent = Agent::ready();
        agent.turn("run");
        let terminal = "t".repeat(2048);
        agent.feed(&update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "run",
            "kind": "execute", "status": "in_progress", "content": [{"type": "terminal", "terminalId": terminal}],
            "_meta": {"terminal_info": {"terminal_id": terminal, "cwd": "/w"}}})));
        agent.feed(&meta(
            "c1",
            json!({"terminal_output_delta": {"terminal_id": terminal, "data": "out\n"}}),
        ));
        assert!(agent.s.outputs.is_empty());
        let Entry::Tool { content, more, .. } = tool_entry(&agent, "c1") else {
            panic!("a tool entry");
        };
        assert!(content.is_empty(), "{content:?}");
        assert_eq!(more, 1, "the terminal item is counted left out");
    }

    /// 40 locations of 64 KiB paths keep 32, each cut at `MAX_PATH`.
    #[test]
    fn locations_keep_32_their_paths_cut() {
        let mut agent = Agent::ready();
        agent.turn("read");
        let path = format!("/{}", "p".repeat(64 * 1024));
        let locations: Vec<Value> = (0..40).map(|_| json!({"path": path})).collect();
        let out = agent.feed(&update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1",
            "title": "Read", "locations": locations})));
        assert_events_fit(&out);
        let Entry::Tool { locations, .. } = tool_entry(&agent, "c1") else {
            panic!("a tool entry");
        };
        assert_eq!(locations.len(), MAX_LOCATIONS);
        assert!(locations.iter().all(|location| location.path.len() == MAX_PATH));
    }

    /// A fourth full output in one call is counted in `more`, not opened:
    /// three fill `CALL_LIMIT` as `wire_len` counts them (task 4.4).
    #[test]
    fn a_fourth_output_in_one_call_is_counted_in_more() {
        let mut agent = Agent::ready();
        agent.turn("run");
        let content: Vec<Value> =
            (0..4).map(|n| json!({"type": "terminal", "terminalId": format!("t{n}")})).collect();
        agent.feed(&update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "run",
            "kind": "execute", "status": "in_progress", "content": content,
            "_meta": {"terminal_info": {"terminal_id": "t0", "cwd": "/w"}}})));
        let Entry::Tool { content, more, .. } = tool_entry(&agent, "c1") else {
            panic!("a tool entry");
        };
        assert_eq!(content.len(), 3);
        assert_eq!(more, 1);
        assert!(!agent.s.outputs.contains_key("t3"));
        // Nor does output for a terminal nothing opened open past it.
        agent.feed(&meta(
            "c1",
            json!({"terminal_output_delta": {"terminal_id": "t9", "data": "x"}}),
        ));
        assert!(!agent.s.outputs.contains_key("t9"));
        assert_eq!(content_of(&tool_entry(&agent, "c1")).len(), 3);
    }

    /// A `session/new` answer with a 2 KiB session id fails the start
    /// saying why, and nothing is sent in its name (task 4.4).
    #[test]
    fn a_session_id_past_its_bound_fails_the_start() {
        for id in ["s".repeat(2048), String::new(), "s\u{1b}1".into()] {
            let mut agent = Agent::new();
            agent.step(Input::Spawned);
            agent.feed(&result(
                1,
                json!({"protocolVersion": 1, "agentInfo": {"name": "fake", "version": "1.0"}}),
            ));
            let sent = agent.sent.len();
            agent.feed(&result(2, json!({"sessionId": id})));
            assert!(
                matches!(agent.agent_state(), AgentState::Failed { message }
                    if message == "the agent's session id is longer than 1 KiB, or isn't one Brindle can send back"),
                "{id:?}: {:?}",
                agent.agent_state()
            );
            assert!(agent.prompt("hi").reply.is_some_and(|reply| reply.is_err()));
            assert_eq!(agent.sent.len(), sent, "{id:?}: nothing sent in its name");
        }
    }

    /// A stop reason named with 100 KiB is cut to `FIELD_LIMIT`.
    #[test]
    fn a_long_stop_reasons_name_is_cut() {
        let mut agent = Agent::ready();
        let id = agent.turn("go");
        agent.prompt("next");
        let out = agent.end(id, &"z".repeat(100 * 1024));
        assert_events_fit(&out);
        let TurnState::Ended {
            stop: StopReason::Other { name },
        } = agent.turn_state()
        else {
            panic!("{:?}", agent.turn_state());
        };
        assert_eq!(name.len(), FIELD_LIMIT);
    }

    /// The piece split counts an output's command and directory, so an
    /// output with a 60 KiB command and a 10 KiB text item go in two
    /// `ToolItems` (task 4.4).
    #[test]
    fn the_piece_split_counts_an_outputs_command_and_directory() {
        let mut agent = Agent::ready();
        agent.turn("run");
        agent.feed(&tool_call("c1", "run", "in_progress"));
        let out = agent.feed(&update(json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1",
            "content": [
                {"type": "content", "content": {"type": "text", "text": "x".repeat(10 * 1024)}},
                {"type": "terminal", "terminalId": "t1"},
            ],
            "_meta": {"terminal_info": {"terminal_id": "t1", "cwd": "/w", "command": "c".repeat(60 * 1024)}}})));
        let pieces: Vec<usize> = out
            .events
            .iter()
            .filter_map(|event| match event {
                Event::ToolItems { items, .. } => Some(items.len()),
                _ => None,
            })
            .collect();
        assert_eq!(pieces, [1, 1]);
    }

    /// `raw_text` of a long value holding controls ends with `…` within
    /// `RAW_LIMIT`, as it did when it cleaned the whole value first; one
    /// within the limit is only cleaned (task 4.6).
    #[test]
    fn raw_text_cuts_a_long_value_with_an_ellipsis_within_its_limit() {
        for long in [
            "\u{1}".repeat(100_000),
            format!("{}\u{1}", "r".repeat(RAW_LIMIT)),
            "é".repeat(RAW_LIMIT),
        ] {
            let text = raw_text(&long);
            assert!(text.len() <= RAW_LIMIT, "{}", text.len());
            assert!(text.ends_with('…'));
            let old = match cut_at(clean(&long), RAW_LIMIT) {
                (text, false) => text,
                (text, true) => format!("{}…", cut_at(text, RAW_LIMIT - "…".len()).0),
            };
            assert_eq!(text, old);
        }
        assert_eq!(raw_text("a\u{1}b"), "a\u{fffd}b");
        assert_eq!(raw_text(&"r".repeat(RAW_LIMIT)), "r".repeat(RAW_LIMIT));
    }

    // --- the emit guard (task 4.5, design decision 4) -------------------

    /// The guard's limit these tests set: room for `fail`'s own events.
    const LOW_LIMIT: usize = 600;

    fn failure(agent: &Agent) -> String {
        match agent.agent_state() {
            AgentState::Failed { message } => message.clone(),
            other => panic!("not failed: {other:?}"),
        }
    }

    /// An `Added` past the limit is not applied, and the session fails
    /// naming it, its entry and both sizes; after it the step emits only
    /// `fail`'s events, which the client folds to the server's log.
    #[test]
    fn an_added_event_past_the_limit_fails_the_session_naming_it() {
        let mut agent = Agent::ready();
        agent.turn("go");
        agent.event_limit = LOW_LIMIT;
        let title = "T".repeat(1000);
        let event = Event::Added {
            id: EntryId(2),
            entry: Entry::tool("c1".into(), title.clone(), ToolKind::Edit, ToolStatus::Pending),
        };
        let len = event.encoded_len();
        let out = agent.feed(&tool_call("c1", &title, "pending"));
        assert_eq!(shape(&out.events), ["turn failed", "agent failed"]);
        assert_eq!(
            failure(&agent),
            format!(
                "ACP handling failed: an added (tool) event for entry 2 is {len} bytes; one transcript event may be at most {LOW_LIMIT}"
            )
        );
        assert!(agent.s.transcript.entry(EntryId(2)).is_none(), "not applied");
    }

    /// An `Appended` past the limit fails the session naming `appended`,
    /// its entry and its size.
    #[test]
    fn an_appended_event_past_the_limit_fails_the_session_naming_it() {
        let mut agent = Agent::ready();
        agent.turn("go");
        agent.feed(&said("hi"));
        agent.event_limit = LOW_LIMIT;
        let text = "a".repeat(1000);
        let len = Event::Appended {
            id: EntryId(2),
            text: text.clone(),
        }
        .encoded_len();
        let out = agent.feed(&said(&text));
        assert_eq!(shape(&out.events), ["turn failed", "agent failed"]);
        assert_eq!(
            failure(&agent),
            format!(
                "ACP handling failed: an appended event for entry 2 is {len} bytes; one transcript event may be at most {LOW_LIMIT}"
            )
        );
        assert_eq!(agent.entries()[1], Entry::Agent { text: "hi".into() });
    }

    /// A `Queue` event past the limit fails the session naming the
    /// session's queue event.
    #[test]
    fn a_queue_event_past_the_limit_fails_the_session_naming_it() {
        let mut agent = Agent::ready();
        agent.turn("go");
        agent.event_limit = LOW_LIMIT;
        let text = "q".repeat(1000);
        let len = Event::Queue {
            slot: Slot::Waiting { text: text.clone() },
        }
        .encoded_len();
        let out = agent.prompt(&text);
        assert_eq!(shape(&out.events), ["turn failed", "agent failed"]);
        assert_eq!(
            failure(&agent),
            format!(
                "ACP handling failed: the session's queue event is {len} bytes; one transcript event may be at most {LOW_LIMIT}"
            )
        );
        assert_eq!(*agent.slot(), Slot::Empty);
    }

    /// A shown `session/load` replay of a 1.5 MiB user message gives two
    /// user entries.
    #[test]
    fn a_shown_replay_of_a_long_user_message_gives_two_user_entries() {
        let mut agent = Agent::new();
        agent.s = AgentSession::continuing(conversation_s1(Continuation::Load), "0.3.0".into());
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        let message = lined(3 * 512 * 1024);
        for piece in message.as_bytes().chunks(32 * 1024) {
            agent.feed(&chunk("user_message_chunk", std::str::from_utf8(piece).expect("ASCII")));
        }
        let texts = replies(&agent);
        assert_eq!(texts.len(), 2);
        assert!(agent.entries()[1..].iter().all(|entry| matches!(entry, Entry::User { .. })));
        assert_eq!(texts.concat(), message);
    }
}
