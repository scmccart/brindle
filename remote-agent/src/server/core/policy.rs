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

use std::fmt;
use std::time::{Duration, Instant};

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::{Hello, Policy};

use super::clients::ClientToken;
use super::sessions::{Life, Ownership};
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

/// What [`placement`] reads of a holder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HolderView<'a> {
    /// The build its files name; `None` when their name says none.
    pub(super) named: Option<&'a str>,
    pub(super) stage: Stage<'a>,
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

    /// Whether spawns may go to it: it is adopted, and of `build`.
    fn takes_spawns(&self, build: &str) -> bool {
        match self.stage {
            Stage::Adopted { build: own } => own == build,
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

/// Where the core's waiting spawns go: to the first adopted holder of its
/// `build` in `holders`' order, else they wait for one that is connected
/// but hasn't welcomed the core yet, or for the one `starting`; with
/// neither, the core starts one.
pub(super) fn placement(holders: &[HolderView], build: &str, starting: bool) -> Placement {
    if let Some(index) = holders.iter().position(|holder| holder.takes_spawns(build)) {
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
        holder.takes_spawns(build)
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

pub(super) fn grace_fate(policy: Policy, life: &Life) -> GraceFate {
    match (life, policy) {
        (Life::ExitedAwaitingOwner { .. }, Policy::Kill | Policy::Detach) => GraceFate::Forget,
        (Life::Starting { .. } | Life::Running { .. } | Life::HungUp { .. }, Policy::Kill) => {
            GraceFate::HangUp
        }
        (Life::Starting { .. } | Life::Running { .. } | Life::HungUp { .. }, Policy::Detach) => {
            GraceFate::LeaveDetached
        }
    }
}

/// What [`attach_refusal`] and [`returning`] read of a session.
#[derive(Debug, Clone, Copy)]
pub(super) struct SessionView<'a> {
    pub(super) life: &'a Life,
    pub(super) ownership: &'a Ownership,
    pub(super) owner: Option<&'a ClientToken>,
}

/// Why an `ATTACH` is refused; its text is the `ERROR`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refusal {
    NoSession(SessionId),
    AttachedHere(SessionId),
    AttachedElsewhere(SessionId),
    Ending(SessionId),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSession(id) => write!(f, "no session {id}"),
            Self::AttachedHere(id) => write!(f, "session {id} is already attached to this client"),
            Self::AttachedElsewhere(id) => write!(f, "session {id} is attached to another client"),
            Self::Ending(id) => write!(f, "session {id} is ending"),
        }
    }
}

/// Why `client` can't attach session `id` (`session`, if there is one):
/// only a session with no client that isn't being hung up is given.
pub(super) fn attach_refusal(
    id: SessionId,
    session: Option<SessionView<'_>>,
    client: Token,
) -> Option<Refusal> {
    let session = match session {
        None => return Some(Refusal::NoSession(id)),
        Some(session) => session,
    };
    match session.ownership.client() {
        Some(attached) if attached == client => Some(Refusal::AttachedHere(id)),
        Some(_) => Some(Refusal::AttachedElsewhere(id)),
        None if session.life.is_hung_up() => Some(Refusal::Ending(id)),
        None => None,
    }
}

/// The sessions a client with `token` gets back when it connects: those it
/// owns that have no client. One still starting isn't given back (its
/// `SPAWN`'s answer went to a connection that is gone, so the client
/// doesn't know it), nor one being hung up.
pub(super) fn returning(
    sessions: &[(SessionId, SessionView<'_>)],
    token: &ClientToken,
) -> Vec<SessionId> {
    sessions
        .iter()
        .filter(|(_, session)| {
            session.ownership.client().is_none()
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

    fn holder<'a>(named: Option<&'a str>, stage: Stage<'a>) -> HolderView<'a> {
        HolderView { named, stage }
    }

    #[test]
    fn spawns_go_to_the_first_adopted_holder_of_the_cores_build() {
        let other = holder(Some("b0"), Stage::Adopted { build: "b0" });
        let adopting = holder(Some("b1"), Stage::Adopting { build: "b1" });
        let own = holder(Some("b1"), Stage::Adopted { build: "b1" });
        // Its files may name another build than its WELCOME.
        let renamed = holder(Some("b0"), Stage::Adopted { build: "b1" });
        assert_eq!(placement(&[other, own], "b1", false), Placement::Spawn(1));
        assert_eq!(placement(&[renamed, own], "b1", true), Placement::Spawn(0));
        assert_eq!(placement(&[adopting, own], "b1", false), Placement::Spawn(1));
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
            assert_eq!(placement(holders, "b1", starting), expected, "{holders:?} {starting}");
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

    fn spawn() -> Spawn {
        Spawn {
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
        }
    }

    /// One of each kind of life.
    fn lives() -> [Life; 5] {
        [
            Life::Starting { spawn: spawn(), hung_up: false },
            Life::Starting { spawn: spawn(), hung_up: true },
            Life::Running { pid: pid(9) },
            Life::HungUp { pid: pid(9) },
            Life::ExitedAwaitingOwner { code: Some(0), pid: Some(pid(9)) },
        ]
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
                Life::ExitedAwaitingOwner { .. } => (GraceFate::Forget, GraceFate::Forget),
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
        let view = |life, ownership| SessionView { life, ownership, owner: None };
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
        for (session, refused) in cases {
            let refusal = attach_refusal(7, session, 3).map(|refusal| refusal.to_string());
            assert_eq!(refusal.as_deref(), refused, "{session:?}");
        }
        // Whatever else they are, sessions with no client that aren't being
        // hung up are given: starting, exited, awaiting their owner.
        let awaiting = Ownership::AwaitingOwner { since: t0 };
        for life in lives() {
            let session = SessionView { life: &life, ownership: &awaiting, owner: None };
            let refusal = attach_refusal(7, Some(session), 3);
            assert_eq!(refusal.is_some(), life.is_hung_up(), "{life:?}");
        }
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
        let awaiting = Ownership::AwaitingOwner { since: t0 };
        let lingering = Ownership::Lingering { until: t0, since: t0 };
        let detached = Ownership::Detached { since: t0 };
        let attached = Ownership::Attached(3);
        let view = |life, ownership, owner| SessionView { life, ownership, owner };
        let sessions = [
            (1, view(&running, &awaiting, Some(&token))),
            (2, view(&running, &lingering, Some(&token))),
            (3, view(&exited, &detached, Some(&token))),
            (4, view(&running, &attached, Some(&token))),
            (5, view(&hung_up, &detached, Some(&token))),
            (6, view(&starting, &detached, Some(&token))),
            (7, view(&running, &detached, Some(&other))),
            (8, view(&running, &detached, None)),
        ];
        assert_eq!(returning(&sessions, &token), [1, 2, 3]);
        assert_eq!(returning(&sessions, &other), [7]);
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
}
