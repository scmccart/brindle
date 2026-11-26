//! The core's sessions: spawning through a holder, their I/O between the
//! client and the holder, the close policies, hang-ups and exits.
//!
//! - **State:** each session's program has a [`Life`] (starting, running,
//!   hung up, or exited with the exit kept for its owner), its I/O an
//!   [`Ownership`] (attached to a client, lingering, awaiting its owner
//!   after an adoption, or detached, each with since when), and an adopted
//!   one's emulator a [`Rebuild`]. What each event does to them is decided
//!   in [`super::policy`].
//! - **`kill`** sessions end on `CLOSE`, `KILL` and `STOP`, and when their
//!   owner is gone: its connection ended, or, for sessions a new core
//!   adopted, it didn't come back within [`super::RECONNECT_GRACE`].
//! - **Linger:** a client whose `HELLO` asked for `linger_secs` (at most
//!   [`super::MAX_LINGER`]) gets that long to come back: its `kill`
//!   sessions keep running when its connection ends, and are hung up only
//!   once the time passes without it (an exit meanwhile is kept for it, as
//!   for an adopted session). The linger lives only in this core: a new
//!   core gives adopted sessions [`super::RECONNECT_GRACE`].
//! - **`detach`** sessions end only on `CLOSE`, `KILL`, `STOP` or their
//!   program's exit. Without a client they keep running.
//! - **Owners** are `client_token`s. A client whose `HELLO` carries a
//!   session's owner token gets every such session that has no client.
//!   A client that attaches becomes the owner. `DETACH` clears the owner,
//!   so no reconnect gives the session back, and makes a `kill` session
//!   `detach`. Both changes go to the holder with `SET_META` (H1 version
//!   2), so a later core sees them; a version 1 holder keeps the spawn's.
//! - **Takeover:** a `HELLO` with the token of a client that is still
//!   connected replaces that connection ([`Core::client_replaced`]): its
//!   sessions go to the new one, with no close policy applied.
//! - **Attach:** `ATTACH` gives a session with no client to the client,
//!   after a replay of its screen (`REPLAY_START`, `DATA`…, `REPLAY_END`),
//!   then applies the client's size. `DETACH` leaves it running with no
//!   client.
//! - **Stored before told:** a replay waits until the session's last
//!   `SET_META` is in its holder's socket (`Session::storing`, released
//!   by [`Core::send_stored_replays`] after the holders' flush). A client
//!   that has its `REPLAY_START` therefore gets the session back from the
//!   next core, however soon this one is killed: the holder reads what
//!   its socket holds before it sees the connection end.
//! - **Checkpoints:** the session's replay goes to its holder past
//!   [`CHECKPOINT_AFTER`] of output since the last one (checked after each
//!   read of the holder's output, and deferred while more is waiting: see
//!   [`is_due`]), when it is detached, and before the core steps down.
//!   Each says how much output it
//!   covers, so the holder keeps what was still on its way. An adopted
//!   session's emulator is rebuilt from the holder's checkpoint and tail
//!   (the tail alone, after `ESC c`, when it was truncated).

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use brindle_remote_protocol::client::{
    self, Data, Exit, Kill, KillTarget, Policy, Request, Resize, SessionInfo, Spawned, ToClient,
};
use brindle_remote_protocol::holder::{self as h1, HeldSession, MetaFault, SessionMeta, ToHolder};
use brindle_remote_protocol::{RequestId, SessionId};
use brindle_term_core::emulator;

use super::agents::Agent;
use super::clients::{ClientToken, error};
use super::holders::WaitingSpawn;
use super::policy::{
    self, AgentCheckpoint, ExitFate, Fate, GraceFate, SessionKind, SessionView,
};
use super::screen::Screen;
use super::{Core, log};
use crate::acp::reduce::Input;
use crate::pty::Foreground;
use crate::server::event_loop::{EventLoop, Token};

/// How much of a replay each `DATA` carries.
const REPLAY_CHUNK: usize = 64 * 1024;
/// Past this much output since the last checkpoint, the core sends the
/// session's holder a new one.
pub(super) const CHECKPOINT_AFTER: u64 = 512 * 1024;
/// The same, while more of the holder's output is waiting to be read.
const CHECKPOINT_AFTER_BEHIND: u64 = 4 * 1024 * 1024;

/// Whether a session with `since` bytes of output since its last
/// checkpoint is due for a new one, after a read of its holder's output.
///
/// A checkpoint renders the whole screen and scrollback, which costs about
/// as much as feeding as many bytes. In a flood that keeps the core behind
/// its holder, checkpoints every [`CHECKPOINT_AFTER`] took a third to a half
/// of the core's time, and they mostly can't keep the holder's tail whole: the
/// holder has already read past them. So while more of the holder's output
/// is waiting (`behind`), a session is due only past
/// [`CHECKPOINT_AFTER_BEHIND`], and the first read that leaves nothing
/// waiting checkpoints at [`CHECKPOINT_AFTER`] again.
fn is_due(since: u64, behind: bool) -> bool {
    let after = if behind { CHECKPOINT_AFTER_BEHIND } else { CHECKPOINT_AFTER };
    since > after
}

/// What a client asked to start: a terminal (`SPAWN`) or an agent
/// (`SPAWN_AGENT`, protocol 4, with the absolute directory the core
/// resolved for it, which its `session/new` gets and `LIST` shows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SpawnRequest {
    Terminal(client::Spawn),
    Agent {
        spawn: client::SpawnAgent,
        cwd: String,
    },
}

impl SpawnRequest {
    /// The request its `SPAWNED` or `ERROR` answers.
    pub(super) fn req(&self) -> RequestId {
        match self {
            Self::Terminal(spawn) => spawn.req,
            Self::Agent { spawn, .. } => spawn.req,
        }
    }

    /// How its program is connected, which decides the holder it may go
    /// to ([`policy::placement`]).
    pub(super) fn mode(&self) -> h1::SpawnMode {
        match self {
            Self::Terminal(_) => h1::SpawnMode::Pty,
            Self::Agent { .. } => h1::SpawnMode::Pipes,
        }
    }
}

/// What a session runs, and the core's state for it (`types R1`).
pub(super) enum Content {
    /// A program on a PTY, with its emulator (boxed: it is large).
    Terminal(Box<Screen>),
    /// An ACP agent on plain pipes, with its transcript.
    Agent(Agent),
}

/// Where a session's program is in its life (`types R1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Life {
    /// The client's `SPAWN` (or `SPAWN_AGENT`), until the holder reports
    /// the program. It is
    /// sent again if the holder goes away first (an empty holder exits, and
    /// may not have read it). With `hung_up`, its holder was told to hang
    /// it up (`KILL`, `STOP`, its `kill` client gone): it ends as soon as
    /// it starts, and its `SPAWNED` still answers the request.
    Starting { spawn: SpawnRequest, hung_up: bool },
    /// Its program runs as `pid`.
    Running { pid: NonZeroU32 },
    /// Its holder was told to hang it up; its `EXITED` follows.
    HungUp { pid: NonZeroU32 },
    /// Its program exited (`code` is `None` for a signal or a lost
    /// holder), and the exit is kept until its owner hears of it: an owner
    /// away, or a client still waiting for its replay. `pid` is what its
    /// program's was, for that replay.
    ExitedAwaitingOwner {
        code: Option<i32>,
        pid: Option<NonZeroU32>,
    },
}

impl Life {
    /// Started from `spawn`, waiting for the holder to report the program.
    fn starting(spawn: SpawnRequest) -> Self {
        Self::Starting {
            spawn,
            hung_up: false,
        }
    }

    /// Its program's pid, once the holder reported it.
    pub(super) fn pid(&self) -> Option<NonZeroU32> {
        match self {
            Self::Starting { .. } => None,
            Self::Running { pid } | Self::HungUp { pid } => Some(*pid),
            Self::ExitedAwaitingOwner { pid, .. } => *pid,
        }
    }

    pub(super) fn is_starting(&self) -> bool {
        match self {
            Self::Starting { .. } => true,
            Self::Running { .. } | Self::HungUp { .. } | Self::ExitedAwaitingOwner { .. } => false,
        }
    }

    /// Its program runs (hung up or not): it started, and hasn't exited.
    /// `LIST` shows these.
    pub(super) fn is_running(&self) -> bool {
        match self {
            Self::Running { .. } | Self::HungUp { .. } => true,
            Self::Starting { .. } | Self::ExitedAwaitingOwner { .. } => false,
        }
    }

    /// Its holder was told to hang it up.
    pub(super) fn is_hung_up(&self) -> bool {
        match self {
            Self::Starting { hung_up, .. } => *hung_up,
            Self::HungUp { .. } => true,
            Self::Running { .. } | Self::ExitedAwaitingOwner { .. } => false,
        }
    }

    /// Its program runs and isn't being hung up: it is checkpointed, and
    /// its foreground program reported.
    pub(super) fn is_live(&self) -> bool {
        match self {
            Self::Running { .. } => true,
            Self::Starting { .. } | Self::HungUp { .. } | Self::ExitedAwaitingOwner { .. } => false,
        }
    }

    /// Its program exited with `code`: the exit is kept for its owner.
    fn exited(&mut self, code: Option<i32>) {
        *self = Self::ExitedAwaitingOwner {
            code,
            pid: self.pid(),
        };
    }

    /// Asks for a hang-up: whether its holder is to be told, which it is
    /// once, and only while its program may still run.
    fn hang_up(&mut self) -> bool {
        match self {
            Self::Starting { hung_up, .. } => !std::mem::replace(hung_up, true),
            Self::Running { pid } => {
                *self = Self::HungUp { pid: *pid };
                true
            }
            Self::HungUp { .. } | Self::ExitedAwaitingOwner { .. } => false,
        }
    }
}

/// Who a session's I/O goes to, and since when it has had none (`types R1`).
/// Its owner, the token a returning client is matched by, is
/// [`Session::owner`] beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ownership {
    /// Its I/O goes to this client.
    Attached(Token),
    /// A `kill` session whose client's connection ended, waiting `until`
    /// for the client to come back (its `HELLO`'s `linger_secs`). No
    /// client `since`.
    Lingering { until: Instant, since: Instant },
    /// Adopted, and its owner hasn't come back yet. No client `since` its
    /// adoption: when its client left before that, no core knows.
    AwaitingOwner { since: Instant },
    /// No client `since` its last one left.
    Detached { since: Instant },
}

impl Ownership {
    /// The client its I/O goes to.
    pub(super) fn client(&self) -> Option<Token> {
        match self {
            Self::Attached(client) => Some(*client),
            Self::Lingering { .. } | Self::AwaitingOwner { .. } | Self::Detached { .. } => None,
        }
    }

    /// Since when it has had no client; `None` while a client has it.
    pub(super) fn detached_since(&self) -> Option<Instant> {
        match self {
            Self::Attached(_) => None,
            Self::Lingering { since, .. }
            | Self::AwaitingOwner { since }
            | Self::Detached { since } => Some(*since),
        }
    }

    /// Its owner may still come back for it: a lingering session, or one
    /// adopted whose owner hasn't come back yet.
    pub(super) fn owner_may_return(&self) -> bool {
        match self {
            Self::Lingering { .. } | Self::AwaitingOwner { .. } => true,
            Self::Attached(_) | Self::Detached { .. } => false,
        }
    }

    /// No owner is waited for any more: it stays without a client since
    /// when it had one last.
    fn stop_waiting(&mut self) {
        match *self {
            Self::Lingering { since, .. } | Self::AwaitingOwner { since } => {
                *self = Self::Detached { since };
            }
            Self::Attached(_) | Self::Detached { .. } => {}
        }
    }
}

/// Whether an adopted session's emulator is rebuilt from its holder's
/// checkpoint and tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Rebuild {
    Ready,
    /// Not until its tail comes, which was `truncated` or not.
    Pending { truncated: bool },
}

pub(super) struct Session {
    /// The holder it runs under.
    pub(super) holder: Token,
    pub(super) life: Life,
    /// What it was started with, kept in its holder through `SET_META`
    /// ([`Session::stored_meta`]). Its `owner` is always empty here: the
    /// owner is [`Session::owner`].
    pub(super) meta: SessionMeta,
    /// The client token whose `HELLO` gets it back ([`policy::returning`]);
    /// `None` for a session that was let go, which no reconnect gets back.
    pub(super) owner: Option<ClientToken>,
    pub(super) cols: u16,
    pub(super) rows: u16,
    pub(super) ownership: Ownership,
    /// Its holder was told to stop reading it, because its client is behind.
    paused: bool,
    rebuild: Rebuild,
    /// Where its last `SET_META` ends in its holder's connection (a count
    /// of bytes written: [`Core::holder_has`]), until the holder's socket
    /// has taken that much.
    storing: Option<u64>,
    /// A replay for its client, waiting for the rebuild or for `storing`.
    /// Its output meanwhile isn't forwarded: the replay has it.
    pub(super) pending_replay: Option<PendingReplay>,
    /// A terminal's emulator, fed with all of its output, or an agent's
    /// transcript, fed with all of its stdout.
    pub(super) content: Content,
    /// It had output or input since the core last looked at its foreground
    /// program, or was just started or attached ([`super::process`]).
    pub(super) active: bool,
    /// When its last output or input was.
    pub(super) active_at: Instant,
    /// The foreground program its client was last told of.
    pub(super) foreground: Option<Foreground>,
}

/// What a replay carries between its `REPLAY_START` and `REPLAY_END`.
enum Replay {
    /// A terminal's screen, as `DATA`.
    Screen(Vec<u8>),
    /// An agent's log, as `TRANSCRIPT` events.
    Transcript(Vec<brindle_remote_protocol::serde_json::Value>),
}

pub(super) struct PendingReplay {
    req: Option<RequestId>,
    /// The size an `ATTACH` asked for.
    size: Option<(u16, u16)>,
}

impl Session {
    /// Whether its client may be told of its foreground program now: its
    /// program runs and isn't being hung up, and the client has its
    /// `SPAWNED` or its replay already.
    pub(super) fn reportable(&self) -> bool {
        self.life.is_live() && self.pending_replay.is_none()
    }

    /// Its meta as its holder keeps it, with its owner's token (empty for
    /// none, `types R9`).
    pub(super) fn stored_meta(&self) -> SessionMeta {
        SessionMeta {
            owner: self
                .owner
                .as_ref()
                .map(|owner| owner.as_str().to_string())
                .unwrap_or_default(),
            ..self.meta.clone()
        }
    }

    /// What `LIST` says of it, as session `id`, at `now`. Only a session
    /// with no client says whether it may still come back for its owner
    /// (`owned`), for how long it has had none, and the layout its last
    /// client detached it with.
    /// An agent session is listed at 0×0 with no title, and with its
    /// activity and the name its agent gave ([`Agent::info`]).
    pub(super) fn info(&self, id: SessionId, now: Instant) -> SessionInfo {
        let detached = self.ownership.client().is_none();
        let (title, agent) = match &self.content {
            Content::Terminal(screen) => (screen.title(), None),
            Content::Agent(agent) => (String::new(), Some(agent.info())),
        };
        SessionInfo {
            session: id,
            pid: self.life.pid().map_or(0, NonZeroU32::get),
            attached: !detached,
            command: self.meta.command.clone(),
            cwd: self.meta.cwd.clone(),
            policy: self.meta.policy,
            cols: self.cols,
            rows: self.rows,
            key: self.meta.key.clone(),
            title,
            started: self.meta.started,
            profile: self.meta.profile.clone(),
            owned: detached && self.owner.is_some(),
            detached_secs: self
                .ownership
                .detached_since()
                .map_or(0, |since| now.saturating_duration_since(since).as_secs()),
            layout: self.meta.layout.clone().filter(|_| detached),
            agent,
        }
    }

    /// What [`policy::attach_refusal`] and [`policy::returning`] read.
    pub(super) fn view(&self) -> SessionView<'_> {
        SessionView {
            life: &self.life,
            ownership: &self.ownership,
            owner: self.owner.as_ref(),
            kind: self.kind(),
        }
    }

    pub(super) fn kind(&self) -> SessionKind {
        match self.content {
            Content::Terminal(_) => SessionKind::Terminal,
            Content::Agent(_) => SessionKind::Agent,
        }
    }

    /// A terminal's emulator; `None` for an agent session.
    fn screen(&mut self) -> Option<&mut Screen> {
        match &mut self.content {
            Content::Terminal(screen) => Some(screen.as_mut()),
            Content::Agent(_) => None,
        }
    }

    /// An agent session's state; `None` for a terminal.
    pub(super) fn agent(&mut self) -> Option<&mut Agent> {
        match &mut self.content {
            Content::Terminal(_) => None,
            Content::Agent(agent) => Some(agent),
        }
    }

    /// Output or input at `now`: the next look at its foreground program
    /// includes it.
    fn mark_active(&mut self, now: Instant) {
        self.active = true;
        self.active_at = now;
    }

    /// Started or attached at `now`: its client is told of its foreground
    /// program at the next look, changed or not.
    fn tell_foreground(&mut self, now: Instant) {
        self.mark_active(now);
        self.foreground = None;
    }

    /// A session reported in a holder's `WELCOME`, adopted at `now`. It has
    /// no client until its owner comes back. One that exited while no core
    /// was connected is gone from the holder already, and its exit is kept
    /// for its owner. A malformed meta is repaired ([`adopt_meta`]): then
    /// the `core.log` line that says so comes with it, and the core stores
    /// the repaired meta. A running session with no pid (0, which no
    /// holder reports) isn't adopted: the `Err` is the line that says so.
    /// Its size is the holder's, within the limits every grid keeps
    /// ([`emulator::clamped`]).
    pub(super) fn adopted(
        holder: Token,
        held: HeldSession,
        now: Instant,
    ) -> Result<(Self, Option<String>), String> {
        let pid = NonZeroU32::new(held.pid);
        let life = match (held.exit, pid) {
            (Some(exit), pid) => Life::ExitedAwaitingOwner {
                code: exit.code,
                pid,
            },
            (None, Some(pid)) => Life::Running { pid },
            (None, None) => {
                return Err(format!(
                    "session {}: its holder reports it running with no pid; not adopted",
                    held.session
                ));
            }
        };
        let (mut meta, repaired) =
            adopt_meta(held.session, SessionMeta::from_value(&held.meta), session_key);
        let owner = ClientToken::from_wire(&std::mem::take(&mut meta.owner));
        // An agent session has no grid: it is 0×0 whatever its holder says.
        let (content, (cols, rows)) = match held.mode {
            h1::SpawnMode::Pty => {
                let (cols, rows) = emulator::clamped(held.cols, held.rows);
                let screen = Screen::new(cols, rows, meta.scrollback);
                (Content::Terminal(Box::new(screen)), (cols, rows))
            }
            h1::SpawnMode::Pipes => (Content::Agent(Agent::adopting()), (0, 0)),
        };
        let session = Self {
            holder,
            life,
            content,
            meta,
            owner,
            cols,
            rows,
            ownership: Ownership::AwaitingOwner { since: now },
            paused: false,
            rebuild: Rebuild::Pending {
                truncated: held.truncated,
            },
            storing: None,
            pending_replay: None,
            active: false,
            active_at: now,
            foreground: None,
        };
        Ok((session, repaired))
    }

    /// A session the core sends its holder a `SPAWN` for at `now`, for
    /// `client`, which owns it (if its `HELLO` named a token). The spawn's
    /// answer goes to that client.
    fn spawning(
        holder: Token,
        client: Token,
        owner: Option<ClientToken>,
        spawn: SpawnRequest,
        now: Instant,
    ) -> Self {
        let (meta, content, cols, rows) = match &spawn {
            SpawnRequest::Terminal(spawn) => (
                SessionMeta {
                    command: command_line(spawn.command.as_deref(), &spawn.args),
                    cwd: spawn.cwd.clone(),
                    key: session_key(),
                    layout: None,
                    owner: String::new(),
                    policy: spawn.policy,
                    profile: spawn.profile.clone(),
                    scrollback: spawn.scrollback,
                    started: unix_now(),
                },
                Content::Terminal(Box::new(Screen::new(spawn.cols, spawn.rows, spawn.scrollback))),
                spawn.cols,
                spawn.rows,
            ),
            // Its listed directory is the absolute one its agent works in.
            SpawnRequest::Agent { spawn, cwd } => (
                SessionMeta {
                    command: command_line(Some(&spawn.command), &spawn.args),
                    cwd: Some(cwd.clone()),
                    key: session_key(),
                    layout: None,
                    owner: String::new(),
                    policy: spawn.policy,
                    profile: spawn.profile.clone(),
                    scrollback: 0,
                    started: unix_now(),
                },
                Content::Agent(Agent::new(cwd.clone())),
                0,
                0,
            ),
        };
        Self {
            holder,
            content,
            meta,
            owner,
            cols,
            rows,
            ownership: Ownership::Attached(client),
            life: Life::starting(spawn),
            paused: false,
            rebuild: Rebuild::Ready,
            storing: None,
            pending_replay: None,
            active: false,
            active_at: now,
            foreground: None,
        }
    }
}

/// How an adopted session's meta, as [`SessionMeta::from_value`] read it,
/// is taken: a good one as it is. A refused one keeps what was read, but
/// with no owner, since an unreadable owner can't come back, and the
/// `detach` policy, since `kill` with no owner would end the program when
/// the grace period ends. A malformed key (or a meta that isn't an object)
/// gets a fresh one from `fresh_key`. The second value is the `core.log`
/// line naming the session and the fields, for a refused meta.
pub(super) fn adopt_meta(
    id: SessionId,
    read: Result<SessionMeta, MetaFault>,
    fresh_key: impl FnOnce() -> String,
) -> (SessionMeta, Option<String>) {
    let MetaFault { malformed, read } = match read {
        Ok(meta) => return (meta, None),
        Err(fault) => fault,
    };
    let key_lost = malformed.iter().any(|field| matches!(*field, "key" | "meta"));
    let key = if key_lost { fresh_key() } else { read.key };
    let line = format!(
        "session {id}: its stored meta is malformed ({}); adopted detached, with no owner{}",
        malformed.join(", "),
        if key_lost { " and a new key" } else { "" }
    );
    let meta = SessionMeta {
        key,
        owner: String::new(),
        policy: Policy::Detach,
        ..read
    };
    (meta, Some(line))
}

/// The layout a client gave session `id` (`DETACH`, `SET_LAYOUT`), unless its
/// JSON is over `MAX_LAYOUT`: then none is kept, and that is logged.
fn kept_layout(
    id: SessionId,
    layout: Option<brindle_remote_protocol::serde_json::Value>,
) -> Option<brindle_remote_protocol::serde_json::Value> {
    layout.filter(|layout| {
        let size = brindle_remote_protocol::serde_json::to_vec(layout)
            .map_or(usize::MAX, |bytes| bytes.len());
        let kept = size <= client::MAX_LAYOUT;
        if !kept {
            log!(
                "session {id}: not keeping its {size}-byte layout (over {})",
                client::MAX_LAYOUT
            );
        }
        kept
    })
}

/// A new session's key: 128 random bits as hex, from `/dev/urandom`. If
/// that can't be read, the time, pid and a counter stand in: unique enough
/// for telling one host's sessions apart.
fn session_key() -> String {
    use std::io::Read as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    let mut bytes = [0u8; 16];
    let random = std::fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut bytes));
    if random.is_err() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let tag = (u64::from(std::process::id()) << 32) | COUNTER.fetch_add(1, Ordering::Relaxed);
        bytes[..8].copy_from_slice(&nanos.to_le_bytes());
        bytes[8..].copy_from_slice(&tag.to_le_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The command line `list` shows; empty for the login shell.
fn command_line(command: Option<&str>, args: &[String]) -> String {
    match command {
        Some(command) => std::iter::once(command)
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        None => String::new(),
    }
}

/// The first free session id from `next` on, and the `next` after it: ids
/// wrap past `u32::MAX` to 1, and skip 0 (never an id) and those `taken`.
fn free_id(mut next: SessionId, taken: impl Fn(SessionId) -> bool) -> (SessionId, SessionId) {
    loop {
        let id = next;
        next = next.checked_add(1).unwrap_or(1);
        if id != 0 && !taken(id) {
            return (id, next);
        }
    }
}

impl<L: EventLoop> Core<L> {
    /// A client's `SPAWN`: it gets an id now and goes to a holder of the
    /// core's build once one is ready. Its size is taken within the limits
    /// every grid keeps ([`emulator::clamped`]), here, so the session's
    /// emulator, its program's terminal and its listing all have the same.
    pub(super) fn client_spawn(&mut self, client: Token, mut spawn: client::Spawn) {
        (spawn.cols, spawn.rows) = emulator::clamped(spawn.cols, spawn.rows);
        self.queue_spawn(client, SpawnRequest::Terminal(spawn));
    }

    /// A spawn gets an id and waits for a holder of the core's build that
    /// takes its kind ([`policy::placement`]); refused while the core is
    /// stopping or stepping down.
    pub(super) fn queue_spawn(&mut self, client: Token, spawn: SpawnRequest) {
        let refusal = if self.stopping.is_some() {
            Some("the server is stopping")
        } else if self.stepping_down.is_some() {
            Some("the core is stepping down for an upgrade; try again")
        } else {
            None
        };
        if let Some(refusal) = refusal {
            return self.send_client(client, error(Some(spawn.req()), None, refusal));
        }
        let id = self.allocate_id();
        self.waiting.push(WaitingSpawn { client, id, spawn });
        self.place_waiting();
    }

    fn allocate_id(&mut self) -> SessionId {
        let (id, next) = free_id(self.next_session, |id| {
            self.sessions.contains_key(&id) || self.waiting.iter().any(|waiting| waiting.id == id)
        });
        self.next_session = next;
        id
    }

    pub(super) fn send_spawn(&mut self, holder: Token, waiting: WaitingSpawn) {
        let WaitingSpawn { client, id, spawn } = waiting;
        let owner = self.client_token(client).cloned();
        let session = Session::spawning(holder, client, owner, spawn.clone(), Instant::now());
        let meta = session.stored_meta().to_value();
        let message = match spawn {
            SpawnRequest::Terminal(spawn) => ToHolder::Spawn(h1::Spawn {
                session: id,
                command: spawn.command,
                args: spawn.args,
                cwd: spawn.cwd,
                env: spawn.env,
                cols: spawn.cols,
                rows: spawn.rows,
                cell_w: spawn.cell_w,
                cell_h: spawn.cell_h,
                meta,
            }),
            // The agent starts in the directory its `session/new` names.
            SpawnRequest::Agent { spawn, cwd } => ToHolder::SpawnPipes(h1::SpawnPipes {
                session: id,
                command: spawn.command,
                args: spawn.args,
                cwd: Some(cwd),
                env: spawn.env,
                meta,
            }),
        };
        // The session first: a `SPAWN` that can't be sent fails it, to its
        // client, as the holder's `SPAWN_FAILED` would.
        self.sessions.insert(id, session);
        self.send_holder(holder, message);
    }

    /// The session if it runs under `holder`; anything else is a stale or
    /// confused report.
    fn holder_session(&mut self, holder: Token, id: SessionId) -> Option<&mut Session> {
        let session = self.sessions.get_mut(&id);
        if session
            .as_ref()
            .is_some_and(|session| session.holder != holder)
        {
            log!("holder {holder} reported session {id}, which isn't its");
            return None;
        }
        session
    }

    /// The holder reports the session's program as `pid`. A session still
    /// starting runs from now on (or is hung up, if it was told to be
    /// meanwhile), and its client's `SPAWN` is answered. A pid of 0, which
    /// no holder reports, is logged and ignored.
    pub(super) fn session_spawned(&mut self, holder: Token, id: SessionId, pid: u32) {
        let now = Instant::now();
        let Some(reported) = NonZeroU32::new(pid) else {
            return log!("holder {holder} reported session {id}'s program as pid 0; ignored");
        };
        let Some(session) = self.holder_session(holder, id) else {
            return;
        };
        session.tell_foreground(now);
        let (life, req) = match &session.life {
            Life::Starting {
                spawn,
                hung_up: false,
            } => (Life::Running { pid: reported }, Some(spawn.req())),
            Life::Starting {
                spawn,
                hung_up: true,
            } => (Life::HungUp { pid: reported }, Some(spawn.req())),
            Life::Running { .. } => (Life::Running { pid: reported }, None),
            Life::HungUp { .. } => (Life::HungUp { pid: reported }, None),
            exited @ Life::ExitedAwaitingOwner { .. } => (exited.clone(), None),
        };
        session.life = life;
        // An agent that starts now (not one hung up before it started) is
        // sent `initialize`, after its client has its `SPAWNED`.
        let agent_started = req.is_some() && session.life.is_live() && session.agent().is_some();
        let client = session.ownership.client();
        let key = session.meta.key.clone();
        log!("session {id}: pid {pid} ({:?})", session.meta.command);
        if let (Some(client), Some(req)) = (client, req) {
            self.send_client(
                client,
                ToClient::Spawned(Spawned {
                    req,
                    session: id,
                    pid,
                    key,
                }),
            );
            if self.client_is_over(client) {
                self.set_paused(id, true);
            }
        }
        if agent_started {
            self.agent_step(id, Input::Spawned);
        }
    }

    pub(super) fn session_spawn_failed(&mut self, holder: Token, id: SessionId, message: String) {
        if self.holder_session(holder, id).is_none() {
            return;
        }
        let Some(session) = self.sessions.remove(&id) else {
            return;
        };
        log!("session {id}: {message}");
        if let (Some(client), Life::Starting { spawn, .. }) = (session.ownership.client(), session.life)
        {
            self.send_client(client, error(Some(spawn.req()), None, message));
        }
    }

    /// A session whose holder went away: one still starting goes to another
    /// holder; the rest have ended.
    pub(super) fn session_lost(&mut self, holder: Token, id: SessionId) {
        if self.holder_session(holder, id).is_none() {
            return;
        }
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        if session.life.is_starting() {
            let Some(session) = self.sessions.remove(&id) else {
                return;
            };
            if let (Some(client), Life::Starting { spawn, .. }) =
                (session.ownership.client(), session.life)
            {
                self.waiting.push(WaitingSpawn { client, id, spawn });
            }
            return;
        }
        // A client attached now is told the session was lost, not that its
        // program exited, so it can keep the tab open and say why. One
        // still waiting for its replay gets that first (it answers its
        // `ATTACH`), and an `EXIT` kept for it ends the session there.
        if let Some(client) = session.ownership.client() {
            log!("session {id}: lost with its holder");
            // An agent's log says it is gone, before the session's end.
            self.agent_exited(id, None);
            self.send_waiting_replay(id);
            if self.sessions.remove(&id).is_none() {
                return;
            }
            return self.send_client(
                client,
                ToClient::Exit(Exit {
                    session: id,
                    code: None,
                    lost: Some("its holder ended".into()),
                }),
            );
        }
        self.session_exited(holder, id, None);
    }

    pub(super) fn session_output(&mut self, holder: Token, id: SessionId, bytes: Vec<u8>) {
        let now = Instant::now();
        let Some(session) = self.holder_session(holder, id) else {
            return;
        };
        // An agent's stdout goes to its reducer, whose events its client
        // gets instead.
        let Some(screen) = session.screen() else {
            self.agent_step(id, Input::Stdout(&bytes));
            return;
        };
        // Attached or not, so a detached session's screen stays current.
        // Checkpoints wait for the end of the read (`checkpoint_due`).
        screen.feed(id, &bytes);
        session.mark_active(now);
        // A client still waiting for its replay gets this output in it.
        if let Some(client) = session.ownership.client()
            && session.pending_replay.is_none()
        {
            self.send_client(client, ToClient::Data(Data { session: id, bytes }));
        }
    }

    /// An adopted session's checkpoint from its holder: the rebuild starts
    /// from it, unless the tail was truncated (and so doesn't follow it).
    /// An agent's is kept for its adoption, which the tail completes
    /// ([`Core::agent_adopted`]).
    pub(super) fn session_checkpoint(&mut self, holder: Token, id: SessionId, bytes: &[u8]) {
        let Some(session) = self.holder_session(holder, id) else {
            return;
        };
        let pending = matches!(session.rebuild, Rebuild::Pending { .. });
        let clean = session.rebuild == (Rebuild::Pending { truncated: false });
        match &mut session.content {
            Content::Terminal(screen) if clean => screen.restore(id, bytes),
            Content::Terminal(_) => {}
            Content::Agent(agent) if pending => agent.checkpoint_read(bytes),
            Content::Agent(_) => {}
        }
    }

    /// An adopted session's tail, the last thing its holder sends for it:
    /// the rebuild ends with it, and a replay waiting for that is sent.
    pub(super) fn session_tail(&mut self, holder: Token, id: SessionId, bytes: &[u8]) {
        let Some(session) = self.holder_session(holder, id) else {
            return;
        };
        let Rebuild::Pending { truncated } = session.rebuild else {
            return;
        };
        session.rebuild = Rebuild::Ready;
        match session.screen() {
            Some(screen) => {
                if truncated {
                    log!(
                        "session {id}: its tail was truncated; rebuilt from the last {} bytes only",
                        bytes.len()
                    );
                    screen.restore(id, b"\x1bc");
                }
                screen.restore(id, bytes);
                screen.since_checkpoint = bytes.len() as u64;
            }
            None => self.agent_adopted(id, truncated, bytes),
        }
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if let (Some(client), Some(pending)) =
            (session.ownership.client(), session.pending_replay.take())
        {
            self.replay(client, id, pending.req, pending.size);
        }
    }

    /// Sends the session's holder a checkpoint of its screen (or an agent's
    /// state, [`Core::agent_checkpoint`]), covering the output fed since
    /// the last one: only for a session whose program runs and isn't being
    /// hung up, once its emulator is rebuilt.
    pub(super) fn checkpoint(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        match (session.rebuild, session.life.is_live()) {
            (Rebuild::Ready, true) => {}
            (Rebuild::Ready, false) | (Rebuild::Pending { .. }, true | false) => return,
        }
        let Some(screen) = session.screen() else {
            return self.agent_checkpoint(id);
        };
        let bytes = screen.render(id);
        let covers = std::mem::take(&mut screen.since_checkpoint);
        let holder = session.holder;
        for message in ToHolder::checkpoint(id, &bytes, covers) {
            self.send_holder(holder, message);
        }
    }

    /// After a read of `holder`'s output: checkpoints its terminal sessions
    /// that are due (see [`is_due`]). With `may_defer`, they wait while more
    /// of the holder's output is waiting. (Agent sessions are checked after
    /// each step, [`policy::agent_checkpoint_due`].)
    pub(super) fn checkpoint_due(&mut self, holder: Token, may_defer: bool) {
        let mut due: Vec<(SessionId, u64)> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.holder == holder)
            .filter_map(|(&id, session)| match &session.content {
                Content::Terminal(screen) => Some((id, screen.since_checkpoint)),
                Content::Agent(_) => None,
            })
            .filter(|&(_, since)| is_due(since, false))
            .collect();
        if due.is_empty() {
            return;
        }
        let behind = may_defer && self.holder_behind(holder);
        due.retain(|&(_, since)| is_due(since, behind));
        for (id, _) in due {
            self.checkpoint(id);
        }
    }

    /// Checkpoints every session, before the core steps down.
    pub(super) fn checkpoint_all(&mut self) {
        let ids: Vec<SessionId> = self.sessions.keys().copied().collect();
        for id in ids {
            self.checkpoint_at(id, AgentCheckpoint::StepDown);
        }
    }

    /// The checkpoint a step-down or a detach takes: a terminal's always,
    /// an agent's as [`policy::agent_checkpoint_due`] says.
    fn checkpoint_at(&mut self, id: SessionId, at: AgentCheckpoint) {
        let due = match self.sessions.get(&id).map(Session::kind) {
            Some(SessionKind::Terminal) => true,
            Some(SessionKind::Agent) => policy::agent_checkpoint_due(at, 0),
            None => false,
        };
        if due {
            self.checkpoint(id);
        }
    }

    /// The program exited (`code` is `None` for a signal, or a lost holder):
    /// the client gets `EXIT` and the session is gone. An adopted or
    /// lingering session whose owner may still come back keeps its exit for
    /// the owner.
    pub(super) fn session_exited(&mut self, holder: Token, id: SessionId, code: Option<i32>) {
        if self.holder_session(holder, id).is_none() {
            return;
        }
        log!("session {id}: exited ({code:?})");
        // An agent's log gets its exit (and the stderr tail that came
        // before it) first, so its client, or a replay, has it before the
        // session's `EXIT`.
        self.agent_exited(id, code);
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        // Whatever happens to the session, a replay of it now (one kept for
        // its owner, or one a client is still waiting for) shows no
        // contexts: the program that announced them is gone.
        if let Some(screen) = session.screen() {
            screen.process_exited(id);
        }
        match policy::exit_fate(&session.ownership, &session.life) {
            ExitFate::KeepForOwner => return session.life.exited(code),
            ExitFate::Report => {}
        }
        // A client still waiting for its replay gets it now, then the
        // `EXIT`: the replay answers its `ATTACH`.
        if session.pending_replay.is_some() {
            session.life.exited(code);
            return self.send_waiting_replay(id);
        }
        let Some(session) = self.sessions.remove(&id) else {
            return;
        };
        if let Some(client) = session.ownership.client() {
            self.send_client(client, ToClient::Exit(Exit { session: id, code, lost: None }));
        }
    }

    /// A returning owner (`HELLO` with the session's `client_token`) gets
    /// back every session of its own that has no client
    /// ([`policy::returning`]), each after its replay (`REPLAY_START`
    /// without a `req`). One that exited meanwhile gets just its `EXIT`.
    pub(super) fn reattach(&mut self, client: Token) {
        let Some(token) = self.client_token(client) else {
            return;
        };
        let Some(protocol) = self.clients.get(&client).map(|entry| entry.protocol) else {
            return;
        };
        let sessions: Vec<(SessionId, SessionView<'_>)> = self
            .sessions
            .iter()
            .map(|(&id, session)| (id, session.view()))
            .collect();
        let returning = policy::returning(&sessions, token, protocol);
        if !returning.is_empty() {
            log!(
                "client {client} is back for {} session(s): {returning:?}",
                returning.len()
            );
        }
        for id in returning {
            let Some(session) = self.sessions.get(&id) else {
                continue;
            };
            match session.life {
                Life::ExitedAwaitingOwner { code, .. } => {
                    self.sessions.remove(&id);
                    self.send_client(client, ToClient::Exit(Exit { session: id, code, lost: None }));
                }
                Life::Starting { .. } | Life::Running { .. } | Life::HungUp { .. } => {
                    self.replay(client, id, None, None);
                }
            }
        }
    }

    /// The reconnect grace period is over: adopted sessions whose owners
    /// didn't come back meet their [`policy::grace_fate`].
    pub(super) fn end_grace(&mut self) {
        let orphans: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, session)| match session.ownership {
                Ownership::AwaitingOwner { .. } => true,
                Ownership::Attached(_) | Ownership::Lingering { .. } | Ownership::Detached { .. } => {
                    false
                }
            })
            .map(|(&id, _)| id)
            .collect();
        for id in orphans {
            let Some(session) = self.sessions.get_mut(&id) else {
                continue;
            };
            session.ownership.stop_waiting();
            match policy::grace_fate(session.meta.policy, &session.life) {
                GraceFate::Forget => {
                    self.sessions.remove(&id);
                }
                GraceFate::HangUp => {
                    log!("session {id}: its owner didn't come back; hanging up");
                    self.hang_up(id);
                }
                GraceFate::LeaveDetached => {
                    log!("session {id}: detached; its owner didn't come back");
                }
            }
        }
    }

    /// The client's connection ended at `now`: each of its sessions meets
    /// its [`policy::gone_fate`].
    pub(super) fn client_gone(&mut self, client: Token, linger: Duration, now: Instant) {
        for id in self.sessions_of(client) {
            self.detach_session(id, now);
            let Some(session) = self.sessions.get_mut(&id) else {
                continue;
            };
            match policy::gone_fate(session.meta.policy, session.life.is_starting(), linger) {
                Fate::HangUp => self.hang_up(id),
                Fate::Linger(linger) => {
                    log!(
                        "session {id}: its client left; it waits {}s for it",
                        linger.as_secs()
                    );
                    session.ownership = Ownership::Lingering {
                        until: now + linger,
                        since: now,
                    };
                    self.checkpoint_at(id, AgentCheckpoint::Detach);
                }
                Fate::Keep => {
                    log!("session {id}: detached");
                    self.checkpoint_at(id, AgentCheckpoint::Detach);
                }
            }
        }
    }

    /// The client was replaced by a newer connection with its token
    /// ([`Core::take_over`]) at `now`: its sessions lose their client but
    /// keep their owner, whatever their policy, so [`Core::reattach`] gives
    /// them to the new connection. A session still starting is hung up, as
    /// its `SPAWN` failed with the old connection.
    pub(super) fn client_replaced(&mut self, client: Token, now: Instant) {
        for id in self.sessions_of(client) {
            self.detach_session(id, now);
            if self
                .sessions
                .get(&id)
                .is_some_and(|session| session.life.is_starting())
            {
                log!("session {id}: its client was replaced before it started; hanging up");
                self.hang_up(id);
            }
        }
    }

    /// Lingering `kill` sessions whose client didn't come back in time are
    /// hung up; one whose program exited meanwhile is forgotten.
    pub(super) fn end_lingering(&mut self, now: Instant) {
        let expired: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, session)| match session.ownership {
                Ownership::Lingering { until, .. } => now >= until,
                Ownership::Attached(_)
                | Ownership::AwaitingOwner { .. }
                | Ownership::Detached { .. } => false,
            })
            .map(|(&id, _)| id)
            .collect();
        for id in expired {
            let Some(session) = self.sessions.get_mut(&id) else {
                continue;
            };
            session.ownership.stop_waiting();
            match session.life {
                Life::ExitedAwaitingOwner { .. } => {
                    self.sessions.remove(&id);
                }
                Life::Starting { .. } | Life::Running { .. } | Life::HungUp { .. } => {
                    log!("session {id}: its client didn't come back in time; hanging up");
                    self.hang_up(id);
                }
            }
        }
    }

    /// `KILL`: hangs up one session or all of them, whatever their policy.
    pub(super) fn client_kill(&mut self, client: Token, kill: Kill) {
        let targets: Vec<SessionId> = match kill.target {
            KillTarget::All => self.sessions.keys().copied().collect(),
            KillTarget::Session(id) if self.sessions.contains_key(&id) => vec![id],
            KillTarget::Session(id) => {
                return self.send_client(
                    client,
                    error(Some(kill.req), Some(id), format!("no session {id}")),
                );
            }
        };
        for id in targets {
            self.end_session(id);
        }
        self.send_client(client, ToClient::Ok(Request { req: kill.req }));
    }

    /// `STOP`: every session is hung up, and the core exits (answering
    /// `OK`) once they have all ended.
    pub(super) fn client_stop(&mut self, client: Token, req: RequestId) {
        log!("stopping: hanging up {} session(s)", self.sessions.len());
        self.stopping = Some(super::Answer { client, req });
        for waiting in std::mem::take(&mut self.waiting) {
            let req = waiting.spawn.req();
            self.send_client(
                waiting.client,
                error(Some(req), None, "the server is stopping"),
            );
        }
        let all: Vec<SessionId> = self.sessions.keys().copied().collect();
        for id in all {
            self.end_session(id);
        }
    }

    /// Hangs a session up, or forgets one that already exited.
    fn end_session(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        match session.life {
            Life::ExitedAwaitingOwner { .. } => {
                self.sessions.remove(&id);
            }
            Life::Starting { .. } | Life::Running { .. } | Life::HungUp { .. } => self.hang_up(id),
        }
    }

    /// Asks the holder to hang the session up, once; its `EXITED` follows.
    /// A session whose program exited has nothing to hang up.
    pub(super) fn hang_up(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if !session.life.hang_up() {
            return;
        }
        let holder = session.holder;
        self.send_holder(holder, ToHolder::Hangup(h1::SessionRef { session: id }));
    }

    /// The session loses its client at `now`, and isn't paused for it any
    /// more. A replay still waiting for that client is dropped.
    pub(super) fn detach_session(&mut self, id: SessionId, now: Instant) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.ownership = Ownership::Detached { since: now };
            session.pending_replay = None;
        }
        self.set_paused(id, false);
    }

    pub(super) fn sessions_of(&self, client: Token) -> Vec<SessionId> {
        self.sessions
            .iter()
            .filter(|(_, session)| session.ownership.client() == Some(client))
            .map(|(&id, _)| id)
            .collect()
    }

    fn set_paused(&mut self, id: SessionId, paused: bool) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.paused == paused {
            return;
        }
        session.paused = paused;
        let holder = session.holder;
        let target = h1::SessionRef { session: id };
        self.send_holder(
            holder,
            if paused {
                ToHolder::Pause(target)
            } else {
                ToHolder::Resume(target)
            },
        );
    }

    /// Pauses (or resumes) every session whose output goes to `client`.
    pub(super) fn pause_client_sessions(&mut self, client: Token, paused: bool) {
        for id in self.sessions_of(client) {
            self.set_paused(id, paused);
        }
    }

    /// The session, if it is attached to `client`; otherwise the `ERROR` is
    /// sent and `None` returned.
    fn client_session(&mut self, client: Token, id: SessionId) -> Option<&mut Session> {
        self.attached_session(client, id, None)
    }

    /// The same, with the `ERROR` answering request `req`.
    pub(super) fn attached_session(
        &mut self,
        client: Token,
        id: SessionId,
        req: Option<RequestId>,
    ) -> Option<&mut Session> {
        if self
            .sessions
            .get(&id)
            .is_some_and(|session| session.ownership.client() == Some(client))
        {
            return self.sessions.get_mut(&id);
        }
        let message = if self.sessions.contains_key(&id) {
            format!("session {id} isn't attached to this client")
        } else {
            format!("no session {id}")
        };
        self.send_client(client, error(req, Some(id), message));
        None
    }

    /// The session attached to `client` if it is a terminal; an agent
    /// session's `ERROR` says it takes no `what`.
    fn terminal_session(&mut self, client: Token, id: SessionId, what: &str) -> Option<&mut Session> {
        let session = self.client_session(client, id)?;
        match session.kind() {
            SessionKind::Terminal => self.sessions.get_mut(&id),
            SessionKind::Agent => {
                let message = format!("session {id} is an agent session, which takes no {what}");
                self.send_client(client, error(None, Some(id), message));
                None
            }
        }
    }

    pub(super) fn client_input(&mut self, client: Token, data: Data) {
        let now = Instant::now();
        let Some(session) = self.terminal_session(client, data.session, "input") else {
            return;
        };
        if let Some(screen) = session.screen() {
            screen.input_seen(data.session);
        }
        session.mark_active(now);
        let holder = session.holder;
        self.send_holder(
            holder,
            ToHolder::Input(h1::Data {
                session: data.session,
                bytes: data.bytes,
            }),
        );
    }

    /// `RESIZE`; an agent session has no size, so its `RESIZE` is refused
    /// here and never reaches its holder.
    pub(super) fn client_resize(&mut self, client: Token, resize: Resize) {
        let Some(session) = self.terminal_session(client, resize.session, "size") else {
            return;
        };
        // It came after the `ATTACH` whose replay still waits, so that
        // one's size, which applies after the replay, must not undo it.
        if let Some(pending) = &mut session.pending_replay {
            pending.size = None;
        }
        self.resize_session(
            resize.session,
            resize.cols,
            resize.rows,
            resize.cell_w,
            resize.cell_h,
        );
    }

    /// `REPARENT` (Windows): the session's holder makes the client's window
    /// the owner of the session's console window. It is advisory and has no
    /// reply; only a session that isn't this client's is refused.
    #[cfg(windows)]
    pub(super) fn client_reparent(&mut self, client: Token, reparent: client::Reparent) {
        let Some(session) = self.terminal_session(client, reparent.session, "console window")
        else {
            return;
        };
        let holder = session.holder;
        self.send_holder(
            holder,
            ToHolder::Reparent(h1::Reparent {
                session: reparent.session,
                hwnd: reparent.hwnd,
            }),
        );
    }

    /// `REPARENT` is for Windows: a session here has no console window.
    #[cfg(not(windows))]
    pub(super) fn client_reparent(&mut self, client: Token, reparent: client::Reparent) {
        self.send_client(
            client,
            error(
                None,
                Some(reparent.session),
                "REPARENT is for Windows; this core runs on Linux",
            ),
        );
    }

    /// `ATTACH`: a session with no client gets this one, after a replay of
    /// its screen; then the client's size applies. A session that has a
    /// client is refused. When the attach changes the owner, the replay
    /// waits until the holder's socket has the change (`store_meta`).
    pub(super) fn client_attach(&mut self, client: Token, attach: client::Attach) {
        let id = attach.session;
        let session = self.sessions.get(&id).map(Session::view);
        let protocol = self.clients.get(&client).map_or(0, |entry| entry.protocol);
        if let Some(refusal) = policy::attach_refusal(id, session, client, protocol) {
            let message = refusal.to_string();
            return self.send_client(client, error(Some(attach.req), Some(id), message));
        }
        log!("session {id}: attached to client {client}");
        // Its owner becomes the client's token, or none when its `HELLO`
        // named none.
        let owner = self.clients.get(&client).map(|entry| entry.token.clone());
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        // An attached session's layout is its client's window: the one it
        // was detached with no longer says where it is.
        let mut changed = session.meta.layout.take().is_some();
        if let Some(owner) = owner
            && session.owner != owner
        {
            session.owner = owner;
            changed = true;
        }
        if changed {
            self.store_meta(id);
        }
        self.replay(
            client,
            id,
            Some(attach.req),
            Some((attach.cols, attach.rows)),
        );
    }

    /// `DETACH`: the session keeps running with no client, whatever its
    /// policy: a `kill` session becomes `detach`. It also loses its owner,
    /// so no reconnect gives it back to a client that let it go. The layout
    /// it comes with is kept in its meta, unless it is over `MAX_LAYOUT`.
    pub(super) fn client_detach(&mut self, client: Token, detach: client::Detach) {
        let id = detach.session;
        if self.client_session(client, id).is_none() {
            return;
        }
        log!("session {id}: detached by client {client}");
        // A replay it still waited for answers its `ATTACH` first (and, for
        // a session whose program had exited, ends it with its `EXIT`).
        self.send_waiting_replay(id);
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.owner = None;
        session.meta.policy = Policy::Detach;
        session.meta.layout = kept_layout(id, detach.layout);
        self.store_meta(id);
        self.detach_session(id, Instant::now());
        self.checkpoint_at(id, AgentCheckpoint::Detach);
    }

    /// `SET_LAYOUT` (protocol 3): the attached client's layout for the
    /// session replaces the one it had, as a `DETACH`'s would, so a client
    /// that ends without detaching leaves its last one. It reaches the
    /// holder only when it changed. Refused on a connection below protocol
    /// 3, and from a client that doesn't have the session attached.
    pub(super) fn client_set_layout(&mut self, client: Token, set: client::SetLayout) {
        let id = set.session;
        if self.clients.get(&client).is_some_and(|c| c.protocol < 3) {
            let message = "SET_LAYOUT needs client protocol 3";
            return self.send_client(client, error(None, Some(id), message));
        }
        let Some(session) = self.client_session(client, id) else {
            return;
        };
        let layout = kept_layout(id, set.layout);
        if session.meta.layout != layout {
            session.meta.layout = layout;
            self.store_meta(id);
        }
    }

    /// Sends the session's meta to its holder (`SET_META`), so a later core
    /// that adopts it knows its owner and policy. A holder driven in H1
    /// version 1 can't take it: the change holds only in this core.
    ///
    /// The message is only queued here. Until the holder's socket has taken
    /// it (`Session::storing`), a core killed now would leave the holder
    /// with the old meta, so the session's replays wait for that: a client
    /// is told it has a session only once the next core would agree.
    pub(super) fn store_meta(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let holder = session.holder;
        let meta = session.stored_meta().to_value();
        match self.holders.get(&holder).and_then(|holder| holder.protocol()) {
            Some(protocol) if protocol >= 2 => {
                // One that couldn't be sent leaves `storing` as it was: no
                // replay waits on bytes that never went out.
                if !self.send_holder(holder, ToHolder::SetMeta(h1::SetMeta { session: id, meta })) {
                    return;
                }
                let end = self.holder_queue_end(holder);
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.storing = end;
                }
            }
            _ => log!("session {id}: its holder speaks H1 version 1; its owner and policy last only in this core"),
        }
    }

    /// After the holders' flush: sessions whose `SET_META` their holder's
    /// socket has now taken (or whose holder is gone) stop holding their
    /// replays back.
    pub(super) fn send_stored_replays(&mut self) {
        let stored: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                session
                    .storing
                    .is_some_and(|end| self.holder_has(session.holder, end))
            })
            .map(|(&id, _)| id)
            .collect();
        for id in stored {
            let Some(session) = self.sessions.get_mut(&id) else {
                continue;
            };
            session.storing = None;
            // One that also waits for the rebuild goes out after its tail.
            if session.rebuild == Rebuild::Ready
                && let (Some(client), Some(pending)) =
                    (session.ownership.client(), session.pending_replay.take())
            {
                self.replay(client, id, pending.req, pending.size);
            }
        }
    }

    /// Sends the replay the session's client still waits for without
    /// waiting for its meta to be stored: the session is ending, or the
    /// client is letting it go. A rebuild is still waited for.
    fn send_waiting_replay(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.storing = None;
        if let (Some(client), Some(pending)) =
            (session.ownership.client(), session.pending_replay.take())
        {
            self.replay(client, id, pending.req, pending.size);
        }
    }

    /// Gives a session with no client to `client`: `REPLAY_START`, the
    /// replay as `DATA`, `REPLAY_END`; then `size` applies, and its live
    /// output follows. A session whose program exited meanwhile gets its
    /// `EXIT` after the replay. One still being rebuilt from its holder
    /// gets its replay once the rebuild is done (no output comes before),
    /// and one whose meta isn't in its holder's socket yet once it is
    /// (output read meanwhile is in the replay, not sent before it).
    pub(super) fn replay(
        &mut self,
        client: Token,
        id: SessionId,
        req: Option<RequestId>,
        size: Option<(u16, u16)>,
    ) {
        let now = Instant::now();
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.ownership = Ownership::Attached(client);
        session.tell_foreground(now);
        if session.rebuild != Rebuild::Ready || session.storing.is_some() {
            session.pending_replay = Some(PendingReplay { req, size });
            return;
        }
        let pid = session.life.pid().map_or(0, NonZeroU32::get);
        let exited = match session.life {
            Life::ExitedAwaitingOwner { code, .. } => Some(code),
            Life::Starting { .. } | Life::Running { .. } | Life::HungUp { .. } => None,
        };
        // Rendered whole, in this turn of the loop: output read after it
        // is fed and forwarded after it, so nothing is lost or reordered.
        // An agent's replay is its whole log as transcript events
        // (`Transcript::replay`), after which its live events follow.
        let body = match &mut session.content {
            Content::Terminal(screen) => Replay::Screen(screen.render(id)),
            Content::Agent(agent) => Replay::Transcript(agent.replay()),
        };
        self.send_client(
            client,
            ToClient::ReplayStart(client::ReplayStart {
                req,
                session: id,
                pid,
            }),
        );
        let resizes = match body {
            Replay::Screen(bytes) => {
                for chunk in bytes.chunks(REPLAY_CHUNK) {
                    self.send_client(
                        client,
                        ToClient::Data(Data {
                            session: id,
                            bytes: chunk.to_vec(),
                        }),
                    );
                }
                true
            }
            Replay::Transcript(events) => {
                self.send_transcript(client, id, events);
                false
            }
        };
        self.send_client(
            client,
            ToClient::ReplayEnd(client::SessionRef { session: id }),
        );
        if let Some(code) = exited {
            self.sessions.remove(&id);
            return self.send_client(client, ToClient::Exit(Exit { session: id, code, lost: None }));
        }
        // An agent session has no size: an `ATTACH`'s is ignored.
        if resizes
            && let Some((cols, rows)) = size
            && cols > 0
            && rows > 0
        {
            self.resize_session(id, cols, rows, 0, 0);
        }
        if self.client_is_over(client) {
            self.set_paused(id, true);
        }
    }

    /// Resizes the session's emulator and its PTY to `cols` × `rows`,
    /// within the limits every grid keeps ([`emulator::clamped`]).
    fn resize_session(&mut self, id: SessionId, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let (cols, rows) = emulator::clamped(cols, rows);
        let Some(screen) = session.screen() else {
            return;
        };
        screen.resize(id, cols, rows);
        session.cols = cols;
        session.rows = rows;
        let holder = session.holder;
        self.send_holder(
            holder,
            ToHolder::Resize(h1::Resize {
                session: id,
                cols,
                rows,
                cell_w,
                cell_h,
            }),
        );
    }

    /// `CLOSE`: the session ends, whatever its policy; its `EXIT` follows.
    pub(super) fn client_close(&mut self, client: Token, id: SessionId) {
        if self.client_session(client, id).is_some() {
            self.hang_up(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoints_wait_while_the_holder_is_ahead() {
        assert!(!is_due(CHECKPOINT_AFTER, false));
        assert!(is_due(CHECKPOINT_AFTER + 1, false));
        assert!(!is_due(CHECKPOINT_AFTER + 1, true));
        assert!(!is_due(CHECKPOINT_AFTER_BEHIND, true));
        assert!(is_due(CHECKPOINT_AFTER_BEHIND + 1, true));
    }

    fn held(truncated: bool) -> HeldSession {
        let meta = SessionMeta {
            key: "k1".into(),
            owner: "tok".into(),
            scrollback: 100,
            ..SessionMeta::default()
        };
        let meta = meta.to_value();
        HeldSession { session: 3, pid: 42, cols: 80, rows: 24, exit: None, meta, truncated, mode: h1::SpawnMode::Pty }
    }

    #[test]
    fn a_good_meta_is_adopted_as_it_is() {
        let meta = SessionMeta {
            key: "k1".into(),
            owner: "tok".into(),
            policy: Policy::Kill,
            scrollback: 100,
            ..SessionMeta::default()
        };
        let fresh = || -> String { panic!("a good meta keeps its key") };
        assert_eq!(adopt_meta(3, Ok(meta.clone()), fresh), (meta, None));
    }

    #[test]
    fn a_faulty_meta_is_adopted_detached_with_no_owner() {
        // What was read of `{"command": "bash", "key": "k1", "owner": 7,
        // "policy": "kill", "scrollback": 100}`.
        let read = SessionMeta {
            command: "bash".into(),
            key: "k1".into(),
            policy: Policy::Kill,
            scrollback: 100,
            ..SessionMeta::default()
        };
        let fault = MetaFault { malformed: vec!["owner"], read: read.clone() };
        let fresh = || -> String { panic!("the key was well formed") };
        let (meta, line) = adopt_meta(3, Err(fault), fresh);
        assert_eq!(
            meta,
            SessionMeta {
                owner: String::new(),
                policy: Policy::Detach,
                ..read
            }
        );
        assert_eq!(
            line.as_deref(),
            Some("session 3: its stored meta is malformed (owner); adopted detached, with no owner")
        );
    }

    #[test]
    fn a_malformed_key_is_replaced() {
        let read = SessionMeta {
            owner: "tok".into(),
            scrollback: 100,
            ..SessionMeta::default()
        };
        let fault = MetaFault { malformed: vec!["key", "started"], read };
        let (meta, line) = adopt_meta(4, Err(fault), || "fresh".to_string());
        assert_eq!((meta.key.as_str(), meta.owner.as_str()), ("fresh", ""));
        assert_eq!((meta.policy, meta.scrollback), (Policy::Detach, 100));
        let line = line.unwrap();
        assert!(line.starts_with("session 4: "), "{line}");
        assert!(line.contains("(key, started)") && line.ends_with("and a new key"), "{line}");

        // A meta that isn't an object has no key to keep either.
        let fault = MetaFault { malformed: vec!["meta"], read: SessionMeta::default() };
        let (meta, _) = adopt_meta(5, Err(fault), || "other".to_string());
        assert_eq!(meta.key, "other");
    }

    fn pid(pid: u32) -> NonZeroU32 {
        NonZeroU32::new(pid).unwrap()
    }

    fn adopted(held: HeldSession, now: Instant) -> Session {
        let (session, repaired) = Session::adopted(7, held, now).unwrap();
        assert_eq!(repaired, None);
        session
    }

    #[test]
    fn an_adopted_session_is_detached_and_active_since_now() {
        let now = Instant::now();
        let session = adopted(held(true), now);
        assert_eq!((session.holder, session.cols, session.rows), (7, 80, 24));
        assert_eq!(session.life, Life::Running { pid: pid(42) });
        // Its owner is taken out of its meta, and put back for its holder.
        assert_eq!(session.owner.as_ref().map(ClientToken::as_str), Some("tok"));
        assert_eq!((session.meta.key.as_str(), session.meta.owner.as_str()), ("k1", ""));
        assert_eq!(session.stored_meta().owner, "tok");
        // No client until its owner comes back, and detached since now.
        assert_eq!(session.ownership, Ownership::AwaitingOwner { since: now });
        assert_eq!(session.ownership.client(), None);
        assert_eq!(session.ownership.detached_since(), Some(now));
        // Not active, as of now: its first output marks it.
        assert!(!session.active);
        assert_eq!(session.active_at, now);
        assert_eq!(session.rebuild, Rebuild::Pending { truncated: true });
        assert!(session.life.is_running());
    }

    /// A holder may hold a session at any size a `u16` has: the core takes
    /// it within the limits every grid keeps, for its emulator and its
    /// listing alike.
    #[test]
    fn an_adopted_session_past_the_grid_limits_is_taken_at_them() {
        let now = Instant::now();
        let huge = adopted(HeldSession { cols: 65535, rows: 65535, ..held(false) }, now);
        assert_eq!((huge.cols, huge.rows), (2048, 1024));
        let info = huge.info(3, now);
        assert_eq!((info.cols, info.rows), (2048, 1024));
        let narrow = adopted(HeldSession { cols: 1, rows: 0, ..held(false) }, now);
        assert_eq!((narrow.cols, narrow.rows), (10, 1));
    }

    #[test]
    fn an_adopted_session_that_exited_keeps_its_exit_and_one_with_no_pid_is_refused() {
        let now = Instant::now();
        let exited = HeldSession {
            exit: Some(h1::ExitStatus { code: Some(3) }),
            ..held(false)
        };
        let session = adopted(exited, now);
        assert_eq!(
            session.life,
            Life::ExitedAwaitingOwner { code: Some(3), pid: Some(pid(42)) }
        );
        assert!(!session.life.is_running());

        let refused = Session::adopted(7, HeldSession { pid: 0, ..held(false) }, now);
        assert_eq!(
            refused.err().as_deref(),
            Some("session 3: its holder reports it running with no pid; not adopted")
        );
        // An exited one needs none.
        let exited = HeldSession { pid: 0, exit: Some(h1::ExitStatus { code: None }), ..held(false) };
        assert_eq!(adopted(exited, now).life, Life::ExitedAwaitingOwner { code: None, pid: None });
    }

    #[test]
    fn a_meta_with_no_owner_adopts_a_session_no_reconnect_gets_back() {
        let meta = SessionMeta { key: "k1".into(), scrollback: 100, ..SessionMeta::default() };
        let held = HeldSession { meta: meta.to_value(), ..held(false) };
        let session = adopted(held, Instant::now());
        assert_eq!(session.owner, None);
        assert_eq!(session.stored_meta().owner, "");
    }

    fn spawn(policy: Policy) -> client::Spawn {
        client::Spawn {
            req: 1,
            command: None,
            args: Vec::new(),
            cwd: None,
            env: Default::default(),
            cols: 80,
            rows: 24,
            cell_w: 0,
            cell_h: 0,
            scrollback: 100,
            policy,
            profile: None,
        }
    }

    fn terminal(policy: Policy) -> SpawnRequest {
        SpawnRequest::Terminal(spawn(policy))
    }

    #[test]
    fn each_life_answers_its_questions() {
        // (life, pid, starting, running, hung up, live)
        let cases = [
            (Life::starting(terminal(Policy::Kill)), None, true, false, false, false),
            (
                Life::Starting { spawn: terminal(Policy::Kill), hung_up: true },
                None,
                true,
                false,
                true,
                false,
            ),
            (Life::Running { pid: pid(9) }, Some(pid(9)), false, true, false, true),
            (Life::HungUp { pid: pid(9) }, Some(pid(9)), false, true, true, false),
            (
                Life::ExitedAwaitingOwner { code: Some(0), pid: Some(pid(9)) },
                Some(pid(9)),
                false,
                false,
                false,
                false,
            ),
        ];
        for (life, pid, starting, running, hung_up, live) in cases {
            let answers = (
                life.pid(),
                life.is_starting(),
                life.is_running(),
                life.is_hung_up(),
                life.is_live(),
            );
            assert_eq!(answers, (pid, starting, running, hung_up, live), "{life:?}");
        }
    }

    #[test]
    fn a_hang_up_is_sent_once_and_never_for_an_exit() {
        let mut starting = Life::starting(terminal(Policy::Kill));
        assert!(starting.hang_up());
        assert!(!starting.hang_up());
        assert!(starting.is_starting() && starting.is_hung_up());

        let mut running = Life::Running { pid: pid(9) };
        assert!(running.hang_up());
        assert_eq!(running, Life::HungUp { pid: pid(9) });
        assert!(!running.hang_up());

        let mut exited = Life::ExitedAwaitingOwner { code: None, pid: None };
        assert!(!exited.hang_up());

        // An exit keeps the pid its program had.
        running.exited(Some(1));
        assert_eq!(running, Life::ExitedAwaitingOwner { code: Some(1), pid: Some(pid(9)) });
        let mut starting = Life::starting(terminal(Policy::Kill));
        starting.exited(None);
        assert_eq!(starting, Life::ExitedAwaitingOwner { code: None, pid: None });
    }

    #[test]
    fn an_ownership_says_its_client_and_since_when_it_has_none() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_secs(5);
        // (ownership, client, detached since, owner may return)
        let cases = [
            (Ownership::Attached(4), Some(4), None, false),
            (Ownership::Lingering { until: t1, since: t0 }, None, Some(t0), true),
            (Ownership::AwaitingOwner { since: t0 }, None, Some(t0), true),
            (Ownership::Detached { since: t0 }, None, Some(t0), false),
        ];
        for (ownership, client, since, may_return) in cases {
            let answers = (ownership.client(), ownership.detached_since(), ownership.owner_may_return());
            assert_eq!(answers, (client, since, may_return), "{ownership:?}");

            // Once no owner is waited for, it is detached since the same
            // time; an attached one stays attached.
            let mut stopped = ownership;
            stopped.stop_waiting();
            let expected = match ownership {
                Ownership::Attached(_) => ownership,
                Ownership::Lingering { .. }
                | Ownership::AwaitingOwner { .. }
                | Ownership::Detached { .. } => Ownership::Detached { since: t0 },
            };
            assert_eq!(stopped, expected);
        }
    }

    #[test]
    fn a_listed_session_shows_its_owner_and_layout_only_while_detached() {
        let t0 = Instant::now();
        let layout = brindle_remote_protocol::serde_json::json!({ "tab": 1 });
        let meta = SessionMeta {
            command: "bash".into(),
            key: "k1".into(),
            owner: "tok".into(),
            layout: Some(layout.clone()),
            policy: Policy::Kill,
            profile: Some("work".into()),
            scrollback: 100,
            started: 1_700_000_000,
            ..SessionMeta::default()
        };
        let held = HeldSession { meta: meta.to_value(), ..held(false) };
        let mut session = adopted(held, t0);
        let info = session.info(3, t0 + Duration::from_secs(5));
        assert_eq!(
            info,
            SessionInfo {
                session: 3,
                pid: 42,
                attached: false,
                command: "bash".into(),
                cwd: None,
                policy: Policy::Kill,
                cols: 80,
                rows: 24,
                key: "k1".into(),
                title: session.screen().expect("a terminal").title(),
                started: 1_700_000_000,
                profile: Some("work".into()),
                owned: true,
                detached_secs: 5,
                layout: Some(layout),
                agent: None,
            }
        );

        session.ownership = Ownership::Attached(9);
        let info = session.info(3, t0 + Duration::from_secs(5));
        assert_eq!((info.attached, info.owned, info.detached_secs), (true, false, 0));
        assert_eq!(info.layout, None);

        // A session let go has no owner to come back.
        session.ownership = Ownership::Detached { since: t0 };
        session.owner = None;
        let info = session.info(3, t0);
        assert_eq!((info.attached, info.owned, info.detached_secs), (false, false, 0));
    }

    #[test]
    fn a_layout_is_kept_up_to_the_limit() {
        use brindle_remote_protocol::serde_json::Value;
        assert_eq!(kept_layout(3, None), None);
        // A JSON string is its text and two quotes.
        let at_limit = Value::String("x".repeat(client::MAX_LAYOUT - 2));
        assert_eq!(kept_layout(3, Some(at_limit.clone())), Some(at_limit));
        let over = Value::String("x".repeat(client::MAX_LAYOUT - 1));
        assert_eq!(kept_layout(3, Some(over)), None);
    }

    #[test]
    fn the_listed_command_line_is_the_command_and_its_arguments() {
        assert_eq!(command_line(None, &[]), "");
        // Arguments with no command are the login shell's: not shown.
        assert_eq!(command_line(None, &["-l".into()]), "");
        assert_eq!(command_line(Some("vim"), &["a b".into(), "-R".into()]), "vim a b -R");
        assert_eq!(command_line(Some("top"), &[]), "top");
    }

    #[test]
    fn session_ids_skip_the_taken_and_zero_and_wrap_to_one() {
        let none = |_: SessionId| false;
        assert_eq!(free_id(1, none), (1, 2));
        assert_eq!(free_id(5, |id| id == 5 || id == 6), (7, 8));
        // Past the largest id they wrap, and 0 is never one.
        assert_eq!(free_id(SessionId::MAX, none), (SessionId::MAX, 1));
        assert_eq!(free_id(0, none), (1, 2));
        assert_eq!(free_id(SessionId::MAX, |id| id == SessionId::MAX), (1, 2));
        assert_eq!(free_id(SessionId::MAX, |id| id == SessionId::MAX || id == 1), (2, 3));
    }

    #[test]
    fn marking_activity_records_the_given_instant() {
        let adopted_at = Instant::now();
        let mut session = adopted(held(false), adopted_at);
        let later = adopted_at + Duration::from_secs(5);
        session.mark_active(later);
        assert!(session.active);
        assert_eq!(session.active_at, later);

        // Telling the foreground program again also forgets the one told.
        session.active = false;
        session.foreground = Some(Foreground { name: Some("vim".into()), cwd: None });
        let latest = later + Duration::from_millis(250);
        session.tell_foreground(latest);
        assert!(session.active);
        assert_eq!(session.active_at, latest);
        assert_eq!(session.foreground, None);
        // It doesn't touch when its client left.
        assert_eq!(session.ownership.detached_since(), Some(adopted_at));
    }
}
