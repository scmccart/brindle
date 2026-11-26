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

use brindle_remote_protocol::serde_json::Value;
use brindle_transcript::{
    AgentState, Continuation, Conversation, Entry, EntryId, Event, LoginKind, LoginMethod,
    NoticeLevel, PermissionOption, PermissionOutcome, Slot, StopReason, TEXT_CHUNK, Transcript,
    TurnState, chunks,
};
use brindle_transcript::{HoldReason, ToolKind, ToolStatus};

use super::lines::{Lines, Piece};
use super::rpc::{self, PROTOCOL_VERSION, Purpose, Refusal, Rpc};
use super::wire::{
    self, AUTH_REQUIRED, Content, ContentKind, Id, Malformed, Message, NotificationKind,
    OptionKind, Patch, RequestKind, RpcError, ToolCall, ToolChange, UNSUPPORTED_VERSION, Update,
};

/// The most the log keeps, encoded (decision 14): past it the oldest
/// entries are dropped whole, with a `Trimmed` event, down to
/// [`LOG_TRIM_TO`].
pub const LOG_LIMIT: usize = 16 * 1024 * 1024;

/// What a trim leaves the log at, encoded: a low mark below [`LOG_LIMIT`],
/// so a long conversation trims once per 4 MiB rather than once per entry,
/// and each trim's whole checkpoint is rare (OpenSpec change
/// `agents-log-paging`, decision 6).
pub const LOG_TRIM_TO: usize = 12 * 1024 * 1024;

/// The longest prompt taken, in bytes of UTF-8 (decision 10).
pub const MAX_PROMPT: usize = 256 * 1024;

/// How long a run of lines that aren't ACP messages may go without another
/// before its notice is written.
pub const BAD_RUN_QUIET: Duration = Duration::from_secs(1);

/// The longest conversation title kept, in characters, `…` included
/// (`agents-session-title` decision 3): a tab's title, read as characters,
/// and well under [`TEXT_CHUNK`] however long the agent's title.
pub const MAX_SESSION_TITLE: usize = 256;

/// Where the agent's ACP conversation is: what the transcript's
/// [`AgentState`] can't say (which start request is out, the ACP session
/// id, why the agent failed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The holder hasn't said the agent started; what it starts.
    Spawning { start: Start },
    /// `initialize` is sent and not answered.
    Initializing { start: Start },
    /// `session/new` is sent and not answered.
    Creating { greeting: Greeting },
    /// `session/resume` of conversation `conversation` is sent and not
    /// answered (OpenSpec change `agents-session-recovery`, decision 2).
    /// `log` says whether the log holds the conversation's earlier
    /// messages: a fresh one gets a notice that it doesn't.
    Resuming {
        greeting: Greeting,
        conversation: String,
        log: Log,
    },
    /// `session/load` of conversation `conversation` is sent and not
    /// answered: the agent replays it first, muted or shown.
    Loading {
        greeting: Greeting,
        conversation: String,
        replay: Replay,
    },
    /// Takes prompts in the ACP session `session`.
    Ready { session: String },
    /// The agent answered `-32000` (decision 17): it waits for a client to
    /// choose one of its login methods (OpenSpec change `agents-login`,
    /// decision 4), and nothing else is sent.
    LoginRequired(Login),
    /// `authenticate` with login method `method` is out
    /// ([`Purpose::Authenticate`] in [`Rpc::outgoing`]), awaited however
    /// long it takes.
    LoggingIn { login: Login, method: String },
    /// `AGENT_RESTART` while the agent needed a login (`agents-login`
    /// decision 8): the agent is hung up, nothing more is read from it, and
    /// once it has exited the core steps [`Input::Restart`] again to spawn
    /// it anew with `start`. Should its exit be stepped instead (the
    /// restart was dropped: the session closed, or its client left), it
    /// ends as any agent does.
    Restarting { start: Start },
    /// The start failed (another ACP version, or an error or unreadable
    /// answer to `initialize`, `session/new`, `session/resume` or
    /// `session/load`, or a continue the agent offers no way to make;
    /// decision 4): the agent is hung up, and its exit leaves the failed
    /// state, adding its stderr as a notice.
    Refused,
    /// The core's handling of this agent failed (decision 13), or its
    /// checkpoint couldn't be restored (decision 11): its lines are
    /// ignored, and it runs until the session is closed.
    Broken,
    /// The agent's process ended.
    Exited,
    /// The agent's holder ended, and the agent with it.
    Lost,
}

/// What a spawned agent starts: a new conversation, or one it continues
/// (decisions 2 and 4 of `agents-session-recovery`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// `session/new` in the session's directory.
    New,
    /// `session/resume` or `session/load` of `conversation`, whichever the
    /// new agent offers, into the log kept from its earlier agent or into a
    /// fresh one.
    Continue {
        conversation: Conversation,
        log: Log,
    },
}

/// Whether a continued conversation's log is the session's own, kept from
/// its earlier agent (`AGENT_RESTART`), or a fresh one in a new session
/// whose server no longer had it (`SPAWN_AGENT` with a conversation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Log {
    Kept,
    Fresh,
}

/// What a `session/load`'s replay does to the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replay {
    /// A kept log already holds it: each update is counted, not applied.
    Muted { updates: u64 },
    /// A fresh log takes it as ordinary updates.
    Shown,
}

/// What `initialize` said that the agent's ready state and conversation
/// carry: its name and version, and what it offers for continuing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Greeting {
    pub name: Option<String>,
    pub version: Option<String>,
    pub continuation: Continuation,
}

/// An agent waiting for a login (`agents-login` decision 4): what its
/// `initialize` said, for the later ready state and conversation, the
/// agent's last login message, and how the session goes on once a login
/// succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub greeting: Greeting,
    pub message: String,
    pub then: AfterLogin,
}

/// How a session goes on after a login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterLogin {
    /// A start request answered `-32000`: it is sent again (`session/new`,
    /// or the continue's `session/resume` or `session/load`).
    Start(Start),
    /// A prompt answered `-32000`: the agent is ready again in the ACP
    /// session the prompt was sent in, and nothing is sent on its own.
    Ready { session: String },
}

/// How a continue goes on once the new agent has answered `initialize`
/// (decision 2's table), from what that agent offers, not the earlier one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// `session/resume`.
    Resume,
    /// `session/load`, its replay muted or shown.
    Load { replay: Replay },
    /// Neither is offered: the continue fails.
    Neither,
}

/// Decision 2's table: a kept log resumes when it can and otherwise loads
/// muted; a fresh log loads (shown) when it can and otherwise resumes.
pub fn choose(log: Log, offered: Continuation) -> Choice {
    match (log, offered) {
        (Log::Kept, Continuation::ResumeAndLoad | Continuation::Resume)
        | (Log::Fresh, Continuation::Resume) => Choice::Resume,
        (Log::Kept, Continuation::Load) => Choice::Load {
            replay: Replay::Muted { updates: 0 },
        },
        (Log::Fresh, Continuation::ResumeAndLoad | Continuation::Load) => Choice::Load {
            replay: Replay::Shown,
        },
        (Log::Kept | Log::Fresh, Continuation::Neither) => Choice::Neither,
    }
}

/// The notice an `AGENT_RESTART` adds before the agent starts again.
pub const RESTARTED: &str = "the agent was started again to continue the conversation";

/// The notice an `AGENT_RESTART` of an agent that needed a login adds
/// (`agents-login` decision 8).
pub const RESTARTED_AFTER_LOGIN: &str = "the agent is started again after its login";

/// The notice a fresh log's shown replay starts with.
pub const REPLAY_SHOWN: &str =
    "this continues an earlier conversation; what follows is the agent's replay of it";

/// The notice a fresh log gets when the agent could only resume.
pub const NOT_REPLAYED: &str =
    "conversation continued; the agent doesn't replay its earlier messages, so they aren't shown";

/// The failure when the continued agent offers neither way to continue.
pub const CANT_CONTINUE: &str =
    "the agent can't continue conversations: it offers neither session/load nor session/resume";

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
    pub(super) bad: Option<BadRun>,
    /// The login methods the agent's `initialize` advertised, in its order
    /// (`agents-login` decision 3), sent with every `LoginRequired`.
    pub(super) methods: Vec<LoginMethod>,
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
            bad: None,
            methods: Vec::new(),
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
        let ended = match self.phase {
            Phase::Exited | Phase::Lost => true,
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Ready { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken => false,
        };
        let agent_ended = match self.transcript.agent() {
            AgentState::Exited { .. } | AgentState::Lost { .. } => true,
            AgentState::Starting
            | AgentState::Ready { .. }
            | AgentState::LoginRequired { .. }
            | AgentState::LoggingIn { .. }
            | AgentState::Failed { .. } => false,
        };
        ended
            && agent_ended
            && self
                .transcript
                .conversation()
                .is_some_and(|conversation| conversation.continuation.continuable())
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
}

/// Steps `session` with one input.
pub fn step(session: &mut AgentSession, input: Input<'_>, now: Instant) -> Output {
    let mut step = Step {
        s: session,
        out: Output::default(),
        fault: None,
        now,
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

/// A turn's end.
enum Ending {
    Stop(StopReason),
    Failed(String),
}

impl Ending {
    fn state(&self) -> TurnState {
        match self {
            Self::Stop(stop) => TurnState::Ended { stop: stop.clone() },
            Self::Failed(message) => TurnState::Failed {
                message: message.clone(),
            },
        }
    }
}

struct Step<'s> {
    s: &'s mut AgentSession,
    out: Output,
    /// A fold error from this step's own events: the rest is skipped.
    fault: Option<String>,
    now: Instant,
}

impl Step<'_> {
    fn finish(mut self) -> Output {
        if let Some(reason) = self.fault.take() {
            self.fail(&reason);
        }
        self.trim();
        self.out.wake = self.s.bad.map(|bad| bad.last + BAD_RUN_QUIET);
        self.out
    }

    // --- emitting -------------------------------------------------------

    /// Applies `event` and keeps it for the client; after a fold error,
    /// does nothing.
    fn emit(&mut self, event: Event) {
        if self.fault.is_some() {
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
                | Moment::Login,
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

    /// Adds a text entry of `kind`, its text split into `Appended`s, and
    /// makes it the chunk run.
    fn add_text(&mut self, kind: RunKind, text: &str) -> EntryId {
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
        id
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
            | AgentState::Lost { .. } => "ACP handling failed".to_owned(),
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

    // --- holder events --------------------------------------------------

    fn spawned(&mut self) {
        let Phase::Spawning { start } = &self.s.phase else {
            return;
        };
        let start = start.clone();
        let line = self.s.rpc.initialize(&self.s.version);
        self.s.phase = Phase::Initializing { start };
        self.out.stdin.extend(line);
        self.mark(Moment::BeforeRequest);
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

    fn exited(&mut self, code: Option<i32>, stderr: &[u8]) {
        let stderr = bound(clean(&String::from_utf8_lossy(stderr)));
        self.end(Phase::Exited, AgentState::Exited { code, stderr });
    }

    fn lost(&mut self, message: &str) {
        self.end(
            Phase::Lost,
            AgentState::Lost {
                message: bound(clean(message)),
            },
        );
    }

    /// The agent's process ended (`phase` says how): its waiting permission
    /// requests are recorded cancelled, a waiting prompt is held, a running
    /// turn fails, and the agent takes `state`. After a refused start the
    /// failure stays the agent's state, and an exit's stderr is a notice.
    /// Nothing more happens once it has ended.
    fn end(&mut self, phase: Phase, state: AgentState) {
        if self.ended() {
            return;
        }
        let refused = self.s.phase == Phase::Refused;
        self.s.phase = phase;
        // A login out when the agent ended is never answered.
        self.s.rpc.forget(Purpose::Authenticate);
        for (entry, _) in self.s.rpc.cancel_permissions() {
            self.emit(Event::Answered {
                id: entry,
                outcome: PermissionOutcome::Cancelled,
            });
        }
        if refused {
            // Decision 4: the version message stays the last agent state.
            match &state {
                AgentState::Exited { stderr, .. } if !stderr.trim().is_empty() => {
                    let text = format!("the agent's standard error:\n{stderr}");
                    self.notice(NoticeLevel::Warning, text);
                }
                AgentState::Exited { .. }
                | AgentState::Lost { .. }
                | AgentState::Starting
                | AgentState::Ready { .. }
                | AgentState::LoginRequired { .. }
                | AgentState::LoggingIn { .. }
                | AgentState::Failed { .. } => {}
            }
        } else {
            self.hold_waiting(HoldReason::AgentStopped);
            if self.running() {
                self.s.run = None;
                self.turn(TurnState::Failed {
                    message: "the agent exited".to_owned(),
                });
            }
            self.agent(state);
        }
        self.mark(Moment::Stopped);
    }

    /// `AGENT_RESTART` (decision 4): an ended agent whose conversation can
    /// be continued is spawned again into this log. The old agent's
    /// requests are dead with it, its stdout's line is gone (the new
    /// agent's holder session counts from 0), and the request ids go on.
    /// No checkpoint: the session has no holder until the spawn is placed,
    /// and [`Input::Spawned`] checkpoints before `initialize`.
    fn restart(&mut self) -> Result<(), Refused> {
        match &self.s.phase {
            Phase::LoginRequired(login) => {
                let login = login.clone();
                return self.restart_for_login(login);
            }
            Phase::Restarting { start } => {
                let start = start.clone();
                self.respawn(start);
                return Ok(());
            }
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Ready { .. }
            | Phase::LoggingIn { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => {}
        }
        if !self.ended() {
            return Err(Refused::NotEnded);
        }
        if !self.s.continuable() {
            return Err(Refused::CannotContinue);
        }
        let Some(conversation) = self.s.transcript.conversation().cloned() else {
            return Err(Refused::CannotContinue);
        };
        self.respawn(Start::Continue {
            conversation,
            log: Log::Kept,
        });
        self.notice(NoticeLevel::Info, RESTARTED.to_owned());
        self.agent(AgentState::Starting);
        Ok(())
    }

    /// The agent is to be spawned again with `start`: the old agent's
    /// requests are dead with it, its stdout's line is gone (the new
    /// agent's holder session counts from 0), and the request ids go on.
    fn respawn(&mut self, start: Start) {
        self.s.rpc.restart();
        self.s.lines = Lines::new();
        self.s.run = None;
        self.s.tools.clear();
        self.s.bad = None;
        self.s.phase = Phase::Spawning { start };
    }

    /// `AGENT_RESTART` while the agent needs a login (`agents-login`
    /// decision 8, after a terminal login): the running agent is hung up,
    /// and the start it is to make again is kept. A start's demand makes
    /// that start again (`session/new`, or the continue); a prompt's takes
    /// up its ACP session as a kept log's continue, refused when the agent
    /// offered no way to continue it. The log, the slot and the methods
    /// stay; the agent is starting from here on.
    fn restart_for_login(&mut self, login: Login) -> Result<(), Refused> {
        let start = match login.then {
            AfterLogin::Start(start) => start,
            AfterLogin::Ready { session } => {
                if !login.greeting.continuation.continuable() {
                    return Err(Refused::CannotContinue);
                }
                Start::Continue {
                    conversation: Conversation {
                        id: session,
                        cwd: self.s.cwd.clone(),
                        continuation: login.greeting.continuation,
                        // `ready` takes the title from the log.
                        title: None,
                    },
                    log: Log::Kept,
                }
            }
        };
        self.s.phase = Phase::Restarting { start };
        self.notice(NoticeLevel::Info, RESTARTED_AFTER_LOGIN.to_owned());
        self.agent(AgentState::Starting);
        self.out.hang_up = true;
        Ok(())
    }

    /// The start failed (decision 4): the agent fails with `message`, is
    /// sent nothing more and is hung up.
    fn refuse(&mut self, message: String) {
        self.s.phase = Phase::Refused;
        self.hold_waiting(HoldReason::AgentStopped);
        self.agent(AgentState::Failed {
            message: bound(message),
        });
        self.out.hang_up = true;
        self.mark(Moment::Stopped);
    }

    /// The core's handling failed (decision 13): the agent fails, a waiting
    /// prompt is held, and the agent is left running.
    fn fail(&mut self, reason: &str) {
        self.fault = None;
        if self.ended() {
            return;
        }
        self.s.phase = Phase::Broken;
        self.s.run = None;
        self.hold_waiting(HoldReason::AgentStopped);
        if self.running() {
            self.turn(TurnState::Failed {
                message: "ACP handling failed".to_owned(),
            });
        }
        self.agent(AgentState::Failed {
            message: bound(format!("ACP handling failed: {}", clean(reason))),
        });
        self.mark(Moment::Stopped);
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
        match wire::decode(line) {
            Ok(message) => self.message(message),
            // An empty line is no message and no fault.
            Err(Malformed::Blank) => {}
            Err(Malformed::InvalidRequest) => {
                self.bad_line();
                self.out
                    .stdin
                    .extend(rpc::error(None, Refusal::InvalidRequest));
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
                match wire::request_kind(&method) {
                    RequestKind::Permission => self.permission(id, &params),
                    RequestKind::Unoffered => self
                        .out
                        .stdin
                        .extend(rpc::error(Some(&id), Refusal::MethodNotFound)),
                }
            }
            Message::Notification { method, params } => match wire::notification_kind(&method) {
                NotificationKind::SessionUpdate => match wire::session_update(&params) {
                    Ok(update) => {
                        self.flush_bad();
                        self.update(update);
                    }
                    // A muted replay counts even an update Brindle can't
                    // read: it would show nothing either way.
                    Err(_) if self.mute() => {}
                    Err(_) => self.bad_line(),
                },
                NotificationKind::CancelRequest => match wire::cancel_request(&params) {
                    Ok(id) => {
                        self.flush_bad();
                        self.withdraw(&id);
                    }
                    Err(_) => self.bad_line(),
                },
                NotificationKind::Other => self.flush_bad(),
            },
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
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => false,
        }
    }

    fn update(&mut self, update: wire::SessionUpdate) {
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
        if self.mute() {
            return;
        }
        match update.update {
            Update::UserChunk(content) => self.chunk(RunKind::User, content),
            Update::AgentChunk(content) => self.chunk(RunKind::Agent, content),
            Update::ThoughtChunk(content) => self.chunk(RunKind::Thought, content),
            Update::ToolCall(call) => self.tool_call(call),
            Update::ToolChanged(change) => self.tool_change(change),
            // Applied above.
            Update::SessionInfo { .. } | Update::Ignored { .. } => {}
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
        if let Some(run) = self.s.run
            && run.kind == kind
            && self.s.transcript.entry(run.entry).is_some()
        {
            for piece in chunks(&text) {
                self.emit(Event::Appended {
                    id: run.entry,
                    text: piece.to_owned(),
                });
            }
        } else {
            self.add_text(kind, &text);
        }
    }

    /// The tool entry for the agent's call id, when the log still has it.
    fn tool(&self, call: &str) -> Option<EntryId> {
        let id = *self.s.tools.get(call)?;
        self.s.transcript.entry(id).map(|_| id)
    }

    fn tool_call(&mut self, call: ToolCall) {
        let id = bound(clean(&call.call));
        let title = bound(clean(&call.title));
        match self.tool(&id) {
            // An agent may send `tool_call` again for a call it already
            // named: its fields change the entry.
            Some(entry) => {
                self.change_tool(entry, Some(title), Some(call.kind), Some(call.status));
            }
            None => self.add_tool(id, title, call.kind, call.status),
        }
    }

    fn tool_change(&mut self, change: ToolChange) {
        let id = bound(clean(&change.call));
        let title = change.title.map(|title| bound(clean(&title)));
        match self.tool(&id) {
            Some(entry) => self.change_tool(entry, title, change.kind, change.status),
            // An update for a call the log doesn't have adds it with the
            // fields it carries.
            None => {
                let title = title.unwrap_or_else(|| id.clone());
                self.add_tool(
                    id,
                    title,
                    change.kind.unwrap_or(ToolKind::Other),
                    change.status.unwrap_or(ToolStatus::Pending),
                );
            }
        }
    }

    fn add_tool(&mut self, call: String, title: String, kind: ToolKind, status: ToolStatus) {
        let entry = self.add(Entry::tool(call.clone(), title, kind, status));
        self.s.tools.insert(call, entry);
    }

    /// Sends the fields that differ from the entry's, in whatever order
    /// they come (statuses can go backwards); nothing when none differ.
    fn change_tool(
        &mut self,
        entry: EntryId,
        title: Option<String>,
        kind: Option<ToolKind>,
        status: Option<ToolStatus>,
    ) {
        let Some(Entry::Tool {
            title: own_title,
            kind: own_kind,
            status: own_status,
            ..
        }) = self.s.transcript.entry(entry)
        else {
            return;
        };
        let title = title.filter(|title| title != own_title);
        let kind = kind.filter(|kind| kind != own_kind);
        let status = status.filter(|status| status != own_status);
        if title.is_none() && kind.is_none() && status.is_none() {
            return;
        }
        self.emit(Event::ToolChanged {
            id: entry,
            title,
            kind,
            status,
            locations: None,
            input: None,
            result: None,
            more: None,
        });
    }

    fn permission(&mut self, id: Id, params: &Value) {
        let Ok(request) = wire::permission_request(params) else {
            // Answered by its envelope id, never left waiting (decision 5).
            self.out.stdin.extend(rpc::permission_answer(&id, None));
            self.notice(
                NoticeLevel::Warning,
                "the agent asked for a permission Brindle couldn't read, which was answered cancelled"
                    .to_owned(),
            );
            return;
        };
        let mut options = Vec::new();
        let mut unknown = Vec::new();
        for option in request.options {
            match option.kind {
                OptionKind::Known(kind) => options.push(PermissionOption {
                    id: option.id,
                    name: bound(clean(&option.name)),
                    kind,
                }),
                OptionKind::Unknown { name } => unknown.push(clean(&name)),
            }
        }
        let call = request.call.as_ref().map(|call| bound(clean(&call.call)));
        let title = request
            .call
            .as_ref()
            .and_then(|call| call.title.as_deref())
            .map(|title| bound(clean(title)))
            .or_else(|| {
                let entry = self.tool(call.as_deref()?)?;
                match self.s.transcript.entry(entry)? {
                    Entry::Tool { title, .. } => Some(title.clone()),
                    Entry::User { .. }
                    | Entry::Agent { .. }
                    | Entry::Thought { .. }
                    | Entry::Permission { .. }
                    | Entry::Notice { .. } => None,
                }
            })
            .unwrap_or_else(|| "the agent asks for permission".to_owned());
        let shown = !options.is_empty();
        let entry = self.add(Entry::Permission {
            call,
            title,
            options,
            outcome: None,
        });
        if !unknown.is_empty() {
            self.notice(
                NoticeLevel::Info,
                format!(
                    "permission request {entry} also offered options of a kind Brindle doesn't know ({}), which aren't shown",
                    unknown.join(", ")
                ),
            );
        }
        if !shown {
            self.cancel_now(entry, &id);
            self.notice(
                NoticeLevel::Warning,
                format!(
                    "permission request {entry} offered no option Brindle can show, so it was answered cancelled"
                ),
            );
        } else if *self.s.transcript.turn() == TurnState::Cancelling {
            self.cancel_now(entry, &id);
        } else if self.s.rpc.hold_permission(entry, id.clone()).is_err() {
            self.cancel_now(entry, &id);
            self.notice(
                NoticeLevel::Warning,
                format!(
                    "more than {} permission requests were waiting, so request {entry} was answered cancelled",
                    rpc::MAX_PENDING_PERMISSIONS
                ),
            );
        } else if *self.s.transcript.turn() != TurnState::Blocked {
            self.turn(TurnState::Blocked);
        }
        self.mark(Moment::Permission);
    }

    /// Answers a permission request `cancelled` at once and records it.
    fn cancel_now(&mut self, entry: EntryId, id: &Id) {
        self.out.stdin.extend(rpc::permission_answer(id, None));
        self.emit(Event::Answered {
            id: entry,
            outcome: PermissionOutcome::Cancelled,
        });
    }

    /// `$/cancel_request` for one of the agent's permission requests.
    fn withdraw(&mut self, id: &Id) {
        if let Some((entry, answer)) = self.s.rpc.withdraw(id) {
            self.out.stdin.extend(answer);
            self.emit(Event::Answered {
                id: entry,
                outcome: PermissionOutcome::Cancelled,
            });
            self.unblock();
            self.mark(Moment::Permission);
        }
    }

    /// With no permission request left waiting, a blocked turn works again
    /// (or, with no prompt out, is idle).
    fn unblock(&mut self) {
        if self.s.rpc.permissions().is_empty() && *self.s.transcript.turn() == TurnState::Blocked {
            let state = if self.prompt_awaited() {
                TurnState::Working
            } else {
                TurnState::Idle
            };
            self.turn(state);
        }
    }

    // --- answers to the core's requests ---------------------------------

    fn response(&mut self, purpose: Purpose, result: Result<&Value, &RpcError>) {
        match purpose {
            Purpose::Initialize => self.initialized(result),
            Purpose::NewSession => self.created(result),
            Purpose::Prompt => self.prompted(result),
            Purpose::Resume | Purpose::Load => self.continued(result),
            Purpose::Authenticate => self.authenticated(result),
        }
    }

    fn initialized(&mut self, result: Result<&Value, &RpcError>) {
        let Phase::Initializing { start } = &self.s.phase else {
            return;
        };
        let start = start.clone();
        match result {
            Ok(result) => match wire::initialize_result(result) {
                Ok(initialized) if initialized.protocol_version == PROTOCOL_VERSION => {
                    let (name, version) = initialized.agent.map_or((None, None), |agent| {
                        (
                            Some(agent_name(&agent)),
                            Some(bound(clean(&agent.version))),
                        )
                    });
                    let greeting = Greeting {
                        name,
                        version,
                        continuation: initialized.continuation,
                    };
                    self.keep_methods(wire::auth_methods(result, true));
                    self.start(start, greeting);
                }
                Ok(initialized) => self.refuse(format!(
                    "the agent speaks ACP version {}; Brindle speaks {PROTOCOL_VERSION}",
                    initialized.protocol_version
                )),
                Err(_) => self.refuse(
                    "the agent's answer to initialize isn't one Brindle can read".to_owned(),
                ),
            },
            Err(error) => {
                let message = match wire::supported_versions(error) {
                    Some(versions) => format!(
                        "the agent supports ACP version{} {}; Brindle speaks {PROTOCOL_VERSION}",
                        if versions.len() == 1 { "" } else { "s" },
                        versions
                            .iter()
                            .map(u16::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    None if error.code == UNSUPPORTED_VERSION => format!(
                        "the agent supports no ACP version Brindle speaks ({PROTOCOL_VERSION})"
                    ),
                    None => format!("the agent couldn't start: {}", clean(&error.message)),
                };
                self.refuse(message);
            }
        }
    }

    /// Keeps the login methods `initialize` advertised, their text cleaned,
    /// with one warning notice when any were left out or cut
    /// (`agents-login` decision 3). Brindle always advertises
    /// `auth.terminal`.
    fn keep_methods(&mut self, read: wire::Methods) {
        self.s.methods = read
            .methods
            .into_iter()
            .map(|method| LoginMethod {
                name: clean(&method.name),
                description: method.description.map(|text| clean(&text)),
                ..method
            })
            .collect();
        if let Some(text) = methods_notice(read.dropped, read.cut) {
            self.notice(NoticeLevel::Warning, text);
        }
    }

    /// After `initialize`: `session/new` for a new conversation, else
    /// `session/resume` or `session/load` by decision 2's table (a shown
    /// replay's notice first), or a refusal when neither is offered. No
    /// continue falls back to `session/new`.
    fn start(&mut self, start: Start, greeting: Greeting) {
        let (conversation, log) = match start {
            Start::New => {
                let line = self.s.rpc.new_session(&self.s.cwd);
                self.s.phase = Phase::Creating { greeting };
                self.out.stdin.extend(line);
                self.mark(Moment::BeforeRequest);
                return;
            }
            Start::Continue { conversation, log } => (conversation, log),
        };
        let line = match choose(log, greeting.continuation) {
            Choice::Resume => {
                let line = self.s.rpc.resume(&conversation.id, &conversation.cwd);
                self.s.phase = Phase::Resuming {
                    greeting,
                    conversation: conversation.id,
                    log,
                };
                line
            }
            Choice::Load { replay } => {
                if replay == Replay::Shown {
                    self.notice(NoticeLevel::Info, REPLAY_SHOWN.to_owned());
                }
                let line = self.s.rpc.load(&conversation.id, &conversation.cwd);
                self.s.phase = Phase::Loading {
                    greeting,
                    conversation: conversation.id,
                    replay,
                };
                line
            }
            Choice::Neither => return self.refuse(CANT_CONTINUE.to_owned()),
        };
        self.out.stdin.extend(line);
        self.mark(Moment::BeforeRequest);
    }

    fn created(&mut self, result: Result<&Value, &RpcError>) {
        let Phase::Creating { greeting } = &self.s.phase else {
            return;
        };
        let greeting = greeting.clone();
        match result {
            Ok(result) => match wire::new_session_result(result) {
                Ok(created) => self.ready(created.session_id, greeting, None),
                Err(_) => self.refuse(
                    "the agent's answer to session/new isn't one Brindle can read".to_owned(),
                ),
            },
            Err(error) if error.code == AUTH_REQUIRED => {
                self.login_required(error, greeting, AfterLogin::Start(Start::New));
            }
            Err(error) => self.refuse(format!(
                "the agent couldn't start a session: {}",
                clean(&error.message)
            )),
        }
    }

    /// `session/resume`'s or `session/load`'s answer (decision 4 of
    /// `agents-session-recovery`): the conversation continues in the same
    /// ACP session, with the notice its way calls for; `-32000` needs a
    /// login; any other error, or an answer Brindle can't read, fails it.
    fn continued(&mut self, result: Result<&Value, &RpcError>) {
        let (greeting, conversation, log, notice, method) = match &self.s.phase {
            Phase::Resuming {
                greeting,
                conversation,
                log,
            } => {
                let notice = match log {
                    Log::Kept => None,
                    Log::Fresh => Some((NoticeLevel::Warning, NOT_REPLAYED.to_owned())),
                };
                (greeting, conversation, *log, notice, "session/resume")
            }
            Phase::Loading {
                greeting,
                conversation,
                replay,
            } => {
                let (notice, log) = match replay {
                    Replay::Muted { updates } => {
                        (Some((NoticeLevel::Info, muted_notice(*updates))), Log::Kept)
                    }
                    Replay::Shown => (None, Log::Fresh),
                };
                (greeting, conversation, log, notice, "session/load")
            }
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Ready { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => return,
        };
        let (greeting, conversation) = (greeting.clone(), conversation.clone());
        let readable = match result {
            Ok(result) if method == "session/load" => wire::load_result(result).is_ok(),
            Ok(result) => wire::resume_result(result).is_ok(),
            // After a login the same continue is sent again (the session's
            // directory is the conversation's).
            Err(error) if error.code == AUTH_REQUIRED => {
                let start = Start::Continue {
                    conversation: Conversation {
                        id: conversation,
                        cwd: self.s.cwd.clone(),
                        continuation: greeting.continuation,
                        // `ready` takes the title from the log.
                        title: None,
                    },
                    log,
                };
                return self.login_required(error, greeting, AfterLogin::Start(start));
            }
            Err(error) => {
                return self.refuse(format!(
                    "the agent couldn't continue the conversation: {}",
                    clean(&error.message)
                ));
            }
        };
        if !readable {
            return self.refuse(format!(
                "the agent couldn't continue the conversation: its answer to {method} isn't one Brindle can read"
            ));
        }
        // The replay's last chunk is no run a live chunk extends.
        self.s.run = None;
        self.ready(conversation, greeting, notice);
    }

    /// The agent takes prompts in ACP session `session`: the conversation,
    /// then `notice` when there is one, then the ready state.
    fn ready(
        &mut self,
        session: String,
        greeting: Greeting,
        notice: Option<(NoticeLevel, String)>,
    ) {
        // The same ACP session keeps its title (a login's restart, a
        // continue by resume or load); a new one has none yet.
        let title = self
            .s
            .transcript
            .conversation()
            .filter(|conversation| conversation.id == session)
            .and_then(|conversation| conversation.title.clone());
        self.emit(Event::Conversation {
            conversation: Conversation {
                id: session.clone(),
                cwd: self.s.cwd.clone(),
                continuation: greeting.continuation,
                title,
            },
        });
        if let Some((level, text)) = notice {
            self.notice(level, text);
        }
        self.s.phase = Phase::Ready { session };
        self.agent(AgentState::Ready {
            name: greeting.name,
            version: greeting.version,
        });
        self.mark(Moment::Started);
    }

    /// `-32000` to a start request (decision 17): the agent waits for a
    /// login, after which `then` goes on.
    fn login_required(&mut self, error: &RpcError, greeting: Greeting, then: AfterLogin) {
        self.needs_login(Login {
            greeting,
            message: bound(clean(&error.message)),
            then,
        });
        self.mark(Moment::Started);
    }

    /// The agent needs a login: the phase and the agent state, with its
    /// methods (`agents-login` decision 4). The caller marks the moment.
    fn needs_login(&mut self, login: Login) {
        let message = login.message.clone();
        self.s.phase = Phase::LoginRequired(login);
        self.agent(AgentState::LoginRequired {
            message,
            methods: self.s.methods.clone(),
        });
    }

    /// What `initialize` said, as the log has it once the agent is ready:
    /// its name and version, and what its conversation offers.
    fn greeting_now(&self) -> Greeting {
        let (name, version) = match self.s.transcript.agent() {
            AgentState::Ready { name, version } => (name.clone(), version.clone()),
            AgentState::Starting
            | AgentState::LoginRequired { .. }
            | AgentState::LoggingIn { .. }
            | AgentState::Failed { .. }
            | AgentState::Exited { .. }
            | AgentState::Lost { .. } => (None, None),
        };
        let continuation = self
            .s
            .transcript
            .conversation()
            .map_or(Continuation::Neither, |conversation| {
                conversation.continuation
            });
        Greeting {
            name,
            version,
            continuation,
        }
    }

    /// `authenticate`'s answer (`agents-login` decision 4): a result, of
    /// whatever shape, is a login, and the session goes on (`Starting`
    /// first); an error needs a login again, with its message.
    fn authenticated(&mut self, result: Result<&Value, &RpcError>) {
        let Phase::LoggingIn { login, .. } = &self.s.phase else {
            return;
        };
        let login = login.clone();
        match result {
            Ok(_) => {
                self.agent(AgentState::Starting);
                self.mark(Moment::Login);
                match login.then {
                    AfterLogin::Start(start) => self.start(start, login.greeting),
                    AfterLogin::Ready { session } => {
                        self.s.phase = Phase::Ready { session };
                        self.agent(AgentState::Ready {
                            name: login.greeting.name,
                            version: login.greeting.version,
                        });
                    }
                }
            }
            Err(error) => {
                self.needs_login(Login {
                    message: bound(clean(&error.message)),
                    ..login
                });
                self.mark(Moment::Login);
            }
        }
    }

    fn prompted(&mut self, result: Result<&Value, &RpcError>) {
        match result {
            Ok(result) => match wire::prompt_result(result) {
                Ok(stop) => self.end_turn(Ending::Stop(stop)),
                Err(_) => self.end_turn(Ending::Failed(
                    "the agent's answer to the prompt isn't one Brindle can read".to_owned(),
                )),
            },
            Err(error) if error.code == AUTH_REQUIRED => self.prompt_needs_login(error),
            Err(error) => self.end_turn(Ending::Failed(bound(clean(&error.message)))),
        }
    }

    /// Decision 17: the turn fails, a waiting prompt is held, then the
    /// agent needs a login (the queue's change first, as decision 10's
    /// table says), after which it is ready again in the same ACP session.
    /// Neither prompt is ever sent again on its own (`agents-login`
    /// decision 5).
    fn prompt_needs_login(&mut self, error: &RpcError) {
        let message = bound(clean(&error.message));
        // A prompt is out only while ready.
        let Phase::Ready { session } = &self.s.phase else {
            return self.end_turn(Ending::Failed(message));
        };
        let login = Login {
            greeting: self.greeting_now(),
            message: message.clone(),
            then: AfterLogin::Ready {
                session: session.clone(),
            },
        };
        self.s.run = None;
        self.hold_waiting(HoldReason::LoginRequired);
        self.turn(TurnState::Failed { message });
        self.needs_login(login);
        self.mark(Moment::TurnEnded);
    }

    /// The prompt's answer ends the turn: a waiting prompt is sent after
    /// `end_turn` with the agent ready, and held after any other ending
    /// (decision 10).
    fn end_turn(&mut self, ending: Ending) {
        self.s.run = None;
        let cancelling = *self.s.transcript.turn() == TurnState::Cancelling;
        let ready = self.readiness().is_ok();
        let waiting = match self.s.transcript.slot() {
            Slot::Waiting { text } => Some(text.clone()),
            Slot::Empty | Slot::Held { .. } => None,
        };
        let sends = ready && matches!(ending, Ending::Stop(StopReason::EndTurn));
        if let Some(text) = waiting {
            if sends {
                self.turn(ending.state());
                self.emit(Event::Queue { slot: Slot::Empty });
                self.mark(Moment::QueueChanged);
                self.send_prompt(text);
                return;
            }
            self.hold_waiting(hold_reason(&ending, cancelling));
        }
        self.turn(ending.state());
        self.mark(Moment::TurnEnded);
    }

    /// Sends `text` as a new turn: its user entry, the turn working, and
    /// the `session/prompt` line, checkpointed before it is written.
    fn send_prompt(&mut self, text: String) {
        let Phase::Ready { session } = &self.s.phase else {
            return;
        };
        let session = session.clone();
        self.add_text(RunKind::User, &text);
        // A prompt is no run the agent's chunks extend.
        self.s.run = None;
        self.turn(TurnState::Working);
        let line = self.s.rpc.prompt(&session, &text);
        self.out.stdin.extend(line);
        self.mark(Moment::BeforeRequest);
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
        };
        self.out.reply = Some(reply);
    }

    /// `AGENT_LOGIN` (`agents-login` decision 4): with the agent waiting
    /// for a login, `authenticate` with one of its agent methods goes out,
    /// checkpointed first, and the agent is logging in. A terminal method
    /// is the client's to run; anything else is refused and changes
    /// nothing.
    fn login(&mut self, method: String) -> Result<(), Refused> {
        let login = match &self.s.phase {
            Phase::LoginRequired(login) => login.clone(),
            Phase::LoggingIn { .. } => return Err(Refused::AlreadyLoggingIn),
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Ready { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => return Err(Refused::NoLoginNeeded),
        };
        let Some(chosen) = self.s.methods.iter().find(|offered| offered.id == method) else {
            return Err(Refused::NoSuchMethod { method });
        };
        let name = match &chosen.kind {
            LoginKind::Agent => chosen.name.clone(),
            LoginKind::Terminal { .. } => return Err(Refused::RunByClient { method }),
        };
        let line = self.s.rpc.authenticate(&method);
        self.s.phase = Phase::LoggingIn {
            login,
            method: method.clone(),
        };
        self.agent(AgentState::LoggingIn { method, name });
        self.out.stdin.extend(line);
        self.mark(Moment::BeforeRequest);
        Ok(())
    }

    /// `AGENT_CANCEL` while logging in: `$/cancel_request` for the
    /// `authenticate`, whose answer is then ignored, and the agent needs a
    /// login again, with its earlier message.
    fn cancel_login(&mut self, login: Login) {
        if let Some(id) = self.s.rpc.awaited(Purpose::Authenticate) {
            self.s.rpc.forget_id(id);
            self.out.stdin.extend(rpc::cancel_request(id));
        }
        self.needs_login(login);
        self.mark(Moment::Login);
    }

    fn prompt(&mut self, text: &str) -> Result<(), Refused> {
        self.readiness()?;
        let text = clean(text);
        if text.len() > MAX_PROMPT {
            return Err(Refused::TooLong { len: text.len() });
        }
        if !self.running() {
            // A held prompt stays held, untouched (decision 10).
            self.send_prompt(text);
            return Ok(());
        }
        match self.s.transcript.slot() {
            Slot::Empty => {
                self.emit(Event::Queue {
                    slot: Slot::Waiting { text },
                });
                self.mark(Moment::QueueChanged);
                Ok(())
            }
            Slot::Waiting { .. } | Slot::Held { .. } => Err(Refused::AlreadyQueued),
        }
    }

    fn unqueue(&mut self) -> Result<(), Refused> {
        match self.s.transcript.slot() {
            Slot::Empty => Err(Refused::NothingQueued),
            Slot::Waiting { .. } | Slot::Held { .. } => {
                self.emit(Event::Queue { slot: Slot::Empty });
                self.mark(Moment::QueueChanged);
                Ok(())
            }
        }
    }

    fn resend(&mut self) -> Result<(), Refused> {
        let Slot::Held { text, .. } = self.s.transcript.slot() else {
            return Err(Refused::NoneHeld);
        };
        let text = text.clone();
        self.readiness()?;
        if self.running() {
            self.emit(Event::Queue {
                slot: Slot::Waiting { text },
            });
            self.mark(Moment::QueueChanged);
        } else {
            self.emit(Event::Queue { slot: Slot::Empty });
            self.mark(Moment::QueueChanged);
            self.send_prompt(text);
        }
        Ok(())
    }

    /// Decision 8: a waiting prompt is held first, then `session/cancel`,
    /// then `cancelled` for each waiting permission request, and the turn
    /// is cancelling until the prompt's answer ends it. With no prompt out
    /// (decision 11's known gap) the turn ends here. While logging in, it
    /// cancels the login (`agents-login` decision 4).
    fn cancel(&mut self) {
        if let Phase::LoggingIn { login, .. } = &self.s.phase {
            let login = login.clone();
            return self.cancel_login(login);
        }
        let Phase::Ready { session } = &self.s.phase else {
            return;
        };
        let session = session.clone();
        match self.s.transcript.turn() {
            TurnState::Working | TurnState::Blocked => {}
            TurnState::Idle
            | TurnState::Cancelling
            | TurnState::Ended { .. }
            | TurnState::Failed { .. } => return,
        }
        self.hold_waiting(HoldReason::Cancelled);
        self.out.stdin.extend(rpc::cancel(&session));
        for (entry, answer) in self.s.rpc.cancel_permissions() {
            self.out.stdin.extend(answer);
            self.emit(Event::Answered {
                id: entry,
                outcome: PermissionOutcome::Cancelled,
            });
        }
        if self.prompt_awaited() {
            self.turn(TurnState::Cancelling);
            self.mark(Moment::Permission);
        } else {
            self.s.run = None;
            self.turn(TurnState::Ended {
                stop: StopReason::Cancelled,
            });
            self.mark(Moment::TurnEnded);
        }
    }

    fn answer(&mut self, entry: EntryId, option: Option<String>) -> Result<(), Refused> {
        match self.s.phase {
            Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => {
                self.readiness()?;
            }
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Ready { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. } => {}
        }
        let Some(Entry::Permission {
            options, outcome, ..
        }) = self.s.transcript.entry(entry)
        else {
            return Err(Refused::NoSuchRequest { entry });
        };
        if outcome.is_some() {
            return Err(Refused::AlreadyAnswered { entry });
        }
        if let Some(chosen) = &option
            && !options.iter().any(|offered| offered.id == *chosen)
        {
            return Err(Refused::NoSuchOption {
                entry,
                option: chosen.clone(),
            });
        }
        let Some(answer) = self.s.rpc.answer_permission(entry, option.as_deref()) else {
            return Err(Refused::AlreadyAnswered { entry });
        };
        self.out.stdin.extend(answer);
        let outcome = match option {
            Some(option) => PermissionOutcome::Selected { option },
            None => PermissionOutcome::Cancelled,
        };
        self.emit(Event::Answered { id: entry, outcome });
        self.unblock();
        self.mark(Moment::Permission);
        Ok(())
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
            | Event::Window { .. } => return,
        };
        self.out.events.push(event);
        self.s.tools.retain(|_, entry| *entry > through);
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

/// Why a waiting prompt is held when its turn ends as `ending` (decision
/// 10). A `cancelled` stop the user didn't ask for (Codex ends a turn so
/// after a rejected permission) is a stop reason, not the user's cancel.
fn hold_reason(ending: &Ending, cancelling: bool) -> HoldReason {
    match ending {
        // Reached only when the agent isn't ready.
        Ending::Stop(StopReason::EndTurn) => HoldReason::AgentStopped,
        Ending::Stop(StopReason::Cancelled) if cancelling => HoldReason::Cancelled,
        Ending::Stop(stop) => HoldReason::Stopped { stop: stop.clone() },
        Ending::Failed(_) => HoldReason::Failed,
    }
}

/// The warning when login methods were left out or their descriptions cut
/// (`agents-login` decision 3), or `None` when neither.
fn methods_notice(dropped: usize, cut: usize) -> Option<String> {
    let left_out = match dropped {
        0 => None,
        1 => Some("1 login method from the agent was left out".to_owned()),
        n => Some(format!("{n} login methods from the agent were left out")),
    };
    let cut = match cut {
        0 => None,
        1 => Some("1 login method's description was cut".to_owned()),
        n => Some(format!("{n} login methods' descriptions were cut")),
    };
    match (left_out, cut) {
        (Some(left_out), Some(cut)) => Some(format!("{left_out}, and {cut}")),
        (Some(text), None) | (None, Some(text)) => Some(text),
        (None, None) => None,
    }
}

/// What a chunk that isn't text shows.
fn placeholder(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Image => "[image]",
        ContentKind::Audio => "[audio]",
        ContentKind::ResourceLink => "[link]",
        ContentKind::Resource => "[resource]",
        ContentKind::Unknown => "[content Brindle can't show]",
    }
}

/// `text` with every control character but `\n`, `\t` and a `\r` before
/// `\n` (or at the end, where a chunk may have cut a `\r\n`) replaced by
/// U+FFFD (decision 8).
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

/// The agent's readable name (`agents-session-title` decision 4):
/// `agentInfo.title` when it is given and not blank, else `agentInfo.name`
/// (which the npm adapters fill with their package's name), cleaned and
/// bounded.
fn agent_name(agent: &wire::AgentInfo) -> String {
    agent
        .title
        .as_deref()
        .map(|title| bound(clean(title)))
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| bound(clean(&agent.name)))
}

/// A conversation's title as kept (`agents-session-title` decision 3):
/// control characters dropped, each run of whitespace (line breaks
/// included) one space, trimmed, and past [`MAX_SESSION_TITLE`] characters
/// cut to one fewer and ended with `…`. A title with nothing left is none.
pub fn session_title(raw: &str) -> Option<String> {
    let mut title = String::with_capacity(raw.len().min(4 * MAX_SESSION_TITLE));
    let mut chars = 0usize;
    let mut gap = false;
    for c in raw.chars() {
        if c.is_whitespace() {
            gap = chars > 0;
            continue;
        }
        if c.is_control() {
            continue;
        }
        if gap {
            title.push(' ');
            chars += 1;
            gap = false;
        }
        title.push(c);
        chars += 1;
        if chars > MAX_SESSION_TITLE {
            break;
        }
    }
    if chars > MAX_SESSION_TITLE {
        let cut = title
            .char_indices()
            .nth(MAX_SESSION_TITLE - 1)
            .map_or(title.len(), |(at, _)| at);
        title.truncate(cut);
        title.truncate(title.trim_end().len());
        title.push('…');
    }
    (!title.is_empty()).then_some(title)
}

/// `text` cut to at most [`TEXT_CHUNK`] bytes at a character boundary, for
/// a field that doesn't grow.
fn bound(mut text: String) -> String {
    if text.len() > TEXT_CHUNK {
        let mut cut = TEXT_CHUNK;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use brindle_remote_protocol::serde_json::{self, json};
    use brindle_transcript::{Activity, PermissionKind};

    /// An agent session under test, with a client's copy of its log folded
    /// from every event `step` returned (checked equal to the session's
    /// after each step) and every line written to the agent.
    struct Agent {
        s: AgentSession,
        client: Transcript,
        now: Instant,
        events: Vec<Event>,
        sent: Vec<Value>,
    }

    impl Agent {
        fn new() -> Self {
            Self::in_dir("/home/u/src")
        }

        fn in_dir(cwd: &str) -> Self {
            Self {
                s: AgentSession::new(cwd.into(), "0.3.0".into()),
                client: Transcript::new(),
                now: Instant::now(),
                events: Vec::new(),
                sent: Vec::new(),
            }
        }

        /// Spawned, `initialize` and `session/new` answered: ready in ACP
        /// session `s1`.
        fn ready() -> Self {
            Self::ready_in("s1")
        }

        fn ready_in(session: &str) -> Self {
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

        fn step(&mut self, input: Input<'_>) -> Output {
            let out = step(&mut self.s, input, self.now);
            for event in &out.events {
                assert_eq!(Event::from_value(&event.to_value()).as_ref(), Ok(event));
                self.client
                    .apply(event)
                    .unwrap_or_else(|e| panic!("{event:?}: {e}"));
            }
            assert_eq!(self.client, self.s.transcript, "the client's fold differs");
            for line in out.stdin.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                self.sent
                    .push(serde_json::from_slice(line).expect("a JSON line"));
            }
            self.events.extend(out.events.iter().cloned());
            out
        }

        fn feed(&mut self, line: &str) -> Output {
            self.step(Input::Stdout(format!("{line}\n").as_bytes()))
        }

        fn cmd(&mut self, command: Command) -> Output {
            self.step(Input::Command(command))
        }

        fn prompt(&mut self, text: &str) -> Output {
            self.cmd(Command::Prompt { text: text.into() })
        }

        /// The id of the last `session/prompt` written.
        fn prompt_id(&self) -> u64 {
            self.last("session/prompt")["id"].as_u64().expect("an id")
        }

        fn last(&self, method: &str) -> &Value {
            self.sent
                .iter()
                .rev()
                .find(|line| line["method"] == method)
                .unwrap_or_else(|| panic!("no {method} was sent"))
        }

        fn sent_methods(&self) -> Vec<&str> {
            self.sent
                .iter()
                .filter_map(|line| line["method"].as_str())
                .collect()
        }

        /// Starts a turn with `text` and returns its prompt's id.
        fn turn(&mut self, text: &str) -> u64 {
            assert_eq!(self.prompt(text).reply, Some(Ok(())));
            self.prompt_id()
        }

        fn end(&mut self, id: u64, stop: &str) -> Output {
            self.feed(&result(id, json!({"stopReason": stop})))
        }

        fn slot(&self) -> &Slot {
            self.s.transcript().slot()
        }

        fn turn_state(&self) -> &TurnState {
            self.s.transcript.turn()
        }

        fn agent_state(&self) -> &AgentState {
            self.s.transcript.agent()
        }

        fn activity(&self) -> Activity {
            self.s.transcript.activity()
        }

        fn entries(&self) -> Vec<Entry> {
            self.s
                .transcript
                .entries()
                .map(|(_, entry)| entry.clone())
                .collect()
        }

        fn last_entry(&self) -> (EntryId, Entry) {
            self.s
                .transcript
                .entries()
                .last()
                .map(|(id, entry)| (id, entry.clone()))
                .expect("an entry")
        }

        fn notices(&self) -> Vec<String> {
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
        fn ask(&mut self, id: Value, options: &[(&str, &str)]) -> EntryId {
            self.feed(&permission(id, "call_1", options));
            let (entry, _) = self.last_permission();
            entry
        }

        fn last_permission(&self) -> (EntryId, Entry) {
            self.s
                .transcript
                .entries()
                .filter(|(_, entry)| matches!(entry, Entry::Permission { .. }))
                .last()
                .map(|(id, entry)| (id, entry.clone()))
                .expect("a permission entry")
        }
    }

    fn result(id: u64, result: Value) -> String {
        json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
    }

    fn error(id: u64, code: i64, message: &str) -> String {
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
    }

    fn update(update: Value) -> String {
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1", "update": update}})
            .to_string()
    }

    fn chunk(kind: &str, text: &str) -> String {
        update(json!({"sessionUpdate": kind, "content": {"type": "text", "text": text}}))
    }

    fn said(text: &str) -> String {
        chunk("agent_message_chunk", text)
    }

    fn tool_call(call: &str, title: &str, status: &str) -> String {
        update(
            json!({"sessionUpdate": "tool_call", "toolCallId": call, "title": title, "kind": "edit", "status": status}),
        )
    }

    fn tool_update(call: &str, status: &str) -> String {
        update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": status}))
    }

    fn permission(id: Value, call: &str, options: &[(&str, &str)]) -> String {
        let options: Vec<Value> = options
            .iter()
            .map(|(id, kind)| json!({"optionId": id, "name": id.to_uppercase(), "kind": kind}))
            .collect();
        json!({"jsonrpc": "2.0", "id": id, "method": "session/request_permission",
               "params": {"sessionId": "s1", "toolCall": {"toolCallId": call, "title": "Write a file"}, "options": options}})
            .to_string()
    }

    const ALLOW_REJECT: [(&str, &str); 2] =
        [("allow-once", "allow_once"), ("reject-once", "reject_once")];

    fn waiting(text: &str) -> Slot {
        Slot::Waiting { text: text.into() }
    }

    fn held(text: &str, reason: HoldReason) -> Slot {
        Slot::Held {
            text: text.into(),
            reason,
        }
    }

    fn ended(stop: StopReason) -> TurnState {
        TurnState::Ended { stop }
    }

    /// The events of kinds that order a turn, as short names.
    fn shape(events: &[Event]) -> Vec<String> {
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
            })
            .collect()
    }

    fn entry_kind(entry: &Entry) -> &'static str {
        match entry {
            Entry::User { .. } => "user",
            Entry::Agent { .. } => "agent",
            Entry::Thought { .. } => "thought",
            Entry::Tool { .. } => "tool",
            Entry::Permission { .. } => "permission",
            Entry::Notice { .. } => "notice",
        }
    }

    fn turn_name(state: &TurnState) -> &'static str {
        match state {
            TurnState::Idle => "idle",
            TurnState::Working => "working",
            TurnState::Blocked => "blocked",
            TurnState::Cancelling => "cancelling",
            TurnState::Ended { .. } => "ended",
            TurnState::Failed { .. } => "failed",
        }
    }

    fn agent_name(state: &AgentState) -> &'static str {
        match state {
            AgentState::Starting => "starting",
            AgentState::Ready { .. } => "ready",
            AgentState::LoginRequired { .. } => "login required",
            AgentState::LoggingIn { .. } => "logging in",
            AgentState::Failed { .. } => "failed",
            AgentState::Exited { .. } => "exited",
            AgentState::Lost { .. } => "lost",
        }
    }

    fn fixture_line(file: &str, n: usize) -> String {
        let text = match file {
            "initialize-32022" => include_str!("../../tests/fixtures/acp/initialize-32022.jsonl"),
            "initialize-v2-reply" => {
                include_str!("../../tests/fixtures/acp/initialize-v2-reply.jsonl")
            }
            "prompt-auth-required" => {
                include_str!("../../tests/fixtures/acp/prompt-auth-required.jsonl")
            }
            other => panic!("no fixture {other}"),
        };
        text.lines().nth(n).expect("a line").to_owned()
    }

    // --- the start sequence (decision 4) --------------------------------

    #[test]
    fn spawning_sends_initialize_for_version_1_advertising_only_auth_terminal() {
        let mut agent = Agent::new();
        let out = agent.step(Input::Spawned);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1,\"clientCapabilities\":{\"auth\":{\"terminal\":true}},\"clientInfo\":{\"name\":\"brindle\",\"version\":\"0.3.0\"}}}\n"
        );
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        assert_eq!(*agent.s.phase(), Phase::Initializing { start: Start::New });
        assert!(agent.step(Input::Spawned).stdin.is_empty(), "spawned twice");
    }

    #[test]
    fn an_answered_initialize_sends_session_new_in_the_absolute_directory() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&result(1, json!({"protocolVersion": 1})));
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/new\",\"params\":{\"cwd\":\"/home/u/src\",\"mcpServers\":[]}}\n"
        );
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        assert_eq!(agent.s.cwd(), "/home/u/src");
    }

    #[test]
    fn an_answered_session_new_makes_the_agent_ready_with_the_name_initialize_gave() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&result(1, json!({"protocolVersion": 1, "agentInfo": {"name": "fake", "title": "Fake", "version": "1.0"}})));
        let out = agent.feed(&result(2, json!({"sessionId": "s1"})));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Ready {
                // The readable title (`agents-session-title` decision 4).
                name: Some("Fake".into()),
                version: Some("1.0".into())
            }
        );
        assert_eq!(out.checkpoint, Some(Moment::Started));
    }

    #[test]
    fn a_32022_with_its_supported_versions_fails_the_agent_naming_them() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&fixture_line("initialize-32022", 0));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent supports ACP version 2; Brindle speaks 1".into()
            }
        );
        assert!(out.hang_up);
        assert!(out.stdin.is_empty(), "no session/new");
        assert_eq!(*agent.s.phase(), Phase::Refused);
    }

    #[test]
    fn several_supported_versions_are_all_named() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32022, "message": "no", "data": {"requested": 1, "supported": [2, 3]}}}).to_string());
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent supports ACP versions 2, 3; Brindle speaks 1".into()
            }
        );
    }

    #[test]
    fn a_32022_without_well_formed_data_fails_naming_brindles_version() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&fixture_line("initialize-32022", 1));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent supports no ACP version Brindle speaks (1)".into()
            }
        );
        assert!(out.hang_up);
    }

    #[test]
    fn a_success_with_another_protocol_version_fails_the_agent_naming_it() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&fixture_line("initialize-v2-reply", 0));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent speaks ACP version 2; Brindle speaks 1".into()
            }
        );
        assert!(out.hang_up);
        assert!(!agent.sent_methods().contains(&"session/new"));
    }

    #[test]
    fn another_initialize_error_fails_the_agent_with_its_message() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&error(1, -32603, "internal error"));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent couldn't start: internal error".into()
            }
        );
        assert!(out.hang_up);
    }

    #[test]
    fn an_unreadable_initialize_answer_fails_the_agent() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&result(1, json!({"nothing": true})));
        assert!(matches!(agent.agent_state(), AgentState::Failed { .. }));
        assert!(out.hang_up);
    }

    #[test]
    fn a_refused_agent_is_sent_nothing_more() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&fixture_line("initialize-v2-reply", 0));
        let out =
            agent.feed(r#"{"jsonrpc":"2.0","id":9,"method":"fs/read_text_file","params":{}}"#);
        assert!(out.stdin.is_empty());
        assert!(agent.feed("not json").events.is_empty());
    }

    #[test]
    fn a_refused_agents_exit_leaves_the_failed_state_and_adds_its_stderr_as_a_notice() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&fixture_line("initialize-32022", 0));
        let out = agent.step(Input::Exited {
            code: Some(1),
            stderr: b"only v2 here\n",
        });
        assert_eq!(shape(&out.events), ["added notice"]);
        assert_eq!(
            agent.notices(),
            ["the agent's standard error:\nonly v2 here\n"]
        );
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent supports ACP version 2; Brindle speaks 1".into()
            }
        );
        let last_agent = agent.events.iter().rev().find_map(|event| match event {
            Event::Agent { state } => Some(state.clone()),
            Event::Added { .. }
            | Event::Appended { .. }
            | Event::ToolChanged { .. }
            | Event::ToolItems { .. }
            | Event::ToolOutput { .. }
            | Event::Answered { .. }
            | Event::Queue { .. }
            | Event::Turn { .. }
            | Event::Conversation { .. }
            | Event::Trimmed { .. }
            | Event::Window { .. } => None,
        });
        assert!(matches!(last_agent, Some(AgentState::Failed { .. })));
    }

    #[test]
    fn a_refused_agents_exit_with_empty_stderr_adds_nothing() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&fixture_line("initialize-32022", 0));
        let out = agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        });
        assert!(out.events.is_empty());
    }

    #[test]
    fn auth_required_from_session_new_needs_a_login_and_keeps_the_agent_running() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&result(1, json!({"protocolVersion": 1})));
        let out = agent.feed(&error(2, -32000, "run agent login"));
        assert_eq!(
            *agent.agent_state(),
            AgentState::LoginRequired {
                message: "run agent login".into(),
                methods: Vec::new(),
            }
        );
        assert!(!out.hang_up);
        assert_eq!(agent.activity(), Activity::Error);
        assert_eq!(agent.prompt("hi").reply, Some(Err(Refused::LoginRequired)));
        assert!(!agent.sent_methods().contains(&"authenticate"));
    }

    #[test]
    fn another_session_new_error_fails_the_start() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&result(1, json!({"protocolVersion": 1})));
        let out = agent.feed(&error(2, -32603, "no such directory"));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent couldn't start a session: no such directory".into()
            }
        );
        assert!(out.hang_up);
    }

    #[test]
    fn a_prompt_before_ready_is_refused_and_nothing_is_queued() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.prompt("hi");
        assert_eq!(out.reply, Some(Err(Refused::NotReady)));
        assert_eq!(
            out.reply
                .as_ref()
                .map(|r| r.as_ref().map_err(ToString::to_string)),
            Some(Err("the agent isn't ready".into()))
        );
        assert!(out.events.is_empty());
        assert_eq!(*agent.slot(), Slot::Empty);
    }

    // --- prompts --------------------------------------------------------

    #[test]
    fn a_prompt_when_idle_is_sent_as_a_turn() {
        let mut agent = Agent::ready();
        let out = agent.prompt("hi");
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["added user", "turn working"]);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"session/prompt\",\"params\":{\"sessionId\":\"s1\",\"prompt\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n"
        );
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
    }

    #[test]
    fn a_turn_gives_the_user_message_the_agent_message_and_the_end_in_order() {
        let mut agent = Agent::ready();
        let id = agent.turn("hi");
        agent.feed(&said("hello"));
        let out = agent.end(id, "end_turn");
        assert_eq!(out.checkpoint, Some(Moment::TurnEnded));
        assert_eq!(
            agent.entries(),
            [
                Entry::User { text: "hi".into() },
                Entry::Agent {
                    text: "hello".into()
                }
            ]
        );
        assert_eq!(*agent.turn_state(), ended(StopReason::EndTurn));
    }

    #[test]
    fn chunks_of_one_kind_merge_into_one_entry() {
        let mut agent = Agent::ready();
        agent.turn("hi");
        agent.feed(&said("Hel"));
        agent.feed(&said("lo"));
        let out = agent.feed(&said("!"));
        assert_eq!(shape(&out.events), ["appended"]);
        assert_eq!(
            agent.last_entry().1,
            Entry::Agent {
                text: "Hello!".into()
            }
        );
    }

    #[test]
    fn a_prompt_during_a_turn_waits_in_the_slot() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let out = agent.prompt("two");
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(*agent.slot(), waiting("two"));
        assert_eq!(out.checkpoint, Some(Moment::QueueChanged));
        assert!(out.stdin.is_empty());
    }

    #[test]
    fn a_third_prompt_is_refused_as_already_queued() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        let out = agent.prompt("three");
        assert_eq!(out.reply, Some(Err(Refused::AlreadyQueued)));
        assert!(out.events.is_empty() && out.stdin.is_empty());
        assert_eq!(*agent.slot(), waiting("two"));
    }

    #[test]
    fn a_prompt_over_256_kib_is_refused_naming_the_limit() {
        let mut agent = Agent::ready();
        let out = agent.prompt(&"x".repeat(300 * 1024));
        assert_eq!(out.reply, Some(Err(Refused::TooLong { len: 300 * 1024 })));
        assert!(
            Refused::TooLong { len: 300 * 1024 }
                .to_string()
                .contains("256 KiB")
        );
        assert!(out.events.is_empty());
        assert!(agent.prompt(&"x".repeat(MAX_PROMPT)).reply == Some(Ok(())));
    }

    #[test]
    fn a_prompt_error_fails_the_turn_with_the_agents_message() {
        let mut agent = Agent::ready();
        let id = agent.turn("hi");
        agent.feed(&error(id, -32603, "rate limited"));
        assert_eq!(
            *agent.turn_state(),
            TurnState::Failed {
                message: "rate limited".into()
            }
        );
        assert_eq!(
            agent.prompt("again").reply,
            Some(Ok(())),
            "the agent is still ready"
        );
    }

    #[test]
    fn a_login_demanded_by_a_prompt_fails_the_turn_holds_the_waiting_prompt_then_needs_the_login() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        assert_eq!(id, 3, "the fixture answers id 3");
        agent.prompt("two");
        let out = agent.feed(&fixture_line("prompt-auth-required", 0));
        assert_eq!(
            shape(&out.events),
            ["queue held", "turn failed", "agent login required"]
        );
        assert_eq!(*agent.slot(), held("two", HoldReason::LoginRequired));
        assert_eq!(
            *agent.turn_state(),
            TurnState::Failed {
                message: "Authentication required".into()
            }
        );
        assert_eq!(
            *agent.agent_state(),
            AgentState::LoginRequired {
                message: "Authentication required".into(),
                methods: Vec::new(),
            }
        );
        assert_eq!(
            agent
                .sent_methods()
                .iter()
                .filter(|m| **m == "session/prompt")
                .count(),
            1
        );
    }

    #[test]
    fn once_a_login_is_needed_prompts_and_resends_are_refused_and_unqueue_still_discards() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.feed(&error(id, -32000, "session expired"));
        assert_eq!(
            agent.prompt("three").reply,
            Some(Err(Refused::LoginRequired))
        );
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::LoginRequired))
        );
        assert_eq!(agent.cmd(Command::Unqueue).reply, Some(Ok(())));
        assert_eq!(*agent.slot(), Slot::Empty);
    }

    #[test]
    fn a_session_whose_handling_failed_refuses_prompts() {
        let mut agent = Agent::ready();
        agent.step(Input::Fault {
            reason: "boom".into(),
        });
        let refused = agent.prompt("hi").reply;
        assert_eq!(
            refused,
            Some(Err(Refused::Failed {
                message: "ACP handling failed: boom".into()
            }))
        );
    }

    #[test]
    fn a_prompt_after_the_agent_exited_is_refused() {
        let mut agent = Agent::ready();
        agent.step(Input::Exited {
            code: Some(0),
            stderr: b"",
        });
        assert_eq!(agent.prompt("hi").reply, Some(Err(Refused::Exited)));
    }

    // --- the slot (decision 10's table, row by row) ---------------------

    #[test]
    fn a_waiting_prompt_is_sent_after_end_turn_as_the_next_turn() {
        let mut agent = Agent::ready();
        let first = agent.turn("one");
        agent.prompt("two");
        let out = agent.end(first, "end_turn");
        assert_eq!(
            shape(&out.events),
            ["turn ended", "queue empty", "added user", "turn working"]
        );
        assert_eq!(agent.prompt_id(), first + 1);
        assert_eq!(
            agent.last("session/prompt")["params"]["prompt"][0]["text"],
            "two"
        );
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
    }

    #[test]
    fn a_cancel_holds_a_waiting_prompt_before_session_cancel_is_sent() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        let out = agent.cmd(Command::Cancel);
        assert_eq!(shape(&out.events), ["queue held", "turn cancelling"]);
        assert_eq!(*agent.slot(), held("two", HoldReason::Cancelled));
        assert_eq!(agent.sent_methods().last(), Some(&"session/cancel"));
    }

    #[test]
    fn a_failed_turn_holds_a_waiting_prompt() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        let out = agent.feed(&error(id, -32603, "rate limited"));
        assert_eq!(shape(&out.events), ["queue held", "turn failed"]);
        assert_eq!(*agent.slot(), held("two", HoldReason::Failed));
        assert_eq!(
            agent
                .sent_methods()
                .iter()
                .filter(|m| **m == "session/prompt")
                .count(),
            1
        );
    }

    #[test]
    fn every_other_stop_reason_holds_a_waiting_prompt_with_that_reason() {
        for (name, stop) in [
            ("max_tokens", StopReason::MaxTokens),
            ("max_turn_requests", StopReason::MaxTurnRequests),
            ("refusal", StopReason::Refusal),
            (
                "paused_for_review",
                StopReason::Other {
                    name: "paused_for_review".into(),
                },
            ),
        ] {
            let mut agent = Agent::ready();
            let id = agent.turn("one");
            agent.prompt("two");
            let out = agent.end(id, name);
            assert_eq!(shape(&out.events), ["queue held", "turn ended"], "{name}");
            assert_eq!(
                *agent.slot(),
                held("two", HoldReason::Stopped { stop: stop.clone() })
            );
            assert_eq!(*agent.turn_state(), ended(stop));
        }
    }

    #[test]
    fn a_cancelled_stop_nobody_asked_for_holds_as_a_stop_reason_not_as_a_cancel() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.end(id, "cancelled");
        assert_eq!(
            *agent.slot(),
            held(
                "two",
                HoldReason::Stopped {
                    stop: StopReason::Cancelled
                }
            )
        );
    }

    #[test]
    fn an_agent_exit_holds_a_waiting_prompt_before_the_exit() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        let out = agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        });
        assert_eq!(
            shape(&out.events),
            ["queue held", "turn failed", "agent exited"]
        );
        assert_eq!(*agent.slot(), held("two", HoldReason::AgentStopped));
    }

    #[test]
    fn a_handling_fault_holds_a_waiting_prompt_before_the_failure() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        let out = agent.step(Input::Fault {
            reason: "boom".into(),
        });
        assert_eq!(
            shape(&out.events),
            ["queue held", "turn failed", "agent failed"]
        );
        assert_eq!(*agent.slot(), held("two", HoldReason::AgentStopped));
    }

    #[test]
    fn lost_output_fails_the_turn_and_holds_a_waiting_prompt() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        let out = agent.step(Input::OutputLost);
        assert_eq!(
            shape(&out.events),
            ["added notice", "queue held", "turn failed"]
        );
        assert_eq!(*agent.slot(), held("two", HoldReason::OutputLost));
        assert_eq!(
            *agent.turn_state(),
            TurnState::Failed {
                message: "output was lost while the terminal server restarted".into()
            }
        );
        assert!(
            agent.s.rpc().outgoing().is_empty(),
            "the prompt is forgotten"
        );
        let next = agent.feed(&said("tail of a lost line"));
        assert!(
            next.events.is_empty(),
            "the rest of the cut line is dropped"
        );
        assert_eq!(
            agent.turn("three"),
            4,
            "the next prompt works, its id never reused"
        );
    }

    #[test]
    fn unqueue_discards_a_waiting_prompt_which_is_then_never_sent() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        let out = agent.cmd(Command::Unqueue);
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["queue empty"]);
        agent.end(id, "end_turn");
        assert_eq!(
            agent
                .sent_methods()
                .iter()
                .filter(|m| **m == "session/prompt")
                .count(),
            1
        );
    }

    #[test]
    fn unqueue_discards_a_held_prompt() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.end(id, "max_tokens");
        assert_eq!(agent.cmd(Command::Unqueue).reply, Some(Ok(())));
        assert_eq!(*agent.slot(), Slot::Empty);
    }

    #[test]
    fn resend_when_idle_sends_the_held_prompt_as_a_new_turn() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.end(id, "refusal");
        let out = agent.cmd(Command::Resend);
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(
            shape(&out.events),
            ["queue empty", "added user", "turn working"]
        );
        assert_eq!(
            agent.last("session/prompt")["params"]["prompt"][0]["text"],
            "two"
        );
    }

    #[test]
    fn resend_during_a_turn_queues_the_held_prompt_again() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.end(id, "refusal");
        let third = agent.turn("three");
        let out = agent.cmd(Command::Resend);
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(*agent.slot(), waiting("two"));
        agent.end(third, "end_turn");
        assert_eq!(
            agent.last("session/prompt")["params"]["prompt"][0]["text"],
            "two"
        );
    }

    #[test]
    fn resend_while_the_agent_isnt_ready_is_refused_and_keeps_it_held() {
        // A held prompt while starting comes only from a restored session.
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent
            .s
            .transcript
            .apply(&Event::Queue {
                slot: held("two", HoldReason::OutputLost),
            })
            .expect("a queue event");
        agent.client = agent.s.transcript.clone();
        let out = agent.cmd(Command::Resend);
        assert_eq!(out.reply, Some(Err(Refused::NotReady)));
        assert!(out.events.is_empty());
        assert_eq!(*agent.slot(), held("two", HoldReason::OutputLost));

        let mut broken = Agent::ready();
        let id = broken.turn("one");
        broken.prompt("two");
        broken.end(id, "max_tokens");
        broken.step(Input::Fault {
            reason: "boom".into(),
        });
        assert!(matches!(
            broken.cmd(Command::Resend).reply,
            Some(Err(Refused::Failed { .. }))
        ));
        assert!(matches!(broken.slot(), Slot::Held { .. }));
    }

    #[test]
    fn a_new_prompt_with_one_held_and_no_turn_is_sent_and_leaves_it_held() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.end(id, "max_tokens");
        let out = agent.prompt("three");
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["added user", "turn working"]);
        assert_eq!(
            *agent.slot(),
            held(
                "two",
                HoldReason::Stopped {
                    stop: StopReason::MaxTokens
                }
            )
        );
    }

    #[test]
    fn a_new_prompt_with_one_waiting_or_held_during_a_turn_is_refused() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        assert_eq!(agent.prompt("x").reply, Some(Err(Refused::AlreadyQueued)));
        agent.end(id, "max_tokens");
        agent.turn("three");
        assert_eq!(agent.prompt("x").reply, Some(Err(Refused::AlreadyQueued)));
        assert!(matches!(agent.slot(), Slot::Held { .. }));
    }

    #[test]
    fn unqueue_or_resend_with_an_empty_slot_is_refused() {
        let mut agent = Agent::ready();
        assert_eq!(
            agent.cmd(Command::Unqueue).reply,
            Some(Err(Refused::NothingQueued))
        );
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::NoneHeld))
        );
        assert_eq!(Refused::NothingQueued.to_string(), "nothing is queued");
        assert_eq!(Refused::NoneHeld.to_string(), "no prompt is held");
    }

    #[test]
    fn resend_of_a_waiting_prompt_is_refused() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::NoneHeld))
        );
        assert_eq!(*agent.slot(), waiting("two"));
    }

    // --- cancel ---------------------------------------------------------

    #[test]
    fn cancel_sends_session_cancel_then_cancels_each_waiting_permission() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        let out = agent.cmd(Command::Cancel);
        assert_eq!(out.reply, None);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"method\":\"session/cancel\",\"params\":{\"sessionId\":\"s1\"}}\n{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"outcome\":{\"outcome\":\"cancelled\"}}}\n"
        );
        assert_eq!(shape(&out.events), ["answered", "turn cancelling"]);
        assert!(matches!(
            agent.s.transcript.entry(entry),
            Some(Entry::Permission {
                outcome: Some(PermissionOutcome::Cancelled),
                ..
            })
        ));
    }

    #[test]
    fn a_cancelled_turn_ends_when_the_agent_answers_the_prompt() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.cmd(Command::Cancel);
        assert_eq!(*agent.turn_state(), TurnState::Cancelling);
        agent.end(id, "cancelled");
        assert_eq!(*agent.turn_state(), ended(StopReason::Cancelled));
    }

    #[test]
    fn a_cancel_with_no_turn_running_produces_nothing() {
        let mut agent = Agent::ready();
        assert_eq!(agent.cmd(Command::Cancel), Output::default());
        let id = agent.turn("one");
        agent.end(id, "end_turn");
        assert_eq!(agent.cmd(Command::Cancel), Output::default());
    }

    #[test]
    fn a_cancel_with_no_prompt_out_ends_the_turn_here() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.s.rpc.forget(Purpose::Prompt);
        let out = agent.cmd(Command::Cancel);
        assert_eq!(*agent.turn_state(), ended(StopReason::Cancelled));
        assert_eq!(out.checkpoint, Some(Moment::TurnEnded));
    }

    // --- permissions ----------------------------------------------------

    #[test]
    fn a_permission_request_is_logged_with_its_options_and_blocks_the_turn() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let out = agent.feed(&permission(json!(0), "call_1", &ALLOW_REJECT));
        assert_eq!(shape(&out.events), ["added permission", "turn blocked"]);
        assert_eq!(out.checkpoint, Some(Moment::Permission));
        assert!(out.stdin.is_empty(), "never answered on its own");
        assert_eq!(
            agent.last_permission().1,
            Entry::Permission {
                call: Some("call_1".into()),
                title: "Write a file".into(),
                options: vec![
                    PermissionOption {
                        id: "allow-once".into(),
                        name: "ALLOW-ONCE".into(),
                        kind: PermissionKind::AllowOnce
                    },
                    PermissionOption {
                        id: "reject-once".into(),
                        name: "REJECT-ONCE".into(),
                        kind: PermissionKind::RejectOnce
                    },
                ],
                outcome: None,
            }
        );
    }

    #[test]
    fn a_tool_calls_id_is_cleaned_and_still_matches_its_updates() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&update(json!({"sessionUpdate": "tool_call_update", "toolCallId": "c\u{1b}]0;x\u{7}"})));
        assert!(matches!(
            agent.last_entry().1,
            Entry::Tool { call, title, .. } if call == "c\u{fffd}]0;x\u{fffd}" && title == "c\u{fffd}]0;x\u{fffd}"
        ));
        let entries = agent.entries().len();
        agent.feed(&update(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "c\u{1b}]0;x\u{7}", "status": "completed"}),
        ));
        assert_eq!(agent.entries().len(), entries);
    }

    #[test]
    fn a_permission_requests_title_falls_back_to_its_tool_calls() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Edit main.rs", "pending"));
        agent.feed(&json!({"jsonrpc": "2.0", "id": 0, "method": "session/request_permission",
            "params": {"sessionId": "s1", "toolCall": {"toolCallId": "call_1"}, "options": [{"optionId": "a", "name": "A", "kind": "allow_once"}]}}).to_string());
        assert!(
            matches!(agent.last_permission().1, Entry::Permission { title, .. } if title == "Edit main.rs")
        );
    }

    #[test]
    fn the_attached_clients_choice_answers_it_and_the_turn_works_again() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        let out = agent.cmd(Command::Answer {
            entry,
            option: Some("allow-once".into()),
        });
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"outcome\":{\"outcome\":\"selected\",\"optionId\":\"allow-once\"}}}\n"
        );
        assert_eq!(shape(&out.events), ["answered", "turn working"]);
        assert_eq!(out.checkpoint, Some(Moment::Permission));
    }

    #[test]
    fn an_answer_with_no_option_is_cancelled() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        let out = agent.cmd(Command::Answer {
            entry,
            option: None,
        });
        assert_eq!(out.reply, Some(Ok(())));
        assert!(out.events.contains(&Event::Answered {
            id: entry,
            outcome: PermissionOutcome::Cancelled
        }));
    }

    #[test]
    fn a_second_answer_is_refused_naming_the_request() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        agent.cmd(Command::Answer {
            entry,
            option: Some("allow-once".into()),
        });
        let out = agent.cmd(Command::Answer {
            entry,
            option: Some("reject-once".into()),
        });
        assert_eq!(out.reply, Some(Err(Refused::AlreadyAnswered { entry })));
        assert!(out.stdin.is_empty());
    }

    #[test]
    fn an_answer_naming_an_option_not_offered_or_an_entry_not_a_request_is_refused() {
        let mut agent = Agent::ready();
        let user = EntryId(1);
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        let out = agent.cmd(Command::Answer {
            entry,
            option: Some("deny".into()),
        });
        assert_eq!(
            out.reply,
            Some(Err(Refused::NoSuchOption {
                entry,
                option: "deny".into()
            }))
        );
        let out = agent.cmd(Command::Answer {
            entry: user,
            option: None,
        });
        assert_eq!(out.reply, Some(Err(Refused::NoSuchRequest { entry: user })));
        assert_eq!(*agent.turn_state(), TurnState::Blocked);
    }

    #[test]
    fn a_withdrawn_request_is_answered_cancelled() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!("7"), &ALLOW_REJECT);
        let out = agent
            .feed(r#"{"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":"7"}}"#);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":\"7\",\"result\":{\"outcome\":{\"outcome\":\"cancelled\"}}}\n"
        );
        assert_eq!(shape(&out.events), ["answered", "turn working"]);
        assert_eq!(
            agent
                .cmd(Command::Answer {
                    entry,
                    option: None
                })
                .reply,
            Some(Err(Refused::AlreadyAnswered { entry }))
        );
    }

    #[test]
    fn a_request_past_sixteen_waiting_is_answered_cancelled_with_a_notice() {
        let mut agent = Agent::ready();
        agent.turn("one");
        for id in 0..16 {
            agent.ask(json!(id), &ALLOW_REJECT);
        }
        let out = agent.feed(&permission(json!(16), "call_1", &ALLOW_REJECT));
        assert_eq!(
            shape(&out.events),
            ["added permission", "answered", "added notice"]
        );
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":16,\"result\":{\"outcome\":{\"outcome\":\"cancelled\"}}}\n"
        );
        assert_eq!(agent.s.rpc().permissions().len(), 16);
    }

    #[test]
    fn an_option_of_an_unknown_kind_is_left_out_with_a_notice() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.ask(
            json!(7),
            &[
                ("allow", "allow_once"),
                ("session", "allow_for_session"),
                ("reject", "reject_once"),
            ],
        );
        let (_, entry) = agent.last_permission();
        let Entry::Permission { options, .. } = entry else {
            panic!("not a permission");
        };
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["allow", "reject"]);
        assert_eq!(agent.notices().len(), 1);
        assert!(agent.notices()[0].contains("allow_for_session"));
        assert_eq!(*agent.turn_state(), TurnState::Blocked);
    }

    #[test]
    fn a_request_with_no_option_brindle_knows_is_answered_cancelled() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let out = agent.feed(&permission(
            json!(0),
            "call_1",
            &[("x", "allow_for_session")],
        ));
        assert_eq!(
            shape(&out.events),
            [
                "added permission",
                "added notice",
                "answered",
                "added notice"
            ]
        );
        assert!(
            out.stdin
                .ends_with(b"\"outcome\":{\"outcome\":\"cancelled\"}}}\n")
        );
        assert_eq!(*agent.turn_state(), TurnState::Working);
    }

    #[test]
    fn a_permission_request_that_cant_be_read_is_answered_cancelled_by_its_id() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let out = agent.feed(r#"{"jsonrpc":"2.0","id":"q","method":"session/request_permission","params":{"sessionId":"s1"}}"#);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":\"q\",\"result\":{\"outcome\":{\"outcome\":\"cancelled\"}}}\n"
        );
        assert_eq!(shape(&out.events), ["added notice"]);
    }

    #[test]
    fn a_permission_request_while_cancelling_is_answered_cancelled_at_once() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.cmd(Command::Cancel);
        let out = agent.feed(&permission(json!(0), "call_1", &ALLOW_REJECT));
        assert_eq!(shape(&out.events), ["added permission", "answered"]);
        assert_eq!(*agent.turn_state(), TurnState::Cancelling);
    }

    // --- tool calls -----------------------------------------------------

    #[test]
    fn a_tool_call_is_one_entry_its_updates_change_by_id() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Write a file", "pending"));
        let out = agent.feed(&tool_update("call_1", "completed"));
        assert_eq!(
            out.events,
            [Event::ToolChanged {
                id: EntryId(2),
                title: None,
                kind: None,
                status: Some(ToolStatus::Completed),
                locations: None,
                input: None,
                result: None,
                more: None,
            }]
        );
        assert_eq!(
            agent.last_entry().1,
            Entry::tool(
                "call_1".into(),
                "Write a file".into(),
                ToolKind::Edit,
                ToolStatus::Completed
            )
        );
    }

    #[test]
    fn tool_statuses_apply_in_the_order_they_come_even_backwards() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Write", "in_progress"));
        for status in ["pending", "in_progress", "failed"] {
            agent.feed(&tool_update("call_1", status));
        }
        assert!(matches!(
            agent.last_entry().1,
            Entry::Tool {
                status: ToolStatus::Failed,
                ..
            }
        ));
        let statuses: Vec<ToolStatus> = agent
            .events
            .iter()
            .filter_map(|event| match event {
                Event::ToolChanged { status, .. } => *status,
                Event::Added { .. }
                | Event::Appended { .. }
                | Event::ToolItems { .. }
                | Event::ToolOutput { .. }
                | Event::Answered { .. }
                | Event::Queue { .. }
                | Event::Turn { .. }
                | Event::Agent { .. }
                | Event::Conversation { .. }
                | Event::Trimmed { .. }
                | Event::Window { .. } => None,
            })
            .collect();
        assert_eq!(
            statuses,
            [
                ToolStatus::Pending,
                ToolStatus::InProgress,
                ToolStatus::Failed
            ]
        );
    }

    #[test]
    fn a_tool_call_sent_again_changes_its_entry() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Write", "pending"));
        let out = agent.feed(&tool_call("call_1", "Write hello.txt", "pending"));
        assert_eq!(shape(&out.events), ["tool changed"]);
        assert_eq!(agent.entries().len(), 2);
    }

    #[test]
    fn fields_that_didnt_change_send_nothing() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Write", "pending"));
        assert!(
            agent
                .feed(&tool_call("call_1", "Write", "pending"))
                .events
                .is_empty()
        );
        assert!(
            agent
                .feed(&tool_update("call_1", "pending"))
                .events
                .is_empty()
        );
    }

    #[test]
    fn an_update_for_a_call_the_log_doesnt_have_adds_it() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_update("call_9", "in_progress"));
        assert_eq!(
            agent.last_entry().1,
            Entry::tool(
                "call_9".into(),
                "call_9".into(),
                ToolKind::Other,
                ToolStatus::InProgress
            )
        );
    }

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

    // --- the conversation's title (agents-session-title) ---------------

    fn titled(title: Value) -> String {
        update(json!({"sessionUpdate": "session_info_update", "title": title}))
    }

    fn title_of(agent: &Agent) -> Option<&str> {
        agent
            .s
            .transcript()
            .conversation()
            .and_then(|conversation| conversation.title.as_deref())
    }

    #[test]
    fn a_session_title_is_one_line_bounded_and_none_when_blank() {
        assert_eq!(
            session_title("Hello.txt file creation").as_deref(),
            Some("Hello.txt file creation")
        );
        assert_eq!(
            session_title("  Fix\tthe\r\n\n build  \u{7}now\u{1b}[31m ").as_deref(),
            Some("Fix the build now[31m")
        );
        assert_eq!(
            session_title("a\u{0}b \u{85} c").as_deref(),
            Some("ab c"),
            "a control character is dropped; NEL is whitespace"
        );
        for blank in ["", "   ", "\n\t\r", "\u{7}\u{1b}", " \u{0} "] {
            assert_eq!(session_title(blank), None, "{blank:?}");
        }
        let exact = "x".repeat(MAX_SESSION_TITLE);
        assert_eq!(session_title(&exact).as_deref(), Some(exact.as_str()));
        let long: String = (0..40).map(|n| format!("word{n:03}\n")).collect();
        assert_eq!(long.chars().count(), 320);
        let kept = session_title(&format!("{long}{}", "y".repeat(80))).expect("a title");
        assert_eq!(kept.chars().count(), MAX_SESSION_TITLE);
        assert!(kept.ends_with('…') && !kept.contains('\n'), "{kept}");
        let long = "z".repeat(400);
        let kept = session_title(&long).expect("a title");
        assert_eq!(kept, format!("{}…", "z".repeat(MAX_SESSION_TITLE - 1)));
        // A cut inside multi-byte characters lands on a boundary.
        let wide = "é".repeat(300);
        let kept = session_title(&wide).expect("a title");
        assert_eq!(kept, format!("{}…", "é".repeat(MAX_SESSION_TITLE - 1)));
        // A cut after a space leaves none before the ellipsis.
        let spaced = format!("{} {}", "a".repeat(MAX_SESSION_TITLE - 2), "b".repeat(10));
        let kept = session_title(&spaced).expect("a title");
        assert_eq!(kept, format!("{}…", "a".repeat(MAX_SESSION_TITLE - 2)));
    }

    #[test]
    fn a_title_names_the_conversation_and_null_or_blank_clears_it() {
        let mut agent = Agent::ready();
        let out = agent.feed(&titled(json!("Hello.txt file creation")));
        assert_eq!(shape(&out.events), ["conversation"]);
        assert_eq!(title_of(&agent), Some("Hello.txt file creation"));
        assert_eq!(out.checkpoint, None, "no new checkpoint moment");
        // The same title again sends nothing, and so does a cleaned one
        // that comes out the same.
        assert_eq!(agent.feed(&titled(json!("Hello.txt file creation"))), Output::default());
        assert_eq!(
            agent.feed(&titled(json!(" Hello.txt\nfile  creation "))),
            Output::default()
        );
        // No title keeps it.
        let out = agent.feed(&update(
            json!({"sessionUpdate": "session_info_update", "_meta": {"codex": {}}}),
        ));
        assert_eq!(out, Output::default());
        assert_eq!(title_of(&agent), Some("Hello.txt file creation"));
        // A number is no title either.
        assert_eq!(agent.feed(&titled(json!(7))), Output::default());
        let out = agent.feed(&titled(Value::Null));
        assert_eq!(shape(&out.events), ["conversation"]);
        assert_eq!(title_of(&agent), None);
        assert_eq!(agent.feed(&titled(Value::Null)), Output::default());
        agent.feed(&titled(json!("Again")));
        agent.feed(&titled(json!("  \n ")));
        assert_eq!(title_of(&agent), None, "a blank title clears");
        // Everything else about the conversation stays.
        assert_eq!(
            agent.s.transcript().conversation().map(|c| (c.id.as_str(), c.cwd.as_str())),
            Some(("s1", "/home/u/src"))
        );
    }

    /// The captures' `agentInfo`, verbatim: the readable `title` names the
    /// agent when given, else `name`.
    #[test]
    fn the_agents_name_is_its_title_when_given_else_its_name() {
        let rows = [
            (
                json!({"name": "@agentclientprotocol/codex-acp", "title": "Codex", "version": "2.12.0"}),
                "Codex",
            ),
            (
                json!({"name": "@agentclientprotocol/claude-agent-acp", "title": "Claude Agent", "version": "0.112.0"}),
                "Claude Agent",
            ),
            (
                json!({"name": "@zed-industries/claude-code-acp", "title": "Claude Code", "version": "0.16.2"}),
                "Claude Code",
            ),
            (
                json!({"name": "Antigravity ACP Server", "version": "1.5.16"}),
                "Antigravity ACP Server",
            ),
            (
                json!({"name": "Copilot", "title": "Copilot", "version": "1.0.115"}),
                "Copilot",
            ),
            // A blank or null title is no title.
            (json!({"name": "pkg", "title": "  ", "version": "1"}), "pkg"),
            (json!({"name": "pkg", "title": null, "version": "1"}), "pkg"),
        ];
        for (info, name) in rows {
            let mut agent = Agent::new();
            agent.step(Input::Spawned);
            agent.feed(&result(1, json!({"protocolVersion": 1, "agentInfo": info})));
            agent.feed(&result(2, json!({"sessionId": "s1"})));
            assert!(
                matches!(agent.agent_state(), AgentState::Ready { name: Some(n), .. } if n == name),
                "{info}: {:?}",
                agent.agent_state()
            );
        }
    }

    #[test]
    fn a_title_mid_turn_adds_no_entry() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.feed(&said("Hel"));
        agent.feed(&titled(json!("Greeting")));
        agent.feed(&said("lo"));
        assert_eq!(agent.last_entry().1, Entry::Agent { text: "Hello".into() });
        agent.end(id, "end_turn");
        assert_eq!(title_of(&agent), Some("Greeting"));
    }

    #[test]
    fn a_title_before_the_session_is_given_names_nothing() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&result(
            1,
            json!({"protocolVersion": 1, "agentInfo": {"name": "fake", "version": "1.0"}}),
        ));
        // `session/new` not answered yet: the session isn't known.
        assert_eq!(agent.feed(&titled(json!("Early"))), Output::default());
        agent.feed(&result(2, json!({"sessionId": "s1"})));
        assert_eq!(title_of(&agent), None);
        // Another session's title names nothing either.
        let line = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "other", "update": {"sessionUpdate": "session_info_update", "title": "Other"}}});
        assert_eq!(agent.feed(&line.to_string()), Output::default());
        assert_eq!(title_of(&agent), None);
    }

    #[test]
    fn a_title_is_in_the_replay() {
        let mut agent = Agent::ready();
        agent.feed(&titled(json!("Hello.txt file creation")));
        let replay = agent.s.transcript().replay();
        assert!(replay.iter().any(|event| matches!(
            event,
            Event::Conversation { conversation } if conversation.title.as_deref() == Some("Hello.txt file creation")
        )));
        let mut fresh = Transcript::new();
        for event in &replay {
            fresh.apply(event).expect("a replay folds");
        }
        assert_eq!(fresh, *agent.s.transcript());
    }

    #[test]
    fn an_update_for_another_acp_session_is_ignored() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let line = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "other", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "x"}}}});
        assert!(agent.feed(&line.to_string()).events.is_empty());
    }

    #[test]
    fn an_answer_to_no_request_the_core_awaits_is_ignored() {
        let mut agent = Agent::ready();
        assert_eq!(agent.feed(&result(99, json!({}))), Output::default());
        assert_eq!(agent.feed(&error(98, -1, "x")), Output::default());
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

    // --- exits and faults -----------------------------------------------

    #[test]
    fn an_exit_reports_its_status_and_the_end_of_its_stderr() {
        let mut agent = Agent::ready();
        let out = agent.step(Input::Exited {
            code: Some(1),
            stderr: b"loading\nfatal: no model\n",
        });
        assert_eq!(
            *agent.agent_state(),
            AgentState::Exited {
                code: Some(1),
                stderr: "loading\nfatal: no model\n".into()
            }
        );
        assert_eq!(out.checkpoint, Some(Moment::Stopped));
        assert!(
            agent
                .step(Input::Exited {
                    code: None,
                    stderr: b""
                })
                .events
                .is_empty()
        );
    }

    #[test]
    fn an_exit_mid_turn_fails_the_turn_and_cancels_the_waiting_permissions() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        let out = agent.step(Input::Exited {
            code: None,
            stderr: b"\x1b[31mkilled\xff",
        });
        assert!(out.stdin.is_empty(), "nothing is written to a gone agent");
        assert_eq!(
            shape(&out.events),
            ["answered", "turn failed", "agent exited"]
        );
        assert!(matches!(
            agent.s.transcript.entry(entry),
            Some(Entry::Permission {
                outcome: Some(PermissionOutcome::Cancelled),
                ..
            })
        ));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Exited {
                code: None,
                stderr: "\u{fffd}[31mkilled\u{fffd}".into()
            }
        );
    }

    #[test]
    fn a_handling_fault_fails_the_agent_and_leaves_it_running_with_its_lines_ignored() {
        let mut agent = Agent::ready();
        let out = agent.step(Input::Fault {
            reason: "panicked at reduce.rs".into(),
        });
        assert!(!out.hang_up);
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "ACP handling failed: panicked at reduce.rs".into()
            }
        );
        assert!(agent.feed(&said("x")).events.is_empty());
        assert!(
            agent
                .feed(r#"{"jsonrpc":"2.0","id":4,"method":"fs/x"}"#)
                .stdin
                .is_empty()
        );
        let before = agent.s.lines().covers();
        agent.feed("more");
        assert_eq!(
            agent.s.lines().covers(),
            before + 5,
            "the bytes are still counted"
        );
    }

    #[test]
    fn an_exit_after_a_handling_fault_still_reports_the_exit() {
        let mut agent = Agent::ready();
        agent.step(Input::Fault {
            reason: "boom".into(),
        });
        agent.step(Input::Exited {
            code: Some(0),
            stderr: b"",
        });
        assert!(matches!(agent.agent_state(), AgentState::Exited { .. }));
    }

    #[test]
    fn a_fold_error_from_the_reducers_own_events_fails_the_handling() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Write", "pending"));
        // A run pointing at the tool entry: the next chunk appends to it.
        agent.s.run = Some(Run {
            kind: RunKind::Agent,
            entry: EntryId(2),
        });
        let out = agent.feed(&said("x"));
        assert_eq!(shape(&out.events), ["turn failed", "agent failed"]);
        assert!(
            matches!(agent.agent_state(), AgentState::Failed { message } if message.contains("has no text to append to"))
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
            include_bytes!("../../tests/fixtures/acp/antigravity-1.5.16-reject.log"),
        ),
        (
            "antigravity-1.5.16-trusted-workspace",
            include_bytes!("../../tests/fixtures/acp/antigravity-1.5.16-trusted-workspace.log"),
        ),
        (
            "claude-agent-acp-0.112.0-auto",
            include_bytes!("../../tests/fixtures/acp/claude-agent-acp-0.112.0-auto.log"),
        ),
        (
            "claude-agent-acp-0.112.0-haiku-reject",
            include_bytes!("../../tests/fixtures/acp/claude-agent-acp-0.112.0-haiku-reject.log"),
        ),
        (
            "claude-agent-acp-0.112.0-reject",
            include_bytes!("../../tests/fixtures/acp/claude-agent-acp-0.112.0-reject.log"),
        ),
        (
            "claude-code-acp-0.16.2-reject",
            include_bytes!("../../tests/fixtures/acp/claude-code-acp-0.16.2-reject.log"),
        ),
        (
            "codex-acp-2.12.0-reject",
            include_bytes!("../../tests/fixtures/acp/codex-acp-2.12.0-reject.log"),
        ),
        (
            "copilot-1.0.115-reject",
            include_bytes!("../../tests/fixtures/acp/copilot-1.0.115-reject.log"),
        ),
        // `agents-tool-content` task 1.1: an allowed edit of an existing
        // file and an allowed `seq 1 4`, with today's `initialize` and once
        // more offering `clientCapabilities.terminal`.
        (
            "antigravity-1.5.16-edit-run",
            include_bytes!("../../tests/fixtures/acp/antigravity-1.5.16-edit-run.log"),
        ),
        (
            "antigravity-1.5.16-edit-run-terminal",
            include_bytes!("../../tests/fixtures/acp/antigravity-1.5.16-edit-run-terminal.log"),
        ),
        (
            "claude-agent-acp-0.112.0-edit-run",
            include_bytes!("../../tests/fixtures/acp/claude-agent-acp-0.112.0-edit-run.log"),
        ),
        (
            "claude-agent-acp-0.112.0-edit-run-terminal",
            include_bytes!(
                "../../tests/fixtures/acp/claude-agent-acp-0.112.0-edit-run-terminal.log"
            ),
        ),
        (
            "codex-acp-2.12.0-edit-run",
            include_bytes!("../../tests/fixtures/acp/codex-acp-2.12.0-edit-run.log"),
        ),
        (
            "codex-acp-2.12.0-edit-run-terminal",
            include_bytes!("../../tests/fixtures/acp/codex-acp-2.12.0-edit-run-terminal.log"),
        ),
    ];

    /// Hand-written lines with values no agent sent yet, for a session
    /// already ready in `sess-1`.
    const HANDWRITTEN: &[u8] =
        include_bytes!("../../tests/fixtures/acp/handwritten-unknown-values.log");

    const GEMINI: &[u8] =
        include_bytes!("../../tests/fixtures/acp/gemini-0.71.1-login-on-stdout.log");

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

    // --- continuing a conversation (agents-session-recovery) -------------

    /// `initialize`'s answer offering `capabilities`.
    fn initialized(id: u64, capabilities: Value) -> String {
        result(
            id,
            json!({"protocolVersion": 1, "agentCapabilities": capabilities,
                   "agentInfo": {"name": "fake", "version": "2.0"}}),
        )
    }

    fn both() -> Value {
        json!({"loadSession": true, "sessionCapabilities": {"resume": {}}})
    }

    fn load_only() -> Value {
        json!({"loadSession": true, "sessionCapabilities": {"list": {}}})
    }

    fn resume_only() -> Value {
        json!({"sessionCapabilities": {"resume": {}}})
    }

    fn conversation_s1(continuation: Continuation) -> Conversation {
        Conversation {
            id: "s1".into(),
            cwd: "/home/u/src".into(),
            continuation,
            title: None,
        }
    }

    impl Agent {
        /// Ready in ACP session `s1`, its agent offering `capabilities`.
        fn ready_offering(capabilities: Value) -> Self {
            let mut agent = Self::new();
            agent.step(Input::Spawned);
            agent.feed(&initialized(1, capabilities));
            agent.feed(&result(2, json!({"sessionId": "s1"})));
            agent
        }

        /// Ready offering both, one turn done ("first" → "Done."), then the
        /// agent exited with status 1.
        fn ended() -> Self {
            let mut agent = Self::ready_offering(both());
            let id = agent.turn("first");
            agent.feed(&said("Done."));
            agent.end(id, "end_turn");
            agent.exit();
            agent
        }

        fn exit(&mut self) -> Output {
            self.step(Input::Exited {
                code: Some(1),
                stderr: b"",
            })
        }

        /// `AGENT_RESTART`, then the holder's `SPAWNED`: returns the
        /// `initialize` id sent.
        fn restart(&mut self) -> u64 {
            assert_eq!(self.step(Input::Restart).reply, Some(Ok(())));
            self.step(Input::Spawned);
            self.last("initialize")["id"].as_u64().expect("an id")
        }

        /// The lines sent from the `n`th on.
        fn sent_since(&self, n: usize) -> &[Value] {
            &self.sent[n..]
        }
    }

    #[test]
    fn the_start_choice_follows_decision_2s_table() {
        let muted = Choice::Load {
            replay: Replay::Muted { updates: 0 },
        };
        let shown = Choice::Load {
            replay: Replay::Shown,
        };
        for (log, offered, choice) in [
            (Log::Kept, Continuation::ResumeAndLoad, Choice::Resume),
            (Log::Kept, Continuation::Resume, Choice::Resume),
            (Log::Kept, Continuation::Load, muted),
            (Log::Kept, Continuation::Neither, Choice::Neither),
            (Log::Fresh, Continuation::ResumeAndLoad, shown),
            (Log::Fresh, Continuation::Load, shown),
            (Log::Fresh, Continuation::Resume, Choice::Resume),
            (Log::Fresh, Continuation::Neither, Choice::Neither),
        ] {
            assert_eq!(choose(log, offered), choice, "{log:?} {offered:?}");
        }
    }

    // 2.1: the conversation is named.

    #[test]
    fn a_new_session_names_its_conversation_before_the_ready_state() {
        let mut agent = Agent::in_dir("/home/u/src");
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        let out = agent.feed(&result(2, json!({"sessionId": "s1"})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        assert_eq!(
            out.events[0],
            Event::Conversation {
                conversation: conversation_s1(Continuation::ResumeAndLoad)
            }
        );
        assert_eq!(out.checkpoint, Some(Moment::Started));
        // What the agent offered is what the conversation says.
        for (capabilities, continuation) in [
            (load_only(), Continuation::Load),
            (resume_only(), Continuation::Resume),
            (json!({}), Continuation::Neither),
        ] {
            let agent = Agent::ready_offering(capabilities);
            assert_eq!(
                agent.s.transcript().conversation(),
                Some(&conversation_s1(continuation))
            );
        }
    }

    #[test]
    fn an_exit_leaves_the_conversation_in_the_log_and_its_replay() {
        let agent = Agent::ended();
        assert!(matches!(agent.agent_state(), AgentState::Exited { .. }));
        let conversation = conversation_s1(Continuation::ResumeAndLoad);
        assert_eq!(agent.s.transcript().conversation(), Some(&conversation));
        assert!(agent.s.transcript().replay().contains(&Event::Conversation { conversation }));
        assert!(agent.s.continuable());
    }

    #[test]
    fn only_an_ended_agent_with_a_conversation_it_offered_to_continue_is_continuable() {
        assert!(!Agent::new().s.continuable(), "no conversation yet");
        assert!(!Agent::ready_offering(both()).s.continuable(), "still running");
        let mut neither = Agent::ready_offering(json!({}));
        neither.exit();
        assert!(!neither.s.continuable(), "neither offered");
        let mut never = Agent::new();
        never.step(Input::Spawned);
        never.feed(&initialized(1, both()));
        never.exit();
        assert!(!never.s.continuable(), "no ACP session was given");
        let mut lost = Agent::ready_offering(load_only());
        lost.step(Input::Lost {
            message: "its holder ended",
        });
        assert!(lost.s.continuable());
    }

    // 2.2: the start choice after the new agent's `initialize`.

    #[test]
    fn a_kept_log_resumes_when_the_new_agent_offers_resume() {
        let mut agent = Agent::ended();
        let before = agent.sent.len();
        let id = agent.restart();
        let out = agent.feed(&initialized(id, both()));
        assert_eq!(
            agent.sent_since(before).last(),
            Some(&json!({"jsonrpc": "2.0", "id": id + 1, "method": "session/resume",
                "params": {"sessionId": "s1", "cwd": "/home/u/src", "mcpServers": []}}))
        );
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        assert!(matches!(agent.s.phase(), Phase::Resuming { log: Log::Kept, .. }));
        let out = agent.feed(&result(id + 1, json!({})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        assert_eq!(out.checkpoint, Some(Moment::Started));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Ready {
                name: Some("fake".into()),
                version: Some("2.0".into())
            }
        );
        assert_eq!(*agent.s.phase(), Phase::Ready { session: "s1".into() });
        let methods: Vec<&str> = agent
            .sent_since(before)
            .iter()
            .filter_map(|line| line["method"].as_str())
            .collect();
        assert_eq!(methods, ["initialize", "session/resume"], "no session/new");
    }

    #[test]
    fn a_kept_log_loads_muted_when_the_new_agent_offers_only_load() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        agent.feed(&initialized(id, load_only()));
        assert_eq!(
            *agent.last("session/load"),
            json!({"jsonrpc": "2.0", "id": id + 1, "method": "session/load",
                "params": {"sessionId": "s1", "cwd": "/home/u/src", "mcpServers": []}})
        );
        assert!(matches!(
            agent.s.phase(),
            Phase::Loading {
                replay: Replay::Muted { updates: 0 },
                ..
            }
        ));
        assert!(!agent.sent_methods().contains(&"session/resume"));
    }

    #[test]
    fn a_fresh_log_loads_and_shows_the_replay_after_its_notice() {
        let mut agent = Agent::in_dir("/ignored");
        agent.s = AgentSession::continuing(
            conversation_s1(Continuation::Load),
            "0.3.0".into(),
        );
        assert_eq!(agent.s.cwd(), "/home/u/src", "the conversation's directory");
        agent.step(Input::Spawned);
        let out = agent.feed(&initialized(1, both()));
        assert_eq!(shape(&out.events), ["added notice"]);
        assert_eq!(agent.notices(), [REPLAY_SHOWN]);
        assert_eq!(agent.last("session/load")["params"]["sessionId"], "s1");
        assert!(!agent.sent_methods().contains(&"session/new"));
    }

    #[test]
    fn a_fresh_log_resumes_with_a_warning_when_the_agent_offers_only_resume() {
        let mut agent = Agent::new();
        agent.s = AgentSession::continuing(conversation_s1(Continuation::Resume), "0.3.0".into());
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, resume_only()));
        assert_eq!(agent.last("session/resume")["params"]["sessionId"], "s1");
        let out = agent.feed(&result(2, json!({})));
        assert_eq!(
            shape(&out.events),
            ["conversation", "added notice", "agent ready"]
        );
        assert!(matches!(
            agent.last_entry().1,
            Entry::Notice { level: NoticeLevel::Warning, text } if text == NOT_REPLAYED
        ));
        assert_eq!(
            agent.s.transcript().conversation(),
            Some(&conversation_s1(Continuation::Resume))
        );
    }

    #[test]
    fn a_new_agent_offering_neither_is_refused_and_sent_nothing_more() {
        let mut agent = Agent::ended();
        let before = agent.sent.len();
        let id = agent.restart();
        let out = agent.feed(&initialized(id, json!({})));
        assert!(out.hang_up);
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: CANT_CONTINUE.into()
            }
        );
        assert_eq!(agent.sent_since(before).len(), 1, "only initialize");
        agent.exit();
        assert!(matches!(agent.agent_state(), AgentState::Failed { .. }));
        assert!(!agent.s.continuable(), "the session ends as after a mismatch");
        assert_eq!(
            agent.step(Input::Restart).reply,
            Some(Err(Refused::CannotContinue))
        );
    }

    // 2.3: the replay, muted or shown.

    #[test]
    fn a_muted_replay_of_7_updates_leaves_the_log_as_it_was_plus_one_notice() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        let kept = agent.entries();
        let next = agent.s.transcript().next_id();
        agent.feed(&initialized(id, load_only()));
        for line in [
            chunk("user_message_chunk", "first"),
            said("Done."),
            chunk("agent_thought_chunk", "hmm"),
            tool_call("call_9", "Edit", "completed"),
            tool_update("call_9", "completed"),
            update(json!({"sessionUpdate": "available_commands_update", "availableCommands": []})),
            // One Brindle can't read still counts.
            update(json!({"sessionUpdate": "agent_message_chunk"})),
        ] {
            assert!(agent.feed(&line).events.is_empty(), "{line}");
        }
        // Another conversation's update isn't counted.
        agent.feed(
            &json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s2",
                "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "x"}}}})
            .to_string(),
        );
        assert_eq!(agent.entries(), kept);
        let out = agent.feed(&result(id + 1, json!(null)));
        assert_eq!(
            shape(&out.events),
            ["conversation", "added notice", "agent ready"]
        );
        let mut expected = kept;
        expected.push(Entry::Notice {
            level: NoticeLevel::Info,
            text: "conversation continued; the agent's replay of 7 updates wasn't shown".into(),
        });
        assert_eq!(agent.entries(), expected);
        assert_eq!(agent.last_entry().0, next);
        assert_eq!(*agent.turn_state(), ended(StopReason::EndTurn), "no turn");
        // A live chunk afterwards is a new entry, not the old run's.
        agent.feed(&said("Back."));
        assert!(matches!(agent.last_entry().1, Entry::Agent { text } if text == "Back."));
    }

    // The title across restarts (agents-session-title decision 3).

    /// [`Agent::ended`], its conversation titled `Old` before the exit.
    fn ended_titled() -> Agent {
        let mut agent = Agent::ready_offering(both());
        let id = agent.turn("first");
        agent.feed(&titled(json!("Old")));
        agent.feed(&said("Done."));
        agent.end(id, "end_turn");
        agent.exit();
        assert_eq!(title_of(&agent), Some("Old"));
        agent
    }

    #[test]
    fn a_resume_of_the_same_session_keeps_its_title() {
        let mut agent = ended_titled();
        let id = agent.restart();
        agent.feed(&initialized(id, both()));
        let out = agent.feed(&result(id + 1, json!({})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        assert!(matches!(
            &out.events[0],
            Event::Conversation { conversation } if conversation.title.as_deref() == Some("Old")
        ));
        assert_eq!(title_of(&agent), Some("Old"));
    }

    #[test]
    fn a_muted_load_applies_a_replayed_title_without_counting_it() {
        let mut agent = ended_titled();
        let id = agent.restart();
        agent.feed(&initialized(id, load_only()));
        assert!(agent.feed(&said("Done.")).events.is_empty());
        let out = agent.feed(&titled(json!("New")));
        assert_eq!(shape(&out.events), ["conversation"]);
        let out = agent.feed(&result(id + 1, json!(null)));
        assert_eq!(
            shape(&out.events),
            ["conversation", "added notice", "agent ready"]
        );
        assert_eq!(
            agent.notices().last().map(String::as_str),
            Some("conversation continued; the agent's replay of 1 update wasn't shown")
        );
        assert_eq!(title_of(&agent), Some("New"));
    }

    #[test]
    fn a_login_restart_of_the_same_session_keeps_its_title() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.feed(&titled(json!("Hello.txt file creation")));
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        assert_eq!(agent.step(Input::Restart).reply, Some(Ok(())));
        let id = agent.restart();
        agent.feed(&initialized(id, resume_only()));
        let resume = agent.last("session/resume")["id"].as_u64().expect("an id");
        agent.feed(&result(resume, json!({})));
        assert!(matches!(agent.s.phase(), Phase::Ready { .. }));
        assert_eq!(title_of(&agent), Some("Hello.txt file creation"));
    }

    #[test]
    fn a_login_by_a_prompts_demand_keeps_the_title() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.feed(&titled(json!("Named")));
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        agent.login("oauth-personal");
        agent.feed(&result(agent.authenticate_id(), json!({})));
        assert!(matches!(agent.s.phase(), Phase::Ready { .. }));
        assert_eq!(title_of(&agent), Some("Named"));
    }

    #[test]
    fn a_new_server_session_starts_untitled_until_the_agent_names_it() {
        let mut agent = Agent::new();
        agent.s = AgentSession::continuing(conversation_s1(Continuation::Load), "0.3.0".into());
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        // The shown replay's title names no conversation the log has yet.
        assert_eq!(agent.feed(&titled(json!("Before"))), Output::default());
        agent.feed(&result(2, json!(null)));
        assert!(matches!(agent.s.phase(), Phase::Ready { .. }));
        assert_eq!(title_of(&agent), None);
        agent.feed(&titled(json!("After")));
        assert_eq!(title_of(&agent), Some("After"));
    }

    #[test]
    fn a_muted_count_of_one_update_is_singular() {
        assert_eq!(
            muted_notice(1),
            "conversation continued; the agent's replay of 1 update wasn't shown"
        );
    }

    #[test]
    fn a_shown_replay_of_a_user_and_an_agent_message_gives_two_entries() {
        let mut agent = Agent::new();
        agent.s = AgentSession::continuing(conversation_s1(Continuation::Load), "0.3.0".into());
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        agent.feed(&chunk("user_message_chunk", "hi"));
        agent.feed(&said("hel"));
        agent.feed(&said("lo"));
        let out = agent.feed(&result(2, json!({})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        assert_eq!(
            agent.entries(),
            [
                Entry::Notice {
                    level: NoticeLevel::Info,
                    text: REPLAY_SHOWN.into()
                },
                Entry::User { text: "hi".into() },
                Entry::Agent {
                    text: "hello".into()
                },
            ]
        );
        assert_eq!(*agent.turn_state(), TurnState::Idle);
        assert_eq!(agent.activity(), Activity::Idle);
        agent.feed(&said("again"));
        assert_eq!(agent.entries().len(), 4, "the replay's run ended");
    }

    #[test]
    fn a_permission_request_during_a_muted_load_is_logged_and_answerable() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        agent.feed(&initialized(id, load_only()));
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        assert!(matches!(
            agent.last_entry().1,
            Entry::Permission { outcome: None, .. }
        ));
        agent.feed(&result(id + 1, json!({})));
        assert_eq!(
            agent
                .cmd(Command::Answer {
                    entry,
                    option: Some("allow-once".into())
                })
                .reply,
            Some(Ok(()))
        );
        assert_eq!(agent.sent.last().map(|line| &line["id"]), Some(&json!(0)));
    }

    // 2.4: the answers to session/resume and session/load.

    #[test]
    fn a_continue_answered_32000_needs_a_login_and_keeps_the_agent() {
        for (capabilities, method, continuation) in [
            (both(), "session/resume", Continuation::ResumeAndLoad),
            (load_only(), "session/load", Continuation::Load),
        ] {
            let mut agent = Agent::ended();
            let id = agent.restart();
            agent.feed(&initialized(id, capabilities));
            let out = agent.feed(&error(id + 1, AUTH_REQUIRED, "log in first"));
            assert!(!out.hang_up);
            assert_eq!(
                *agent.agent_state(),
                AgentState::LoginRequired {
                    message: "log in first".into(),
                    methods: Vec::new(),
                }
            );
            // After a login, the same continue of the same conversation
            // into the kept log.
            let Phase::LoginRequired(login) = agent.s.phase() else {
                panic!("{:?}", agent.s.phase());
            };
            assert_eq!(
                login.then,
                AfterLogin::Start(Start::Continue {
                    conversation: conversation_s1(continuation),
                    log: Log::Kept,
                })
            );
            assert_eq!(agent.last(method)["params"]["sessionId"], json!("s1"));
        }
    }

    #[test]
    fn after_a_login_a_continue_is_sent_again_and_continues() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        agent.feed(&initialized_with_methods(id, both()));
        agent.feed(&error(id + 1, AUTH_REQUIRED, "log in first"));
        assert_eq!(agent.login("oauth-personal").reply, Some(Ok(())));
        let n = agent.sent.len();
        agent.feed(&result(id + 2, json!({})));
        let methods: Vec<&Value> = agent.sent_since(n).iter().map(|l| &l["method"]).collect();
        assert_eq!(methods, [&json!("session/resume")]);
        agent.feed(&result(id + 3, json!({})));
        assert_eq!(
            *agent.s.phase(),
            Phase::Ready {
                session: "s1".into()
            }
        );
        assert_eq!(agent.notices().last().map(String::as_str), Some(RESTARTED));
    }

    #[test]
    fn a_continue_answered_with_another_error_fails_and_hangs_up() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        agent.feed(&initialized(id, both()));
        let out = agent.feed(&error(id + 1, -32603, "no such session"));
        assert!(out.hang_up);
        let failed = AgentState::Failed {
            message: "the agent couldn't continue the conversation: no such session".into(),
        };
        assert_eq!(*agent.agent_state(), failed);
        assert_eq!(out.checkpoint, Some(Moment::Stopped));
        agent.step(Input::Exited {
            code: Some(0),
            stderr: b"bye\n",
        });
        assert_eq!(*agent.agent_state(), failed, "its exit leaves the failure");
        assert!(!agent.s.continuable());
    }

    #[test]
    fn an_unreadable_continue_answer_fails_naming_the_request() {
        for (capabilities, method) in [(both(), "session/resume"), (load_only(), "session/load")] {
            let mut agent = Agent::ended();
            let id = agent.restart();
            agent.feed(&initialized(id, capabilities));
            let out = agent.feed(&result(id + 1, json!("done")));
            assert!(out.hang_up);
            assert_eq!(
                *agent.agent_state(),
                AgentState::Failed {
                    message: format!(
                        "the agent couldn't continue the conversation: its answer to {method} isn't one Brindle can read"
                    )
                }
            );
        }
    }

    #[test]
    fn output_lost_mid_continue_fails_it() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        agent.feed(&initialized(id, both()));
        let out = agent.step(Input::OutputLost);
        assert!(out.hang_up);
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent couldn't continue the conversation: output was lost while the terminal server restarted".into()
            }
        );
    }

    // 2.5: restart and a lost holder.

    #[test]
    fn a_restart_adds_its_notice_starts_the_agent_and_asks_for_no_checkpoint() {
        let mut agent = Agent::ended();
        let out = agent.step(Input::Restart);
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["added notice", "agent starting"]);
        assert_eq!(agent.notices().last().map(String::as_str), Some(RESTARTED));
        assert_eq!(out.checkpoint, None);
        assert!(out.stdin.is_empty() && !out.hang_up);
        assert!(matches!(
            agent.s.phase(),
            Phase::Spawning {
                start: Start::Continue { log: Log::Kept, .. }
            }
        ));
        assert_eq!(agent.s.lines().covers(), 0, "the new holder session's");
        let out = agent.step(Input::Spawned);
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
    }

    #[test]
    fn ids_continue_after_a_restart() {
        let mut agent = Agent::ended();
        let next_request = agent.s.rpc().next_id();
        let last_entry = agent.last_entry().0;
        let id = agent.restart();
        assert_eq!(id, next_request, "request ids go on");
        agent.feed(&initialized(id, both()));
        agent.feed(&result(id + 1, json!({})));
        agent.turn("again");
        let (user, entry) = agent
            .s
            .transcript()
            .entries()
            .find(|(_, entry)| matches!(entry, Entry::User { text } if text == "again"))
            .map(|(id, entry)| (id, entry.clone()))
            .expect("the prompt's entry");
        assert!(matches!(entry, Entry::User { .. }));
        // The restart's notice took the next id, the prompt the one after.
        assert_eq!(user, EntryId(last_entry.0 + 2));
        assert_eq!(agent.prompt_id(), id + 2);
    }

    #[test]
    fn the_turn_in_flight_at_an_exit_fails_and_is_never_sent_to_the_new_agent() {
        let mut agent = Agent::ready_offering(both());
        agent.turn("fix it");
        agent.exit();
        assert_eq!(
            *agent.turn_state(),
            TurnState::Failed {
                message: "the agent exited".into()
            }
        );
        let before = agent.sent.len();
        let id = agent.restart();
        agent.feed(&initialized(id, both()));
        let out = agent.feed(&result(id + 1, json!({})));
        assert!(out.stdin.is_empty(), "nothing sent on its own");
        let after = serde_json::to_string(agent.sent_since(before)).expect("JSON");
        assert!(!after.contains("fix it"), "{after}");
        assert!(matches!(agent.turn_state(), TurnState::Failed { .. }));
    }

    #[test]
    fn a_held_prompt_can_be_sent_again_once_the_continued_agent_is_ready_and_not_before() {
        let mut agent = Agent::ready_offering(both());
        agent.turn("first");
        agent.prompt("next");
        agent.exit();
        assert_eq!(*agent.slot(), held("next", HoldReason::AgentStopped));
        let before = agent.sent.len();
        let id = agent.restart();
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::NotReady))
        );
        agent.feed(&initialized(id, both()));
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::NotReady))
        );
        assert_eq!(*agent.slot(), held("next", HoldReason::AgentStopped));
        let out = agent.feed(&result(id + 1, json!({})));
        assert!(out.stdin.is_empty(), "not sent on its own");
        let methods: Vec<&str> = agent
            .sent_since(before)
            .iter()
            .filter_map(|line| line["method"].as_str())
            .collect();
        assert_eq!(methods, ["initialize", "session/resume"]);
        assert_eq!(agent.cmd(Command::Resend).reply, Some(Ok(())));
        assert_eq!(agent.last("session/prompt")["params"]["prompt"][0]["text"], "next");
        assert_eq!(*agent.slot(), Slot::Empty);
    }

    #[test]
    fn a_permission_cancelled_at_the_exit_stays_cancelled_and_is_never_answered_again() {
        let mut agent = Agent::ready_offering(both());
        agent.turn("first");
        let entry = agent.ask(json!(5), &ALLOW_REJECT);
        agent.exit();
        let cancelled = |agent: &Agent| {
            matches!(
                agent.s.transcript().entry(entry),
                Some(Entry::Permission {
                    outcome: Some(PermissionOutcome::Cancelled),
                    ..
                })
            )
        };
        assert!(cancelled(&agent));
        let before = agent.sent.len();
        let id = agent.restart();
        agent.feed(&initialized(id, both()));
        agent.feed(&result(id + 1, json!({})));
        assert!(cancelled(&agent));
        assert!(agent.s.rpc().permissions().is_empty());
        assert!(
            agent.sent_since(before).iter().all(|line| line.get("result").is_none()),
            "no answer to the old request"
        );
        assert_eq!(
            agent
                .cmd(Command::Answer {
                    entry,
                    option: Some("allow-once".into())
                })
                .reply,
            Some(Err(Refused::AlreadyAnswered { entry }))
        );
    }

    #[test]
    fn a_restart_is_refused_unless_the_agent_ended_with_a_conversation_to_continue() {
        let mut running = Agent::ready_offering(both());
        let out = running.step(Input::Restart);
        assert_eq!(out.reply, Some(Err(Refused::NotEnded)));
        assert!(out.events.is_empty() && out.stdin.is_empty());
        let mut starting = Agent::ended();
        starting.restart();
        assert_eq!(
            starting.step(Input::Restart).reply,
            Some(Err(Refused::NotEnded))
        );
        let mut neither = Agent::ready_offering(json!({}));
        neither.exit();
        assert_eq!(
            neither.step(Input::Restart).reply,
            Some(Err(Refused::CannotContinue))
        );
        let mut refused = Agent::new();
        refused.step(Input::Spawned);
        refused.feed(&result(1, json!({"protocolVersion": 2})));
        refused.exit();
        assert_eq!(
            refused.step(Input::Restart).reply,
            Some(Err(Refused::CannotContinue))
        );
        for (refusal, text) in [
            (Refused::NotEnded, "the agent hasn't ended"),
            (
                Refused::CannotContinue,
                "the agent can't continue this conversation",
            ),
            (Refused::Lost, "the agent was lost with its holder"),
        ] {
            assert_eq!(refusal.to_string(), text);
        }
    }

    #[test]
    fn a_restart_that_couldnt_be_placed_returns_to_ended_and_can_be_tried_again() {
        let mut agent = Agent::ended();
        assert_eq!(agent.step(Input::Restart).reply, Some(Ok(())));
        agent.step(Input::Exited {
            code: None,
            stderr: b"no such file: fake-agent",
        });
        assert_eq!(
            *agent.agent_state(),
            AgentState::Exited {
                code: None,
                stderr: "no such file: fake-agent".into()
            }
        );
        assert!(agent.s.continuable());
        agent.restart();
    }

    #[test]
    fn a_lost_holder_ends_the_agent_as_an_exit_does_but_lost() {
        let mut agent = Agent::ready_offering(load_only());
        agent.turn("first");
        let entry = agent.ask(json!(1), &ALLOW_REJECT);
        agent.prompt("next");
        let out = agent.step(Input::Lost {
            message: "its holder ended",
        });
        assert_eq!(
            shape(&out.events),
            ["answered", "queue held", "turn failed", "agent lost"]
        );
        assert!(matches!(
            agent.s.transcript().entry(entry),
            Some(Entry::Permission {
                outcome: Some(PermissionOutcome::Cancelled),
                ..
            })
        ));
        assert_eq!(*agent.slot(), held("next", HoldReason::AgentStopped));
        assert_eq!(
            *agent.turn_state(),
            TurnState::Failed {
                message: "the agent exited".into()
            }
        );
        assert_eq!(
            *agent.agent_state(),
            AgentState::Lost {
                message: "its holder ended".into()
            }
        );
        assert_eq!(*agent.s.phase(), Phase::Lost);
        assert_eq!(out.checkpoint, Some(Moment::Stopped));
        assert_eq!(agent.activity(), Activity::Error);
        assert_eq!(agent.prompt("x").reply, Some(Err(Refused::Lost)));
        // Nothing more ends it.
        assert!(agent.exit().events.is_empty());
        assert!(agent
            .step(Input::Lost {
                message: "again"
            })
            .events
            .is_empty());
        let id = agent.restart();
        agent.feed(&initialized(id, load_only()));
        assert_eq!(agent.last("session/load")["params"]["sessionId"], "s1");
    }

    // agents-login 3.3: logging in.

    /// Two agent methods and a terminal one.
    fn auth_methods() -> Value {
        json!([
            {"id": "oauth-personal", "name": "Log in with Google", "description": "Use your\u{1b}[31m Google account"},
            {"id": "api-key", "name": "API Key"},
            {"id": "cli", "name": "CLI", "type": "terminal", "args": ["--login"], "env": {"MODE": "device"}},
        ])
    }

    fn methods() -> Vec<LoginMethod> {
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

    fn initialized_with_methods(id: u64, capabilities: Value) -> String {
        result(
            id,
            json!({"protocolVersion": 1, "agentCapabilities": capabilities,
                   "agentInfo": {"name": "fake", "version": "2.0"}, "authMethods": auth_methods()}),
        )
    }

    fn login_required(message: &str) -> AgentState {
        AgentState::LoginRequired {
            message: message.into(),
            methods: methods(),
        }
    }

    fn logging_in() -> AgentState {
        AgentState::LoggingIn {
            method: "oauth-personal".into(),
            name: "Log in with Google".into(),
        }
    }

    impl Agent {
        /// `initialize` (id 1) answered with [`auth_methods`] and
        /// `session/new` (id 2) with `-32000`.
        fn needing_login() -> Self {
            let mut agent = Self::new();
            agent.step(Input::Spawned);
            agent.feed(&initialized_with_methods(1, both()));
            agent.feed(&error(2, AUTH_REQUIRED, "Authentication required"));
            agent
        }

        /// Ready in `s1`, its agent having advertised [`auth_methods`].
        fn ready_with_methods() -> Self {
            let mut agent = Self::new();
            agent.step(Input::Spawned);
            agent.feed(&initialized_with_methods(1, both()));
            agent.feed(&result(2, json!({"sessionId": "s1"})));
            agent
        }

        fn login(&mut self, method: &str) -> Output {
            self.cmd(Command::Login {
                method: method.into(),
            })
        }

        /// The id of the last `authenticate` written.
        fn authenticate_id(&self) -> u64 {
            self.last("authenticate")["id"].as_u64().expect("an id")
        }
    }

    #[test]
    fn the_methods_are_kept_from_initialize_cleaned_in_the_agents_order() {
        let agent = Agent::needing_login();
        assert_eq!(agent.s.methods, methods());
        assert!(agent.notices().is_empty(), "{:?}", agent.notices());
    }

    #[test]
    fn methods_left_out_or_cut_add_one_warning() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let long = "d".repeat(wire::MAX_LOGIN_DESCRIPTION + 1);
        let out = agent.feed(&result(
            1,
            json!({"protocolVersion": 1, "authMethods": [
                {"id": "a", "name": "A", "description": long},
                {"id": "env", "name": "Env", "type": "env_var"},
                {"name": "no id"},
            ]}),
        ));
        assert_eq!(
            agent.notices(),
            ["2 login methods from the agent were left out, and 1 login method's description was cut"]
        );
        assert!(matches!(
            out.events.last(),
            Some(Event::Added { entry: Entry::Notice { level: NoticeLevel::Warning, .. }, .. })
        ));
        assert_eq!(agent.s.methods.len(), 1);
        assert_eq!(methods_notice(0, 0), None);
        assert_eq!(
            methods_notice(1, 0).as_deref(),
            Some("1 login method from the agent was left out")
        );
        assert_eq!(
            methods_notice(0, 2).as_deref(),
            Some("2 login methods' descriptions were cut")
        );
    }

    #[test]
    fn session_new_answered_32000_needs_a_login_offering_the_methods() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized_with_methods(1, both()));
        let out = agent.feed(&error(2, AUTH_REQUIRED, "Authentication required"));
        assert_eq!(*agent.agent_state(), login_required("Authentication required"));
        assert_eq!(out.checkpoint, Some(Moment::Started));
        assert!(!out.hang_up && out.stdin.is_empty());
        assert_eq!(
            *agent.s.phase(),
            Phase::LoginRequired(Login {
                greeting: Greeting {
                    name: Some("fake".into()),
                    version: Some("2.0".into()),
                    continuation: Continuation::ResumeAndLoad,
                },
                message: "Authentication required".into(),
                then: AfterLogin::Start(Start::New),
            })
        );
    }

    #[test]
    fn a_prompt_answered_32000_needs_a_login_in_its_session_offering_the_methods() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.prompt("two");
        let out = agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        assert_eq!(
            shape(&out.events),
            ["queue held", "turn failed", "agent login required"]
        );
        assert_eq!(*agent.agent_state(), login_required("expired"));
        assert_eq!(out.checkpoint, Some(Moment::TurnEnded));
        let Phase::LoginRequired(login) = agent.s.phase() else {
            panic!("{:?}", agent.s.phase());
        };
        assert_eq!(
            login.then,
            AfterLogin::Ready {
                session: "s1".into()
            }
        );
        assert_eq!(login.greeting.name.as_deref(), Some("fake"));
        assert_eq!(login.greeting.continuation, Continuation::ResumeAndLoad);
    }

    #[test]
    fn a_login_with_an_agent_method_sends_authenticate_and_is_logging_in() {
        let mut agent = Agent::needing_login();
        let out = agent.login("oauth-personal");
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["agent logging in"]);
        assert_eq!(*agent.agent_state(), logging_in());
        assert_eq!(agent.activity(), Activity::Working);
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        assert_eq!(
            String::from_utf8_lossy(&out.stdin),
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"authenticate\",\"params\":{\"methodId\":\"oauth-personal\"}}\n"
        );
        assert!(matches!(
            agent.s.phase(),
            Phase::LoggingIn { method, .. } if method == "oauth-personal"
        ));
        assert_eq!(agent.s.rpc().awaited(Purpose::Authenticate), Some(3));
    }

    #[test]
    fn a_login_with_a_terminal_or_unknown_method_is_refused_and_changes_nothing() {
        for (method, refused) in [
            (
                "cli",
                Refused::RunByClient {
                    method: "cli".into(),
                },
            ),
            (
                "nope",
                Refused::NoSuchMethod {
                    method: "nope".into(),
                },
            ),
        ] {
            let mut agent = Agent::needing_login();
            let before = agent.s.clone();
            let out = agent.login(method);
            assert_eq!(out.reply, Some(Err(refused)));
            assert!(out.events.is_empty() && out.stdin.is_empty());
            assert_eq!(out.checkpoint, None);
            assert_eq!(agent.s, before);
        }
    }

    #[test]
    fn a_login_while_none_is_needed_or_one_runs_is_refused() {
        let mut ready = Agent::ready_with_methods();
        assert_eq!(
            ready.login("oauth-personal").reply,
            Some(Err(Refused::NoLoginNeeded))
        );
        assert!(!ready.sent_methods().contains(&"authenticate"));
        let mut starting = Agent::new();
        assert_eq!(
            starting.login("oauth-personal").reply,
            Some(Err(Refused::NoLoginNeeded))
        );
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.login("api-key");
        assert_eq!(out.reply, Some(Err(Refused::AlreadyLoggingIn)));
        assert!(out.stdin.is_empty());
    }

    #[test]
    fn a_login_at_the_start_answered_starts_the_session() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.feed(&result(3, json!({})));
        assert_eq!(shape(&out.events), ["agent starting"]);
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        assert_eq!(agent.last("session/new")["id"], json!(4));
        assert!(matches!(agent.s.phase(), Phase::Creating { .. }));
        agent.feed(&result(4, json!({"sessionId": "s1"})));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Ready {
                name: Some("fake".into()),
                version: Some("2.0".into())
            }
        );
        assert_eq!(agent.turn("hi"), 5);
    }

    #[test]
    fn session_new_answering_32000_again_after_a_login_needs_one_again() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        agent.feed(&result(3, json!(null)));
        let out = agent.feed(&error(4, AUTH_REQUIRED, "still not"));
        assert_eq!(*agent.agent_state(), login_required("still not"));
        assert_eq!(out.checkpoint, Some(Moment::Started));
        assert!(matches!(
            agent.s.phase(),
            Phase::LoginRequired(Login { then: AfterLogin::Start(Start::New), .. })
        ));
        assert_eq!(agent.login("api-key").reply, Some(Ok(())));
    }

    #[test]
    fn a_login_after_a_prompts_demand_is_ready_in_the_same_session_and_sends_nothing() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        agent.login("api-key");
        let auth = agent.authenticate_id();
        let n = agent.sent.len();
        let out = agent.feed(&result(auth, json!({})));
        assert_eq!(shape(&out.events), ["agent starting", "agent ready"]);
        assert_eq!(out.checkpoint, Some(Moment::Login));
        assert!(out.stdin.is_empty());
        assert_eq!(agent.sent.len(), n);
        assert_eq!(
            *agent.s.phase(),
            Phase::Ready {
                session: "s1".into()
            }
        );
        assert_eq!(
            *agent.agent_state(),
            AgentState::Ready {
                name: Some("fake".into()),
                version: Some("2.0".into())
            }
        );
        // The next prompt goes to the same session.
        agent.turn("two");
        assert_eq!(agent.last("session/prompt")["params"]["sessionId"], json!("s1"));
    }

    /// `agents-login` decision 5: neither the failed prompt nor the held
    /// one is sent on its own; the held one goes again only by a resend,
    /// refused while logging in and taken once ready.
    #[test]
    fn a_failed_prompt_is_never_sent_again_and_a_held_one_only_by_a_resend_once_ready() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::LoginRequired))
        );
        agent.login("oauth-personal");
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::LoggingIn))
        );
        assert_eq!(agent.prompt("three").reply, Some(Err(Refused::LoggingIn)));
        let auth = agent.authenticate_id();
        agent.feed(&result(auth, json!({})));
        let prompts = |agent: &Agent| {
            agent
                .sent
                .iter()
                .filter(|line| line["method"] == "session/prompt")
                .map(|line| line["params"]["prompt"][0]["text"].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(prompts(&agent), [json!("one")]);
        assert_eq!(*agent.slot(), held("two", HoldReason::LoginRequired));
        assert_eq!(agent.cmd(Command::Resend).reply, Some(Ok(())));
        assert_eq!(prompts(&agent), [json!("one"), json!("two")]);
    }

    #[test]
    fn while_a_login_is_needed_or_runs_unqueue_still_discards() {
        for running in [false, true] {
            let mut agent = Agent::ready_with_methods();
            let id = agent.turn("one");
            agent.prompt("two");
            agent.feed(&error(id, AUTH_REQUIRED, "expired"));
            if running {
                agent.login("oauth-personal");
            }
            assert_eq!(agent.cmd(Command::Unqueue).reply, Some(Ok(())));
            assert_eq!(*agent.slot(), Slot::Empty);
        }
    }

    #[test]
    fn a_failed_login_needs_one_again_with_its_message() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.feed(&error(3, -32603, "the browser was closed"));
        assert_eq!(*agent.agent_state(), login_required("the browser was closed"));
        assert_eq!(out.checkpoint, Some(Moment::Login));
        assert!(out.stdin.is_empty() && !out.hang_up);
        // `-32000` from authenticate itself is just an error.
        agent.login("api-key");
        agent.feed(&error(4, AUTH_REQUIRED, "bad key"));
        assert_eq!(*agent.agent_state(), login_required("bad key"));
        assert!(matches!(
            agent.s.phase(),
            Phase::LoginRequired(Login { then: AfterLogin::Start(Start::New), .. })
        ));
    }

    #[test]
    fn a_cancelled_login_sends_cancel_request_and_ignores_the_late_answer() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.cmd(Command::Cancel);
        assert_eq!(out.reply, None);
        assert_eq!(
            String::from_utf8_lossy(&out.stdin),
            "{\"jsonrpc\":\"2.0\",\"method\":\"$/cancel_request\",\"params\":{\"requestId\":3}}\n"
        );
        assert_eq!(*agent.agent_state(), login_required("Authentication required"));
        assert_eq!(out.checkpoint, Some(Moment::Login));
        assert_eq!(agent.s.rpc().awaited(Purpose::Authenticate), None);
        // The agent's late answer, a success or an error, is ignored.
        for late in [result(3, json!({})), error(3, -32800, "cancelled")] {
            let out = agent.feed(&late);
            assert!(out.events.is_empty() && out.stdin.is_empty(), "{out:?}");
        }
        // No session/new beyond the one answered -32000.
        let news = agent.sent_methods().iter().filter(|m| **m == "session/new").count();
        assert_eq!(news, 1);
        // And a new login can be chosen.
        assert_eq!(agent.login("api-key").reply, Some(Ok(())));
        assert_eq!(agent.authenticate_id(), 4);
    }

    #[test]
    fn a_cancel_while_a_login_is_needed_does_nothing() {
        let mut agent = Agent::needing_login();
        let out = agent.cmd(Command::Cancel);
        assert_eq!(out, Output::default());
    }

    #[test]
    fn the_agent_exiting_while_logging_in_exits_and_forgets_the_login() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        });
        assert_eq!(shape(&out.events), ["agent exited"]);
        assert_eq!(*agent.s.phase(), Phase::Exited);
        assert_eq!(agent.s.rpc().awaited(Purpose::Authenticate), None);
        assert_eq!(
            agent.login("oauth-personal").reply,
            Some(Err(Refused::NoLoginNeeded))
        );
    }

    #[test]
    fn output_lost_while_logging_in_needs_the_login_again() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.step(Input::OutputLost);
        assert_eq!(
            *agent.agent_state(),
            login_required("the login's answer was lost while the terminal server restarted")
        );
        assert_eq!(out.checkpoint, Some(Moment::Login));
        assert_eq!(agent.s.rpc().awaited(Purpose::Authenticate), None);
        assert!(matches!(
            agent.s.phase(),
            Phase::LoginRequired(Login { then: AfterLogin::Start(Start::New), .. })
        ));
        // The answer, should it still come, matches nothing.
        assert!(agent.feed(&result(3, json!({}))).events.is_empty());
        // Lost output while a login is only needed leaves it needed.
        let mut needed = Agent::needing_login();
        needed.step(Input::OutputLost);
        assert_eq!(*needed.agent_state(), login_required("Authentication required"));
    }

    #[test]
    fn the_login_refusals_say_why() {
        for (refused, text) in [
            (Refused::LoginRequired, "the agent needs a login"),
            (Refused::NoLoginNeeded, "the agent isn't waiting for a login"),
            (Refused::AlreadyLoggingIn, "a login is already running"),
            (
                Refused::NoSuchMethod {
                    method: "x".into(),
                },
                "the agent offers no login method \"x\"",
            ),
            (
                Refused::RunByClient {
                    method: "x".into(),
                },
                "login method \"x\" runs in a terminal, which Brindle opens",
            ),
            (Refused::LoggingIn, "the agent is logging in"),
        ] {
            assert_eq!(refused.to_string(), text);
        }
    }

    // agents-login 5.3: a restart while a login is needed.

    #[test]
    fn a_restart_at_a_starts_login_demand_hangs_up_then_sends_session_new_again() {
        let mut agent = Agent::needing_login();
        let out = agent.step(Input::Restart);
        assert_eq!(out.reply, Some(Ok(())));
        assert!(out.hang_up, "the running agent is hung up");
        assert!(out.stdin.is_empty());
        assert_eq!(out.checkpoint, None);
        assert_eq!(shape(&out.events), ["added notice", "agent starting"]);
        assert_eq!(
            agent.notices().last().map(String::as_str),
            Some(RESTARTED_AFTER_LOGIN)
        );
        assert_eq!(*agent.s.phase(), Phase::Restarting { start: Start::New });
        assert!(!agent.s.continuable());
        // Nothing is read from the agent being hung up, and nothing taken.
        let n = agent.events.len();
        agent.feed(&said("late words"));
        assert_eq!(agent.events.len(), n);
        assert_eq!(agent.prompt("hi").reply, Some(Err(Refused::NotReady)));
        assert_eq!(
            agent.login("oauth-personal").reply,
            Some(Err(Refused::NoLoginNeeded))
        );

        // Its exit: the core steps the restart again, and the new agent
        // starts as a new conversation, the request ids going on.
        let out = agent.step(Input::Restart);
        assert_eq!(out.reply, Some(Ok(())));
        assert!(out.events.is_empty() && !out.hang_up);
        assert_eq!(*agent.s.phase(), Phase::Spawning { start: Start::New });
        agent.step(Input::Spawned);
        let init = agent.last("initialize");
        assert_eq!(init["id"], json!(3));
        assert_eq!(
            init["params"]["clientCapabilities"],
            json!({"auth": {"terminal": true}})
        );
        agent.feed(&initialized_with_methods(3, both()));
        assert_eq!(agent.last("session/new")["id"], json!(4));
        let out = agent.feed(&result(4, json!({"sessionId": "s2"})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        assert_eq!(
            *agent.s.phase(),
            Phase::Ready {
                session: "s2".into()
            }
        );
    }

    #[test]
    fn a_restart_at_a_prompts_login_demand_takes_up_its_session_keeping_the_log() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        let log = agent.entries().len();
        assert_eq!(agent.step(Input::Restart).reply, Some(Ok(())));
        assert_eq!(
            *agent.s.phase(),
            Phase::Restarting {
                start: Start::Continue {
                    conversation: conversation_s1(Continuation::ResumeAndLoad),
                    log: Log::Kept,
                }
            }
        );
        let id = agent.restart();
        agent.feed(&initialized(id, resume_only()));
        let resume = agent.last("session/resume");
        assert_eq!(resume["params"]["sessionId"], json!("s1"));
        assert_eq!(resume["params"]["cwd"], json!("/home/u/src"));
        let out = agent.feed(&result(resume["id"].as_u64().expect("an id"), json!({})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        // The log (with its notice) and the held prompt are kept, and the
        // held prompt goes again only by a resend.
        assert_eq!(agent.entries().len(), log + 1);
        assert_eq!(
            *agent.s.transcript().slot(),
            Slot::Held {
                text: "two".into(),
                reason: HoldReason::LoginRequired
            }
        );
        assert_eq!(agent.cmd(Command::Resend).reply, Some(Ok(())));
        assert_eq!(agent.last("session/prompt")["params"]["sessionId"], json!("s1"));
    }

    #[test]
    fn a_restart_at_a_prompts_demand_is_refused_when_the_agent_cant_continue() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized_with_methods(1, json!({})));
        agent.feed(&result(2, json!({"sessionId": "s1"})));
        let id = agent.turn("one");
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        let phase = agent.s.phase().clone();
        let out = agent.step(Input::Restart);
        assert_eq!(out.reply, Some(Err(Refused::CannotContinue)));
        assert!(!out.hang_up && out.events.is_empty());
        assert_eq!(*agent.s.phase(), phase);
    }

    #[test]
    fn a_restart_while_logging_in_or_ready_is_refused() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        assert_eq!(
            agent.step(Input::Restart).reply,
            Some(Err(Refused::NotEnded))
        );
        let mut ready = Agent::ready_with_methods();
        assert_eq!(
            ready.step(Input::Restart).reply,
            Some(Err(Refused::NotEnded))
        );
    }

    /// A restart dropped before the agent's exit (the session closed, its
    /// client gone): the exit is stepped, and the agent ends as any does.
    #[test]
    fn an_exit_stepped_while_restarting_ends_the_agent() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        agent.step(Input::Restart);
        agent.exit();
        assert_eq!(*agent.s.phase(), Phase::Exited);
        assert!(matches!(
            agent.agent_state(),
            AgentState::Exited { code: Some(1), .. }
        ));
        assert!(agent.s.continuable(), "its conversation can be continued");
    }
}
