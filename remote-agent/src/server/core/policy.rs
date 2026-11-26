//! The server core's policies, as pure functions of what its loop gathers
//! (`fcis R1`): the handlers in `core.rs` and `core/` read their state,
//! call these, and apply what comes back. `now` is always an argument.
//!
//! - **Lifecycle** ([`Lifecycle`]): when clients are first accepted, the
//!   reconnect grace for adopted sessions' owners, and when the core exits.
//! - **Clients:** what a `HELLO` asks for ([`client_options`]), the holder
//!   protocols a `WELCOME` names ([`holder_protocols`]), and the marks a
//!   client's queue crosses ([`queue_mark`]).
//! - **Holders:** where waiting spawns go ([`placement`]), whether the
//!   holder the core started was reached ([`holder_reached`]), and whether
//!   an adopted holder drains ([`adoption`]).
//! - **Sessions:** what a client's leaving ([`gone_fate`]), a program's exit
//!   ([`exit_fate`]) and the end of the grace period ([`grace_fate`]) do to
//!   a session, which `ATTACH`es are refused ([`attach_refusal`]) and which
//!   sessions a returning owner gets back ([`returning`]).
//! - **Process info:** which sessions a look reads ([`look`]), and whether
//!   a foreground program changed ([`foreground_changed`]).
//! - **Agent sessions** (OpenSpec change `agents-sessions`): when one is
//!   checkpointed ([`agent_checkpoint_due`]), that its spawns go only to a
//!   holder that negotiated H1 3 ([`placement`] with
//!   [`SpawnMode::Pipes`]), and what a client below protocol 4 is refused
//!   ([`shown`], [`agent_request_refusal`], and [`attach_refusal`] and
//!   [`returning`] by the session's [`SessionKind`]). With change
//!   `agents-session-recovery`: what an agent's end does ([`end_fate`]),
//!   what a client's leaving and a `DETACH` do to an ended one
//!   ([`ended_gone_fate`], [`detach_ends`]), and which `AGENT_RESTART`s are
//!   refused ([`restart_refusal`]). With change `agents-log-paging`: how
//!   a checkpoint is written, whole or appended to what the holder stores
//!   ([`Stored`], [`agent_checkpoint_form`]), the budget of a replay's or
//!   a fetch's page ([`page_budget`]), and which `AGENT_FETCH`es are
//!   refused ([`agent_fetch_refusal`]).

use std::fmt;
use std::time::{Duration, Instant};

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::{Hello, Policy};
use brindle_remote_protocol::holder::{MAX_CHECKPOINT_LEN, SpawnMode};
use brindle_transcript::PAGE_BYTES;

use super::clients::ClientToken;
use super::sessions::{CHECKPOINT_AFTER, Life, Ownership};
use crate::acp::reduce::Moment;
use super::{CLIENT_QUEUE_HIGH, CLIENT_QUEUE_LOW, MAX_LINGER, RECONNECT_GRACE, UNUSED_LIMIT};
use crate::pty::Foreground;
use crate::server::event_loop::Token;

/// Activity this recent at a look keeps the session flagged for the next.
pub(super) const ACTIVITY_SETTLE: Duration = Duration::from_millis(250);

/// Where the core is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    /// Adopting the holders found at startup: clients wait, so that
    /// returning clients find their sessions.
    Adopting,
    /// Accepting clients.
    Serving,
}

/// The core's lifecycle: when it starts accepting clients, the grace period
/// its adopted sessions' owners have to come back, and since when it has
/// had nothing to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Lifecycle {
    phase: Phase,
    /// While the owners of adopted sessions may still come back.
    grace_until: Option<Instant>,
    /// Since when it has been idle (nothing in [`Seen::busy`]).
    idle_since: Option<Instant>,
}

/// What the core's loop gathers for [`Lifecycle::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Seen {
    /// The holders found at startup are adopted, or have had long enough.
    pub(super) startup_adopted: bool,
    /// Some adopted session's owner hasn't come back yet.
    pub(super) awaiting_owner: bool,
    /// It has sessions, clients, connections still to say `HELLO`, spawns
    /// waiting for a holder, or a holder starting.
    pub(super) busy: bool,
}

/// What a [`Lifecycle::tick`] tells the loop to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Tick {
    pub(super) accept: Accept,
    /// The grace period is over: adopted sessions whose owners didn't come
    /// back meet their [`grace_fate`].
    pub(super) end_grace: bool,
}

/// Whether the core starts accepting clients at this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Accept {
    No,
    /// From now on; with `grace`, the grace period started too.
    Now { grace: bool },
}

/// What is ending the core, for [`Lifecycle::should_exit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ending {
    No,
    /// A `STOP`: it exits once every session has ended.
    Stopping { sessions_left: bool },
    /// A `STEP_DOWN`: it exits at once, for the next core.
    SteppingDown,
}

impl Lifecycle {
    /// A core that has just started: adopting, with no grace period, and
    /// not idle yet.
    pub(super) fn new() -> Self {
        Self {
            phase: Phase::Adopting,
            grace_until: None,
            idle_since: None,
        }
    }

    pub(super) fn accepting(&self) -> bool {
        match self.phase {
            Phase::Adopting => false,
            Phase::Serving => true,
        }
    }

    /// One turn of the loop at `now`. Clients are accepted once the startup
    /// adoption is done, and then, when an adopted session waits for its
    /// owner, the grace period of [`RECONNECT_GRACE`] starts. The grace
    /// period ends once. The idle time counts from the first tick at which
    /// the core isn't [`Seen::busy`].
    pub(super) fn tick(&mut self, now: Instant, seen: Seen) -> Tick {
        let accept = match self.phase {
            Phase::Adopting if seen.startup_adopted => {
                self.phase = Phase::Serving;
                let grace = seen.awaiting_owner;
                if grace {
                    self.grace_until = Some(now + RECONNECT_GRACE);
                }
                Accept::Now { grace }
            }
            Phase::Adopting | Phase::Serving => Accept::No,
        };
        let end_grace = self.grace_until.is_some_and(|until| now >= until);
        if end_grace {
            self.grace_until = None;
        }
        self.idle_since = match (seen.busy, self.idle_since) {
            (true, _) => None,
            (false, Some(since)) => Some(since),
            (false, None) => Some(now),
        };
        Tick { accept, end_grace }
    }

    /// A holder found after startup gave the core sessions at `now`: their
    /// owners get a grace period of their own, which never shortens one
    /// already running. While the core is still adopting, the grace period
    /// that starts with accepting covers them.
    pub(super) fn holder_adopted(&mut self, now: Instant) {
        match self.phase {
            Phase::Adopting => {}
            Phase::Serving => {
                let until = now + RECONNECT_GRACE;
                self.grace_until = Some(self.grace_until.map_or(until, |grace| grace.max(until)));
            }
        }
    }

    /// Whether the core exits now: at once after a `STEP_DOWN`, once a
    /// `STOP` has ended every session, or after [`UNUSED_LIMIT`] idle.
    pub(super) fn should_exit(&self, now: Instant, ending: Ending) -> bool {
        match ending {
            Ending::SteppingDown | Ending::Stopping { sessions_left: false } => true,
            Ending::Stopping { sessions_left: true } | Ending::No => self
                .idle_since
                .is_some_and(|since| now.duration_since(since) >= UNUSED_LIMIT),
        }
    }
}

/// What a client's `HELLO` asked for, as the core grants it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ClientOptions {
    /// How long its `kill` sessions wait for it once its connection ends:
    /// its `linger_secs`, at most [`MAX_LINGER`].
    pub(super) linger: Duration,
    /// It is told of its sessions' foreground programs (`PROCESS`): it
    /// asked, and the connection speaks protocol 2 or later.
    pub(super) process_info: bool,
}

/// What a `HELLO` that negotiated `protocol` gets.
pub(super) fn client_options(protocol: u32, hello: &Hello) -> ClientOptions {
    ClientOptions {
        linger: Duration::from_secs(hello.linger_secs.into()).min(MAX_LINGER),
        process_info: protocol >= 2 && hello.process_info,
    }
}

/// The H1 versions a `WELCOME` names: those of the holders that have
/// welcomed the core (`None` for one that hasn't), each once, in order.
pub(super) fn holder_protocols(holders: impl IntoIterator<Item = Option<u32>>) -> Vec<u32> {
    let mut protocols: Vec<u32> = holders.into_iter().flatten().collect();
    protocols.sort_unstable();
    protocols.dedup();
    protocols
}

/// A mark a client's queue crossed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mark {
    /// Past [`CLIENT_QUEUE_HIGH`]: its sessions are paused in their holders.
    Pause,
    /// Back under [`CLIENT_QUEUE_LOW`]: they are resumed.
    Resume,
}

/// The mark a client's queue of `queued` bytes crossed, given whether it
/// was `over` its marks: it goes over past the high mark and stays over
/// until it is under the low one (`process R9`).
pub(super) fn queue_mark(over: bool, queued: usize) -> Option<Mark> {
    match over {
        false if queued > CLIENT_QUEUE_HIGH => Some(Mark::Pause),
        true if queued < CLIENT_QUEUE_LOW => Some(Mark::Resume),
        false | true => None,
    }
}

/// The first H1 version with sessions on plain pipes (`SPAWN_PIPES`).
pub(super) const PIPES_PROTOCOL: u32 = 3;

/// What [`placement`] reads of a holder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HolderView<'a> {
    /// The build its files name; `None` when their name says none.
    pub(super) named: Option<&'a str>,
    pub(super) stage: Stage<'a>,
    /// The H1 version it negotiated, once it has welcomed the core.
    pub(super) protocol: Option<u32>,
}

/// How far the core has got with a holder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage<'a> {
    /// Connected; its `WELCOME` hasn't come yet.
    Connected,
    /// It welcomed the core as a holder of `build`; its sessions'
    /// checkpoints and tails are still coming.
    Adopting { build: &'a str },
    /// It and its sessions are adopted.
    Adopted { build: &'a str },
}

impl HolderView<'_> {
    /// Whether it is of `build`: by its `WELCOME`, or before that by the
    /// name of its files.
    fn of_build(&self, build: &str) -> bool {
        match self.stage {
            Stage::Connected => self.named == Some(build),
            Stage::Adopting { build: own } | Stage::Adopted { build: own } => own == build,
        }
    }

    /// Whether spawns of `mode` may go to it: it is adopted, of `build`,
    /// and for pipe sessions it negotiated [`PIPES_PROTOCOL`] (a holder of
    /// the core's build always does; the check keeps a pipe spawn from
    /// ever reaching a holder that would drop it).
    fn takes_spawns(&self, build: &str, mode: SpawnMode) -> bool {
        let runs = match mode {
            SpawnMode::Pty => true,
            SpawnMode::Pipes => self
                .protocol
                .is_some_and(|protocol| protocol >= PIPES_PROTOCOL),
        };
        match self.stage {
            Stage::Adopted { build: own } => own == build && runs,
            Stage::Connected | Stage::Adopting { .. } => false,
        }
    }
}

/// Where the spawns waiting for a holder go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Placement {
    /// To this one of the holders given.
    Spawn(usize),
    /// The core starts a holder of its build.
    Start,
    /// They wait: a holder of the core's build hasn't welcomed it yet, or
    /// one is starting.
    Wait,
}

/// Where the core's waiting spawns of `mode` go: to the first adopted
/// holder of its `build` in `holders`' order (for [`SpawnMode::Pipes`],
/// one that negotiated [`PIPES_PROTOCOL`]), else they wait for one of its
/// build that is connected but hasn't welcomed the core yet, or for the
/// one `starting`; with neither, the core starts one.
pub(super) fn placement(
    holders: &[HolderView],
    build: &str,
    starting: bool,
    mode: SpawnMode,
) -> Placement {
    if let Some(index) = holders
        .iter()
        .position(|holder| holder.takes_spawns(build, mode))
    {
        return Placement::Spawn(index);
    }
    let coming = holders
        .iter()
        .any(|holder| holder.of_build(build) && holder.stage == Stage::Connected);
    if coming || starting {
        Placement::Wait
    } else {
        Placement::Start
    }
}

/// After the holder the core started said it was listening and the core
/// looked for it: whether a holder of `build` takes spawns or is still
/// being adopted. If not, the waiting spawns fail.
pub(super) fn holder_reached(holders: &[HolderView], build: &str) -> bool {
    holders.iter().any(|holder| {
        // A holder of the core's build runs both kinds of session.
        holder.takes_spawns(build, SpawnMode::Pty)
            || (holder.of_build(build) && !matches!(holder.stage, Stage::Adopted { .. }))
    })
}

/// What becomes of a holder that welcomed the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Adoption {
    /// It is of the core's build, and takes new sessions.
    Keep,
    /// It is of another build: told to `DRAIN`, it takes no new sessions
    /// and exits after its last one.
    Drain,
}

pub(super) fn adoption(holder_build: &str, core_build: &str) -> Adoption {
    if holder_build == core_build {
        Adoption::Keep
    } else {
        Adoption::Drain
    }
}

/// What becomes of a session whose client's connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fate {
    HangUp,
    /// It keeps running this long, waiting for its client to come back.
    Linger(Duration),
    /// It keeps running with no client.
    Keep,
}

/// A client's connection ended: its `detach` sessions keep running, and its
/// `kill` sessions are hung up, or wait for it the `linger` it asked for.
/// A `kill` session still `starting` is hung up even then: its client never
/// learns of it.
pub(super) fn gone_fate(policy: Policy, starting: bool, linger: Duration) -> Fate {
    match policy {
        Policy::Kill if linger.is_zero() || starting => Fate::HangUp,
        Policy::Kill => Fate::Linger(linger),
        Policy::Detach => Fate::Keep,
    }
}

/// What a session's program exiting does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExitFate {
    /// Its owner may still come back, so the exit is kept for it.
    KeepForOwner,
    /// Its client, if it has one, is told (after the replay it waits for,
    /// if any), and the session is gone.
    Report,
}

/// An adopted or lingering session whose owner may still come back keeps
/// its exit for it, unless it was hung up: then the hang-up ended it.
pub(super) fn exit_fate(ownership: &Ownership, life: &Life) -> ExitFate {
    if ownership.owner_may_return() && !life.is_hung_up() {
        ExitFate::KeepForOwner
    } else {
        ExitFate::Report
    }
}

/// What the end of the grace period does to an adopted session whose
/// owner didn't come back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GraceFate {
    /// Its program exited meanwhile: it is forgotten.
    Forget,
    /// A `kill` session: hung up.
    HangUp,
    /// A `detach` session: it keeps running with no client.
    LeaveDetached,
}

/// What an agent session's agent ending (exiting, or lost with its holder)
/// does (OpenSpec change `agents-session-recovery`, decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EndFate {
    /// The session stays, ended ([`Life::AgentEnded`]), for its client to
    /// continue the conversation, or for its owner to come back to.
    Keep,
    /// As [`ExitFate::KeepForOwner`]: only the exit is kept, for the owner.
    KeepForOwner,
    /// As [`ExitFate::Report`].
    Report,
}

/// What an agent session's end does: one that can be `continuable` and
/// isn't being hung up is kept while a client has it attached or its owner
/// may come back; any other ends as [`exit_fate`] says.
pub(super) fn end_fate(ownership: &Ownership, life: &Life, continuable: bool) -> EndFate {
    let wanted = ownership.client().is_some() || ownership.owner_may_return();
    if continuable && wanted && !life.is_hung_up() {
        return EndFate::Keep;
    }
    match exit_fate(ownership, life) {
        ExitFate::KeepForOwner => EndFate::KeepForOwner,
        ExitFate::Report => EndFate::Report,
    }
}

/// What a client's connection ending does to an ended agent session
/// ([`Life::AgentEnded`]): it waits for a lingering client, as a running
/// `kill` session does, and otherwise ends ([`Fate::HangUp`]), since only
/// an owner that may come back keeps one (decision 3).
pub(super) fn ended_gone_fate(policy: Policy, linger: Duration) -> Fate {
    match gone_fate(policy, false, linger) {
        Fate::Linger(linger) => Fate::Linger(linger),
        Fate::HangUp | Fate::Keep => Fate::HangUp,
    }
}

/// Whether a client's `DETACH` ends the session rather than leaving it
/// detached: an ended agent session's does (decision 3), since no one is
/// left to continue it.
pub(super) fn detach_ends(life: &Life) -> bool {
    match life {
        Life::AgentEnded { .. } => true,
        Life::Starting { .. }
        | Life::Running { .. }
        | Life::HungUp { .. }
        | Life::ExitedAwaitingOwner { .. } => false,
    }
}

/// Why an `AGENT_RESTART` is refused once the client and its protocol are
/// vetted; its text is the `ERROR`'s, after the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RestartRefusal {
    /// The agent runs, or is starting.
    NotEnded,
    /// The conversation can't be continued (unreachable while [`end_fate`]
    /// ends such sessions).
    CannotContinue,
    NoCommand,
}

impl fmt::Display for RestartRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnded => f.write_str("the agent hasn't ended"),
            Self::CannotContinue => f.write_str("the agent can't continue this conversation"),
            Self::NoCommand => f.write_str("AGENT_RESTART needs a command"),
        }
    }
}

/// Why an `AGENT_RESTART` of `command` for a session in `life` is refused:
/// only an ended agent whose conversation is `continuable`, or a running
/// one that `needs_login` (OpenSpec change `agents-login`, decision 8), is
/// started again, and it needs a command.
pub(super) fn restart_refusal(
    life: &Life,
    continuable: bool,
    needs_login: bool,
    command: &str,
) -> Option<RestartRefusal> {
    match life {
        Life::AgentEnded { .. } if !continuable => return Some(RestartRefusal::CannotContinue),
        Life::AgentEnded { .. } => {}
        Life::Running { .. } if needs_login => {}
        Life::Starting { .. }
        | Life::Running { .. }
        | Life::HungUp { .. }
        | Life::ExitedAwaitingOwner { .. } => return Some(RestartRefusal::NotEnded),
    }
    command
        .trim()
        .is_empty()
        .then_some(RestartRefusal::NoCommand)
}

pub(super) fn grace_fate(policy: Policy, life: &Life) -> GraceFate {
    match (life, policy) {
        (
            Life::ExitedAwaitingOwner { .. } | Life::AgentEnded { .. },
            Policy::Kill | Policy::Detach,
        ) => GraceFate::Forget,
        (Life::Starting { .. } | Life::Running { .. } | Life::HungUp { .. }, Policy::Kill) => {
            GraceFate::HangUp
        }
        (Life::Starting { .. } | Life::Running { .. } | Life::HungUp { .. }, Policy::Detach) => {
            GraceFate::LeaveDetached
        }
    }
}

/// The first client protocol with agent sessions.
pub(super) const AGENT_PROTOCOL: u32 = 4;

/// What a session runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionKind {
    /// A program on a PTY, with an emulator in the core.
    Terminal,
    /// An ACP agent on plain pipes, with its transcript in the core.
    Agent,
}

/// Whether a client on `protocol` is shown sessions of `kind`: listed,
/// given back when it returns, attachable. Below [`AGENT_PROTOCOL`] it
/// knows nothing of agent sessions (decision 9), so they don't exist for
/// it; `KILL` and `STOP` still end them, as any session.
pub(super) fn shown(kind: SessionKind, protocol: u32) -> bool {
    match kind {
        SessionKind::Terminal => true,
        SessionKind::Agent => protocol >= AGENT_PROTOCOL,
    }
}

/// When the core checkpoints an agent session (decision 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AgentCheckpoint {
    /// After `acp::reduce::step`, with the moment it asked to checkpoint
    /// at: a start, a request about to be written (a prompt's entry and
    /// id reach the holder before its bytes), a turn's end, a change of
    /// the queue slot, a permission request arriving or answered, the agent
    /// stopping.
    Stepped(Option<Moment>),
    /// The core is about to step down for an upgrade.
    StepDown,
    /// The session's client detached it.
    Detach,
}

/// Whether an agent session with `since` bytes of stdout fed since its
/// last checkpoint is checkpointed now: at every moment the reducer names,
/// before a step-down and on a detach, and otherwise past
/// [`CHECKPOINT_AFTER`] (512 KiB), as a terminal is.
pub(super) fn agent_checkpoint_due(at: AgentCheckpoint, since: u64) -> bool {
    match at {
        AgentCheckpoint::Stepped(Some(
            Moment::Started
            | Moment::BeforeRequest
            | Moment::TurnEnded
            | Moment::QueueChanged
            | Moment::Permission
            | Moment::Stopped
            | Moment::Login,
        ))
        | AgentCheckpoint::StepDown
        | AgentCheckpoint::Detach => true,
        AgentCheckpoint::Stepped(None) => since > CHECKPOINT_AFTER,
    }
}

/// What an agent session's holder stores of its checkpoint, as far as the
/// core knows (OpenSpec change `agents-log-paging`, decision 9). Kept on
/// the core's `Agent` only: a core restart starts again at `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))] // until the core uses it (agents-log-paging 3.2-3.4)
pub(super) enum Stored {
    /// Nothing written to this holder session yet, an adopted session, a
    /// restarted one, or the holder sent an `ERROR` naming the session:
    /// only a whole checkpoint is safe.
    Unknown,
    /// A base of `base` bytes (header included), then `appended` bytes of
    /// segments, the last numbered `seq`.
    Known { base: u64, appended: u64, seq: u32 },
}

#[cfg_attr(not(test), allow(dead_code))] // until the core uses it (agents-log-paging 3.2-3.4)
impl Stored {
    /// The sequence number the next segment takes.
    pub(super) fn next_seq(self) -> u32 {
        match self {
            Self::Unknown => 1,
            Self::Known { seq, .. } => seq.saturating_add(1),
        }
    }

    /// After a whole checkpoint of `len` bytes.
    pub(super) fn whole(len: u64) -> Self {
        Self::Known {
            base: len,
            appended: 0,
            seq: 0,
        }
    }

    /// After a segment of `len` bytes is appended; `Unknown` stays so.
    pub(super) fn appended(self, len: u64) -> Self {
        match self {
            Self::Unknown => Self::Unknown,
            Self::Known {
                base,
                appended,
                seq,
            } => Self::Known {
                base,
                appended: appended.saturating_add(len),
                seq: seq.saturating_add(1),
            },
        }
    }
}

/// How a due agent checkpoint is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))] // until the core uses it (agents-log-paging 3.2-3.4)
pub(super) enum CheckpointForm {
    /// A header and a base: `CHECKPOINT_BEGIN`, replacing what is stored.
    Whole,
    /// One segment record: `CHECKPOINT_APPEND`, added to what is stored.
    Append,
}

/// Whether a segment is worth encoding for a checkpoint due `at`: what is
/// stored is known, it isn't a step-down, nothing was trimmed since, and
/// the holder is of the core's build. Otherwise the checkpoint is whole
/// whatever the segment's length.
#[cfg_attr(not(test), allow(dead_code))] // until the core uses it (agents-log-paging 3.2-3.4)
pub(super) fn agent_appendable(
    stored: Stored,
    at: AgentCheckpoint,
    trimmed: bool,
    holder: Adoption,
) -> bool {
    let known = match stored {
        Stored::Unknown => false,
        Stored::Known { .. } => true,
    };
    let stepping_down = match at {
        AgentCheckpoint::StepDown => true,
        AgentCheckpoint::Stepped(_) | AgentCheckpoint::Detach => false,
    };
    let own_build = match holder {
        Adoption::Keep => true,
        Adoption::Drain => false,
    };
    known && !stepping_down && !trimmed && own_build
}

/// How an agent checkpoint due `at` is written (decision 9): appended
/// when [`agent_appendable`] and the segment, `segment_len` bytes (`None`
/// when none was encoded or it was too large), keeps the segments no
/// larger than the base and the whole blob within
/// [`MAX_CHECKPOINT_LEN`]; whole otherwise.
#[cfg_attr(not(test), allow(dead_code))] // until the core uses it (agents-log-paging 3.2-3.4)
pub(super) fn agent_checkpoint_form(
    stored: Stored,
    at: AgentCheckpoint,
    segment_len: Option<u64>,
    trimmed: bool,
    holder: Adoption,
) -> CheckpointForm {
    if !agent_appendable(stored, at, trimmed, holder) {
        return CheckpointForm::Whole;
    }
    let (Stored::Known { base, appended, .. }, Some(segment)) = (stored, segment_len) else {
        return CheckpointForm::Whole;
    };
    let segments = appended.saturating_add(segment);
    if segments > base || base.saturating_add(segments) > MAX_CHECKPOINT_LEN {
        CheckpointForm::Whole
    } else {
        CheckpointForm::Append
    }
}

/// The page a replay or a fetch sends, in bytes of entries: what the
/// client `asked` for (`ATTACH.page`, `AGENT_FETCH.max_bytes`), at most
/// [`PAGE_BYTES`], which is also the budget when it asked nothing
/// (decision 5). Even 0 gives the newest entry.
#[cfg_attr(not(test), allow(dead_code))] // until the core uses it (agents-log-paging 3.2-3.4)
pub(super) fn page_budget(asked: Option<u32>) -> usize {
    asked.map_or(PAGE_BYTES, |asked| {
        usize::try_from(asked).map_or(PAGE_BYTES, |asked| asked.min(PAGE_BYTES))
    })
}

/// What [`agent_fetch_refusal`] reads of the session a fetch names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))] // until the core uses it (agents-log-paging 3.2-3.4)
pub(super) struct FetchView {
    pub(super) kind: SessionKind,
    /// It is attached to the client that fetches.
    pub(super) attached_here: bool,
    /// That client still waits for its replay (an adoption's rebuild
    /// included).
    pub(super) replay_pending: bool,
}

/// Why an `AGENT_FETCH` is refused; its text is the `ERROR`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))] // until the core uses it (agents-log-paging 3.2-3.4)
pub(super) enum FetchRefusal {
    NeedsAgentProtocol(NeedsAgentProtocol),
    NoSession(SessionId),
    NotAgent(SessionId),
    NotAttached(SessionId),
    ReplayPending(SessionId),
}

impl fmt::Display for FetchRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedsAgentProtocol(refusal) => refusal.fmt(f),
            Self::NoSession(id) => write!(f, "no session {id}"),
            Self::NotAgent(id) => write!(f, "session {id} isn't an agent session"),
            Self::NotAttached(id) => write!(f, "session {id} isn't attached to this client"),
            Self::ReplayPending(id) => write!(f, "session {id}'s replay hasn't ended"),
        }
    }
}

/// Why a client on `protocol` can't fetch from session `id` (`session`,
/// if there is one): only the client it is attached to may, once that
/// client has its replay (decision 5). A fetch reads the core's log, so
/// the agent's phase doesn't matter.
#[cfg_attr(not(test), allow(dead_code))] // until the core uses it (agents-log-paging 3.2-3.4)
pub(super) fn agent_fetch_refusal(
    id: SessionId,
    session: Option<FetchView>,
    protocol: u32,
) -> Option<FetchRefusal> {
    if let Some(refusal) = agent_request_refusal(protocol, AgentRequest::Fetch) {
        return Some(FetchRefusal::NeedsAgentProtocol(refusal));
    }
    let Some(session) = session else {
        return Some(FetchRefusal::NoSession(id));
    };
    match session.kind {
        SessionKind::Terminal => return Some(FetchRefusal::NotAgent(id)),
        SessionKind::Agent => {}
    }
    if !session.attached_here {
        return Some(FetchRefusal::NotAttached(id));
    }
    session.replay_pending.then_some(FetchRefusal::ReplayPending(id))
}

/// Why a `SPAWN_AGENT` of `command` is refused before anything starts: an
/// agent session has no login-shell default, so it needs a command.
pub(super) fn agent_spawn_refusal(command: &str) -> Option<&'static str> {
    command
        .trim()
        .is_empty()
        .then_some("an agent session needs a command")
}

/// A client protocol 4 message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AgentRequest {
    Spawn,
    Prompt,
    Cancel,
    Permission,
    Unqueue,
    Resend,
    Restart,
    Login,
    #[cfg_attr(not(test), allow(dead_code))] // until the core uses it (agents-log-paging 3.3)
    Fetch,
}

impl AgentRequest {
    fn name(self) -> &'static str {
        match self {
            Self::Spawn => "SPAWN_AGENT",
            Self::Prompt => "AGENT_PROMPT",
            Self::Cancel => "AGENT_CANCEL",
            Self::Permission => "AGENT_PERMISSION",
            Self::Unqueue => "AGENT_UNQUEUE",
            Self::Resend => "AGENT_RESEND",
            Self::Restart => "AGENT_RESTART",
            Self::Login => "AGENT_LOGIN",
            Self::Fetch => "AGENT_FETCH",
        }
    }
}

/// An agent message on a connection below [`AGENT_PROTOCOL`]; its text is
/// the `ERROR`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NeedsAgentProtocol(AgentRequest);

impl fmt::Display for NeedsAgentProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} needs client protocol {AGENT_PROTOCOL}", self.0.name())
    }
}

/// Why a client on `protocol` can't send `request`: only a connection
/// that negotiated [`AGENT_PROTOCOL`] may (`process R5`; a correct client
/// below it never sends one).
pub(super) fn agent_request_refusal(
    protocol: u32,
    request: AgentRequest,
) -> Option<NeedsAgentProtocol> {
    (protocol < AGENT_PROTOCOL).then_some(NeedsAgentProtocol(request))
}

/// What [`attach_refusal`] and [`returning`] read of a session.
#[derive(Debug, Clone, Copy)]
pub(super) struct SessionView<'a> {
    pub(super) life: &'a Life,
    pub(super) ownership: &'a Ownership,
    pub(super) owner: Option<&'a ClientToken>,
    pub(super) kind: SessionKind,
}

/// Why an `ATTACH` is refused; its text is the `ERROR`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refusal {
    NoSession(SessionId),
    /// An agent session, to a client below [`AGENT_PROTOCOL`].
    NeedsAgentProtocol(SessionId),
    AttachedHere(SessionId),
    AttachedElsewhere(SessionId),
    Ending(SessionId),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSession(id) => write!(f, "no session {id}"),
            Self::NeedsAgentProtocol(id) => write!(
                f,
                "session {id} is an agent session, which needs client protocol {AGENT_PROTOCOL}"
            ),
            Self::AttachedHere(id) => write!(f, "session {id} is already attached to this client"),
            Self::AttachedElsewhere(id) => write!(f, "session {id} is attached to another client"),
            Self::Ending(id) => write!(f, "session {id} is ending"),
        }
    }
}

/// Why `client`, on client `protocol`, can't attach session `id`
/// (`session`, if there is one): only a session it is [`shown`] with no
/// client that isn't being hung up is given. An agent session is refused
/// to a client below protocol 4 before anything else is said of it.
pub(super) fn attach_refusal(
    id: SessionId,
    session: Option<SessionView<'_>>,
    client: Token,
    protocol: u32,
) -> Option<Refusal> {
    let session = match session {
        None => return Some(Refusal::NoSession(id)),
        Some(session) => session,
    };
    if !shown(session.kind, protocol) {
        return Some(Refusal::NeedsAgentProtocol(id));
    }
    match session.ownership.client() {
        Some(attached) if attached == client => Some(Refusal::AttachedHere(id)),
        Some(_) => Some(Refusal::AttachedElsewhere(id)),
        None if session.life.is_hung_up() => Some(Refusal::Ending(id)),
        None => None,
    }
}

/// The sessions a client with `token`, on client `protocol`, gets back
/// when it connects: those it owns, is [`shown`], that have no client. One
/// still starting isn't given back (its `SPAWN`'s answer went to a
/// connection that is gone, so the client doesn't know it), nor one being
/// hung up.
pub(super) fn returning(
    sessions: &[(SessionId, SessionView<'_>)],
    token: &ClientToken,
    protocol: u32,
) -> Vec<SessionId> {
    sessions
        .iter()
        .filter(|(_, session)| {
            shown(session.kind, protocol)
                && session.ownership.client().is_none()
                && !session.life.is_starting()
                && !session.life.is_hung_up()
                && session.owner == Some(token)
        })
        .map(|&(id, _)| id)
        .collect()
}

/// What a look at the foreground programs does with a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Look {
    /// It had output or input that has settled: it is looked at, and isn't
    /// flagged for the next look.
    Read,
    /// It had activity younger than [`ACTIVITY_SETTLE`]: it is looked at,
    /// and stays flagged for the next look, since the echo of the key that
    /// starts a program comes before the shell makes that program the
    /// foreground.
    Keep,
    /// It had no output or input since the last look.
    Skip,
}

/// Whether a look at `now` reads a session that is `active` (flagged since
/// the last look), with its last output or input at `active_at`.
pub(super) fn look(active: bool, active_at: Instant, now: Instant) -> Look {
    if !active {
        Look::Skip
    } else if now.saturating_duration_since(active_at) < ACTIVITY_SETTLE {
        Look::Keep
    } else {
        Look::Read
    }
}

/// Whether the foreground program `seen` is news to a client last told of
/// `last`.
pub(super) fn foreground_changed(last: Option<&Foreground>, seen: &Foreground) -> bool {
    last != Some(seen)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU32;

    use brindle_remote_protocol::client::Spawn;

    use super::super::sessions::SpawnRequest;
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    const QUIET: Seen = Seen {
        startup_adopted: true,
        awaiting_owner: false,
        busy: false,
    };

    // 3.1: the lifecycle.

    #[test]
    fn clients_are_accepted_once_the_startup_adoption_is_done() {
        let t0 = Instant::now();
        let mut life = Lifecycle::new();
        let adopting = Seen {
            startup_adopted: false,
            busy: true,
            ..QUIET
        };
        assert_eq!(life.tick(t0, adopting).accept, Accept::No);
        assert!(!life.accepting());
        let done = Seen { busy: true, ..QUIET };
        assert_eq!(life.tick(t0 + secs(1), done).accept, Accept::Now { grace: false });
        assert!(life.accepting());
        // Once.
        assert_eq!(life.tick(t0 + secs(2), done).accept, Accept::No);
        assert!(life.accepting());
    }

    #[test]
    fn the_grace_period_starts_with_accepting_and_ends_once() {
        let t0 = Instant::now();
        let mut life = Lifecycle::new();
        let awaiting = Seen {
            awaiting_owner: true,
            busy: true,
            ..QUIET
        };
        let tick = life.tick(t0, awaiting);
        assert_eq!(tick, Tick { accept: Accept::Now { grace: true }, end_grace: false });
        let before = t0 + RECONNECT_GRACE - Duration::from_millis(1);
        assert!(!life.tick(before, awaiting).end_grace);
        assert!(life.tick(t0 + RECONNECT_GRACE, awaiting).end_grace);
        assert!(!life.tick(t0 + RECONNECT_GRACE + secs(1), awaiting).end_grace);

        // Without an owner to wait for there is none.
        let mut life = Lifecycle::new();
        life.tick(t0, Seen { busy: true, ..QUIET });
        assert!(!life.tick(t0 + secs(60), QUIET).end_grace);
    }

    #[test]
    fn a_holder_adopted_later_gives_its_owners_a_grace_period_of_their_own() {
        let t0 = Instant::now();
        let busy = Seen { busy: true, ..QUIET };

        // While adopting, the grace period that starts with accepting is
        // theirs.
        let mut life = Lifecycle::new();
        life.holder_adopted(t0);
        life.tick(t0 + secs(1), Seen { startup_adopted: false, ..busy });
        assert!(!life.tick(t0 + RECONNECT_GRACE + secs(1), busy).end_grace);
        assert!(!life.tick(t0 + secs(60), busy).end_grace);

        // Once serving, one starts.
        let mut life = Lifecycle::new();
        life.tick(t0, busy);
        life.holder_adopted(t0 + secs(1));
        assert!(!life.tick(t0 + RECONNECT_GRACE, busy).end_grace);
        assert!(life.tick(t0 + secs(1) + RECONNECT_GRACE, busy).end_grace);

        // A later one never shortens one already running.
        let mut life = Lifecycle::new();
        life.tick(t0, Seen { awaiting_owner: true, ..busy });
        life.holder_adopted(t0 + secs(5));
        life.holder_adopted(t0 + secs(1));
        assert!(!life.tick(t0 + RECONNECT_GRACE, busy).end_grace);
        assert!(life.tick(t0 + secs(5) + RECONNECT_GRACE, busy).end_grace);
    }

    #[test]
    fn an_idle_core_exits_after_the_unused_limit() {
        let t0 = Instant::now();
        let mut life = Lifecycle::new();
        life.tick(t0, Seen { busy: true, ..QUIET });
        assert!(!life.should_exit(t0 + secs(60), Ending::No), "busy");
        // Idle from its first idle tick.
        life.tick(t0 + secs(1), QUIET);
        life.tick(t0 + secs(2), QUIET);
        let limit = t0 + secs(1) + UNUSED_LIMIT;
        assert!(!life.should_exit(limit - Duration::from_millis(1), Ending::No));
        assert!(life.should_exit(limit, Ending::No));
        // Anything busy starts it over.
        life.tick(t0 + secs(3), Seen { busy: true, ..QUIET });
        assert!(!life.should_exit(limit, Ending::No));
        life.tick(t0 + secs(4), QUIET);
        assert!(!life.should_exit(limit, Ending::No));
        assert!(life.should_exit(t0 + secs(4) + UNUSED_LIMIT, Ending::No));

        // A core still adopting is idle too.
        let mut life = Lifecycle::new();
        life.tick(t0, Seen { startup_adopted: false, ..QUIET });
        assert!(life.should_exit(t0 + UNUSED_LIMIT, Ending::No));
    }

    #[test]
    fn a_stop_exits_once_the_sessions_ended_and_a_step_down_at_once() {
        let t0 = Instant::now();
        let mut life = Lifecycle::new();
        life.tick(t0, Seen { busy: true, ..QUIET });
        let cases = [
            (Ending::SteppingDown, true),
            (Ending::Stopping { sessions_left: false }, true),
            (Ending::Stopping { sessions_left: true }, false),
            (Ending::No, false),
        ];
        for (ending, exits) in cases {
            assert_eq!(life.should_exit(t0, ending), exits, "{ending:?}");
        }
        // A stopping core with sessions left still exits when idle long
        // enough, as any core does.
        life.tick(t0, QUIET);
        let stopping = Ending::Stopping { sessions_left: true };
        assert!(life.should_exit(t0 + UNUSED_LIMIT, stopping));
    }

    // 3.2: clients.

    #[test]
    fn a_hellos_linger_is_capped_and_process_info_needs_protocol_2() {
        let hello = |linger_secs, process_info| Hello {
            linger_secs,
            process_info,
            ..Hello::default()
        };
        let cases = [
            (1, hello(0, true), secs(0), false),
            (2, hello(0, true), secs(0), true),
            (3, hello(300, true), secs(300), true),
            (2, hello(300, false), secs(300), false),
            (2, hello(600, false), MAX_LINGER, false),
            (2, hello(u32::MAX, false), MAX_LINGER, false),
        ];
        for (protocol, hello, linger, process_info) in cases {
            assert_eq!(
                client_options(protocol, &hello),
                ClientOptions { linger, process_info },
                "{protocol} {hello:?}"
            );
        }
    }

    #[test]
    fn a_welcome_names_each_welcomed_holders_protocol_once() {
        assert_eq!(holder_protocols([]), Vec::<u32>::new());
        assert_eq!(holder_protocols([None]), Vec::<u32>::new());
        assert_eq!(holder_protocols([Some(2), None, Some(1), Some(2)]), [1, 2]);
    }

    #[test]
    fn a_clients_queue_pauses_past_the_high_mark_and_resumes_under_the_low() {
        let cases = [
            (false, 0, None),
            (false, CLIENT_QUEUE_HIGH, None),
            (false, CLIENT_QUEUE_HIGH + 1, Some(Mark::Pause)),
            (true, CLIENT_QUEUE_HIGH + 1, None),
            (true, CLIENT_QUEUE_LOW, None),
            (true, CLIENT_QUEUE_LOW - 1, Some(Mark::Resume)),
            (false, CLIENT_QUEUE_LOW - 1, None),
        ];
        for (over, queued, mark) in cases {
            assert_eq!(queue_mark(over, queued), mark, "over {over}, {queued} queued");
        }
    }

    // 3.2: holders.

    /// A holder that negotiated H1 3 once it welcomed the core.
    fn holder<'a>(named: Option<&'a str>, stage: Stage<'a>) -> HolderView<'a> {
        let protocol = match stage {
            Stage::Connected => None,
            Stage::Adopting { .. } | Stage::Adopted { .. } => Some(3),
        };
        HolderView { named, stage, protocol }
    }

    const MODES: [SpawnMode; 2] = [SpawnMode::Pty, SpawnMode::Pipes];

    #[test]
    fn spawns_go_to_the_first_adopted_holder_of_the_cores_build() {
        let other = holder(Some("b0"), Stage::Adopted { build: "b0" });
        let adopting = holder(Some("b1"), Stage::Adopting { build: "b1" });
        let own = holder(Some("b1"), Stage::Adopted { build: "b1" });
        // Its files may name another build than its WELCOME.
        let renamed = holder(Some("b0"), Stage::Adopted { build: "b1" });
        for mode in MODES {
            assert_eq!(placement(&[other, own], "b1", false, mode), Placement::Spawn(1));
            assert_eq!(placement(&[renamed, own], "b1", true, mode), Placement::Spawn(0));
            assert_eq!(placement(&[adopting, own], "b1", false, mode), Placement::Spawn(1));
        }
    }

    #[test]
    fn pipe_spawns_go_only_to_a_holder_that_negotiated_h1_3_else_one_starts() {
        let old = HolderView {
            named: Some("b1"),
            stage: Stage::Adopted { build: "b1" },
            protocol: Some(2),
        };
        let own = holder(Some("b1"), Stage::Adopted { build: "b1" });
        assert_eq!(placement(&[old, own], "b1", false, SpawnMode::Pipes), Placement::Spawn(1));
        assert_eq!(placement(&[old], "b1", false, SpawnMode::Pipes), Placement::Start);
        assert_eq!(placement(&[old], "b1", true, SpawnMode::Pipes), Placement::Wait);
        // A terminal's spawn goes to it as before.
        assert_eq!(placement(&[old, own], "b1", false, SpawnMode::Pty), Placement::Spawn(0));
    }

    #[test]
    fn without_one_they_wait_for_a_holder_coming_or_starting_else_one_starts() {
        let connected = holder(Some("b1"), Stage::Connected);
        let connected_other = holder(Some("b0"), Stage::Connected);
        let unnamed = holder(None, Stage::Connected);
        let adopting = holder(Some("b1"), Stage::Adopting { build: "b1" });
        let other = holder(Some("b0"), Stage::Adopted { build: "b0" });
        let cases: [(&[HolderView], bool, Placement); 8] = [
            (&[], false, Placement::Start),
            (&[], true, Placement::Wait),
            (&[connected], false, Placement::Wait),
            (&[connected_other, unnamed], false, Placement::Start),
            (&[other], false, Placement::Start),
            (&[other], true, Placement::Wait),
            // A holder of its build still adopting its sessions isn't
            // waited for: a holder starts.
            (&[adopting], false, Placement::Start),
            (&[adopting], true, Placement::Wait),
        ];
        for (holders, starting, expected) in cases {
            for mode in MODES {
                assert_eq!(
                    placement(holders, "b1", starting, mode),
                    expected,
                    "{holders:?} {starting} {mode:?}"
                );
            }
        }
    }

    #[test]
    fn a_started_holder_is_reached_once_one_of_the_build_takes_or_is_adopting() {
        let cases = [
            (vec![], false),
            (vec![holder(Some("b1"), Stage::Connected)], true),
            (vec![holder(Some("b0"), Stage::Connected)], false),
            (vec![holder(None, Stage::Connected)], false),
            (vec![holder(Some("b0"), Stage::Adopting { build: "b1" })], true),
            (vec![holder(Some("b1"), Stage::Adopting { build: "b0" })], false),
            (vec![holder(Some("b1"), Stage::Adopted { build: "b1" })], true),
            (vec![holder(Some("b1"), Stage::Adopted { build: "b0" })], false),
        ];
        for (holders, reached) in cases {
            assert_eq!(holder_reached(&holders, "b1"), reached, "{holders:?}");
        }
    }

    #[test]
    fn a_holder_of_another_build_drains() {
        assert_eq!(adoption("b1", "b1"), Adoption::Keep);
        assert_eq!(adoption("b0", "b1"), Adoption::Drain);
        assert_eq!(adoption("", "b1"), Adoption::Drain);
    }

    // 3.2: sessions.

    fn pid(pid: u32) -> NonZeroU32 {
        NonZeroU32::new(pid).unwrap()
    }

    fn spawn() -> SpawnRequest {
        SpawnRequest::Terminal(Spawn {
            req: 1,
            command: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            cols: 80,
            rows: 24,
            cell_w: 0,
            cell_h: 0,
            scrollback: 100,
            policy: Policy::Kill,
            profile: None,
        })
    }

    /// One of each kind of life.
    fn lives() -> [Life; 6] {
        [
            Life::Starting { spawn: spawn(), hung_up: false },
            Life::Starting { spawn: spawn(), hung_up: true },
            Life::Running { pid: pid(9) },
            Life::HungUp { pid: pid(9) },
            Life::ExitedAwaitingOwner { code: Some(0), pid: Some(pid(9)) },
            Life::AgentEnded { code: Some(1) },
        ]
    }

    /// One of each kind of ownership, and whether its owner may come back.
    fn ownerships(t0: Instant) -> [Ownership; 4] {
        [
            Ownership::Attached(3),
            Ownership::Lingering { until: t0, since: t0 },
            Ownership::AwaitingOwner { since: t0 },
            Ownership::Detached { since: t0 },
        ]
    }

    // agents-session-recovery 3.1.

    #[test]
    fn a_continuable_end_is_kept_while_attached_or_owned_unless_hung_up() {
        let t0 = Instant::now();
        for ownership in ownerships(t0) {
            for life in lives() {
                for continuable in [false, true] {
                    let wanted = match ownership {
                        Ownership::Attached(_)
                        | Ownership::Lingering { .. }
                        | Ownership::AwaitingOwner { .. } => true,
                        Ownership::Detached { .. } => false,
                    };
                    let expected = if continuable && wanted && !life.is_hung_up() {
                        EndFate::Keep
                    } else {
                        match exit_fate(&ownership, &life) {
                            ExitFate::KeepForOwner => EndFate::KeepForOwner,
                            ExitFate::Report => EndFate::Report,
                        }
                    };
                    assert_eq!(
                        end_fate(&ownership, &life, continuable),
                        expected,
                        "{ownership:?} {life:?} {continuable}"
                    );
                }
            }
        }
        // Spelled out for a running agent.
        let running = Life::Running { pid: pid(9) };
        let hung_up = Life::HungUp { pid: pid(9) };
        let [attached, lingering, awaiting, detached] = ownerships(t0);
        assert_eq!(end_fate(&attached, &running, true), EndFate::Keep);
        assert_eq!(end_fate(&lingering, &running, true), EndFate::Keep);
        assert_eq!(end_fate(&awaiting, &running, true), EndFate::Keep);
        assert_eq!(end_fate(&detached, &running, true), EndFate::Report);
        assert_eq!(end_fate(&attached, &hung_up, true), EndFate::Report);
        assert_eq!(end_fate(&attached, &running, false), EndFate::Report);
        assert_eq!(end_fate(&lingering, &running, false), EndFate::KeepForOwner);
        assert_eq!(end_fate(&lingering, &hung_up, true), EndFate::Report);
    }

    #[test]
    fn an_ended_agent_waits_only_for_a_lingering_client_and_a_detach_ends_it() {
        let cases = [
            (Policy::Kill, secs(0), Fate::HangUp),
            (Policy::Kill, secs(300), Fate::Linger(secs(300))),
            (Policy::Detach, secs(0), Fate::HangUp),
            (Policy::Detach, secs(300), Fate::HangUp),
        ];
        for (policy, linger, fate) in cases {
            assert_eq!(ended_gone_fate(policy, linger), fate, "{policy:?} {linger:?}");
        }
        for life in lives() {
            let ends = matches!(life, Life::AgentEnded { .. });
            assert_eq!(detach_ends(&life), ends, "{life:?}");
        }
    }

    #[test]
    fn a_restart_is_refused_unless_the_agent_ended_continuably_or_needs_a_login_with_a_command() {
        for life in lives() {
            for continuable in [false, true] {
                for needs_login in [false, true] {
                    for command in ["", " ", "fake-agent"] {
                        let no_command = command.trim().is_empty();
                        let expected = match (&life, continuable, needs_login, no_command) {
                            (Life::AgentEnded { .. }, true, _, false)
                            | (Life::Running { .. }, _, true, false) => None,
                            (Life::AgentEnded { .. }, true, _, true)
                            | (Life::Running { .. }, _, true, true) => {
                                Some("AGENT_RESTART needs a command")
                            }
                            (Life::AgentEnded { .. }, false, _, _) => {
                                Some("the agent can't continue this conversation")
                            }
                            (Life::Running { .. }, _, false, _)
                            | (
                                Life::Starting { .. }
                                | Life::HungUp { .. }
                                | Life::ExitedAwaitingOwner { .. },
                                _,
                                _,
                                _,
                            ) => Some("the agent hasn't ended"),
                        };
                        let refusal = restart_refusal(&life, continuable, needs_login, command)
                            .map(|r| r.to_string());
                        assert_eq!(
                            refusal.as_deref(),
                            expected,
                            "{life:?} {continuable} {needs_login} {command:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_gone_clients_kill_sessions_hang_up_unless_they_linger() {
        let cases = [
            (Policy::Kill, false, secs(0), Fate::HangUp),
            (Policy::Kill, false, secs(300), Fate::Linger(secs(300))),
            (Policy::Kill, true, secs(300), Fate::HangUp),
            (Policy::Kill, true, secs(0), Fate::HangUp),
            (Policy::Detach, false, secs(0), Fate::Keep),
            (Policy::Detach, true, secs(0), Fate::Keep),
            (Policy::Detach, false, secs(300), Fate::Keep),
        ];
        for (policy, starting, linger, fate) in cases {
            assert_eq!(gone_fate(policy, starting, linger), fate, "{policy:?} {starting} {linger:?}");
        }
    }

    #[test]
    fn an_exit_is_kept_for_an_owner_that_may_come_back_unless_hung_up() {
        let t0 = Instant::now();
        let ownerships = [
            (Ownership::Attached(3), false),
            (Ownership::Lingering { until: t0, since: t0 }, true),
            (Ownership::AwaitingOwner { since: t0 }, true),
            (Ownership::Detached { since: t0 }, false),
        ];
        let running = Life::Running { pid: pid(9) };
        let hung_up = Life::HungUp { pid: pid(9) };
        for (ownership, may_return) in ownerships {
            let kept = if may_return { ExitFate::KeepForOwner } else { ExitFate::Report };
            assert_eq!(exit_fate(&ownership, &running), kept, "{ownership:?}");
            assert_eq!(exit_fate(&ownership, &hung_up), ExitFate::Report, "{ownership:?}");
        }
        // A session hung up as it started is hung up too.
        let starting = Life::Starting { spawn: spawn(), hung_up: true };
        let awaiting = Ownership::AwaitingOwner { since: t0 };
        assert_eq!(exit_fate(&awaiting, &starting), ExitFate::Report);
    }

    #[test]
    fn at_the_grace_periods_end_exits_are_forgotten_and_kill_sessions_hung_up() {
        for life in lives() {
            let (kill, detach) = match life {
                Life::ExitedAwaitingOwner { .. } | Life::AgentEnded { .. } => {
                    (GraceFate::Forget, GraceFate::Forget)
                }
                Life::Starting { .. } | Life::Running { .. } | Life::HungUp { .. } => {
                    (GraceFate::HangUp, GraceFate::LeaveDetached)
                }
            };
            assert_eq!(grace_fate(Policy::Kill, &life), kill, "{life:?}");
            assert_eq!(grace_fate(Policy::Detach, &life), detach, "{life:?}");
        }
    }

    #[test]
    fn an_attach_is_refused_with_todays_texts() {
        let t0 = Instant::now();
        let running = Life::Running { pid: pid(9) };
        let hung_up = Life::HungUp { pid: pid(9) };
        let view = |life, ownership| SessionView {
            life,
            ownership,
            owner: None,
            kind: SessionKind::Terminal,
        };
        let detached = Ownership::Detached { since: t0 };
        let mine = Ownership::Attached(3);
        let theirs = Ownership::Attached(4);
        let cases = [
            (None, Some("no session 7")),
            (Some(view(&running, &mine)), Some("session 7 is already attached to this client")),
            (Some(view(&hung_up, &mine)), Some("session 7 is already attached to this client")),
            (Some(view(&running, &theirs)), Some("session 7 is attached to another client")),
            (Some(view(&hung_up, &detached)), Some("session 7 is ending")),
            (Some(view(&running, &detached)), None),
        ];
        // A terminal's attach is the same on every protocol.
        for protocol in 1..=AGENT_PROTOCOL {
            for (session, refused) in cases {
                let refusal =
                    attach_refusal(7, session, 3, protocol).map(|refusal| refusal.to_string());
                assert_eq!(refusal.as_deref(), refused, "{session:?} on {protocol}");
            }
        }
        // Whatever else they are, sessions with no client that aren't being
        // hung up are given: starting, exited, awaiting their owner.
        let awaiting = Ownership::AwaitingOwner { since: t0 };
        for life in lives() {
            let session = SessionView {
                life: &life,
                ownership: &awaiting,
                owner: None,
                kind: SessionKind::Terminal,
            };
            let refusal = attach_refusal(7, Some(session), 3, 3);
            assert_eq!(refusal.is_some(), life.is_hung_up(), "{life:?}");
        }
    }

    #[test]
    fn an_agent_session_is_refused_to_a_client_below_protocol_4_before_anything_else() {
        let t0 = Instant::now();
        let running = Life::Running { pid: pid(9) };
        let hung_up = Life::HungUp { pid: pid(9) };
        let agent = |life, ownership| SessionView {
            life,
            ownership,
            owner: None,
            kind: SessionKind::Agent,
        };
        let detached = Ownership::Detached { since: t0 };
        let theirs = Ownership::Attached(4);
        let text = "session 7 is an agent session, which needs client protocol 4";
        for session in [
            agent(&running, &detached),
            agent(&running, &theirs),
            agent(&hung_up, &detached),
        ] {
            for protocol in 1..AGENT_PROTOCOL {
                let refusal = attach_refusal(7, Some(session), 3, protocol);
                assert_eq!(refusal, Some(Refusal::NeedsAgentProtocol(7)));
                assert_eq!(refusal.map(|r| r.to_string()).as_deref(), Some(text));
            }
        }
        // On protocol 4 it is attached as a terminal is.
        assert_eq!(attach_refusal(7, Some(agent(&running, &detached)), 3, 4), None);
        assert_eq!(
            attach_refusal(7, Some(agent(&running, &theirs)), 3, 4),
            Some(Refusal::AttachedElsewhere(7))
        );
        // A session that doesn't exist is still "no session".
        assert_eq!(attach_refusal(7, None, 3, 3), Some(Refusal::NoSession(7)));
    }

    #[test]
    fn agent_sessions_are_shown_only_from_protocol_4_and_terminals_always() {
        for protocol in 1..=5 {
            assert!(shown(SessionKind::Terminal, protocol));
            assert_eq!(shown(SessionKind::Agent, protocol), protocol >= 4, "{protocol}");
        }
    }

    #[test]
    fn a_returning_owner_below_protocol_4_gets_no_agent_session_back() {
        let t0 = Instant::now();
        let token = ClientToken::from_wire("tok").unwrap();
        let running = Life::Running { pid: pid(9) };
        let awaiting = Ownership::AwaitingOwner { since: t0 };
        let view = |kind| SessionView {
            life: &running,
            ownership: &awaiting,
            owner: Some(&token),
            kind,
        };
        let sessions = [(1, view(SessionKind::Terminal)), (2, view(SessionKind::Agent))];
        assert_eq!(returning(&sessions, &token, 3), [1]);
        assert_eq!(returning(&sessions, &token, 4), [1, 2]);
    }

    #[test]
    fn agent_messages_below_protocol_4_are_refused_naming_the_message() {
        let cases = [
            (AgentRequest::Spawn, "SPAWN_AGENT needs client protocol 4"),
            (AgentRequest::Prompt, "AGENT_PROMPT needs client protocol 4"),
            (AgentRequest::Cancel, "AGENT_CANCEL needs client protocol 4"),
            (AgentRequest::Permission, "AGENT_PERMISSION needs client protocol 4"),
            (AgentRequest::Unqueue, "AGENT_UNQUEUE needs client protocol 4"),
            (AgentRequest::Resend, "AGENT_RESEND needs client protocol 4"),
            (AgentRequest::Restart, "AGENT_RESTART needs client protocol 4"),
            (AgentRequest::Login, "AGENT_LOGIN needs client protocol 4"),
            (AgentRequest::Fetch, "AGENT_FETCH needs client protocol 4"),
        ];
        for (request, text) in cases {
            for protocol in 1..AGENT_PROTOCOL {
                let refusal = agent_request_refusal(protocol, request).map(|r| r.to_string());
                assert_eq!(refusal.as_deref(), Some(text), "{protocol}");
            }
            assert_eq!(agent_request_refusal(4, request), None);
        }
    }

    #[test]
    fn an_agent_spawn_needs_a_command() {
        for command in ["", "  ", "\t"] {
            assert_eq!(agent_spawn_refusal(command), Some("an agent session needs a command"));
        }
        assert_eq!(agent_spawn_refusal("fake-agent"), None);
    }

    // agents-sessions 4.6: agent checkpoints.

    #[test]
    fn an_agent_session_is_checkpointed_at_every_moment_the_reducer_names() {
        let moments = [
            Moment::Started,
            Moment::BeforeRequest,
            Moment::TurnEnded,
            Moment::QueueChanged,
            Moment::Permission,
            Moment::Stopped,
            Moment::Login,
        ];
        for moment in moments {
            assert!(agent_checkpoint_due(AgentCheckpoint::Stepped(Some(moment)), 0), "{moment:?}");
        }
    }

    #[test]
    fn an_agent_session_is_checkpointed_before_a_step_down_and_on_a_detach() {
        assert!(agent_checkpoint_due(AgentCheckpoint::StepDown, 0));
        assert!(agent_checkpoint_due(AgentCheckpoint::Detach, 0));
    }

    #[test]
    fn otherwise_an_agent_session_is_checkpointed_past_512_kib_of_stdout() {
        let quiet = AgentCheckpoint::Stepped(None);
        assert_eq!(CHECKPOINT_AFTER, 512 * 1024);
        assert!(!agent_checkpoint_due(quiet, 0));
        assert!(!agent_checkpoint_due(quiet, CHECKPOINT_AFTER));
        assert!(agent_checkpoint_due(quiet, CHECKPOINT_AFTER + 1));
    }

    #[test]
    fn a_returning_owner_gets_its_sessions_that_have_no_client() {
        let t0 = Instant::now();
        let token = ClientToken::from_wire("tok").unwrap();
        let other = ClientToken::from_wire("other").unwrap();
        let running = Life::Running { pid: pid(9) };
        let exited = Life::ExitedAwaitingOwner { code: None, pid: Some(pid(9)) };
        let hung_up = Life::HungUp { pid: pid(9) };
        let starting = Life::Starting { spawn: spawn(), hung_up: false };
        let ended = Life::AgentEnded { code: Some(1) };
        let awaiting = Ownership::AwaitingOwner { since: t0 };
        let lingering = Ownership::Lingering { until: t0, since: t0 };
        let detached = Ownership::Detached { since: t0 };
        let attached = Ownership::Attached(3);
        let view = |life, ownership, owner| SessionView {
            life,
            ownership,
            owner,
            kind: SessionKind::Terminal,
        };
        let sessions = [
            (1, view(&running, &awaiting, Some(&token))),
            (2, view(&running, &lingering, Some(&token))),
            (3, view(&exited, &detached, Some(&token))),
            (4, view(&running, &attached, Some(&token))),
            (5, view(&hung_up, &detached, Some(&token))),
            (6, view(&starting, &detached, Some(&token))),
            (7, view(&running, &detached, Some(&other))),
            (8, view(&running, &detached, None)),
            // An ended agent kept for its owner is given back, for its
            // replay.
            (9, view(&ended, &lingering, Some(&token))),
        ];
        for protocol in 1..=AGENT_PROTOCOL {
            assert_eq!(returning(&sessions, &token, protocol), [1, 2, 3, 9]);
            assert_eq!(returning(&sessions, &other, protocol), [7]);
        }
    }

    // 3.2: process info.

    #[test]
    fn a_look_reads_flagged_sessions_and_keeps_the_unsettled_flagged() {
        let t0 = Instant::now();
        let cases = [
            (false, t0, t0 + secs(5), Look::Skip),
            (true, t0, t0, Look::Keep),
            (true, t0, t0 + ACTIVITY_SETTLE - Duration::from_millis(1), Look::Keep),
            (true, t0, t0 + ACTIVITY_SETTLE, Look::Read),
            (true, t0, t0 + secs(5), Look::Read),
            // Activity after the look (a clock read before it) is recent.
            (true, t0 + secs(1), t0, Look::Keep),
        ];
        for (active, active_at, now, expected) in cases {
            assert_eq!(look(active, active_at, now), expected, "{active} {:?}", now - t0);
        }
    }

    #[test]
    fn a_client_is_told_of_a_foreground_program_only_when_it_changed() {
        let vim = Foreground { name: Some("vim".into()), cwd: Some("/src".into()) };
        let moved = Foreground { cwd: Some("/tmp".into()), ..vim.clone() };
        assert!(foreground_changed(None, &vim));
        assert!(!foreground_changed(Some(&vim), &vim));
        assert!(foreground_changed(Some(&vim), &moved));
        assert!(foreground_changed(None, &Foreground::default()));
    }

    // agents-log-paging 2.3: how a checkpoint is written, pages, fetches.

    const KNOWN: Stored = Stored::Known {
        base: 1000,
        appended: 400,
        seq: 3,
    };
    const STEPPED: AgentCheckpoint = AgentCheckpoint::Stepped(None);

    #[test]
    fn a_checkpoint_is_appended_to_a_known_base_by_a_holder_of_the_cores_build() {
        assert_eq!(
            agent_checkpoint_form(KNOWN, STEPPED, Some(100), false, Adoption::Keep),
            CheckpointForm::Append
        );
        assert_eq!(
            agent_checkpoint_form(KNOWN, AgentCheckpoint::Detach, Some(600), false, Adoption::Keep),
            CheckpointForm::Append,
            "segments as large as the base still append"
        );
        assert_eq!(KNOWN.next_seq(), 4);
        assert_eq!(
            KNOWN.appended(100),
            Stored::Known {
                base: 1000,
                appended: 500,
                seq: 4
            }
        );
        assert_eq!(
            Stored::whole(77),
            Stored::Known {
                base: 77,
                appended: 0,
                seq: 0
            }
        );
        assert_eq!(Stored::Unknown.next_seq(), 1);
        assert_eq!(Stored::Unknown.appended(5), Stored::Unknown);
    }

    #[test]
    fn each_reason_to_write_a_checkpoint_whole() {
        let whole = |stored: Stored,
                     at: AgentCheckpoint,
                     segment: Option<u64>,
                     trimmed: bool,
                     holder: Adoption| {
            assert_eq!(
                agent_checkpoint_form(stored, at, segment, trimmed, holder),
                CheckpointForm::Whole,
                "{stored:?} {at:?} {segment:?} {trimmed} {holder:?}"
            );
        };
        // Unknown: new, adopted, restarted, or after a holder's ERROR.
        whole(Stored::Unknown, STEPPED, Some(1), false, Adoption::Keep);
        // A step-down: the next core adopts one compact base.
        whole(KNOWN, AgentCheckpoint::StepDown, Some(1), false, Adoption::Keep);
        // A trim since the last checkpoint compacts it.
        whole(KNOWN, STEPPED, Some(1), true, Adoption::Keep);
        // The segments would outgrow the base.
        whole(KNOWN, STEPPED, Some(601), false, Adoption::Keep);
        // The blob would pass 64 MiB.
        let big = Stored::Known {
            base: MAX_CHECKPOINT_LEN / 2,
            appended: MAX_CHECKPOINT_LEN / 2 - 10,
            seq: 9,
        };
        whole(big, STEPPED, Some(11), false, Adoption::Keep);
        assert_eq!(
            agent_checkpoint_form(big, STEPPED, Some(10), false, Adoption::Keep),
            CheckpointForm::Append,
            "exactly 64 MiB still appends"
        );
        // A segment too large to encode, or none encoded.
        whole(KNOWN, STEPPED, None, false, Adoption::Keep);
        // A draining holder of another build.
        whole(KNOWN, STEPPED, Some(1), false, Adoption::Drain);
        assert!(!agent_appendable(KNOWN, STEPPED, false, Adoption::Drain));
        assert!(agent_appendable(KNOWN, STEPPED, false, Adoption::Keep));
    }

    #[test]
    fn a_page_is_at_most_256_kib_and_that_when_unasked() {
        assert_eq!(PAGE_BYTES, 256 * 1024);
        assert_eq!(page_budget(None), PAGE_BYTES);
        assert_eq!(page_budget(Some(16_384)), 16_384);
        assert_eq!(page_budget(Some(u32::MAX)), PAGE_BYTES);
        assert_eq!(page_budget(Some(0)), 0);
    }

    #[test]
    fn each_fetch_refusal_and_its_text() {
        let view = |kind, attached_here, replay_pending| FetchView {
            kind,
            attached_here,
            replay_pending,
        };
        let refused = |session, protocol| agent_fetch_refusal(2, session, protocol).map(|r| r.to_string());
        let ready = view(SessionKind::Agent, true, false);
        assert_eq!(refused(Some(ready), 4), None);
        assert_eq!(
            refused(Some(ready), 3).as_deref(),
            Some("AGENT_FETCH needs client protocol 4")
        );
        assert_eq!(refused(None, 4).as_deref(), Some("no session 2"));
        assert_eq!(
            refused(Some(view(SessionKind::Terminal, true, false)), 4).as_deref(),
            Some("session 2 isn't an agent session")
        );
        assert_eq!(
            refused(Some(view(SessionKind::Agent, false, false)), 4).as_deref(),
            Some("session 2 isn't attached to this client")
        );
        assert_eq!(
            refused(Some(view(SessionKind::Agent, true, true)), 4).as_deref(),
            Some("session 2's replay hasn't ended")
        );
    }

    /// Drives a reducer session the way the core does: checkpoints when
    /// [`agent_checkpoint_due`] says, written as [`agent_checkpoint_form`]
    /// says, counting the bytes the holder is sent.
    struct Checkpointing {
        session: crate::acp::reduce::AgentSession,
        stored: Stored,
        unsaved: Vec<brindle_transcript::Event>,
        since: u64,
        written: u64,
        wholes_after_trims: u64,
        now: Instant,
    }

    impl Checkpointing {
        fn step(&mut self, input: crate::acp::reduce::Input<'_>) -> crate::acp::reduce::Output {
            let fed = match &input {
                crate::acp::reduce::Input::Stdout(bytes) => bytes.len() as u64,
                _ => 0,
            };
            let out = crate::acp::reduce::step(&mut self.session, input, self.now);
            self.since += fed;
            self.unsaved.extend(out.events.iter().cloned());
            let at = AgentCheckpoint::Stepped(out.checkpoint);
            if agent_checkpoint_due(at, self.since) {
                self.checkpoint(at);
            }
            out
        }

        fn checkpoint(&mut self, at: AgentCheckpoint) {
            use crate::acp::checkpoint::{encode, encode_segment};
            let trimmed = self
                .unsaved
                .iter()
                .any(|event| matches!(event, brindle_transcript::Event::Trimmed { .. }));
            let segment = agent_appendable(self.stored, at, trimmed, Adoption::Keep)
                .then(|| encode_segment(&self.session, &self.unsaved, self.stored.next_seq()).ok())
                .flatten();
            let len = segment.as_ref().map(|segment| segment.len() as u64);
            match agent_checkpoint_form(self.stored, at, len, trimmed, Adoption::Keep) {
                CheckpointForm::Whole => {
                    let bytes = encode(&self.session).expect("it fits");
                    self.written += bytes.len() as u64;
                    self.stored = Stored::whole(bytes.len() as u64);
                    if trimmed {
                        self.wholes_after_trims += 1;
                    }
                }
                CheckpointForm::Append => {
                    let len = len.expect("an append has its segment");
                    self.written += len;
                    self.stored = self.stored.appended(len);
                }
            }
            self.unsaved.clear();
            self.since = 0;
        }
    }

    fn json_line(value: brindle_remote_protocol::serde_json::Value) -> Vec<u8> {
        let mut line = value.to_string().into_bytes();
        line.push(b'\n');
        line
    }

    /// Decision 9's cost: through a 64 MiB conversation, appending writes
    /// under 4x the log bytes added, plus one base per trim, where writing
    /// whole at every moment would copy the log again each time.
    #[test]
    fn checkpoints_of_a_64_mib_conversation_write_a_small_multiple_of_what_it_added() {
        use brindle_remote_protocol::serde_json::json;
        use crate::acp::reduce::{AgentSession, Command, Input, LOG_LIMIT};
        let mut core = Checkpointing {
            session: AgentSession::new("/w".into(), "0.3.0".into()),
            stored: Stored::Unknown,
            unsaved: Vec::new(),
            since: 0,
            written: 0,
            wholes_after_trims: 0,
            now: Instant::now(),
        };
        core.step(Input::Spawned);
        core.step(Input::Stdout(&json_line(json!({"jsonrpc": "2.0", "id": 1,
            "result": {"protocolVersion": 1}}))));
        core.step(Input::Stdout(&json_line(json!({"jsonrpc": "2.0", "id": 2,
            "result": {"sessionId": "s1"}}))));
        let text = "x".repeat(64 * 1024);
        let chunk = json_line(json!({"jsonrpc": "2.0", "method": "session/update",
            "params": {"sessionId": "s1", "update": {"sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": text}}}}));
        let mut added: u64 = 0;
        for _ in 0..64 {
            let out = core.step(Input::Command(Command::Prompt { text: "go on".into() }));
            assert_eq!(out.reply, Some(Ok(())));
            let sent: brindle_remote_protocol::serde_json::Value =
                brindle_remote_protocol::serde_json::from_slice(
                    out.stdin.split(|b| *b == b'\n').next().expect("a line"),
                )
                .expect("JSON");
            let id = sent["id"].as_u64().expect("the prompt's id");
            for _ in 0..16 {
                core.step(Input::Stdout(&chunk));
                added += text.len() as u64;
            }
            core.step(Input::Stdout(&json_line(json!({"jsonrpc": "2.0", "id": id,
                "result": {"stopReason": "end_turn"}}))));
        }
        assert!(added >= 64 * 1024 * 1024);
        assert!(core.session.transcript().dropped() > 0, "the log was trimmed");
        assert!(core.wholes_after_trims > 0);
        let bound = 4 * added + core.wholes_after_trims * LOG_LIMIT as u64;
        assert!(
            core.written < bound,
            "{} bytes written for {added} added and {} trims (bound {bound})",
            core.written,
            core.wholes_after_trims
        );
    }
}
