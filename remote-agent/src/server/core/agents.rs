//! The core's agent sessions (OpenSpec change `agents-sessions`, design
//! decisions 4, 9, 10, 11, 13 and 15): an ACP agent run by a holder on
//! plain pipes, whose ACP client the core is. The session's state is an
//! [`AgentSession`] from `acp::reduce`; this shell only gathers what
//! happens to it, calls [`reduce::step`] and applies the [`Output`]
//! (`fcis R1`, `R9`). Everything else (ownership, close policies,
//! hang-ups, layouts, replays' ordering) is the terminal sessions' code in
//! `sessions.rs`, which this module's [`Agent`] sits in as the session's
//! [`Content::Agent`](super::sessions::Content).
//!
//! - **Starting:** `SPAWN_AGENT` (client protocol 4 only) is refused with
//!   an empty command; its directory is resolved to the absolute one the
//!   agent's `session/new` gets ([`platform::agent_directory`]), which is
//!   the spawn's and the listed `cwd`; it goes to a holder that speaks H1
//!   3 as `SPAWN_PIPES`. Its `SPAWNED` reaches the client before any of
//!   its events; then `initialize` goes out ([`Input::Spawned`]).
//! - **Stepping:** the agent's stdout, the holder's exit and stderr tail,
//!   the attached client's commands and the reducer's own wake-ups
//!   ([`Output::wake`]) each step the reducer, under `catch_unwind`: a
//!   panic is that session's ACP handling failing ([`Input::Fault`]), and
//!   the agent runs on until the session is closed (`process R3`).
//!   `BRINDLE_TEST_ACP_PANIC` (debug builds) injects one.
//! - **Applying:** the events go to the attached client as `TRANSCRIPT`
//!   (unless it waits for its replay, which has them), then a checkpoint
//!   when [`policy::agent_checkpoint_due`] says so, then the bytes for the
//!   agent's stdin as `INPUT`, then the hang-up: so a prompt's checkpoint
//!   is in the holder's connection before its bytes. Each event is
//!   encoded once ([`encoded_events`]) and packed as those bytes
//!   ([`client::transcript_frames`]), then moved into `unsaved`.
//! - **Checkpoints** are `acp::checkpoint`'s encoding; `covers` is the
//!   stdout fed since the last one ([`crate::acp::lines::Lines::covers`]).
//!   With change `agents-log-paging` (decision 9) each [`Agent`] keeps
//!   what its holder stores ([`Stored`]) and the events applied since its
//!   last checkpoint (`unsaved`), and [`policy::agent_checkpoint_form`]
//!   says whether the next is whole (`CHECKPOINT_BEGIN`) or a segment
//!   appended to what is stored (`CHECKPOINT_APPEND`). A segment carries
//!   deltas (change `agents-checkpoint-deltas`): those events, the partial
//!   line's bytes since the last record, and the login methods and open
//!   outputs only where they changed, against the `Written` that
//!   [`Stored`] keeps. Either is encoded into one buffer and queued to
//!   the holder from slices of it ([`h1::checkpoint_frames`]). What is
//!   stored starts again as unknown, so the next is whole, on an
//!   adoption, a restart and any holder `ERROR` naming the session.
//! - **Pages** (decision 5): a replay sends the newest page of the log
//!   ([`Agent::replay`]), and the attached client's `AGENT_FETCH`es get
//!   the entries before it as `AGENT_PAGE` frames
//!   ([`Core::client_agent_fetch`]), refused as
//!   [`policy::agent_fetch_refusal`] says. A fetch reads the core's log,
//!   so it is answered whatever the agent's phase.
//! - **Adopting:** the holder's checkpoint is kept until its tail comes;
//!   then [`checkpoint::adopt`] rebuilds the session and the tail is fed
//!   as stdout. A session that exited while no core ran gets its exit once
//!   its stderr tail follows the tail.
//! - **Ending** (change `agents-session-recovery`, decisions 5 and 6): an
//!   agent that exits, or whose holder is lost, steps `Input::Exited` or
//!   `Input::Lost`; when it can continue its conversation
//!   ([`Agent::continuable`]) and a client has the session or may return,
//!   [`policy::end_fate`] keeps it as `Life::AgentEnded`, in this core's
//!   memory only (no program, no holder, no `EXIT`). A `DETACH`, `CLOSE`,
//!   `KILL` or `STOP` ends it (`EXIT { code, lost: None }`), as does its
//!   client's leaving unless it lingers ([`policy::ended_gone_fate`]).
//! - **Continuing:** `AGENT_RESTART` from the attached client
//!   ([`Core::client_agent_restart`]), refused as
//!   [`policy::restart_refusal`] says or while the core stops, steps
//!   `Input::Restart`, updates the listed command and makes the session
//!   `Life::Restarting` (change `agents-process-lifecycle`, decision 4):
//!   waiting for a holder, then placed with a `SPAWN_PIPES` under the same
//!   session id in the conversation's `cwd`, on any H1 3 holder of the
//!   core's build, answered `OK` at once and never `SPAWNED`. A
//!   `SPAWN_FAILED` steps `Exited { None, message }` and returns it to
//!   ended; a restart still waiting abandoned by a hang-up (`CLOSE`,
//!   `KILL`, `STOP`, a takeover, a gone `kill` client) ends the session.
//!   A holder lost before its `SPAWNED` places it again while its client
//!   has it ([`policy::lost_fate`]). A `SPAWN_AGENT` naming a
//!   `conversation` starts as [`Agent::continuing`] in its `cwd`. Either
//!   is checkpointed right after its `SPAWN_PIPES`, so a core killed
//!   before `SPAWNED` still continues it. The reducer chooses resume or
//!   load and mutes or shows the replay.
//! - **Logging in** (change `agents-login`): `AGENT_LOGIN` steps
//!   `Command::Login`, and `AGENT_CANCEL` cancels a running login as the
//!   reducer says. An `AGENT_RESTART` is also taken while the running
//!   agent needs a login (after a terminal login, decision 8): the reducer
//!   hangs it up (`Phase::Restarting`), the restart is kept on the
//!   [`Agent`] until the holder's `EXITED`, and that exit steps
//!   `Input::Restart` again and queues the restart as an ended agent's,
//!   with no `Exited` in the log and no `EXIT`
//!   ([`Core::restart_after_login`]). Any other hang-up (`CLOSE`, `KILL`,
//!   `STOP`, a gone `kill` client) drops the kept restart, as does an exit
//!   with no client attached, and the exit then ends the agent as any.
//! - **Listing:** [`Agent::info`] maps the log's [`Activity`] onto the
//!   wire's [`AgentActivity`] with an exhaustive `match` ([`activity`]).
//! - **Choosing** (change `agents-session-list`): a `SPAWN_AGENT` with
//!   `start: "choose"` starts as [`Agent::choosing`], whose agent waits
//!   after `initialize`. `AGENT_LIST` steps `Command::List`: nothing is
//!   sent at once, and the step whose [`Output::listed`] answers it sends
//!   `AGENT_CONVERSATIONS` or the `ERROR` to the attached client.
//!   `AGENT_CHOOSE` steps `Command::Choose`, answered `OK` or `ERROR` as
//!   any command, and so does `AGENT_SET_OPTION`'s `Command::SetOption`
//!   (OpenSpec change `agents-commands`): `OK` means the set was sent (or
//!   asked for the current value), and the agent's answer reaches every
//!   attached client as an `Options` event or a notice.

use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::OnceLock;
use std::time::Instant;

use brindle_remote_protocol::client::{self, AgentActivity, AgentInfo, Exit, JsonObject, ToClient};
use brindle_remote_protocol::holder::{self as h1, ToHolder};
use brindle_remote_protocol::serde_json;
use brindle_remote_protocol::{RequestId, SessionId};
use brindle_transcript::{
    Activity, AgentState, Continuation, Conversation, EntryId, Event, Fetched,
};

use super::clients::error;
use super::policy::{self, AgentCheckpoint, CheckpointForm, FetchView, Stored};
use super::sessions::{Life, PendingReplay, Restart, RestartStage, SpawnRequest, command_line};
use super::{Core, log};
use crate::acp::checkpoint;
use crate::acp::lines::Partial;
use crate::acp::reduce::{self, AgentSession, Command, Input, ListAnswer, Output, Refused, Store};
use crate::server::event_loop::{EventLoop, Token};
use crate::server::fault;
use crate::server::platform;

/// The version an agent's `initialize` names as the client's: the core's.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A due agent checkpoint, encoded as the holder is to store it.
enum ToStore {
    /// Replacing what is stored.
    Whole(checkpoint::Encoded),
    /// Appended to it.
    Append(checkpoint::Segment),
}

/// Where an agent session's ACP state is.
enum Acp {
    /// Adopted from a holder, whose checkpoint came (`checkpoint`, empty
    /// when the core never stored one) and whose tail hasn't yet.
    Adopting { checkpoint: Vec<u8> },
    /// The conversation, stepped by [`reduce::step`].
    Running(Box<AgentSession>),
}

/// An agent session's state in the core.
pub(super) struct Agent {
    acp: Acp,
    /// [`Lines::covers`](crate::acp::lines::Lines::covers) at the last
    /// checkpoint: what the next one covers starts there.
    checkpointed: u64,
    /// The holder's `STDERR_TAIL`, until its `EXITED`.
    stderr: Option<Vec<u8>>,
    /// When the reducer asked to be stepped with [`Input::Tick`].
    wake: Option<Instant>,
    /// Its checkpoint came out too large to store, so its handling failed
    /// and it isn't checkpointed again. `acp::limits`' I2 makes this
    /// unreachable: a whole checkpoint is at most `LARGEST_CHECKPOINT`,
    /// under `MAX_CHECKPOINT_LEN`, which its const assertions and
    /// `the_frame_and_checkpoint_bounds_follow_from_the_constants` check.
    unstorable: bool,
    /// What its holder stores of its checkpoint (`agents-log-paging`
    /// decision 9).
    stored: Stored,
    /// The events applied since its last checkpoint, which a segment
    /// carries; none kept for an `unstorable` session.
    unsaved: Vec<Event>,
    /// An `AGENT_RESTART` taken while its agent needed a login
    /// (`agents-login` decision 8): the agent is hung up, and its exit
    /// starts this restart. Any other hang-up drops it, and so does an exit
    /// with no client to start it for.
    pub(super) login_restart: Option<LoginRestart>,
}

/// An `AGENT_RESTART` waiting for its agent's exit, and the directory the
/// new agent works in.
pub(super) struct LoginRestart {
    pub(super) restart: client::AgentRestart,
    pub(super) cwd: String,
}

impl Agent {
    /// A new agent session working in `cwd` (absolute).
    pub(super) fn new(cwd: String) -> Self {
        Self::with(Acp::Running(Box::new(AgentSession::new(cwd, VERSION.to_owned()))))
    }

    /// A new agent session in `cwd` whose agent waits for a client to
    /// choose a conversation (`agents-session-list` decision 1).
    pub(super) fn choosing(cwd: String) -> Self {
        Self::with(Acp::Running(Box::new(AgentSession::choosing(cwd, VERSION.to_owned()))))
    }

    /// A new agent session continuing `conversation` (a `SPAWN_AGENT` that
    /// names one), into a fresh log, in the conversation's directory.
    pub(super) fn continuing(conversation: &client::AgentConversation) -> Self {
        let conversation = Conversation {
            id: conversation.id.clone(),
            cwd: conversation.cwd.clone(),
            // What the new agent offers decides; the reducer reads that.
            continuation: Continuation::Neither,
            // A new server session starts untitled (`agents-session-title`
            // decision 3); the agent names it again.
            title: None,
        };
        Self::with(Acp::Running(Box::new(AgentSession::continuing(
            conversation,
            VERSION.to_owned(),
        ))))
    }

    /// Its agent is started again ([`Input::Restart`]): the new holder
    /// session's stdout counts from 0, and no stderr tail is kept.
    /// The new holder session stores nothing yet, so its first checkpoint
    /// is whole.
    pub(super) fn restarted(&mut self) {
        self.checkpointed = 0;
        self.stderr = None;
        self.wake = None;
        self.login_restart = None;
        self.forget_stored();
    }

    /// What its holder stores is no longer known (a holder `ERROR` naming
    /// the session, or a restart): the next checkpoint is whole.
    pub(super) fn forget_stored(&mut self) {
        self.stored = Stored::Unknown;
        self.unsaved.clear();
    }

    /// Whether its agent ended and its conversation can be continued
    /// ([`AgentSession::continuable`]).
    pub(super) fn continuable(&self) -> bool {
        match &self.acp {
            Acp::Adopting { .. } => false,
            Acp::Running(acp) => acp.continuable(),
        }
    }

    /// Whether its agent's end has been stepped
    /// ([`AgentSession::has_ended`]); not while it is adopted.
    pub(super) fn has_ended(&self) -> bool {
        match &self.acp {
            Acp::Adopting { .. } => false,
            Acp::Running(acp) => acp.has_ended(),
        }
    }

    /// Whether its agent runs and waits for a login
    /// ([`AgentSession::needs_login`]).
    pub(super) fn needs_login(&self) -> bool {
        match &self.acp {
            Acp::Adopting { .. } => false,
            Acp::Running(acp) => acp.needs_login(),
        }
    }

    /// An agent session adopted from a holder, waiting for its checkpoint
    /// and tail.
    pub(super) fn adopting() -> Self {
        Self::with(Acp::Adopting {
            checkpoint: Vec::new(),
        })
    }

    fn with(acp: Acp) -> Self {
        Self {
            acp,
            checkpointed: 0,
            stderr: None,
            wake: None,
            unstorable: false,
            stored: Stored::Unknown,
            unsaved: Vec::new(),
            login_restart: None,
        }
    }

    /// Its conversation, once it isn't being adopted.
    pub(super) fn running(&mut self) -> Option<&mut AgentSession> {
        match &mut self.acp {
            Acp::Adopting { .. } => None,
            Acp::Running(acp) => Some(acp),
        }
    }

    /// This agent keeping attached text in `store` (OpenSpec change
    /// `agents-prompt-context`): set at its spawn.
    pub(super) fn keeping(mut self, store: Store) -> Self {
        if let Some(acp) = self.running() {
            *acp.store_mut() = store;
        }
        self
    }

    /// The holder's stored checkpoint, while adopting.
    pub(super) fn checkpoint_read(&mut self, bytes: &[u8]) {
        if let Acp::Adopting { checkpoint } = &mut self.acp {
            *checkpoint = bytes.to_vec();
        }
    }

    /// What `LIST` says of it: its activity, and the name its agent gave
    /// in `initialize` while it is ready. One still being adopted is
    /// `working`, as a starting agent is.
    pub(super) fn info(&self) -> AgentInfo {
        match &self.acp {
            Acp::Adopting { .. } => AgentInfo {
                activity: AgentActivity::Working,
                name: None,
            },
            Acp::Running(acp) => {
                let transcript = acp.transcript();
                let name = match transcript.agent() {
                    AgentState::Ready { name, .. } => name.clone(),
                    AgentState::Starting
                    | AgentState::LoginRequired { .. }
                    | AgentState::LoggingIn { .. }
                    | AgentState::Failed { .. }
                    | AgentState::Exited { .. }
                    | AgentState::Lost { .. }
                    | AgentState::Choosing { .. } => None,
                };
                AgentInfo {
                    activity: activity(transcript.activity()),
                    name,
                }
            }
        }
    }

    /// The events that rebuild the newest page of its log within `budget`
    /// bytes of entries ([`brindle_transcript::Transcript::page`]),
    /// encoded once for `TRANSCRIPT`.
    pub(super) fn replay(&self, budget: usize) -> io::Result<Vec<JsonObject>> {
        match &self.acp {
            Acp::Adopting { .. } => Ok(Vec::new()),
            Acp::Running(acp) => encoded_events(&acp.transcript().page(budget)),
        }
    }

    /// The whole entries before `before` within `budget` bytes, as they
    /// stand now ([`brindle_transcript::Transcript::fetch`]). One still
    /// being adopted has none yet (its client waits for its replay).
    fn fetch(&self, before: EntryId, budget: usize) -> Fetched {
        match &self.acp {
            Acp::Adopting { .. } => Fetched {
                events: Vec::new(),
                more: false,
            },
            Acp::Running(acp) => acp.transcript().fetch(before, budget),
        }
    }
}

/// Each of `events` encoded once, as `TRANSCRIPT` and `AGENT_PAGE` carry
/// it (OpenSpec change `agents-checkpoint-deltas`, decision 9): the bytes
/// [`Event::encoded_len`] counts.
pub(super) fn encoded_events(events: &[Event]) -> io::Result<Vec<JsonObject>> {
    events
        .iter()
        .map(|event| {
            let bytes = serde_json::to_vec(event)?;
            JsonObject::new(bytes).map_err(|refused| io::Error::new(io::ErrorKind::InvalidData, refused.to_string()))
        })
        .collect()
}

/// The listing's activity for the log's (decision 15): the same five
/// names, one to one.
pub(super) fn activity(activity: Activity) -> AgentActivity {
    match activity {
        Activity::Idle => AgentActivity::Idle,
        Activity::Working => AgentActivity::Working,
        Activity::Blocked => AgentActivity::Blocked,
        Activity::Done => AgentActivity::Done,
        Activity::Error => AgentActivity::Error,
    }
}

/// Which kind of step [`guarded_step`] ran (OpenSpec change
/// `agents-process-lifecycle`, decision 6).
enum Stepped {
    Clean(Output),
    /// The step panicked; this is the [`Input::Fault`] step's output (or
    /// an empty one if that panicked too). The session's log may hold
    /// events that no client and no checkpoint has.
    Faulted(Output),
}

/// How a step's events reach the session's attached client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivery {
    /// As `TRANSCRIPT` frames, and into the next checkpoint's segment.
    Events,
    /// Not at all: a fresh replay follows, and the next checkpoint is
    /// whole.
    Replay,
}

/// [`reduce::step`] under `catch_unwind`: a panic fails only this
/// session's ACP handling, as [`Input::Fault`] with the panic's text. A
/// panic while failing it too leaves the session as it is, logged.
fn guarded_step(id: SessionId, acp: &mut AgentSession, input: Input<'_>, now: Instant) -> Stepped {
    let stepped = panic::catch_unwind(AssertUnwindSafe(|| {
        inject_fault(&input);
        let late = late_fault(&input);
        let output = reduce::step(acp, input, now);
        inject_late(late, &output);
        output
    }));
    let reason = match stepped {
        Ok(output) => return Stepped::Clean(output),
        Err(payload) => fault::describe(&*payload),
    };
    log!("session {id}: ACP handling panicked ({reason}); its agent runs on, refusing prompts");
    let failed = panic::catch_unwind(AssertUnwindSafe(|| {
        reduce::step(acp, Input::Fault { reason }, now)
    }));
    Stepped::Faulted(failed.unwrap_or_else(|payload| {
        log!(
            "session {id}: failing its ACP handling panicked too ({})",
            fault::describe(&*payload)
        );
        Output::default()
    }))
}

/// Panics when the agent's stdout or a prompt holds
/// `BRINDLE_TEST_ACP_PANIC`'s marker (debug builds only).
fn inject_fault(input: &Input<'_>) {
    static MARKER: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    if marked(input, fault::marker(fault::ACP_PANIC, &MARKER)) {
        panic!("{} matched", fault::ACP_PANIC);
    }
}

/// Whether the agent's stdout or a prompt holds
/// `BRINDLE_TEST_ACP_PANIC_LATE`'s marker (debug builds only), read before
/// the step takes the input.
fn late_fault(input: &Input<'_>) -> bool {
    static MARKER: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    marked(input, fault::marker(fault::ACP_PANIC_LATE, &MARKER))
}

/// Panics after a step whose input held the late switch's marker applied
/// events: the session's log then has events its `Output` takes with it.
fn inject_late(late: bool, output: &Output) {
    if late && !output.events.is_empty() {
        panic!("{} matched", fault::ACP_PANIC_LATE);
    }
}

/// Whether `input`'s bytes a fault switch looks at (the agent's stdout, a
/// prompt's text) hold `marker`; never without one.
fn marked(input: &Input<'_>, marker: Option<&[u8]>) -> bool {
    let Some(marker) = marker else {
        return false;
    };
    let bytes: &[u8] = match input {
        Input::Stdout(bytes) => bytes,
        Input::Command(Command::Prompt { text, .. }) => text.as_bytes(),
        Input::Spawned
        | Input::OutputLost
        | Input::Exited { .. }
        | Input::Lost { .. }
        | Input::Restart
        | Input::Command(
            Command::Unqueue
            | Command::Resend
            | Command::Cancel
            | Command::Answer { .. }
            | Command::Login { .. }
            | Command::List { .. }
            | Command::Choose { .. }
            | Command::SetOption { .. },
        )
        | Input::Fault { .. }
        | Input::Tick => return false,
    };
    fault::holds(bytes, marker)
}

/// A protocol 4 client's command for an agent session.
pub(super) struct AgentCommand {
    /// The request its `OK` or `ERROR` answers; `None` for `AGENT_CANCEL`.
    pub(super) req: Option<RequestId>,
    pub(super) session: SessionId,
    pub(super) command: Command,
}

impl AgentCommand {
    pub(super) fn cancel(target: client::SessionRef) -> Self {
        Self {
            req: None,
            session: target.session,
            command: Command::Cancel,
        }
    }

    pub(super) fn answer(answer: client::AgentPermission) -> Self {
        Self {
            req: Some(answer.req),
            session: answer.session,
            command: Command::Answer {
                entry: EntryId(answer.entry),
                option: answer.option,
            },
        }
    }

    pub(super) fn unqueue(request: client::AgentRequest) -> Self {
        Self {
            req: Some(request.req),
            session: request.session,
            command: Command::Unqueue,
        }
    }

    pub(super) fn resend(request: client::AgentRequest) -> Self {
        Self {
            req: Some(request.req),
            session: request.session,
            command: Command::Resend,
        }
    }

    /// `AGENT_LOGIN` (OpenSpec change `agents-login`).
    pub(super) fn login(login: client::AgentLogin) -> Self {
        Self {
            req: Some(login.req),
            session: login.session,
            command: Command::Login {
                method: login.method,
            },
        }
    }

    /// `AGENT_LIST` (OpenSpec change `agents-session-list`): answered by
    /// `AGENT_CONVERSATIONS` once the agent answers, not by `OK`.
    pub(super) fn list(list: client::AgentList) -> Self {
        Self {
            req: Some(list.req),
            session: list.session,
            command: Command::List {
                req: list.req,
                cursor: list.cursor,
            },
        }
    }

    /// `AGENT_CHOOSE` (`agents-session-list`).
    pub(super) fn choose(choose: client::AgentChoose) -> Self {
        Self {
            req: Some(choose.req),
            session: choose.session,
            command: Command::Choose {
                conversation: choose.conversation,
            },
        }
    }

    /// `AGENT_SET_OPTION` (OpenSpec change `agents-commands`).
    pub(super) fn set_option(set: client::AgentSetOption) -> Self {
        Self {
            req: Some(set.req),
            session: set.session,
            command: Command::SetOption {
                option: set.option,
                value: set.value,
            },
        }
    }
}

impl<L: EventLoop> Core<L> {
    /// `SPAWN_AGENT` (which `handle_client` refuses below protocol 4):
    /// refused with no command; else
    /// queued for a holder that runs pipe sessions, in the absolute
    /// directory its agent will work in. One that continues a conversation
    /// works in the conversation's directory, which ACP ties it to.
    pub(super) fn client_spawn_agent(&mut self, client: Token, spawn: client::SpawnAgent) {
        let refusal = policy::agent_spawn_refusal(&spawn.command)
            .map(str::to_owned)
            .or_else(|| {
                (spawn.conversation.is_some() && spawn.start.is_some())
                    .then(|| "SPAWN_AGENT can't both continue a conversation and choose one".to_owned())
            });
        if let Some(refusal) = refusal {
            return self.send_client(client, error(Some(spawn.req), None, refusal));
        }
        let cwd = match &spawn.conversation {
            Some(conversation) => conversation.cwd.clone(),
            None => platform::agent_directory(spawn.cwd.as_deref(), &spawn.env),
        };
        self.queue_spawn(client, SpawnRequest::Agent { spawn, cwd });
    }

    /// `AGENT_RESTART` (change `agents-session-recovery`, decision 5): an
    /// ended agent session the client has attached is started again with
    /// the command given, in its conversation's directory, under its own
    /// id, key and meta (the command updated), on any holder of the
    /// core's build. Refused (below protocol 4 by `handle_client`) for a
    /// session the client hasn't attached, while the core stops or steps down, and as
    /// [`policy::restart_refusal`] says; the reducer's own refusal
    /// ([`Input::Restart`]) comes last. `OK` once the spawn is queued; what
    /// follows comes as transcript events.
    ///
    /// An agent that runs and needs a login (`agents-login` decision 8) is
    /// hung up instead, the restart kept ([`Agent::login_restart`]) until
    /// its exit starts it ([`Core::restart_after_login`]); `OK` at once. It
    /// works in its conversation's directory, or its session's when it has
    /// none yet.
    pub(super) fn client_agent_restart(&mut self, client: Token, restart: client::AgentRestart) {
        let (req, id) = (restart.req, restart.session);
        let stopping = self.stopping.is_some() || self.stepping_down.is_some();
        let Some(session) = self.attached_session(client, id, Some(req)) else {
            return;
        };
        let life = session.life.clone();
        let session_cwd = session.meta.cwd.clone();
        let Some(agent) = session.agent() else {
            let message = format!("session {id} isn't an agent session");
            return self.send_client(client, error(Some(req), Some(id), message));
        };
        let continuable = agent.continuable();
        let needs_login = agent.needs_login();
        let cwd = match &agent.acp {
            Acp::Running(acp) => acp.transcript().conversation().map(|c| c.cwd.clone()),
            Acp::Adopting { .. } => None,
        };
        let cwd = if needs_login { cwd.or(session_cwd) } else { cwd };
        let refusal = if stopping {
            Some("the server is stopping; try again".to_owned())
        } else {
            policy::restart_refusal(&life, continuable, needs_login, &restart.command)
                .map(|r| r.to_string())
        };
        if let Some(refusal) = refusal {
            let message = format!("session {id}: {refusal}");
            return self.send_client(client, error(Some(req), Some(id), message));
        }
        let Some(cwd) = cwd else {
            let message = format!("session {id}: {}", Refused::CannotContinue);
            return self.send_client(client, error(Some(req), Some(id), message));
        };
        match self.agent_step(id, Input::Restart) {
            Some(Ok(())) => {}
            Some(Err(refused)) => {
                let message = format!("session {id}: {refused}");
                return self.send_client(client, error(Some(req), Some(id), message));
            }
            None => {
                let message = format!("session {id}: the terminal server's ACP handling failed");
                return self.send_client(client, error(Some(req), Some(id), message));
            }
        }
        if needs_login {
            // Hung up by the step: its exit starts the restart.
            log!(
                "session {id}: its agent is hung up to start again after its login ({:?})",
                restart.command
            );
            if let Some(agent) = self.sessions.get_mut(&id).and_then(|session| session.agent()) {
                agent.login_restart = Some(LoginRestart { restart, cwd });
            }
            return self.send_client(client, ToClient::Ok(client::Request { req }));
        }
        log!(
            "session {id}: its agent starts again to continue its conversation ({:?})",
            restart.command
        );
        let command = (restart.command.clone(), restart.args.clone());
        self.start_again(id, &command.0, &command.1, Restart { restart, cwd });
        self.send_client(client, ToClient::Ok(client::Request { req }));
        self.place_waiting();
    }

    /// The restart agent session `id`'s exit starts, if any: one taken
    /// while its agent needed a login, with a client attached to start it
    /// for, while the core isn't stopping or stepping down. The restart is
    /// dropped either way.
    pub(super) fn login_restart_due(&mut self, id: SessionId) -> Option<LoginRestart> {
        let stopping = self.stopping.is_some() || self.stepping_down.is_some();
        let session = self.sessions.get_mut(&id)?;
        let client = session.ownership.client();
        let restart = session.agent()?.login_restart.take()?;
        match client {
            Some(_) if !stopping => Some(restart),
            Some(_) | None => {
                log!("session {id}: its restart after a login is dropped: no client to start it for");
                None
            }
        }
    }

    /// Agent session `id`'s agent, hung up for a restart after a login,
    /// exited: the reducer spawns it anew ([`Input::Restart`] in
    /// `Phase::Restarting`), and the restart is queued as an ended agent's
    /// is, under the same id. Should the reducer refuse (it can't: only
    /// `Restarting` keeps a restart), the exit is stepped instead.
    pub(super) fn restart_after_login(
        &mut self,
        id: SessionId,
        code: Option<i32>,
        pending: LoginRestart,
    ) {
        match self.agent_step(id, Input::Restart) {
            Some(Ok(())) => {}
            Some(Err(refused)) => {
                log!("session {id}: its restart after a login was refused ({refused})");
                self.agent_exited(id, code);
                return self.ended(id, code);
            }
            // The step panicked: its handling failed, and the exit ends it.
            None => {
                self.agent_exited(id, code);
                return self.ended(id, code);
            }
        }
        let LoginRestart { restart, cwd } = pending;
        log!(
            "session {id}: its agent starts again after its login ({:?})",
            restart.command
        );
        let command = (restart.command.clone(), restart.args.clone());
        self.start_again(id, &command.0, &command.1, Restart { restart, cwd });
        self.place_waiting();
    }

    /// A protocol 4 client's command for an agent session it has attached:
    /// stepped, its events sent, then its reply (`OK` or an `ERROR` naming
    /// the session; none for a cancel the reducer took).
    pub(super) fn client_agent_command(&mut self, client: Token, command: AgentCommand) {
        let AgentCommand {
            req,
            session: id,
            command,
        } = command;
        let Some(session) = self.attached_session(client, id, req) else {
            return;
        };
        let running = match session.agent().map(|agent| &agent.acp) {
            None => Err(format!("session {id} isn't an agent session")),
            Some(Acp::Adopting { .. }) => Err(format!("session {id}: {}", Refused::NotReady)),
            Some(Acp::Running(_)) => Ok(()),
        };
        if let Err(message) = running {
            return self.send_client(client, error(req, Some(id), message));
        }
        // A list taken is answered by `AGENT_CONVERSATIONS` later.
        let answered_later = matches!(command, Command::List { .. });
        match self.agent_step(id, Input::Command(command)) {
            Some(Ok(())) if answered_later => {}
            Some(Ok(())) => {
                if let Some(req) = req {
                    self.send_client(client, ToClient::Ok(client::Request { req }));
                }
            }
            Some(Err(refused)) => {
                self.send_client(client, error(req, Some(id), format!("session {id}: {refused}")));
            }
            // The step panicked: the command wasn't taken, and the log
            // says the handling failed.
            None => {
                let message = format!("session {id}: the terminal server's ACP handling failed");
                self.send_client(client, error(req, Some(id), message));
            }
        }
    }

    /// `AGENT_FETCH` from `client` (decision 5): the whole entries before
    /// `before` within the clamped budget, as `AGENT_PAGE` frames, the last
    /// saying whether more are earlier; or the `ERROR`
    /// [`policy::agent_fetch_refusal`] gives. It reads the core's log only,
    /// so nothing is stepped, checkpointed or sent to the holder.
    pub(super) fn client_agent_fetch(&mut self, client: Token, fetch: client::AgentFetch) {
        let client::AgentFetch {
            req,
            session: id,
            before,
            max_bytes,
        } = fetch;
        let view = self.sessions.get(&id).map(|session| FetchView {
            kind: session.kind(),
            attached_here: session.ownership.client() == Some(client),
            replay_pending: session.pending_replay.is_some(),
        });
        if let Some(refusal) = policy::agent_fetch_refusal(id, view) {
            return self.send_client(client, error(Some(req), Some(id), refusal.to_string()));
        }
        let budget = policy::page_budget(Some(max_bytes));
        let Some(Fetched { events, more }) = self
            .sessions
            .get_mut(&id)
            .and_then(|session| session.agent())
            .map(|agent| agent.fetch(EntryId(before), budget))
        else {
            return;
        };
        let frames = encoded_events(&events).and_then(|events| client::page_frames(req, id, &events, more));
        match frames {
            Ok(frames) => {
                for frame in &frames {
                    self.send_client_frame(client, frame);
                }
            }
            Err(failure) => {
                log!("session {id}: can't send a page of its log: {failure}");
                let message = format!("the terminal server couldn't send the page: {failure}");
                self.send_client(client, error(Some(req), Some(id), message));
            }
        }
    }

    /// The answer to a list of session `id`'s conversations, to the client
    /// attached now (`agents-session-list` decision 2): `AGENT_CONVERSATIONS`
    /// in one frame, or the `ERROR` naming the session. Dropped with no
    /// client attached, which asked on a link since gone.
    fn send_listed(&mut self, id: SessionId, answer: ListAnswer) {
        let ListAnswer { req, result } = answer;
        let Some(client) = self.sessions.get(&id).and_then(|session| session.ownership.client()) else {
            return log!("session {id}: a list of its conversations answered no client");
        };
        match result {
            Ok(page) => {
                if page.unreadable > 0 {
                    log!(
                        "session {id}: {} of its agent's listed conversations couldn't be read",
                        page.unreadable
                    );
                }
                let listed = client::AgentConversations {
                    req,
                    session: id,
                    conversations: page.conversations,
                    next: page.next,
                };
                self.send_client(client, ToClient::AgentConversations(listed));
            }
            Err(message) => {
                self.send_client(client, error(Some(req), Some(id), format!("session {id}: {message}")));
            }
        }
    }

    /// Steps agent session `id` with `input` and applies what it says;
    /// gives the reply to a client's command. Nothing for a session that
    /// isn't a running agent's.
    pub(super) fn agent_step(
        &mut self,
        id: SessionId,
        input: Input<'_>,
    ) -> Option<Result<(), Refused>> {
        let now = Instant::now();
        let agent = self.sessions.get_mut(&id)?.agent()?;
        let Acp::Running(acp) = &mut agent.acp else {
            return None;
        };
        match guarded_step(id, acp, input, now) {
            Stepped::Clean(output) => self.agent_apply(id, output, Delivery::Events),
            Stepped::Faulted(output) => self.agent_faulted(id, output),
        }
    }

    /// A step of agent session `id` panicked, and `output` is its fault
    /// step's (decision 6): what the holder stores no longer matches the
    /// log, so the next checkpoint is whole; the output is applied without
    /// its events, and an attached client not waiting for a replay is sent
    /// a fresh one, the log as the server holds it.
    fn agent_faulted(&mut self, id: SessionId, output: Output) -> Option<Result<(), Refused>> {
        if let Some(agent) = self.sessions.get_mut(&id).and_then(|session| session.agent()) {
            agent.forget_stored();
        }
        let reply = self.agent_apply(id, output, Delivery::Replay);
        let client = self
            .sessions
            .get(&id)
            .filter(|session| session.pending_replay.is_none())
            .and_then(|session| session.ownership.client());
        if let Some(client) = client {
            log!("session {id}: its client is sent the log again after the fault");
            self.replay(client, id, PendingReplay::reconnect());
        }
        reply
    }

    /// Applies a step's output, in its order: the events to the attached
    /// client (unless a replay it waits for has them, or `delivery` says a
    /// replay follows), a checkpoint when due, the agent's stdin, the
    /// hang-up. Returns the reply.
    fn agent_apply(
        &mut self,
        id: SessionId,
        output: Output,
        delivery: Delivery,
    ) -> Option<Result<(), Refused>> {
        let Output {
            events,
            stdin,
            reply,
            checkpoint,
            hang_up,
            wake,
            listed,
        } = output;
        if let Some(answer) = listed {
            self.send_listed(id, answer);
        }
        // What the session references may have changed: a prompt queued,
        // sent or discarded, entries trimmed (`agents-prompt-context`).
        let references = events
            .iter()
            .any(|event| matches!(event, Event::Queue { .. } | Event::Trimmed { .. }));
        if let Some(line) = failure_line(id, &events) {
            log!("{line}");
        }
        let session = self.sessions.get_mut(&id)?;
        let delivered = match delivery {
            Delivery::Events => true,
            Delivery::Replay => false,
        };
        let client = session
            .ownership
            .client()
            .filter(|_| session.pending_replay.is_none() && delivered);
        let live = session.life.is_live();
        let holder = session.holder;
        let agent = session.agent()?;
        agent.wake = wake;
        // Each event is encoded once, for the client, then moved into what
        // the next checkpoint holds.
        let sent = client
            .filter(|_| !events.is_empty())
            .map(|client| (client, encoded_events(&events)));
        if !agent.unstorable && delivered {
            agent.unsaved.extend(events);
        }
        let since = match &agent.acp {
            Acp::Running(acp) => acp.lines().covers().saturating_sub(agent.checkpointed),
            Acp::Adopting { .. } => 0,
        };
        if let Some((client, events)) = sent {
            self.send_transcript(client, id, events);
        }
        let at = AgentCheckpoint::Stepped(checkpoint);
        if policy::agent_checkpoint_due(at, since) {
            self.checkpoint_agent(id, at);
        }
        // Nothing reaches an agent being hung up, or gone.
        if !stdin.is_empty() && live {
            self.send_holder(
                holder,
                ToHolder::Input(h1::Data {
                    session: id,
                    bytes: stdin,
                }),
            );
        }
        if hang_up {
            self.hang_up(id);
        }
        if references {
            self.store_changed(id);
        }
        reply
    }

    /// Sends `events` of session `id` to `client` as `TRANSCRIPT` frames of
    /// whole events. One that can't fit a frame is reported to the client
    /// and logged; `acp::limits`' I1 rules it out, since every event the
    /// reducer emits, replays or pages encodes within `MAX_EVENT`, which
    /// leaves `TRANSCRIPT_ENVELOPE` of the frame for the batch. The events
    /// come encoded once ([`encoded_events`]), and are packed as they are.
    pub(super) fn send_transcript(&mut self, client: Token, id: SessionId, events: io::Result<Vec<JsonObject>>) {
        match events.and_then(|events| client::transcript_frames(id, &events)) {
            Ok(frames) => {
                for frame in &frames {
                    self.send_client_frame(client, frame);
                }
            }
            Err(failure) => {
                log!("session {id}: can't send its transcript: {failure}");
                let message = format!("the terminal server couldn't send its transcript: {failure}");
                self.send_client(client, error(None, Some(id), message));
            }
        }
    }

    /// Checkpoints agent session `id`, due `at`, when it is
    /// [`checkpointable`](Core::checkpointable), as
    /// [`write_agent_checkpoint`](Core::write_agent_checkpoint) writes it.
    pub(super) fn checkpoint_agent(&mut self, id: SessionId, at: AgentCheckpoint) {
        if self.checkpointable(id) {
            self.write_agent_checkpoint(id, at);
        }
    }

    /// Checkpoints agent session `id` (its liveness checked by
    /// [`Core::checkpoint_agent`], or by a spawn or restart just sent),
    /// due `at`: its encoded state whole, or a segment of the events since
    /// the last one appended to what its holder stores
    /// ([`policy::agent_checkpoint_form`]), covering the stdout fed since
    /// the last one. A state too large to store fails its handling, which
    /// `acp::limits`' I2 makes unreachable for a whole checkpoint; a
    /// segment too large is written whole instead.
    pub(super) fn write_agent_checkpoint(&mut self, id: SessionId, at: AgentCheckpoint) {
        let Some(holder) = self.sessions.get(&id).map(|session| session.holder) else {
            return;
        };
        let adoption = self
            .holders
            .get(&holder)
            .map_or(policy::Adoption::Drain, |holder| holder.adoption());
        let Some(agent) = self.sessions.get_mut(&id).and_then(|session| session.agent()) else {
            return;
        };
        let Acp::Running(acp) = &agent.acp else {
            return;
        };
        if agent.unstorable {
            return;
        }
        let trimmed = agent
            .unsaved
            .iter()
            .any(|event| matches!(event, Event::Trimmed { .. }));
        // The events since the last checkpoint are the segment's, moved;
        // a whole checkpoint has them in its log.
        let unsaved = std::mem::take(&mut agent.unsaved);
        let segment = match &agent.stored {
            Stored::Known { written, .. }
                if policy::agent_appendable(&agent.stored, at, trimmed, adoption) =>
            {
                checkpoint::encode_segment(acp, unsaved, agent.stored.next_seq(), written).ok()
            }
            Stored::Known { .. } | Stored::Unknown => None,
        };
        let segment_len = segment.as_ref().map(|segment| segment.bytes.len() as u64);
        let form = policy::agent_checkpoint_form(&agent.stored, at, segment_len, trimmed, adoption);
        let encoded = match (form, segment) {
            (CheckpointForm::Append, Some(segment)) => Ok(ToStore::Append(segment)),
            (CheckpointForm::Append, None) | (CheckpointForm::Whole, _) => {
                checkpoint::encode(acp).map(ToStore::Whole)
            }
        };
        match encoded {
            Ok(to_store) => {
                let covers = acp.lines().covers();
                let since = covers.saturating_sub(agent.checkpointed);
                agent.checkpointed = covers;
                // Only a line longer than a checkpoint's spacing, or a
                // moment met while one arrives, is held: rare enough to say.
                let held = match acp.lines().partial() {
                    Partial::Collecting(line) => line.len(),
                    Partial::TooLong { .. } | Partial::Resyncing { .. } => 0,
                };
                let stored = std::mem::replace(&mut agent.stored, Stored::Unknown);
                let (bytes, form) = match to_store {
                    ToStore::Whole(encoded) => {
                        agent.stored = Stored::whole(encoded.bytes.len() as u64, encoded.written);
                        (encoded.bytes, h1::BlobForm::Whole)
                    }
                    ToStore::Append(segment) => {
                        let len = segment.bytes.len() as u64;
                        agent.stored = stored.appended(len, segment.changes);
                        (segment.bytes, h1::BlobForm::Append)
                    }
                };
                if held > 0 {
                    log!("session {id}: checkpointed with {held} bytes of a line still arriving");
                }
                // Queued from slices of the one buffer it was encoded into.
                self.send_holder_sliced(holder, h1::checkpoint_frames(id, &bytes, since, form));
            }
            Err(too_large) => {
                log!("session {id}: {too_large}; failing its ACP handling");
                agent.unstorable = true;
                agent.unsaved = Vec::new();
                self.agent_step(
                    id,
                    Input::Fault {
                        reason: too_large.to_string(),
                    },
                );
            }
        }
    }

    /// The holder's `STDERR_TAIL`: kept for the `EXITED` that follows; a
    /// session adopted as exited (it ended while no core ran) has its exit
    /// stepped now, since its tail is all that was still to come.
    pub(super) fn agent_stderr(&mut self, holder: Token, id: SessionId, bytes: Vec<u8>) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return log!("holder {holder}: a stderr tail for session {id}, which it doesn't have");
        };
        if session.holder != holder {
            return log!("holder {holder} reported session {id}, which isn't its");
        }
        let exited = match session.life {
            Life::ExitedAwaitingOwner { code, .. } => Some(code),
            Life::Starting { .. }
            | Life::Restarting { .. }
            | Life::Running { .. }
            | Life::HungUp { .. }
            | Life::AgentEnded { .. } => None,
        };
        let Some(agent) = session.agent() else {
            return log!("holder {holder}: a stderr tail for terminal session {id}; ignored");
        };
        agent.stderr = Some(bytes);
        if let Some(code) = exited {
            self.agent_exited(id, code);
            // A returning owner's replay waited for the exit (decision 7).
            self.send_pending_replay(id);
        }
    }

    /// Agent session `id`'s program exited (or was lost with its holder):
    /// its exit is stepped with the stderr tail that came before, so the
    /// log has it before the session's `EXIT`. Nothing for a terminal.
    pub(super) fn agent_exited(&mut self, id: SessionId, code: Option<i32>) {
        let Some(agent) = self.sessions.get_mut(&id).and_then(|session| session.agent()) else {
            return;
        };
        let stderr = agent.stderr.take().unwrap_or_default();
        self.agent_step(
            id,
            Input::Exited {
                code,
                stderr: &stderr,
            },
        );
    }

    /// An adopted agent session's tail came after its checkpoint: the
    /// session is rebuilt ([`checkpoint::adopt`], under `catch_unwind`),
    /// what that did is applied, and the tail is fed as stdout.
    pub(super) fn agent_adopted(&mut self, id: SessionId, truncated: bool, tail: &[u8]) {
        let now = Instant::now();
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let cwd = session.meta.cwd.clone().unwrap_or_default();
        let Some(agent) = session.agent() else {
            return;
        };
        let Acp::Adopting { checkpoint: stored } = &agent.acp else {
            return;
        };
        let adopted = panic::catch_unwind(AssertUnwindSafe(|| {
            checkpoint::adopt(stored, truncated, &cwd, VERSION, now)
        }));
        let (acp, stepped) = match adopted {
            Ok(adopted) => {
                if let Err(why) = &adopted.restored {
                    log!("session {id}: {}: {why}", checkpoint::NOT_RESTORED);
                }
                (adopted.session, Stepped::Clean(adopted.output))
            }
            // A fresh log, which isn't what the holder stores: handled as
            // a faulted step (decision 6).
            Err(payload) => {
                let reason = fault::describe(&*payload);
                log!("session {id}: adopting its conversation panicked ({reason})");
                let mut acp = AgentSession::new(cwd, VERSION.to_owned());
                let output = match guarded_step(id, &mut acp, Input::Fault { reason }, now) {
                    Stepped::Clean(output) | Stepped::Faulted(output) => output,
                };
                (acp, Stepped::Faulted(output))
            }
        };
        if truncated {
            log!(
                "session {id}: its stdout was truncated; adopted from its checkpoint and the last {} bytes",
                tail.len()
            );
        }
        agent.checkpointed = acp.lines().covers();
        agent.acp = Acp::Running(Box::new(acp));
        // Its store is known again, and its queued prompt's texts read,
        // before the tail can send that prompt (`agents-prompt-context`).
        self.store_adopted(id);
        match stepped {
            Stepped::Clean(output) => self.agent_apply(id, output, Delivery::Events),
            Stepped::Faulted(output) => self.agent_faulted(id, output),
        };
        if !tail.is_empty() {
            self.agent_step(id, Input::Stdout(tail));
        }
    }

    /// Steps every agent session whose wake-up is due at `now`.
    pub(super) fn tick_agents(&mut self, now: Instant) {
        let due: Vec<SessionId> = self
            .sessions
            .iter_mut()
            .filter_map(|(&id, session)| {
                let agent = session.agent()?;
                agent.wake.is_some_and(|wake| wake <= now).then_some(id)
            })
            .collect();
        for id in due {
            self.agent_step(id, Input::Tick);
        }
    }

    /// Places a waiting `AGENT_RESTART` on `holder`: the ended session runs
    /// its new agent there under its own id, with its meta (the new command
    /// in it) in the `SPAWN_PIPES`. Its state is stored right after, so a
    /// core that adopts the agent before its `SPAWNED` continues the
    /// conversation ([`checkpoint::adopt`](crate::acp::checkpoint::adopt)).
    /// Nothing for a session that isn't a waiting restart.
    pub(super) fn send_restart(&mut self, holder: Token, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let Life::Restarting {
            restart,
            stage: stage @ RestartStage::Waiting,
        } = &mut session.life
        else {
            return log!("session {id}: isn't a restart waiting for a holder");
        };
        *stage = RestartStage::Placed { hung_up: false };
        let Restart { restart, cwd } = restart.clone();
        session.holder = holder;
        session.paused = false;
        let meta = session.stored_meta().to_value();
        let message = ToHolder::SpawnPipes(h1::SpawnPipes {
            session: id,
            command: restart.command,
            args: restart.args,
            cwd: Some(cwd),
            env: restart.env,
            meta,
        });
        if self.send_holder(holder, message) {
            self.write_agent_checkpoint(id, AgentCheckpoint::Stepped(None)); // whole: restarted
        }
    }

    /// An ended agent session starts again from `restart` (an
    /// `AGENT_RESTART`'s): waiting for a holder, listed with its new
    /// `command` and `args`, which its holder gets in the spawn's meta.
    pub(super) fn start_again(
        &mut self,
        id: SessionId,
        command: &str,
        args: &[String],
        restart: Restart,
    ) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.meta.command = command_line(Some(command), args);
        session.paused = false;
        session.life = Life::Restarting {
            restart,
            stage: RestartStage::Waiting,
        };
        if let Some(agent) = session.agent() {
            agent.restarted();
        }
    }

    /// A restart that couldn't start (`SPAWN_FAILED`, or no holder for it):
    /// its agent ends as an exit with no code whose standard error is
    /// `message`, which keeps the session ended for another try.
    pub(super) fn restart_failed(&mut self, id: SessionId, message: &str) {
        self.agent_ended(
            id,
            None,
            Input::Exited {
                code: None,
                stderr: message.as_bytes(),
            },
        );
    }

    /// A restart still waiting for a holder is hung up (`CLOSE`, `KILL`,
    /// `STOP`, its client gone): it never starts, and the session ends, as
    /// a hung-up session's end does, whoever wanted it kept.
    pub(super) fn abandon_restart(&mut self, id: SessionId) {
        log!("session {id}: its restart ends before it was placed");
        self.agent_step(
            id,
            Input::Exited {
                code: None,
                stderr: b"",
            },
        );
        self.end_reported(id, None);
    }

    /// An agent session's agent ended as `input` says (an exit with no
    /// code, or a loss), outside of its holder's `EXITED`: stepped, then
    /// kept or ended as [`policy::end_fate`] says.
    pub(super) fn agent_ended(&mut self, id: SessionId, code: Option<i32>, input: Input<'_>) {
        self.agent_step(id, input);
        self.ended(id, code);
    }

    /// Ends an ended agent session: it is gone, and a client that has it
    /// gets its `EXIT` (after a replay it still waits for).
    pub(super) fn end_ended(&mut self, id: SessionId, code: Option<i32>) {
        log!("session {id}: its ended agent's session ends");
        self.send_waiting_replay(id);
        let Some(session) = self.sessions.remove(&id) else {
            return;
        };
        if let Some(client) = session.ownership.client() {
            self.send_client(client, ToClient::Exit(Exit { session: id, code, lost: None }));
        }
    }
}

/// The line the core logs for a step whose events fail session `id`'s
/// agent: its message, which names a fold error or an event too large to
/// send (`agents-event-bounds` decision 4), so the cause isn't only in the
/// client's view.
fn failure_line(id: SessionId, events: &[Event]) -> Option<String> {
    events.iter().rev().find_map(|event| {
        if let Event::Agent {
            state: AgentState::Failed { message },
        } = event
        {
            Some(format!("session {id}: {message}"))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A step whose events fail the agent, a fold error or an event too
    /// large to send among them, logs why, naming the session
    /// (`agents-event-bounds` task 4.5); one that doesn't logs nothing.
    #[test]
    fn a_failure_in_a_steps_events_reaches_the_log() {
        let message = "ACP handling failed: an added (tool) event for entry 2 is 2000000 bytes; one transcript event may be at most 1047552";
        let events = [
            Event::Turn {
                state: brindle_transcript::TurnState::Failed {
                    message: "ACP handling failed".into(),
                },
            },
            Event::Agent {
                state: AgentState::Failed {
                    message: message.into(),
                },
            },
        ];
        assert_eq!(failure_line(7, &events), Some(format!("session 7: {message}")));
        assert_eq!(failure_line(7, &events[..1]), None);
        let mut session = AgentSession::new("/w".into(), "0.3.0".into());
        let output = reduce::step(
            &mut session,
            Input::Fault {
                reason: "has no text to append to".into(),
            },
            Instant::now(),
        );
        assert!(
            failure_line(3, &output.events).is_some_and(|line| line.starts_with("session 3: ACP handling failed: ")
                && line.contains("has no text to append to")),
            "{:?}",
            output.events
        );
    }

    #[test]
    fn each_activity_is_listed_by_its_own_name() {
        let pairs = [
            (Activity::Idle, AgentActivity::Idle),
            (Activity::Working, AgentActivity::Working),
            (Activity::Blocked, AgentActivity::Blocked),
            (Activity::Done, AgentActivity::Done),
            (Activity::Error, AgentActivity::Error),
        ];
        for (log, listed) in pairs {
            assert_eq!(activity(log), listed);
        }
    }

    #[test]
    fn a_new_agent_is_listed_working_and_replays_its_starting_state() {
        let agent = Agent::new("/home/u".into());
        assert_eq!(
            agent.info(),
            AgentInfo {
                activity: AgentActivity::Working,
                name: None
            }
        );
        let events: Vec<Event> = agent
            .replay(brindle_transcript::PAGE_BYTES)
            .unwrap()
            .iter()
            .map(|object| Event::from_value(&serde_json::from_slice(object.as_bytes()).unwrap()).unwrap())
            .collect();
        assert!(
            events.iter().any(|event| matches!(event, Event::Agent { state: AgentState::Starting })),
            "{events:?}"
        );

        let adopting = Agent::adopting();
        assert_eq!(adopting.info().activity, AgentActivity::Working);
        assert!(adopting.replay(brindle_transcript::PAGE_BYTES).unwrap().is_empty());
    }

    #[test]
    fn a_fault_fails_the_agent_with_its_reason_and_leaves_it_running() {
        let mut acp = AgentSession::new("/home/u".into(), "test".into());
        let now = Instant::now();
        let Stepped::Clean(output) = guarded_step(1, &mut acp, Input::Spawned, now) else {
            panic!("the start faulted");
        };
        assert!(!output.stdin.is_empty(), "initialize goes out");
        // What `guarded_step` steps when the reducer panics: the agent
        // fails with the panic's text.
        let caught = panic::catch_unwind(|| panic!("boom")).unwrap_err();
        let reason = fault::describe(&*caught);
        let output = reduce::step(&mut acp, Input::Fault { reason }, now);
        assert!(matches!(
            acp.transcript().agent(),
            AgentState::Failed { message } if message.contains("boom")
        ));
        assert!(!output.hang_up, "the agent runs on");

        // What a faulted step does to the core's agent (decision 6): what
        // its holder stores is forgotten, so the checkpoint after the fault
        // is whole, never a segment on a base the log no longer matches.
        let mut agent = Agent::new("/home/u".into());
        agent.stored = Stored::whole(100, checkpoint::Written::default());
        agent.unsaved.push(Event::Agent { state: AgentState::Starting });
        let at = AgentCheckpoint::Stepped(Some(reduce::Moment::Stopped));
        assert!(policy::agent_appendable(&agent.stored, at, false, policy::Adoption::Keep));
        agent.forget_stored();
        assert!(!policy::agent_appendable(&agent.stored, at, false, policy::Adoption::Keep));
        assert!(agent.unsaved.is_empty());
    }
}
