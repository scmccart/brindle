//! A Brindle process's connection to one endpoint's core, shared by every
//! terminal on that endpoint ([`super::shared`]).
//!
//! - **Transport:** the local core's socket or pipe, or a relay at a remote
//!   endpoint (`relay.rs`). Either way the open link is a [`Link`].
//! - **Reader thread:** reads frames and demultiplexes them by session. Each
//!   session's output goes to its [`SessionSink`], which parses it on this
//!   thread (as alacritty's I/O thread parsed a PTY's) and wakes the UI.
//!   `SPAWNED` moves a spawn's sink into the session map before the next
//!   frame is read, so no output is missed.
//! - **Writer thread:** sends queued messages in order, so the UI thread
//!   never blocks on the link. While the link is down it holds them, so
//!   input typed meanwhile reaches each session once reconnected. A
//!   request's message that nobody waits for any more (withdrawn, failed or
//!   timed out) isn't sent. [`Connection::flush`] waits, for a bounded
//!   time, until it is done with what was sent so far: a process that
//!   exits right after its tabs closed would otherwise end the writer
//!   before their `DETACH`es were written.
//! - **Reconnect:** when a link ends, the reader connects again with the
//!   same `client_token`. The core then replays every session it still has
//!   for this client. A `LIST` sent right after `WELCOME` tells which
//!   sessions it doesn't have, by id and key: a fresh core restarts ids at
//!   1, so a session whose id it gave another is one it doesn't have. Those
//!   are lost, and their tabs say so. Requests written on a link that ended
//!   fail, since their answers are gone, except attaches (restored zones),
//!   which stay pending: the reconnect's replay of the session answers one,
//!   and one whose session the core lists with its key and no client is
//!   sent again. The others wait for the next link.
//!   - **Local:** a core is started if none appears within [`START_AFTER`],
//!     and every session is lost when none answers within
//!     [`RECONNECT_LIMIT`]. A request waits up to [`REQUEST_LIMIT`].
//!   - **Relay:** attempts back off ([`relay::backoff`]) with no time limit,
//!     and requests wait as long as it takes. A session is lost only when
//!     the reconnected core's `LIST` lacks it. Each loss writes a dim line
//!     into the endpoint's terminals; the reconnect's replay starts with
//!     `ESC c`, so it never stays on a full-screen program's screen.
//! - **Relay liveness:** the reader drops `RELAY_HEARTBEAT` and stamps each
//!   frame it reads; a watchdog thread ends a link that read nothing for
//!   [`Timings::dead`] (killing its process), and the writer sends a
//!   heartbeat after [`Timings::heartbeat`] without a write. A frame being
//!   handled isn't silence.
//! - **Why a relay link ended:** the reason goes into the log line, the
//!   offline report ([`LinkState::Offline`]) and the requests that fail with
//!   the link. [`Connection::read`] says how the stream ended, and
//!   `relay_link_ended` picks the reason: if the watchdog ended the link
//!   (it marks the link's `Stamp` expired first, so the reader can tell),
//!   `nothing read from <name> for <limit>`; else the relay process's own
//!   account ([`Wire::why`]: `ssh exited with 255: <cause>`, the cause being
//!   the last line it wrote to stderr), which a killed process never gets
//!   to give; else `the relay closed the connection`, as a process that
//!   exited 0 with nothing to say, or `reading from the relay failed: …`.
//!   Only the local core's socket or pipe says `the terminal server closed
//!   the connection`.
//! - **A relay's first connect:** until a link has been up, a failed attempt
//!   fails new tabs' requests (spawns, lists, kills) with its error, while
//!   attaches (restored zones) keep waiting through the retries and are told
//!   the error once. With no attach or listing hook waiting, the connection
//!   ends, unless it is eager (connects at launch). A new tab's request made
//!   while it retries asks for an attempt at once.
//! - **A refusal:** a core that refuses this Brindle ([`Refused`]) ends a
//!   connection that isn't eager, so its tab shows the refusal at once. An
//!   eager one reports it offline, fails new tabs' requests with it (after a
//!   first link too, since no retry clears it soon) and retries like any
//!   failed attempt, at most [`Timings::eager_cap`] apart: the refusal ends
//!   once the refusing core exits and the next relay starts this build's.
//! - **A relay's lifetime:** it is in use while it shows a session, a
//!   request waits on it, a launch's listing hook waits on it, or a `CLOSE`
//!   is still to be written. A hook counts until it has its listing or is
//!   withdrawn ([`Connection::stop_listing`]: the window's last tab on the
//!   endpoint went). A `CLOSE` queued while the link is down keeps it
//!   reconnecting for at most [`Timings::close_limit`] (5 min) after the
//!   oldest was queued; if no link is up by then, the closes are given up,
//!   never written, and their programs left running: the log, and the
//!   connection's end if nothing else uses it, say `not ending session <ids>
//!   on <name>: it couldn't be reached within 5 minutes of the close`.
//!   Unless it is eager, it ends [`Timings::idle`] after it was last in use:
//!   its process is killed and [`super::shared`] forgets it.
//! - **State reports:** a relay connection made with a [`Report`] tells it
//!   each change of its state ([`LinkState`], for `remote::hosts::Hosts`):
//!   connecting until its first link is up, connected while one is, offline
//!   with why from a failed attempt or a lost link until the next link is
//!   up, and ended once for good. Reports are sent under the state lock, so
//!   they come in order, and nothing follows the end.
//! - **This machine only:** a relay's sessions run on another machine, so
//!   they are never sent `REPARENT` (a window handle of this one), and
//!   what is known of their programs comes from `PROCESS` ([`Foreground`]).
//! - **Listings:** a session a restored zone recorded
//!   ([`Attaching::Recorded`]) and a launch's [`Connection::on_listing`]
//!   both wait for the core's `LIST`, sent on the first link when either
//!   waits (and at once when the link is up). The listing must have the
//!   recorded session with its key and no client; then its `ATTACH` goes
//!   out at the listed size, which its replay is rendered at. Both keep a
//!   relay connecting, as an attach does; a hook only until it is
//!   withdrawn.
//!
//! The reader holds the state lock only to look a sink up; each sink has
//! its own lock, taken without the state lock, so parsing never waits on
//! the map and closing a tab never waits on parsing.
//!
//! Every call into a sink goes through [`Connection::deliver`], which
//! confines a panic to its session (`process R3`): Brindle's emulator
//! failing on one session's output detaches that session, which keeps
//! running, and tells its sink it is broken, while the reader carries on
//! for the endpoint's other sessions.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufRead, BufReader, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use parking_lot::{Condvar, Mutex};

use super::link::{Link, Wire};
use super::relay::{self, Dial, Timings};
use super::{Connected, Endpoint, Refused};
use crate::remote::budget::{Delivery, InputBudget, Offer};
use crate::remote::protocol::client::{
    self, Data, Kill, KillTarget, Policy, Reparent, ReplayStart, Request, Resize, SessionInfo,
    Detach, SessionRef, SetLayout, ToClient, ToCore,
};
use crate::remote::hosts::LinkState;
use crate::remote::protocol::relay::{RelayHost, RelayMessage, is_relay_type};
use crate::remote::protocol::{Frame, RequestId, SessionId};
use crate::remote::{ListedSession, SessionKey};
use crate::terminal::tab_note;

/// How long a request waits for its answer (a spawn's `SPAWNED`, an
/// attach's `REPLAY_START`) on the local core, a reconnect included.
const REQUEST_LIMIT: Duration = Duration::from_secs(30);
/// While reconnecting, how long to wait for a core to appear before
/// starting one, so an upgrading client's new core wins (design.md).
pub const START_AFTER: Duration = Duration::from_secs(5);
/// How long reconnecting may take before the sessions are lost.
pub const RECONNECT_LIMIT: Duration = Duration::from_secs(15);
const RECONNECT_POLL: Duration = Duration::from_millis(100);
/// The largest `DATA` frame sent; the frame limit is 1 MiB.
const INPUT_CHUNK: usize = 64 * 1024;
/// The reader's buffer.
const READ_BUFFER: usize = 64 * 1024;

/// A session's input budget, shared by its handle and the writer.
pub type Budget = Arc<Mutex<InputBudget>>;

/// A message for the writer. A `DATA` frame of a session's input carries
/// its budget and length, which the writer frees once it has written the
/// frame or skipped it (`process R9`); nothing else counts.
struct Outgoing {
    message: ToCore,
    backlog: Option<(Budget, usize)>,
}

impl Outgoing {
    /// The writer is done with it: its input no longer waits.
    fn handled(&self) {
        if let Some((budget, len)) = &self.backlog {
            budget.lock().handled(*len);
        }
    }
}

/// Where a relay connection tells its changes of state: its id (each
/// connection's own, increasing) and its new state. Called with the state
/// lock held, so it must not block.
pub type Report = Arc<dyn Fn(u64, LinkState) + Send + Sync>;

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

/// A connection id no connection had: each connection takes one, and so
/// does a report made for a connection that couldn't be made, so `Hosts`
/// orders it with the connections' own reports.
pub fn next_connection_id() -> u64 {
    NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed)
}

/// Where a session's output and events go: its terminal.
pub trait SessionSink: Send {
    /// Output from the program, or from a replay.
    fn output(&mut self, bytes: &[u8]);
    /// A failure that belongs to this tab but doesn't end the session: one
    /// dim line after what the terminal shows (`tab_note`). It is held back
    /// while a replay is, and a repeat is shown once until the session
    /// prints again.
    fn note(&mut self, text: &str);
    /// A replay starts (`true`) or ends.
    fn replay(&mut self, active: bool);
    /// The program exited.
    fn exited(&mut self, code: Option<i32>);
    /// The session can't be got back; `reason` says why.
    fn lost(&mut self, reason: &str);
    /// The session never started: no server, or the core refused it.
    fn failed(&mut self, error: &anyhow::Error);
    /// The core reported another foreground program (`PROCESS`); the
    /// session's [`Foreground`] has it.
    fn foreground(&mut self) {}
    /// The size the core listed a recorded session at, which its replay is
    /// rendered at; told before its `ATTACH` goes out.
    fn replay_size(&mut self, _cols: u16, _rows: u16) {}
    /// An earlier call into this sink panicked, saying `reason`
    /// ([`Connection::deliver`]): the session was let go, detached, and the
    /// sink gets nothing more.
    fn broken(&mut self, reason: &str);
}

/// What a launch does with the first listing of an endpoint's sessions
/// ([`Connection::on_listing`]). It runs on the connection's reader thread,
/// so it must not block.
pub type ListingHook = Box<dyn FnOnce(Vec<SessionInfo>) + Send>;

/// What the core last reported about a session's foreground program
/// (`PROCESS`, which a relay connection asks for): what a remote tab's
/// title and directory come from, since this machine can't look.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Foreground {
    /// Its name (`comm`).
    pub name: Option<String>,
    /// Its working directory, a path on the session's host.
    pub cwd: Option<String>,
}

pub type Sink = Arc<Mutex<Box<dyn SessionSink>>>;

/// What became of a call into a session's sink ([`Connection::deliver`]).
#[derive(Debug, PartialEq, Eq)]
enum Delivered {
    Done,
    /// It panicked, saying this: the sink was told it is broken, and its
    /// session, if still shown, let go.
    Panicked(String),
}

/// What a caught panic said: its message, or "no message".
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".into())
}

/// What a new session runs.
pub struct SpawnRequest {
    /// The program, never empty; `None` runs the host's login shell.
    pub command: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// The program's whole environment. On a relay connection, what goes
    /// over the endpoint's environment (`RELAY_HOST`'s) instead.
    pub env: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    pub cell_w: u16,
    pub cell_h: u16,
    pub scrollback: u32,
    /// `detach` keeps the program running when its tab closes or Brindle
    /// goes away; `kill` ends it.
    pub policy: Policy,
    /// The profile that started it, for whoever attaches it later.
    pub profile: Option<String>,
}

impl SpawnRequest {
    /// The `SPAWN` asking for this under `req`, where no command is the
    /// wire's empty field.
    fn into_spawn(self, req: RequestId) -> client::Spawn {
        client::Spawn {
            req,
            command: self.command,
            args: self.args,
            cwd: self.cwd,
            env: self.env,
            cols: self.cols,
            rows: self.rows,
            cell_w: self.cell_w,
            cell_h: self.cell_h,
            scrollback: self.scrollback,
            policy: self.policy,
            profile: self.profile,
        }
    }
}

/// Which detached session to attach, and when.
pub struct AttachRequest {
    pub session: SessionId,
    /// The key the session was listed or recorded with, which tells it
    /// from another core's session with its id; `None` from a core that
    /// predates keys.
    pub key: Option<SessionKey>,
    pub when: Attaching,
}

/// When an attach's `ATTACH` goes out, and at which size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attaching {
    /// A session just listed: at once, at the tab's size.
    Now { cols: u16, rows: u16 },
    /// A session a restored zone recorded: only once the core's `LIST`
    /// still has it, with its key and no client, at the size listed.
    Recorded,
}

impl AttachRequest {
    /// What the attach waits for under `req`, and the message to send now,
    /// if any: a recorded session's waits for a listing.
    fn waiting(self, req: RequestId, sink: Sink, reply: Reply<u32>) -> (Waiting, Option<ToCore>) {
        let (cols, rows, awaits) = match self.when {
            Attaching::Now { cols, rows } => (cols, rows, Awaits::Replay),
            // The wire's "no size", which the core takes as the session's
            // own, unless a listing gives one first.
            Attaching::Recorded => (0, 0, Awaits::Listing),
        };
        let attach = client::Attach { req, session: self.session, cols, rows };
        let message = match awaits {
            Awaits::Replay => Some(ToCore::Attach(attach.clone())),
            Awaits::Listing => None,
        };
        (Waiting::Attach { sink, reply, attach, key: self.key, awaits, told: false }, message)
    }
}

type Reply<T> = mpsc::SyncSender<Result<T, String>>;

/// A relay that was started and greeted: its reader, the stamp its watchdog
/// reads and the wire to write to.
type GreetedRelay = (BufReader<Box<dyn Read + Send>>, Arc<Stamp>, Arc<dyn Wire>);

/// What an attach waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Awaits {
    /// A recorded session's: a listing that has it with its key and no
    /// client. Nothing is sent until then.
    Listing,
    /// Its `REPLAY_START`: its `ATTACH` is out, or will go out.
    Replay,
}

/// What `SPAWNED` reports, its key decoded where it was read.
#[derive(Debug, PartialEq, Eq)]
struct Started {
    session: SessionId,
    pid: u32,
    key: Option<SessionKey>,
}

/// What a request waits for.
enum Waiting {
    /// `SPAWNED`, whose sink then joins the session map.
    Spawn { sink: Sink, reply: Reply<Started> },
    /// The `REPLAY_START` answering an `ATTACH` (its pid), whose sink joins
    /// the map, with `key`, before the replay's first `DATA` is read. A
    /// reconnect's replay of the session answers it too, and `attach` is
    /// sent again when the reconnected core lists the session with `key`
    /// and didn't give it back ([`carried_attaches`]). `told`: while it
    /// waits for a relay's first link, it was told why it can't connect
    /// yet ([`State::connect_failed`]).
    Attach {
        sink: Sink,
        reply: Reply<u32>,
        attach: client::Attach,
        key: Option<SessionKey>,
        awaits: Awaits,
        told: bool,
    },
    /// `SESSIONS`.
    List(Reply<Vec<SessionInfo>>),
    /// `OK`.
    Done(Reply<()>),
}

impl Waiting {
    fn fail(self, message: String) {
        match self {
            Self::Spawn { reply, .. } => reply.try_send(Err(message)).ok(),
            Self::Attach { reply, .. } => reply.try_send(Err(message)).ok(),
            Self::List(reply) => reply.try_send(Err(message)).ok(),
            Self::Done(reply) => reply.try_send(Err(message)).ok(),
        };
    }
}

/// Where a request's message stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sent {
    /// Not on the current link: queued for the writer, or to go out again
    /// on the next link after a write failed.
    Queued,
    /// Written on the current link: if that ends, the answer is lost with
    /// it.
    Written,
    /// An attach the next `LIST` decides ([`carried_attaches`]): one that
    /// went out on a link that ended before its answer
    /// ([`State::link_lost`]), or a recorded one not sent yet
    /// ([`Pending::recorded`]). Nothing else makes one.
    Carried,
}

struct Pending {
    waiting: Waiting,
    sent: Sent,
}

impl Pending {
    /// A request whose message goes to the writer now.
    fn queued(waiting: Waiting) -> Self {
        Self { waiting, sent: Sent::Queued }
    }

    /// A recorded attach, which sends nothing until a listing confirms its
    /// session.
    fn recorded(waiting: Waiting) -> Self {
        Self { waiting, sent: Sent::Carried }
    }

    /// Its message went out on the current link (`true`), or writing it
    /// failed and it goes out on the next (`false`). A carried attach whose
    /// write failed stays carried: the next listing decides it.
    fn mark_written(&mut self, written: bool) {
        self.sent = match (self.sent, written) {
            (Sent::Queued | Sent::Written | Sent::Carried, true) => Sent::Written,
            (Sent::Queued | Sent::Written, false) => Sent::Queued,
            (Sent::Carried, false) => Sent::Carried,
        };
    }

    fn is_written(&self) -> bool {
        match self.sent {
            Sent::Written => true,
            Sent::Queued | Sent::Carried => false,
        }
    }

    fn is_carried(&self) -> bool {
        match self.sent {
            Sent::Carried => true,
            Sent::Queued | Sent::Written => false,
        }
    }

    fn is_attach(&self) -> bool {
        matches!(self.waiting, Waiting::Attach { .. })
    }

    /// A recorded attach that waits for a listing to confirm its session.
    fn is_check(&self) -> bool {
        match &self.waiting {
            Waiting::Attach { awaits, .. } => match awaits {
                Awaits::Listing => true,
                Awaits::Replay => false,
            },
            Waiting::Spawn { .. } | Waiting::List(_) | Waiting::Done(_) => false,
        }
    }
}

/// The `LIST` the reader waits for, outside the requests: after a link
/// comes up, and for what waits on a listing.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Listing {
    req: RequestId,
    /// The first after a lost link: the sessions it lacks are lost.
    after_loss: bool,
}

/// A session this connection shows: where its output goes, the key that
/// tells it from another core's session with its id, and what is kept for
/// it while it is shown, which goes with it.
struct Shown {
    sink: Sink,
    /// `None` from a core that predates keys.
    key: Option<SessionKey>,
    /// The window its console is parented to (Windows), sent again after a
    /// reconnect's replay: a core doesn't keep it.
    parent: Option<u64>,
    /// Its last `PROCESS`.
    foreground: Option<Foreground>,
}

impl Shown {
    fn new(sink: Sink, key: Option<SessionKey>) -> Self {
        Self { sink, key, parent: None, foreground: None }
    }
}

#[derive(Default)]
struct State {
    /// By id, which is all `DATA`, `EXIT`, `REPLAY_*` and `PROCESS` carry:
    /// within one link, the core's ids are unique. Across links an id can
    /// name another session (a fresh core restarts them at 1), so whatever
    /// decides about a session after a link changed matches its key too
    /// ([`State::keep_listed`], [`State::shows`], [`State::forget`]).
    sessions: HashMap<SessionId, Shown>,
    requests: HashMap<RequestId, Pending>,
    /// The `LIST` sent after a link's `WELCOME`, or for a recorded attach
    /// or a listing hook.
    list: Option<Listing>,
    /// What the next listing goes to ([`Connection::on_listing`]).
    on_listing: Vec<ListingHook>,
    /// Whether a link is up, has been, or never was, and the client
    /// protocol the current (or last) link's `WELCOME` agreed on.
    link: LinkPhase,
    /// A new tab's request wants a relay attempt now, not after the pause.
    hurry: bool,
    /// The sessions whose `CLOSE`s went to the writer and aren't written
    /// yet, with when each was queued, oldest first. The writer skips one
    /// that is no longer here: it was given up ([`State::give_up_closes`]).
    closing: Vec<(SessionId, Instant)>,
    /// Whether a relay connection is in use ([`State::in_use`]), as its idle
    /// thread last looked ([`State::idle_step`]).
    idle: Idle,
    /// When [`Connection::end_when_unused`] last took its idle decision. An
    /// observation for tests, which decides nothing: a look at or after
    /// `idle` past the moment a connection became idle-eligible shows that
    /// the decision was taken there, so a test can assert what it decided
    /// without waiting out a fixed window.
    #[cfg_attr(not(test), allow(dead_code))]
    last_look: Option<Instant>,
    /// The connection ended for good, and why; the next tab makes a new one.
    ended: Option<String>,
    /// Run once when it ends ([`Connection::on_end`]).
    on_end: Option<Box<dyn FnOnce() + Send>>,
    /// What was last reported ([`Connection::report`]).
    reported: Option<LinkState>,
}

/// The client protocol a link's `WELCOME` agreed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Negotiated(u32);

impl Negotiated {
    /// Whether the core takes `SET_LAYOUT`, which client protocol 3 added.
    fn sets_layout(self) -> bool {
        self.0 >= 3
    }
}

/// Where a connection's links stand. An up link carries the protocol its
/// `WELCOME` agreed on, and a link that went down keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LinkPhase {
    /// No link has been up yet.
    #[default]
    NeverUp,
    /// A link is up.
    Up(Negotiated),
    /// The last link went down; the next is a reconnect.
    Down(Negotiated),
}

impl LinkPhase {
    /// A link is up now.
    fn is_up(self) -> bool {
        match self {
            Self::Up(_) => true,
            Self::NeverUp | Self::Down(_) => false,
        }
    }

    /// A link has been up, and may be down again.
    fn has_been_up(self) -> bool {
        match self {
            Self::Up(_) | Self::Down(_) => true,
            Self::NeverUp => false,
        }
    }

    /// The protocol of the link that is up, else of the last one; none
    /// before the first.
    fn protocol(self) -> Option<Negotiated> {
        match self {
            Self::Up(protocol) | Self::Down(protocol) => Some(protocol),
            Self::NeverUp => None,
        }
    }

    /// The link that was up went down. Before the first there is none to
    /// lose.
    fn down(self) -> Self {
        match self {
            Self::Up(protocol) | Self::Down(protocol) => Self::Down(protocol),
            Self::NeverUp => Self::NeverUp,
        }
    }
}

/// Whether a relay connection is in use, as its idle thread last looked.
#[derive(Debug, Default, PartialEq, Eq)]
enum Idle {
    /// In use, or not looked at yet.
    #[default]
    InUse,
    /// Out of use since `at`. `given_up` says which `CLOSE`s were given up
    /// meanwhile ([`not_ending`]), which is then why the connection ends.
    Since { at: Instant, given_up: Option<String> },
}

/// What a failed relay attempt does to what waits on the connection
/// ([`State::connect_failed`]).
struct ConnectFailed {
    /// New tabs' requests, to fail with the attempt's message.
    failed: Vec<Pending>,
    /// The attaches to tell the message, once, with their sessions.
    told: Vec<(SessionId, Sink)>,
    next: Next,
}

/// Whether a relay connection tries again after a failed attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    Retry,
    /// It ended, with the attempt's message as its reason.
    End,
}

/// What a relay connection's idle thread does after a look
/// ([`State::idle_step`]).
#[derive(Debug, PartialEq, Eq)]
struct IdleStep {
    /// The sessions whose `CLOSE`s were given up at this look.
    given_up: Vec<SessionId>,
    then: Then,
}

#[derive(Debug, PartialEq, Eq)]
enum Then {
    /// Look again after this long.
    Wait(Duration),
    /// The connection ended for this reason, now its [`State::ended`].
    End(String),
}

/// What the reader does with an answer that would show a session: `SPAWNED`
/// ([`State::route_spawned`]), or the `REPLAY_START` that answers an
/// `ATTACH` ([`State::route_replay_start`]).
enum Routed<T> {
    /// Its request waited for it: the session is shown here with `sink`,
    /// displacing `displaced` (whose tab is to be told it is lost), and the
    /// requester is answered `answer` through `reply`.
    Shown { sink: Sink, reply: Reply<T>, answer: T, displaced: Option<Shown> },
    /// Nobody here waits for this new session: it is ended (`CLOSE`). A
    /// request of another kind under its id fails, saying what answered it.
    Close { session: SessionId, fail: Option<(Waiting, &'static str)> },
    /// Nobody here waits for this attached session: it is left detached
    /// (`DETACH`), as it was. A request of another kind under its id fails,
    /// saying what answered it.
    Detach { session: SessionId, fail: Option<(Waiting, &'static str)> },
}

/// The longest pause between relay attempts: longer for an eager
/// connection that nothing waits on (no session shown, no request).
fn retry_cap(eager: bool, nothing_waits: bool, timings: &Timings) -> Duration {
    if eager && nothing_waits { timings.eager_cap } else { timings.retry_cap }
}

impl State {
    /// Shows `session` as `shown`. Gives back the entry it displaced, if
    /// any, whose tab no longer gets its session's output and is to be told
    /// it is lost.
    fn show(&mut self, session: SessionId, shown: Shown) -> Option<Shown> {
        self.sessions.insert(session, shown)
    }

    /// Whether `session` is shown as the session with `key`: the one a
    /// handle names, not another that got its id since.
    fn shows(&self, session: SessionId, key: Option<&SessionKey>) -> bool {
        self.sessions.get(&session).is_some_and(|shown| shown.key.as_ref() == key)
    }

    /// The entry of `session` while it is shown as the session with `key`
    /// ([`State::shows`]).
    fn shown(&mut self, session: SessionId, key: Option<&SessionKey>) -> Option<&mut Shown> {
        self.sessions.get_mut(&session).filter(|shown| shown.key.as_ref() == key)
    }

    /// Stops showing `session` when it is the one with `key`, giving back
    /// its entry; one shown under its id with another key stays.
    fn forget(&mut self, session: SessionId, key: Option<&SessionKey>) -> Option<Shown> {
        if !self.shows(session, key) {
            return None;
        }
        self.sessions.remove(&session)
    }

    /// After a lost link, keeps showing only the sessions the core still
    /// has for this client: `listed` with their keys ([`ListedSession::is`])
    /// and a client. The others' sinks are taken out and given back with
    /// their ids, their windows and foreground programs gone.
    fn keep_listed(&mut self, listed: &[ListedSession]) -> Vec<(SessionId, Sink)> {
        let gone: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|&(&session, shown)| {
                !listed.iter().any(|listing| listing.attached && listing.is(session, shown.key.as_ref()))
            })
            .map(|(&session, _)| session)
            .collect();
        gone.into_iter()
            .filter_map(|session| Some((session, self.sessions.remove(&session)?.sink)))
            .collect()
    }

    /// Something needs the connection: a session it shows, a request
    /// waiting on it, a listing hook (until it is withdrawn), or a `CLOSE` it
    /// hasn't written yet. A `CLOSE` counts until it is written, or, with no
    /// link up, until [`Timings::close_limit`] after the oldest was queued:
    /// then the closes are given up ([`State::give_up_closes`]) and count no
    /// more.
    fn in_use(&self) -> bool {
        !self.sessions.is_empty() || !self.requests.is_empty() || !self.on_listing.is_empty() || !self.closing.is_empty()
    }

    /// Queues `session`'s `CLOSE` at `now`, which keeps the connection in use.
    fn closing(&mut self, session: SessionId, now: Instant) {
        self.closing.push((session, now));
    }

    /// The `CLOSE`s given up at `now`, taken out: all of them once the
    /// oldest was queued `limit` ago with no link up, else none.
    fn give_up_closes(&mut self, limit: Duration, now: Instant) -> Vec<SessionId> {
        match self.closing.first() {
            Some(&(_, queued)) if !self.link.is_up() && now.saturating_duration_since(queued) >= limit => {
                self.closing.drain(..).map(|(session, _)| session).collect()
            }
            _ => Vec::new(),
        }
    }

    /// When the `CLOSE`s are given up if no link comes up first.
    fn closes_due(&self, limit: Duration) -> Option<Instant> {
        let (_, queued) = self.closing.first().filter(|_| !self.link.is_up())?;
        Some(*queued + limit)
    }

    /// A recorded attach or a listing hook waits for the core's `LIST`.
    fn waits_for_listing(&self) -> bool {
        !self.on_listing.is_empty() || self.requests.values().any(Pending::is_check)
    }

    /// No session is shown and no request waits: what an eager relay's
    /// pauses are longer for ([`retry_cap`]).
    fn nothing_waits(&self) -> bool {
        self.sessions.is_empty() && self.requests.is_empty()
    }

    /// A relay attempt failed with `message`; `refused` when the core
    /// refused this Brindle ([`Refused`]). Before the first link, new tabs'
    /// requests fail with it, and the attaches that wait on are told it
    /// once; after a refusal, new tabs' requests fail with it after a first
    /// link too, since retrying won't clear it soon. Attaches carried over a
    /// lost link aren't told: their tabs say the link was lost.
    ///
    /// It tries again: after a refusal only while the connection is
    /// `eager`; otherwise always once a link has been up, and before that
    /// while an attach or a listing hook waits or it is `eager`. When it
    /// doesn't, it ends here, so no attach can start to wait on a
    /// connection that has stopped trying.
    fn connect_failed(&mut self, refused: bool, eager: bool, message: &str) -> ConnectFailed {
        if self.link.has_been_up() && !refused {
            return ConnectFailed { failed: Vec::new(), told: Vec::new(), next: Next::Retry };
        }
        let new_tabs: Vec<RequestId> =
            self.requests.iter().filter(|(_, pending)| !pending.is_attach()).map(|(&req, _)| req).collect();
        let failed: Vec<Pending> = new_tabs.into_iter().filter_map(|req| self.requests.remove(&req)).collect();
        // This attempt answered them: only a later tab hurries the next.
        self.hurry = false;
        let first = !self.link.has_been_up();
        let told: Vec<(SessionId, Sink)> = self
            .requests
            .values_mut()
            .filter(|_| first)
            .filter_map(|pending| match &mut pending.waiting {
                Waiting::Attach { sink, attach, told, .. } if !*told => {
                    *told = true;
                    Some((attach.session, sink.clone()))
                }
                Waiting::Spawn { .. } | Waiting::Attach { .. } | Waiting::List(_) | Waiting::Done(_) => None,
            })
            .collect();
        let keep =
            eager || (!refused && (self.requests.values().any(Pending::is_attach) || !self.on_listing.is_empty()));
        let next = match keep {
            true => Next::Retry,
            false => {
                self.ended = Some(message.to_string());
                Next::End
            }
        };
        ConnectFailed { failed, told, next }
    }

    /// The idle thread's look at `now`, at a relay connection whose endpoint
    /// is `name`. With no link up, the `CLOSE`s not written are given up
    /// `close_limit` after the oldest was queued ([`State::give_up_closes`]),
    /// eager or not. The connection ends `idle` after it was last in use
    /// ([`State::in_use`]), unless it is `eager`, with the closes given up
    /// meanwhile as its reason, if any, else that it was no longer used.
    /// Otherwise the thread looks again a fifth of `idle` later, or when the
    /// closes are due if sooner, so it ends at most a fifth of `idle` late.
    fn idle_step(&mut self, now: Instant, eager: bool, idle: Duration, close_limit: Duration, name: &str) -> IdleStep {
        let given_up = self.give_up_closes(close_limit, now);
        let reason = (!given_up.is_empty()).then(|| not_ending(&given_up, name, close_limit));
        if eager || self.in_use() {
            self.idle = Idle::InUse;
        } else {
            let since = match std::mem::take(&mut self.idle) {
                Idle::InUse => Idle::Since { at: now, given_up: reason },
                Idle::Since { at, given_up: before } => Idle::Since { at, given_up: reason.or(before) },
            };
            match since {
                Idle::Since { at, given_up: why } if now.saturating_duration_since(at) >= idle => {
                    let why = why.unwrap_or_else(|| "the connection was no longer used".to_string());
                    // Here, so no request slips in to wait on it.
                    self.ended = Some(why.clone());
                    return IdleStep { given_up, then: Then::End(why) };
                }
                Idle::InUse | Idle::Since { .. } => self.idle = since,
            }
        }
        let look = (idle / 5).max(Duration::from_millis(1));
        let due = self.closes_due(close_limit).map(|due| due.saturating_duration_since(now));
        let wait = due.map_or(look, |due| due.min(look)).max(Duration::from_millis(1));
        IdleStep { given_up, then: Then::Wait(wait) }
    }

    /// The link went down. The requests written on it are taken out and
    /// given back to fail, since their answers are gone, except attaches,
    /// which are carried over for the next link's listing to decide
    /// ([`carried_attaches`]). The listing waited for is gone with it.
    fn link_lost(&mut self) -> Vec<Pending> {
        self.list = None;
        self.link = self.link.down();
        let lost: Vec<RequestId> =
            self.requests.iter().filter(|(_, pending)| pending.is_written()).map(|(&req, _)| req).collect();
        let mut failed = Vec::new();
        for req in lost {
            let Some(pending) = self.requests.get_mut(&req) else { continue };
            if pending.is_attach() {
                pending.sent = Sent::Carried;
            } else if let Some(pending) = self.requests.remove(&req) {
                failed.push(pending);
            }
        }
        failed
    }

    /// What `SPAWNED` does: the session is shown with its spawn's sink when
    /// the spawn still waits for it, its key decoded here. Otherwise its
    /// spawner gave up, nobody shows it, and it is ended; a request of
    /// another kind under that `req` fails.
    fn route_spawned(&mut self, spawned: &client::Spawned) -> Routed<Started> {
        let session = spawned.session;
        match self.requests.remove(&spawned.req).map(|pending| pending.waiting) {
            Some(Waiting::Spawn { sink, reply }) => {
                let key = SessionKey::from_wire(&spawned.key);
                let displaced = self.show(session, Shown::new(sink.clone(), key.clone()));
                Routed::Shown { sink, reply, answer: Started { session, pid: spawned.pid, key }, displaced }
            }
            Some(other @ (Waiting::Attach { .. } | Waiting::List(_) | Waiting::Done(_))) => {
                Routed::Close { session, fail: Some((other, "answered with SPAWNED")) }
            }
            None => Routed::Close { session, fail: None },
        }
    }

    /// What a `REPLAY_START` answering request `req` does: the session is
    /// shown with its attach's sink and key when the attach still waits for
    /// it. Otherwise its attacher gave up, and the session is left detached,
    /// as it was; a request of another kind under that `req` fails.
    fn route_replay_start(&mut self, req: RequestId, session: SessionId, pid: u32) -> Routed<u32> {
        match self.requests.remove(&req).map(|pending| pending.waiting) {
            Some(Waiting::Attach { sink, reply, key, .. }) => {
                let displaced = self.show(session, Shown::new(sink.clone(), key));
                Routed::Shown { sink, reply, answer: pid, displaced }
            }
            Some(other @ (Waiting::Spawn { .. } | Waiting::List(_) | Waiting::Done(_))) => {
                Routed::Detach { session, fail: Some((other, "answered with REPLAY_START")) }
            }
            None => Routed::Detach { session, fail: None },
        }
    }
}

/// The link the writer writes to.
#[derive(Clone)]
struct Current {
    wire: Arc<dyn Wire>,
    /// The relay's `RELAY_HOST`, refreshed with each link: a remote
    /// `SPAWN`'s environment goes over its.
    host: Option<Arc<RelayHost>>,
}

#[derive(Default)]
struct Slot {
    /// `None` while reconnecting.
    link: Option<Current>,
    closed: bool,
    /// How many of the messages sent ([`Connection::sent`]) the writer is
    /// done with: written, or dropped as no longer wanted.
    handled: u64,
}

/// When a relay link counts as dead, as the watchdog reads it: a deadline
/// in milliseconds from `epoch`, or held while a frame is handled.
struct Stamp {
    epoch: Instant,
    deadline: AtomicU64,
    /// Set by the watchdog before it ends the link, so the reader can tell
    /// a link that went silent from a process that ended by itself.
    expired: AtomicBool,
}

impl Stamp {
    const HELD: u64 = u64::MAX;

    /// A stamp made at `now`, dead `limit` later.
    fn new(now: Instant, limit: Duration) -> Self {
        let stamp = Self { epoch: now, deadline: AtomicU64::new(0), expired: AtomicBool::new(false) };
        stamp.touch(now, limit);
        stamp
    }

    /// Read at `now`: dead if nothing more is read within `limit`. The
    /// deadline is kept in whole milliseconds from the epoch, rounded down,
    /// and one too far for them is the furthest short of [`Stamp::HELD`].
    fn touch(&self, now: Instant, limit: Duration) {
        let at = (now.saturating_duration_since(self.epoch) + limit).as_millis();
        self.deadline.store(u64::try_from(at).unwrap_or(Self::HELD - 1), Ordering::Release);
    }

    /// Never dead until the next [`Stamp::touch`].
    fn hold(&self) {
        self.deadline.store(Self::HELD, Ordering::Release);
    }

    /// The watchdog is ending the link for want of anything read.
    fn expire(&self) {
        self.expired.store(true, Ordering::Release);
    }

    fn expired(&self) -> bool {
        self.expired.load(Ordering::Acquire)
    }

    fn deadline(&self) -> Option<Instant> {
        let at = self.deadline.load(Ordering::Acquire);
        (at != Self::HELD).then(|| self.epoch + Duration::from_millis(at))
    }
}

/// The relay link the watchdog watches.
#[derive(Default)]
struct Watch {
    link: Option<(Arc<dyn Wire>, Arc<Stamp>)>,
    closed: bool,
}

/// How a connection reaches its core, and reaches it again.
pub enum Transport {
    /// The local core: its socket or pipe.
    Local(Endpoint),
    /// A remote endpoint's core, through relays that `dial` starts.
    Relay { dial: Arc<dyn Dial>, timings: Timings },
}

/// The connection to a core.
pub struct Connection {
    state: Mutex<State>,
    /// Wakes a relay's pause between attempts: a request hurries it, or the
    /// connection ended.
    wake: Condvar,
    outgoing: mpsc::Sender<Outgoing>,
    /// How many messages went to the writer, counted before each is queued
    /// ([`Connection::flush`] waits for [`Slot::handled`] to catch up).
    sent: AtomicU64,
    slot: Mutex<Slot>,
    /// A link came up or went, the writer finished a message, or the
    /// connection ended.
    slot_ready: Condvar,
    watch: Mutex<Watch>,
    watch_changed: Condvar,
    next_req: AtomicU64,
    transport: Transport,
    token: String,
    /// Keep connecting while nothing waits (a profile connects at launch).
    eager: AtomicBool,
    /// This connection's own, never another's in this process: for its
    /// reports and the registry ([`Connection::id`]).
    id: u64,
    report: Option<Report>,
}

/// Lets a starting session's tab let go of its request while it waits:
/// a request that hasn't gone out is withdrawn ([`Connection::withdraw`]),
/// so a relay that never comes back keeps nothing waiting.
#[derive(Default)]
pub struct Abandon(Mutex<Abandoning>);

#[derive(Default)]
struct Abandoning {
    asked: Option<(Weak<Connection>, RequestId)>,
    abandoned: bool,
}

impl Abandon {
    pub fn abandon(&self) {
        let asked = {
            let mut abandoning = self.0.lock();
            abandoning.abandoned = true;
            abandoning.asked.take()
        };
        if let Some((connection, req)) = asked
            && let Some(connection) = connection.upgrade()
        {
            connection.withdraw(req);
        }
    }

    /// Records the request; false when the tab has already let go.
    fn asked(&self, connection: Weak<Connection>, req: RequestId) -> bool {
        let mut abandoning = self.0.lock();
        if abandoning.abandoned {
            return false;
        }
        abandoning.asked = Some((connection, req));
        true
    }
}

impl Connection {
    fn new(transport: Transport, token: &str) -> (Arc<Self>, mpsc::Receiver<Outgoing>) {
        Self::made(transport, token, false, None)
    }

    fn made(
        transport: Transport,
        token: &str,
        eager: bool,
        report: Option<Report>,
    ) -> (Arc<Self>, mpsc::Receiver<Outgoing>) {
        let (outgoing, queue) = mpsc::channel();
        let this = Arc::new(Self {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            outgoing,
            sent: AtomicU64::new(0),
            slot: Mutex::new(Slot::default()),
            slot_ready: Condvar::new(),
            watch: Mutex::new(Watch::default()),
            watch_changed: Condvar::new(),
            next_req: AtomicU64::new(1),
            transport,
            token: token.to_string(),
            eager: AtomicBool::new(eager),
            id: next_connection_id(),
            report,
        });
        (this, queue)
    }

    /// Starts the reader and writer threads on a negotiated local connection.
    pub fn start(endpoint: Endpoint, token: &str, connected: Connected) -> Result<Arc<Self>> {
        let protocol = connected.welcome.protocol;
        let link = Link::local(connected.stream)?;
        let (this, queue) = Self::new(Transport::Local(endpoint), token);
        this.up(link.wire, None, protocol)?;
        let writing = this.clone();
        std::thread::Builder::new()
            .name("server-writer".into())
            .spawn(move || writing.write_loop(queue))?;
        let reading = this.clone();
        std::thread::Builder::new()
            .name("server-reader".into())
            .spawn(move || reading.run_local(link.reader))?;
        Ok(this)
    }

    /// A connection to a remote endpoint's core through relays that `dial`
    /// starts. It returns at once: the reader thread connects, and requests
    /// wait for it. The process makes its own with [`Connection::relay_with`].
    #[cfg(test)]
    pub fn relay(dial: Arc<dyn Dial>, token: &str, timings: Timings) -> Result<Arc<Self>> {
        Self::relay_with(dial, token, timings, false, None)
    }

    /// [`Connection::relay`], eager from the start when `eager` (so even its
    /// first failure keeps it trying), telling `report` its changes of state.
    pub fn relay_with(
        dial: Arc<dyn Dial>,
        token: &str,
        timings: Timings,
        eager: bool,
        report: Option<Report>,
    ) -> Result<Arc<Self>> {
        let name = dial.name();
        let (this, queue) = Self::made(Transport::Relay { dial, timings }, token, eager, report);
        let writing = this.clone();
        std::thread::Builder::new()
            .name("relay-writer".into())
            .spawn(move || writing.write_loop(queue))?;
        let watching = this.clone();
        std::thread::Builder::new()
            .name("relay-watch".into())
            .spawn(move || watching.watchdog(&name, timings.dead))?;
        let idling = this.clone();
        std::thread::Builder::new()
            .name("relay-idle".into())
            .spawn(move || idling.end_when_unused(timings.idle, timings.close_limit))?;
        let reading = this.clone();
        std::thread::Builder::new()
            .name("relay-reader".into())
            .spawn(move || reading.run_relay())?;
        Ok(this)
    }

    pub fn is_dead(&self) -> bool {
        self.state.lock().ended.is_some()
    }

    /// This connection's id, given to no other in this process, so a
    /// registry slot tells its own connection from a newer one by it.
    pub(super) fn id(&self) -> u64 {
        self.id
    }

    /// Takes the connection for another terminal; false once it has ended.
    /// A relay out of use starts its idle time over, so it can't end
    /// between this and the terminal's request.
    pub(super) fn reuse(&self) -> bool {
        let mut state = self.state.lock();
        if state.ended.is_some() {
            return false;
        }
        if let Idle::Since { at, .. } = &mut state.idle {
            *at = Instant::now();
        }
        true
    }

    /// Runs `hook` once the connection ends, on the thread that ends it.
    /// False (and `hook` dropped) when it has ended already.
    pub(super) fn on_end(&self, hook: Box<dyn FnOnce() + Send>) -> bool {
        let mut state = self.state.lock();
        if state.ended.is_some() {
            return false;
        }
        state.on_end = Some(hook);
        true
    }

    /// Ends a relay connection `idle` ([`Timings::idle`]) after it was last
    /// in use, unless it is eager, and gives up the `CLOSE`s not written
    /// `close_limit` after the oldest was queued, logging the sessions left
    /// running ([`State::idle_step`] decides both). It reads the clock and
    /// waits.
    fn end_when_unused(&self, idle: Duration, close_limit: Duration) {
        let name = self.name();
        let mut state = self.state.lock();
        loop {
            if state.ended.is_some() {
                return;
            }
            let eager = self.eager.load(Ordering::Acquire);
            // Under the lock, so no request slips in to wait on a connection
            // that ends.
            let step = state.idle_step(Instant::now(), eager, idle, close_limit, &name);
            state.last_look = Some(Instant::now());
            if !step.given_up.is_empty() {
                log::warn!("{}", not_ending(&step.given_up, &name, close_limit));
            }
            match step.then {
                Then::Wait(wait) => {
                    self.wake.wait_for(&mut state, wait);
                }
                Then::End(reason) => {
                    drop(state);
                    log::info!("closing the unused connection to {name}");
                    return self.ended(&reason);
                }
            }
        }
    }

    /// The endpoint, for log lines.
    fn name(&self) -> String {
        match &self.transport {
            Transport::Local(_) => "this machine".to_string(),
            Transport::Relay { dial, .. } => dial.name(),
        }
    }

    /// Makes the connection keep connecting while nothing waits for it, as
    /// for a profile that connects at launch.
    pub fn set_eager(&self, eager: bool) {
        self.eager.store(eager, Ordering::Release);
    }

    fn is_relay(&self) -> bool {
        matches!(self.transport, Transport::Relay { .. })
    }

    /// Tells the connection's [`Report`] its new state, under the state
    /// lock: once, and nothing after it has ended but that it has.
    fn report(&self, state: &mut State, now: LinkState) {
        let Some(report) = &self.report else { return };
        let ending = matches!(now, LinkState::Ended(_));
        if matches!(state.reported, Some(LinkState::Ended(_)))
            || (state.ended.is_some() && !ending)
            || state.reported.as_ref() == Some(&now)
        {
            return;
        }
        state.reported = Some(now.clone());
        report(self.id, now);
    }

    fn report_now(&self, now: LinkState) {
        let mut state = self.state.lock();
        self.report(&mut state, now);
    }

    fn send(&self, message: ToCore) {
        self.queue(Outgoing { message, backlog: None });
    }

    /// Queues `outgoing` for the writer. One the writer can't take any more
    /// (the connection ended) frees its input at once.
    fn queue(&self, outgoing: Outgoing) {
        self.sent.fetch_add(1, Ordering::AcqRel);
        if let Err(mpsc::SendError(outgoing)) = self.outgoing.send(outgoing) {
            outgoing.handled();
        }
    }

    /// Waits until the writer is done with every message sent before the
    /// call, or until `deadline`; true once it is. This is for a process
    /// about to exit: its tabs' `DETACH`es (and `CLOSE`s) are only queued,
    /// and an exit before the writer runs loses them, so the core takes the
    /// quit for a client that dropped and may come back. A link that is
    /// down writes nothing until the next, which won't come, so it isn't
    /// waited for: false at once, as for a connection that ended.
    pub fn flush(&self, deadline: Instant) -> bool {
        let target = self.sent.load(Ordering::Acquire);
        let mut slot = self.slot.lock();
        loop {
            if slot.handled >= target {
                return true;
            }
            if slot.closed || slot.link.is_none() {
                return false;
            }
            if self.slot_ready.wait_until(&mut slot, deadline).timed_out() {
                return slot.handled >= target;
            }
        }
    }

    /// Sends `CLOSE` for `session`, which keeps the connection in use until
    /// it is written or given up ([`State::in_use`]).
    fn send_close(&self, session: SessionId) {
        self.state.lock().closing(session, Instant::now());
        self.send(ToCore::Close(SessionRef { session }));
    }

    /// Stops showing `session`, the one with `key` ([`State::forget`]);
    /// says whether it was shown. With `closing`, the `CLOSE` the caller
    /// sends next is counted under the same lock, so the connection is never
    /// out of use in between.
    fn forget(&self, session: SessionId, key: Option<&SessionKey>, closing: bool) -> bool {
        let mut state = self.state.lock();
        let shown = state.forget(session, key).is_some();
        if shown && closing {
            state.closing(session, Instant::now());
        }
        shown
    }

    fn req(&self) -> RequestId {
        self.next_req.fetch_add(1, Ordering::Relaxed)
    }

    /// Sends the request `make` builds and waits for its answer: up to
    /// `limit` when given, else up to [`REQUEST_LIMIT`] on the local core
    /// and as long as it takes on a relay. A request with no message yet (a
    /// recorded attach) waits for the next listing, which decides it. One
    /// given up on is never sent if it hasn't been, and its late answer is
    /// dropped. Not on the UI thread: this blocks.
    fn ask<T>(
        &self,
        what: &str,
        abandon: Option<(Weak<Self>, &Abandon)>,
        limit: Option<Duration>,
        make: impl FnOnce(RequestId, Reply<T>) -> (Waiting, Option<ToCore>),
    ) -> Result<T> {
        let req = self.req();
        let (reply, answer) = mpsc::sync_channel(1);
        let (waiting, message) = make(req, reply);
        {
            let mut state = self.state.lock();
            if let Some(reason) = &state.ended {
                bail!("{reason}");
            }
            // A new tab tries the endpoint now, not after the pause.
            if self.is_relay() && !state.link.has_been_up() && !matches!(waiting, Waiting::Attach { .. }) {
                state.hurry = true;
                self.wake.notify_all();
            }
            let listed = message.is_none();
            let pending = if listed { Pending::recorded(waiting) } else { Pending::queued(waiting) };
            state.requests.insert(req, pending);
            if listed {
                self.request_list(&mut state);
            }
        }
        if let Some((connection, abandon)) = abandon
            && !abandon.asked(connection, req)
        {
            self.withdraw(req);
        }
        if let Some(message) = message {
            self.send(message);
        }
        let answer = match limit.or_else(|| (!self.is_relay()).then_some(REQUEST_LIMIT)) {
            None => answer.recv().map_err(|_| format!("the terminal server didn't {what}")),
            Some(limit) => answer
                .recv_timeout(limit)
                .map_err(|_| format!("the terminal server didn't {what} within {} s", limit.as_secs_f32())),
        };
        // Gone already when answered; a late answer is dealt with by `handle`.
        self.state.lock().requests.remove(&req);
        match answer {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(message)) | Err(message) => Err(anyhow!("{message}")),
        }
    }

    /// Withdraws a request that hasn't gone out: its tab let go while it
    /// waited. One already written is left to its answer.
    fn withdraw(&self, req: RequestId) {
        let pending = {
            let mut state = self.state.lock();
            match state.requests.get(&req) {
                Some(pending) if !pending.is_written() => state.requests.remove(&req),
                Some(_) | None => None,
            }
        };
        if let Some(pending) = pending {
            pending.waiting.fail("closed before it started".into());
        }
    }

    /// Starts a session whose output goes to `sink`, and waits for the core
    /// to report its id and pid. `abandon` lets its tab withdraw it while it
    /// waits. Not on the UI thread: this blocks.
    pub fn spawn(self: &Arc<Self>, request: SpawnRequest, sink: Sink, abandon: Option<&Abandon>) -> Result<Running> {
        let abandon = abandon.map(|abandon| (Arc::downgrade(self), abandon));
        let started = self.ask("start the session", abandon, None, |req, reply| {
            (Waiting::Spawn { sink, reply }, Some(ToCore::Spawn(request.into_spawn(req))))
        })?;
        let Started { session, pid, key } = started;
        Ok(Running::new(session, pid, key, self.clone()))
    }

    /// Attaches a detached session, whose replay and then live output go to
    /// `sink`, and waits for its `REPLAY_START`. A recorded session is
    /// looked up in the core's listing first. Not on the UI thread.
    pub fn attach(self: &Arc<Self>, request: AttachRequest, sink: Sink, abandon: Option<&Abandon>) -> Result<Running> {
        let (session, key) = (request.session, request.key.clone());
        let abandon = abandon.map(|abandon| (Arc::downgrade(self), abandon));
        let pid = self.ask("attach the session", abandon, None, |req, reply| request.waiting(req, sink, reply))?;
        Ok(Running::new(session, pid, key, self.clone()))
    }

    /// The core's sessions, this client's and detached ones. Blocks.
    #[cfg(test)]
    pub fn list(&self) -> Result<Vec<SessionInfo>> {
        self.list_within(None)
    }

    /// The core's sessions, this client's and detached ones, waiting at
    /// most `limit` when given (a relay otherwise waits through
    /// reconnects; a local connection waits its usual limit). Blocks.
    pub fn list_within(&self, limit: Option<Duration>) -> Result<Vec<SessionInfo>> {
        self.ask("list its sessions", None, limit, |req, reply| {
            (Waiting::List(reply), Some(ToCore::List(Request { req })))
        })
    }

    #[cfg(test)]
    pub fn kill(&self, session: SessionId) -> Result<()> {
        self.kill_within(session, None)
    }

    /// How many requests wait for their answer, for tests in the sibling
    /// modules.
    #[cfg(test)]
    pub(super) fn requests_waiting(&self) -> usize {
        self.state.lock().requests.len()
    }

    /// Ends a session whatever its policy or client, waiting at most `limit`
    /// when given (as [`Connection::list_within`]). Blocks.
    pub fn kill_within(&self, session: SessionId, limit: Option<Duration>) -> Result<()> {
        self.ask("end the session", None, limit, |req, reply| {
            let target = KillTarget::Session(session);
            (Waiting::Done(reply), Some(ToCore::Kill(Kill { req, target })))
        })
    }

    /// Hands `hook` the core's next listing: at once once a link is up,
    /// else on the first link. It keeps a relay connecting until then, as
    /// an attach does, unless it is withdrawn ([`Connection::stop_listing`]).
    /// Gives `hook` back when the connection has ended.
    pub fn on_listing(&self, hook: ListingHook) -> std::result::Result<(), ListingHook> {
        let mut state = self.state.lock();
        if state.ended.is_some() {
            return Err(hook);
        }
        state.on_listing.push(hook);
        self.request_list(&mut state);
        Ok(())
    }

    /// Withdraws the listing hooks still waiting ([`Connection::on_listing`]):
    /// they are dropped without a listing, and no longer keep the connection
    /// in use, so with nothing else using it, it ends [`Timings::idle`] later.
    /// Says whether any was waiting.
    pub fn stop_listing(&self) -> bool {
        let hooks = std::mem::take(&mut self.state.lock().on_listing);
        // The idle thread counts from now.
        self.wake.notify_all();
        !hooks.is_empty()
    }

    /// Asks for the core's sessions for what waits on a listing, unless a
    /// listing is on its way. Before the first link, the link asks
    /// ([`Connection::up`]); while reconnecting, the writer holds it and the
    /// next link's own `LIST` replaces it.
    fn request_list(&self, state: &mut State) {
        if !state.link.has_been_up() || state.list.is_some() {
            return;
        }
        let req = self.req();
        state.list = Some(Listing { req, after_loss: false });
        self.send(ToCore::List(Request { req }));
    }

    /// Reads each local link in turn, reconnecting when one ends.
    fn run_local(self: Arc<Self>, mut reader: Box<dyn Read + Send>) {
        let Transport::Local(endpoint) = &self.transport else { unreachable!("a local connection") };
        loop {
            let reason = self.read(&mut BufReader::with_capacity(READ_BUFFER, reader), None).local_text();
            log::warn!("terminal server connection ended ({reason}); reconnecting");
            self.disconnected(&reason);
            match self.reconnect_local(endpoint) {
                Ok(next) => reader = next,
                Err(reason) => {
                    log::warn!("terminal server lost: {reason}");
                    return self.ended(&reason);
                }
            }
        }
    }

    /// Connects relays and reads their links, pausing between attempts.
    fn run_relay(self: Arc<Self>) {
        let Transport::Relay { dial, timings } = &self.transport else { unreachable!("a relay connection") };
        let name = dial.name();
        let mut failures = 0;
        self.report_now(LinkState::Connecting);
        loop {
            match self.open_relay(dial.as_ref(), timings) {
                Ok((mut reader, stamp, wire)) => {
                    failures = 0;
                    let ending = self.read(&mut reader, Some((&stamp, timings.dead)));
                    if self.is_dead() {
                        return;
                    }
                    // `why` may wait for the process to exit: asked only when
                    // the watchdog didn't end the link.
                    let account = match stamp.expired() {
                        true => Some(RelayAccount::Silent(timings.dead)),
                        false => wire.why().map(RelayAccount::Said),
                    };
                    let reason = relay_link_ended(&name, &ending, account);
                    log::warn!("connection to {name} ended ({reason}); reconnecting");
                    self.disconnected(&reason);
                    self.lost_link(&name, &reason);
                    self.report_now(LinkState::Offline(format!("connection lost: {reason}")));
                }
                Err(_) if self.is_dead() => return,
                Err(error) => {
                    let message = format!("can't connect to {name}: {error:#}");
                    log::warn!("{message}");
                    let refused = error.downcast_ref::<Refused>().is_some();
                    match self.failed_to_connect(&message, refused) {
                        Next::Retry => {}
                        Next::End => return self.ended(&message),
                    }
                    self.report_now(LinkState::Offline(format!("{error:#}")));
                    failures += 1;
                }
            }
            let nothing_waits = self.state.lock().nothing_waits();
            let cap = retry_cap(self.eager.load(Ordering::Acquire), nothing_waits, timings);
            if !self.pause(relay::backoff(failures, timings.first_retry, cap)) {
                return;
            }
        }
    }

    /// Starts a relay and greets the core behind it, with the watchdog
    /// ending a link that takes longer than [`Timings::bootstrap`].
    fn open_relay(&self, dial: &dyn Dial, timings: &Timings) -> Result<GreetedRelay> {
        let Link { reader, wire } = dial.launch()?;
        let stamp = Arc::new(Stamp::new(Instant::now(), timings.bootstrap));
        self.watch_link(&wire, &stamp);
        let mut reader = BufReader::with_capacity(READ_BUFFER, reader);
        let greeted = self.greet(&dial.name(), &mut reader, wire.as_ref());
        match greeted.and_then(|(host, protocol)| self.up(wire.clone(), Some(host), protocol)) {
            Ok(()) => {
                // Sooner than the bootstrap's deadline the watchdog waits for.
                stamp.touch(Instant::now(), timings.dead);
                self.watch_changed.notify_all();
                self.report_now(LinkState::Connected);
                Ok((reader, stamp, wire))
            }
            Err(error) => {
                self.unwatch_link();
                wire.end();
                Err(error)
            }
        }
    }

    /// Reads the relay's `RELAY_HOST` (or its `RELAY_ERROR`) and says
    /// `HELLO` at once, since the core drops a client that hasn't within
    /// 5 s; then waits for `WELCOME`. Heartbeats are skipped.
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "railway R3 exception: any answer to HELLO but `Welcome` or `Error` fails the greeting, naming it"
    )]
    fn greet(&self, name: &str, reader: &mut impl BufRead, wire: &dyn Wire) -> Result<(RelayHost, u32)> {
        let host = loop {
            let frame = next_frame(reader, wire)?;
            if !is_relay_type(frame.msg_type) {
                bail!("the relay sent message {} before RELAY_HOST", frame.msg_type);
            }
            match RelayMessage::decode(&frame)? {
                RelayMessage::Host(host) => break host,
                RelayMessage::Error(error) => bail!("{}", error.message),
                RelayMessage::Heartbeat => {}
            }
        };
        wire.send(&ToCore::Hello(super::hello_message(&self.token, true)).encode()?)?;
        loop {
            let frame = next_frame(reader, wire)?;
            if is_relay_type(frame.msg_type) {
                continue;
            }
            match ToClient::decode(&frame)? {
                ToClient::Welcome(welcome) => {
                    log::info!(
                        "connected to {name}'s terminal server (core {} pid {}, relay {} {})",
                        welcome.core_version,
                        welcome.pid,
                        host.version,
                        host.build
                    );
                    return Ok((host, welcome.protocol));
                }
                ToClient::Error(error) => return Err(super::refused(&error.message)),
                other => bail!("the terminal server answered HELLO with {other:?}"),
            }
        }
    }

    /// A link is up. After a loss, `LIST` goes out first, to tell which
    /// sessions the core no longer has, and so it does on the first link
    /// when a recorded attach or a listing hook waits for one; then the
    /// writer gets the link.
    fn up(&self, wire: Arc<dyn Wire>, host: Option<RelayHost>, protocol: u32) -> Result<()> {
        let list = {
            let mut state = self.state.lock();
            if let Some(reason) = &state.ended {
                // It ended while this link was starting.
                bail!("{reason}");
            }
            let after_loss = state.link.has_been_up();
            let wanted = after_loss || state.waits_for_listing();
            // Under the same lock: what registers after this asks itself.
            state.link = LinkPhase::Up(Negotiated(protocol));
            wanted.then(|| {
                let listing = Listing { req: self.req(), after_loss };
                state.list = Some(listing);
                listing
            })
        };
        if let Some(listing) = list {
            wire.send(&ToCore::List(Request { req: listing.req }).encode()?)?;
        }
        let mut slot = self.slot.lock();
        if slot.closed {
            bail!("the connection ended");
        }
        slot.link = Some(Current { wire, host: host.map(Arc::new) });
        drop(slot);
        self.slot_ready.notify_all();
        Ok(())
    }

    /// Reads one link until its stream ends, saying how. On a relay,
    /// `liveness`'s stamp is held while each frame is handled and then
    /// touched.
    fn read(&self, reader: &mut impl BufRead, liveness: Option<(&Stamp, Duration)>) -> Ending {
        loop {
            let frame = match Frame::read_from_or_eof(reader) {
                Ok(Some(frame)) => frame,
                Ok(None) => return Ending::Closed,
                Err(error) => return Ending::Failed(error),
            };
            if let Some((stamp, _)) = liveness {
                stamp.hold();
            }
            if is_relay_type(frame.msg_type) {
                // Heartbeats only keep the link alive.
                match RelayMessage::decode(&frame) {
                    Ok(RelayMessage::Heartbeat) => {}
                    Ok(other) => log::debug!("relay: {other:?} after HOST"),
                    Err(error) => log::warn!("relay: {error}"),
                }
            } else {
                match ToClient::decode(&frame) {
                    Ok(message) => self.handle(message),
                    Err(error) => log::warn!("terminal server: {error}"),
                }
            }
            if let Some((stamp, dead)) = liveness {
                stamp.touch(Instant::now(), dead);
            }
        }
    }

    fn handle(&self, message: ToClient) {
        match message {
            ToClient::Data(Data { session, bytes }) => {
                if let Some(sink) = self.sink(session) {
                    self.deliver(session, &sink, |sink| sink.output(&bytes));
                }
            }
            ToClient::Spawned(spawned) => {
                let routed = self.state.lock().route_spawned(&spawned);
                if let Some((_, reply, started, displaced)) = self.route(routed) {
                    let session = started.session;
                    reply.try_send(Ok(started)).ok();
                    self.displaced_lost(session, displaced);
                }
            }
            ToClient::Error(error) => {
                let pending = error.req.and_then(|req| self.state.lock().requests.remove(&req));
                match pending {
                    Some(pending) => pending.waiting.fail(error.message),
                    None => match error.session {
                        // An answer to no request, about one session: its
                        // tab says so (the holder's fault reports, a refused
                        // input), once until the session prints again.
                        Some(session) if error.req.is_none() => self.session_error(session, &error.message),
                        Some(_) | None => log::debug!("terminal server: {}", error.message),
                    },
                }
            }
            ToClient::Sessions(sessions) if self.state.lock().list.is_none_or(|listing| listing.req != sessions.req) => {
                let pending = self.state.lock().requests.remove(&sessions.req);
                match pending.map(|pending| pending.waiting) {
                    Some(Waiting::List(reply)) => {
                        reply.try_send(Ok(sessions.sessions)).ok();
                    }
                    Some(other) => other.fail("answered with SESSIONS".into()),
                    None => {}
                }
            }
            ToClient::Ok(ok) => {
                let pending = self.state.lock().requests.remove(&ok.req);
                match pending.map(|pending| pending.waiting) {
                    Some(Waiting::Done(reply)) => {
                        reply.try_send(Ok(())).ok();
                    }
                    Some(other) => other.fail("answered with OK".into()),
                    None => {}
                }
            }
            ToClient::ReplayStart(ReplayStart { req: Some(req), session, pid }) => {
                let routed = self.state.lock().route_replay_start(req, session, pid);
                if let Some((sink, reply, pid, displaced)) = self.route(routed) {
                    self.displaced_lost(session, displaced);
                    self.deliver(session, &sink, |sink| sink.replay(true));
                    reply.try_send(Ok(pid)).ok();
                }
            }
            ToClient::Exit(exit) => {
                // Its session and window go with it.
                let shown = self.state.lock().sessions.remove(&exit.session);
                if let Some(Shown { sink, .. }) = shown {
                    self.deliver(exit.session, &sink, |sink| match exit.lost {
                        Some(reason) => sink.lost(&format!("the terminal server lost it: {reason}")),
                        None => sink.exited(exit.code),
                    });
                }
            }
            ToClient::ReplayStart(start) => {
                let session = start.session;
                if let Some(sink) = self.sink(session) {
                    self.deliver(session, &sink, |sink| sink.replay(true));
                } else if let Some((sink, reply, displaced)) = self.attached_by_replay(session) {
                    self.displaced_lost(session, displaced);
                    self.deliver(session, &sink, |sink| sink.replay(true));
                    reply.try_send(Ok(start.pid)).ok();
                } else {
                    // Not one of this process's tabs (any more): it stays
                    // with the core, detached. Only a user's close ends a
                    // program.
                    self.send(ToCore::Detach(Detach { session, layout: None }));
                }
            }
            ToClient::ReplayEnd(end) => {
                if let Some(sink) = self.sink(end.session) {
                    // A broken one is no longer shown, so has no window.
                    self.deliver(end.session, &sink, |sink| sink.replay(false));
                }
                // The core that replayed may be a new one, which doesn't
                // know the session's window.
                let parent = self.state.lock().sessions.get(&end.session).and_then(|shown| shown.parent);
                if let Some(hwnd) = parent {
                    self.send(ToCore::Reparent(Reparent { session: end.session, hwnd }));
                }
            }
            ToClient::Sessions(sessions) => {
                let name = self.name();
                let (lost, decided, hooks) = {
                    let mut state = self.state.lock();
                    let Some(listing) = state.list.filter(|listing| listing.req == sessions.req) else {
                        return;
                    };
                    state.list = None;
                    let listed: Vec<ListedSession> = sessions.sessions.iter().map(ListedSession::of).collect();
                    // Only a lost link's sessions can be missing: on a link
                    // that stayed up, every session shown here is listed.
                    let lost = if listing.after_loss { state.keep_listed(&listed) } else { Vec::new() };
                    let decided = carried_attaches(&mut state.requests, &listed, &name);
                    (lost, decided, std::mem::take(&mut state.on_listing))
                };
                for (session, sink) in lost {
                    self.deliver(session, &sink, |sink| sink.lost(NO_LONGER_LISTED));
                }
                for (pending, reason) in decided.gone {
                    pending.waiting.fail(reason);
                }
                for (attach, recorded) in decided.again {
                    // Told before the ATTACH goes out, so before its replay.
                    if let Some((sink, (cols, rows))) = recorded
                        && let Delivered::Panicked(reason) =
                            self.deliver(attach.session, &sink, |sink| sink.replay_size(cols, rows))
                    {
                        // Its tab can't show it: it stays detached.
                        let pending = self.state.lock().requests.remove(&attach.req);
                        if let Some(pending) = pending {
                            pending.waiting.fail(format!("Brindle can't show session {}: {reason}", attach.session));
                        }
                        continue;
                    }
                    self.send(ToCore::Attach(attach));
                }
                for hook in hooks {
                    hook(sessions.sessions.clone());
                }
            }
            ToClient::Process(process) => {
                let renamed = {
                    let mut state = self.state.lock();
                    // Only for a session shown here: SPAWNED or REPLAY_START
                    // put its sink in the map before this was read.
                    let Some(shown) = state.sessions.get_mut(&process.session) else { return };
                    let foreground = Foreground { name: process.name, cwd: process.cwd };
                    let previous = shown.foreground.replace(foreground.clone());
                    (previous.map(|previous| previous.name) != Some(foreground.name)).then(|| shown.sink.clone())
                };
                if let Some(sink) = renamed {
                    self.deliver(process.session, &sink, |sink| sink.foreground());
                }
            }
            ToClient::Welcome(_) | ToClient::ServerStatus(_) => {}
        }
    }

    /// Applies what a [`Routed`] answer does besides showing its session:
    /// fails the request of another kind it answered, then ends or detaches
    /// the session nobody here waits for. Gives a shown session's sink,
    /// reply, answer and displaced entry back to the caller. Called without
    /// the state lock.
    fn route<T>(&self, routed: Routed<T>) -> Option<(Sink, Reply<T>, T, Option<Shown>)> {
        let fail = |fail: Option<(Waiting, &str)>| {
            if let Some((waiting, why)) = fail {
                waiting.fail(why.into());
            }
        };
        match routed {
            Routed::Shown { sink, reply, answer, displaced } => Some((sink, reply, answer, displaced)),
            Routed::Close { session, fail: misanswered } => {
                fail(misanswered);
                self.send_close(session);
                None
            }
            Routed::Detach { session, fail: misanswered } => {
                fail(misanswered);
                self.send(ToCore::Detach(Detach { session, layout: None }));
                None
            }
        }
    }

    /// A server `ERROR` about `session`, with no request: its tab shows it
    /// as a notice while the session is shown here; otherwise it is logged
    /// (the core sends these only for sessions this client has attached, so
    /// this is a session already forgotten).
    fn session_error(&self, session: SessionId, message: &str) {
        self.note_session(session, &format!("the terminal server: {message}"), || {
            log::warn!("terminal server, about session {session} on {}: {message}", self.name());
        });
    }

    /// Shows `text` in the tab of `session`, or calls `unshown` when no
    /// session by that id is shown. Called without the state lock.
    fn note_session(&self, session: SessionId, text: &str, unshown: impl FnOnce()) {
        match self.sink(session) {
            Some(sink) => {
                self.deliver(session, &sink, |sink| sink.note(text));
            }
            None => unshown(),
        }
    }

    fn sink(&self, session: SessionId) -> Option<Sink> {
        self.state.lock().sessions.get(&session).map(|shown| shown.sink.clone())
    }

    /// Calls `call` on `session`'s sink, confining a panic to that session
    /// (`process R3`): the reader serves every session on the endpoint, and
    /// Brindle's emulator failing on one session's output must not stop it
    /// for the rest. After a panic, a session still shown with this sink
    /// stops being shown and is detached: its program is fine, only
    /// Brindle's view of it failed, so it keeps running for the user to
    /// attach again (not at once: the replay may carry the same bytes).
    /// Then the sink is told it is broken, under a guard of its own. A sink
    /// no longer shown (its session exited, was lost, or isn't shown yet)
    /// is only told. Called without the state lock.
    fn deliver(&self, session: SessionId, sink: &Sink, call: impl FnOnce(&mut dyn SessionSink)) -> Delivered {
        let Err(payload) = catch_unwind(AssertUnwindSafe(|| call(&mut **sink.lock()))) else {
            return Delivered::Done;
        };
        let reason = panic_text(&*payload);
        log::error!("Brindle can't show session {session} on {} any more ({reason}): detaching it", self.name());
        {
            let mut state = self.state.lock();
            let key = state.sessions.get(&session).filter(|shown| Arc::ptr_eq(&shown.sink, sink)).map(|shown| shown.key.clone());
            if let Some(key) = key
                && state.forget(session, key.as_ref()).is_some()
            {
                // Under the state lock, as a handle's messages are.
                self.send(ToCore::Detach(Detach { session, layout: None }));
            }
        }
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| sink.lock().broken(&reason))) {
            log::error!("telling session {session}'s tab that it can't be shown failed too: {}", panic_text(&*payload));
        }
        Delivered::Panicked(reason)
    }

    /// Tells the tab of an entry another session displaced from its id
    /// `session` that its session is lost.
    fn displaced_lost(&self, session: SessionId, displaced: Option<Shown>) {
        if let Some(Shown { sink, key, .. }) = displaced {
            let key = key.as_ref().map_or("none", SessionKey::as_str);
            log::warn!("session {session} (key {key}) is lost: the terminal server gave its id to another session");
            self.deliver(session, &sink, |sink| sink.lost(NO_LONGER_LISTED));
        }
    }

    /// A replay without a `req` (a reconnect's) of a session an attach
    /// waits for answers that attach: its sink joins the map, with the
    /// attach's key, and the entry it displaced (if any) comes back. A
    /// recorded attach not sent yet asked the core for nothing, so it isn't
    /// one.
    ///
    /// It matches by id: `REPLAY_START` carries no key, and a replay without
    /// a `req` comes only for a session the core gives back to this client's
    /// token. A core that adopted the holders gives back the same session; a
    /// fresh core, only sessions this client spawned or attached on it since
    /// it reconnected. So the id names the session the attach waits for.
    fn attached_by_replay(&self, session: SessionId) -> Option<(Sink, Reply<u32>, Option<Shown>)> {
        let mut state = self.state.lock();
        let req = state.requests.iter().find_map(|(&req, pending)| match &pending.waiting {
            Waiting::Attach { attach, awaits, .. } if attach.session == session => match awaits {
                Awaits::Replay => Some(req),
                Awaits::Listing => None,
            },
            Waiting::Attach { .. } | Waiting::Spawn { .. } | Waiting::List(_) | Waiting::Done(_) => None,
        })?;
        let Some(Waiting::Attach { sink, reply, key, .. }) = state.requests.remove(&req).map(|pending| pending.waiting)
        else {
            unreachable!("an attach")
        };
        let displaced = state.show(session, Shown::new(sink.clone(), key));
        Some((sink, reply, displaced))
    }

    /// The link ended: the writer holds its messages, and requests that
    /// went out on it fail, since their answers are gone. Attaches stay,
    /// carried over to the next link.
    fn disconnected(&self, reason: &str) {
        if let Some(current) = self.slot.lock().link.take() {
            current.wire.end();
        }
        self.unwatch_link();
        let failed = self.state.lock().link_lost();
        for pending in failed {
            pending.waiting.fail(format!("the terminal server went away ({reason})"));
        }
    }

    /// A relay link was lost, for `reason`: each of the endpoint's
    /// terminals says so.
    fn lost_link(&self, name: &str, reason: &str) {
        let sinks: Vec<(SessionId, Sink)> =
            self.state.lock().sessions.iter().map(|(&session, shown)| (session, shown.sink.clone())).collect();
        let line = lost_line(name, reason);
        for (session, sink) in sinks {
            self.deliver(session, &sink, |sink| sink.output(line.as_bytes()));
        }
    }

    /// A relay attempt failed with `message`; `refused` when the core
    /// refused this Brindle ([`Refused`]). Fails and tells what
    /// [`State::connect_failed`] says, and says whether to keep trying.
    fn failed_to_connect(&self, message: &str, refused: bool) -> Next {
        let eager = self.eager.load(Ordering::Acquire);
        let ConnectFailed { failed, told, next } = self.state.lock().connect_failed(refused, eager, message);
        for pending in failed {
            pending.waiting.fail(message.to_string());
        }
        let line = retrying_line(message);
        for (session, sink) in told {
            self.deliver(session, &sink, |sink| sink.output(line.as_bytes()));
        }
        next
    }

    /// Waits `delay` before the next relay attempt, or less when a new tab
    /// asks for one. False when the connection ended meanwhile.
    fn pause(&self, delay: Duration) -> bool {
        let deadline = Instant::now() + delay;
        let mut state = self.state.lock();
        while !state.hurry && state.ended.is_none() {
            if self.wake.wait_until(&mut state, deadline).timed_out() {
                break;
            }
        }
        state.hurry = false;
        state.ended.is_none()
    }

    fn watch_link(&self, wire: &Arc<dyn Wire>, stamp: &Arc<Stamp>) {
        self.watch.lock().link = Some((wire.clone(), stamp.clone()));
        self.watch_changed.notify_all();
    }

    fn unwatch_link(&self) {
        self.watch.lock().link = None;
    }

    /// Ends a relay link whose deadline passed with nothing read: the read
    /// then ends, and the reader reconnects. It never blocks on the link.
    fn watchdog(&self, name: &str, recheck: Duration) {
        let mut watch = self.watch.lock();
        loop {
            if watch.closed {
                return;
            }
            let Some((wire, stamp)) = watch.link.clone() else {
                self.watch_changed.wait(&mut watch);
                continue;
            };
            match stamp.deadline() {
                // A frame is being handled.
                None => {
                    self.watch_changed.wait_for(&mut watch, recheck);
                }
                Some(deadline) if Instant::now() >= deadline => {
                    log::warn!("nothing from {name} in time: ending the link");
                    watch.link = None;
                    drop(watch);
                    // Before the end, which is what the reader sees.
                    stamp.expire();
                    wire.end();
                    watch = self.watch.lock();
                }
                Some(deadline) => {
                    self.watch_changed.wait_until(&mut watch, deadline);
                }
            }
        }
    }

    /// Connects again with the same token, starting a core if none appears
    /// within [`START_AFTER`], for at most [`RECONNECT_LIMIT`]. Then asks
    /// for the session list and hands the writer the new stream.
    fn reconnect_local(&self, endpoint: &Endpoint) -> std::result::Result<Box<dyn Read + Send>, String> {
        let started = Instant::now();
        let mut next_start = START_AFTER;
        // The latest error of an attempt, for the line that says it gave up.
        let mut last: Option<String> = None;
        loop {
            let elapsed = started.elapsed();
            if elapsed >= RECONNECT_LIMIT {
                return Err(reconnect_gave_up(RECONNECT_LIMIT, last.as_deref()));
            }
            match super::try_connect(endpoint) {
                Ok(Some(stream)) => match super::hello(stream, &self.token) {
                    Ok(connected) => return self.resume_local(connected).map_err(|error| format!("{error:#}")),
                    Err(error) if error.downcast_ref::<Refused>().is_some() => return Err(format!("{error:#}")),
                    Err(error) => {
                        log::debug!("reconnecting: {error:#}");
                        last = Some(format!("{error:#}"));
                    }
                },
                Ok(None) if elapsed >= next_start => {
                    next_start += START_AFTER;
                    if let Err(error) = super::start_core(endpoint) {
                        log::warn!("reconnecting: {error:#}");
                        last = Some(format!("{error:#}"));
                    }
                    continue;
                }
                Ok(None) => {}
                Err(error) => {
                    log::debug!("reconnecting: {error:#}");
                    last = Some(format!("{error:#}"));
                }
            }
            std::thread::sleep(RECONNECT_POLL);
        }
    }

    fn resume_local(&self, connected: Connected) -> Result<Box<dyn Read + Send>> {
        let Connected { stream, welcome } = connected;
        log::info!("reconnected to the terminal server (core pid {})", welcome.pid);
        let link = Link::local(stream)?;
        self.up(link.wire, None, welcome.protocol)?;
        Ok(link.reader)
    }

    /// The connection is gone for good: every session is lost and every
    /// request fails. Its link ends (a relay's process is killed), even one
    /// still starting. Never called with the state or watch lock held.
    fn ended(&self, reason: &str) {
        let (sessions, requests, hook) = {
            let mut state = self.state.lock();
            state.ended.get_or_insert_with(|| reason.to_string());
            self.report(&mut state, LinkState::Ended(reason.to_string()));
            // No listing will come.
            state.on_listing.clear();
            (std::mem::take(&mut state.sessions), std::mem::take(&mut state.requests), state.on_end.take())
        };
        self.wake.notify_all();
        {
            let mut slot = self.slot.lock();
            slot.closed = true;
            if let Some(current) = slot.link.take() {
                current.wire.end();
            }
        }
        self.slot_ready.notify_all();
        let watched = {
            let mut watch = self.watch.lock();
            watch.closed = true;
            watch.link.take()
        };
        // A relay still greeting the core isn't the writer's yet.
        if let Some((wire, _)) = watched {
            wire.end();
        }
        self.watch_changed.notify_all();
        for pending in requests.into_values() {
            pending.waiting.fail(reason.to_string());
        }
        for (session, shown) in sessions {
            self.deliver(session, &shown.sink, |sink| sink.lost(reason));
        }
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Sends queued messages in order, and on a relay a heartbeat after
    /// [`Timings::heartbeat`] without a write.
    fn write_loop(&self, queue: mpsc::Receiver<Outgoing>) {
        let heartbeat = match &self.transport {
            Transport::Relay { timings, .. } => Some(timings.heartbeat),
            Transport::Local(_) => None,
        };
        let mut last = Instant::now();
        loop {
            let outgoing = match heartbeat {
                None => match queue.recv() {
                    Ok(outgoing) => outgoing,
                    Err(_) => return,
                },
                Some(after) => match queue.recv_timeout(after.saturating_sub(last.elapsed())) {
                    Ok(outgoing) => outgoing,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if !self.heartbeat() {
                            return;
                        }
                        last = Instant::now();
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                },
            };
            let open = self.write(&outgoing.message);
            // Written, skipped, or never to be written: its input no longer
            // waits.
            outgoing.handled();
            self.slot.lock().handled += 1;
            self.slot_ready.notify_all();
            if !open {
                return;
            }
            last = Instant::now();
        }
    }

    /// Sends a heartbeat on the current link, if there is one. False once
    /// the connection ended.
    fn heartbeat(&self) -> bool {
        let current = {
            let slot = self.slot.lock();
            if slot.closed {
                return false;
            }
            slot.link.clone()
        };
        if let Some(current) = current
            && let Err(error) = RelayMessage::Heartbeat.encode().and_then(|frame| current.wire.send(&frame))
        {
            log::debug!("sending a heartbeat failed: {error}");
            self.drop_link(&current.wire);
        }
        true
    }

    /// Writes `message`, waiting for a link while there is none; a message
    /// whose write failed goes out on the next link. False once the
    /// connection ended.
    fn write(&self, message: &ToCore) -> bool {
        #[allow(
            clippy::wildcard_enum_match_arm,
            reason = "railway R3 exception: the other messages carry no request id the link has to keep"
        )]
        let request = match message {
            ToCore::Spawn(client::Spawn { req, .. })
            | ToCore::Attach(client::Attach { req, .. })
            | ToCore::List(Request { req })
            | ToCore::Kill(Kill { req, .. }) => Some(*req),
            _ => None,
        };
        loop {
            let current = {
                let mut slot = self.slot.lock();
                loop {
                    if slot.closed {
                        return false;
                    }
                    if let Some(current) = &slot.link {
                        break current.clone();
                    }
                    self.slot_ready.wait(&mut slot);
                }
            };
            // Nobody waits for its answer any more.
            if let Some(req) = request
                && !self.mark_written(req, true)
            {
                return true;
            }
            // Given up while no link was up ([`State::give_up_closes`]).
            if let ToCore::Close(close) = message
                && !self.state.lock().closing.iter().any(|&(session, _)| session == close.session)
            {
                return true;
            }
            let frame = match encode(message, current.host.as_deref()) {
                Ok(frame) => frame,
                Err(error) => {
                    log::warn!("terminal server message not sent: {error}");
                    // Its answer would never come.
                    let pending = request.and_then(|req| self.state.lock().requests.remove(&req));
                    match (pending, session_of(message)) {
                        (Some(pending), _) => pending.waiting.fail(format!("the request can't be sent: {error}")),
                        // A message of a shown session: its tab says so.
                        (None, Some((session, kind))) => self.note_session(
                            session,
                            &format!("Brindle couldn't send this session's {kind}: {error}"),
                            || {},
                        ),
                        (None, None) => {}
                    }
                    return true;
                }
            };
            match current.wire.send(&frame) {
                Ok(()) => {
                    if let ToCore::Close(close) = message {
                        let mut state = self.state.lock();
                        if let Some(ix) = state.closing.iter().position(|&(session, _)| session == close.session) {
                            state.closing.remove(ix);
                        }
                    }
                    return true;
                }
                Err(error) => {
                    log::debug!("writing to the terminal server failed: {error}");
                    if let Some(req) = request {
                        self.mark_written(req, false);
                    }
                    self.drop_link(&current.wire);
                }
            }
        }
    }

    /// Takes `wire` from the writer, unless a newer link replaced it.
    fn drop_link(&self, wire: &Arc<dyn Wire>) {
        let mut slot = self.slot.lock();
        if slot.link.as_ref().is_some_and(|current| Arc::ptr_eq(&current.wire, wire)) {
            slot.link = None;
            drop(slot);
            // A flush waiting on it gives up.
            self.slot_ready.notify_all();
        }
    }

    /// Marks a request written or not; false when nobody waits for it. The
    /// listing the reader waits for isn't a request, and goes out.
    fn mark_written(&self, req: RequestId, written: bool) -> bool {
        let mut state = self.state.lock();
        if state.list.is_some_and(|listing| listing.req == req) {
            return true;
        }
        match state.requests.get_mut(&req) {
            Some(pending) => {
                pending.mark_written(written);
                true
            }
            None => false,
        }
    }
}

/// How a link's stream ended.
enum Ending {
    /// The far side closed it.
    Closed,
    /// Reading from it failed.
    Failed(io::Error),
}

impl Ending {
    /// The reason for the local core's socket or pipe, which only the core
    /// can close.
    fn local_text(&self) -> String {
        match self {
            Ending::Closed => "the terminal server closed the connection".to_string(),
            Ending::Failed(error) => format!("reading from the terminal server failed: {error}"),
        }
    }
}

/// A duration as a reason gives it: whole minutes as `5 minutes`, whole
/// seconds as `30 s`, else `400 ms`.
fn span_text(span: Duration) -> String {
    match (span.as_secs(), span.subsec_nanos()) {
        (60, 0) => "1 minute".to_string(),
        (secs, 0) if secs > 0 && secs % 60 == 0 => format!("{} minutes", secs / 60),
        (secs, 0) => format!("{secs} s"),
        _ => format!("{} ms", span.as_millis()),
    }
}

/// What a connection logs, and ends with if nothing else uses it, when it
/// gives up the `CLOSE`s of `sessions` on endpoint `name`: no link came up
/// within `limit` ([`Timings::close_limit`]) of the oldest.
fn not_ending(sessions: &[SessionId], name: &str, limit: Duration) -> String {
    let ids: Vec<String> = sessions.iter().map(ToString::to_string).collect();
    let noun = if sessions.len() == 1 { "session" } else { "sessions" };
    format!(
        "not ending {noun} {} on {name}: it couldn't be reached within {} of the close",
        ids.join(", "),
        span_text(limit)
    )
}

/// What the reader knows of a relay link's end besides the stream: the
/// watchdog's verdict or the relay process's own account.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RelayAccount {
    /// The watchdog ended the link after this long without a read. The
    /// process was killed, so what it says would only report that, and it
    /// isn't asked.
    Silent(Duration),
    /// What the relay's process says ([`Wire::why`]: its status and what it
    /// wrote to stderr). Made only when it says something.
    Said(String),
}

/// Why a relay link ended, in the order that tells the most (design
/// decision 2): the watchdog ended it for silence; else what the relay's
/// process says; else, with no account (`None`), how the stream ended. A
/// process that exited 0 with nothing to say is a relay that closed the
/// connection, as it does when its core connection closes.
fn relay_link_ended(name: &str, ending: &Ending, account: Option<RelayAccount>) -> String {
    match account {
        Some(RelayAccount::Silent(dead)) => format!("nothing read from {name} for {}", span_text(dead)),
        Some(RelayAccount::Said(why)) => why,
        None => match ending {
            Ending::Closed => "the relay closed the connection".to_string(),
            Ending::Failed(error) => format!("reading from the relay failed: {error}"),
        },
    }
}

/// The next frame of a relay link that is starting; its end says why.
fn next_frame(reader: &mut impl BufRead, wire: &dyn Wire) -> Result<Frame> {
    match Frame::read_from_or_eof(reader) {
        Ok(Some(frame)) => Ok(frame),
        Ok(None) => match wire.why() {
            None => bail!("the relay ended before the terminal server answered"),
            Some(why) => bail!("the relay ended before the terminal server answered: {why}"),
        },
        Err(error) => bail!("reading from the relay failed: {error}"),
    }
}

/// `message` as a frame. A relay's `SPAWN` carries the endpoint's
/// environment with the request's over it.
fn encode(message: &ToCore, host: Option<&RelayHost>) -> io::Result<Frame> {
    if let (ToCore::Spawn(spawn), Some(host)) = (message, host) {
        let env = relay::relay_env(&host.env, &spawn.env);
        return ToCore::Spawn(client::Spawn { env, ..spawn.clone() }).encode();
    }
    message.encode()
}

/// Why a session shown here is lost when the core no longer has it: a
/// reconnect's listing lacks it with its key, or another session took its id.
const NO_LONGER_LISTED: &str = "the terminal server no longer has this session";

/// What a relay's loss writes into each of its terminals: one dim line,
/// with the first line of why (`railway R8`), through the notice line's
/// control filter, since the reason may carry what a relay or the host
/// printed.
fn lost_line(name: &str, reason: &str) -> String {
    let first = reason.lines().map(str::trim).find(|line| !line.is_empty());
    match first {
        Some(first) => tab_note::line(&format!("connection to {name} lost ({first}); reconnecting…")),
        None => tab_note::line(&format!("connection to {name} lost; reconnecting…")),
    }
}

/// Why reconnecting to the local terminal server gave up after `limit`:
/// the latest error it met, or, when nothing failed and no server came
/// back, only the timeout.
fn reconnect_gave_up(limit: Duration, last: Option<&str>) -> String {
    let timeout = format!("no terminal server came back within {} s", limit.as_secs());
    match last {
        Some(last) => format!("{timeout}: {last}"),
        None => timeout,
    }
}

/// A layout whose JSON is over [`client::MAX_LAYOUT`], which the core would
/// drop anyway.
#[derive(Debug, PartialEq, Eq)]
struct LayoutTooLarge {
    size: usize,
}

impl std::fmt::Display for LayoutTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "its layout is {} bytes, over the {} the terminal server keeps", self.size, client::MAX_LAYOUT)
    }
}

/// The layout to send with a `SET_LAYOUT` or `DETACH`: itself, unless it is
/// too large for the core to keep.
fn layout_to_send(layout: Option<serde_json::Value>) -> std::result::Result<Option<serde_json::Value>, LayoutTooLarge> {
    let Some(value) = &layout else { return Ok(None) };
    let size = serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len());
    if size > client::MAX_LAYOUT {
        return Err(LayoutTooLarge { size });
    }
    Ok(layout)
}

/// The layout a session's handle last dealt with, so an unchanged one isn't
/// sent (or complained about) again.
#[derive(Debug, PartialEq)]
enum LayoutSent {
    /// Sent with `SET_LAYOUT` (nothing yet: no layout).
    Sent(Option<serde_json::Value>),
    /// Too large to send, logged once; the next change is tried again.
    TooLarge(Option<serde_json::Value>),
}

impl Default for LayoutSent {
    fn default() -> Self {
        Self::Sent(None)
    }
}

impl LayoutSent {
    fn layout(&self) -> &Option<serde_json::Value> {
        match self {
            Self::Sent(layout) | Self::TooLarge(layout) => layout,
        }
    }
}

/// The session a message is about, and what the message is (for the line a
/// failure to send it puts in that session's tab). None for the messages
/// that name no session: a request's failure is its own answer.
fn session_of(message: &ToCore) -> Option<(SessionId, &'static str)> {
    match message {
        ToCore::Data(data) => Some((data.session, "input")),
        ToCore::Resize(resize) => Some((resize.session, "resize")),
        ToCore::Close(close) => Some((close.session, "close")),
        ToCore::Detach(detach) => Some((detach.session, "detach")),
        ToCore::Reparent(reparent) => Some((reparent.session, "window")),
        ToCore::SetLayout(layout) => Some((layout.session, "layout")),
        ToCore::Hello(_)
        | ToCore::Spawn(_)
        | ToCore::Attach(_)
        | ToCore::List(_)
        | ToCore::Kill(_)
        | ToCore::Status(_)
        | ToCore::Stop(_)
        | ToCore::StepDown(_) => None,
    }
}

/// What an attach waiting on a relay's first link is told once.
fn retrying_line(message: &str) -> String {
    format!("\r\n\x1b[2m{}; retrying…\x1b[22m\r\n", message.replace('\n', "\r\n"))
}

/// An attach to send (again). A recorded session's comes with its sink and
/// the size it was listed at, which its replay is rendered at: the sink is
/// told it before the `ATTACH` goes out. A core that lists no size tells it
/// nothing.
type Again = (client::Attach, Option<(Sink, (u16, u16))>);

/// What a listing decides for the attaches that wait on it.
#[derive(Default)]
struct Decided {
    again: Vec<Again>,
    /// Taken out, with why.
    gone: Vec<(Pending, String)>,
}

/// What becomes of the attaches waiting on the core's `LIST` (`listed`),
/// at endpoint `name`. A session is listed when the listing has its id with
/// its key ([`ListedSession::is`]):
/// - **Recorded (`Awaits::Listing`, not sent yet):** sent at the listed
///   size (else the request's) when its session is listed, with a key, and
///   no client; it then waits for its replay. Taken out when another client
///   has it, or when it has no key or isn't listed (it ended, and the id may
///   be another session's now).
/// - **Carried over a lost link (`Awaits::Replay`):** the core gave this
///   client back the sessions it owns before it answered, so one listed with
///   a client waits for that replay. One listed without a client is sent
///   again. One not listed, or listed under another key, is taken out.
fn carried_attaches(requests: &mut HashMap<RequestId, Pending>, listed: &[ListedSession], name: &str) -> Decided {
    let mut decided = Decided::default();
    let carried: Vec<RequestId> =
        requests.iter().filter(|(_, pending)| pending.is_carried()).map(|(&req, _)| req).collect();
    for req in carried {
        let Some(pending) = requests.get_mut(&req) else { continue };
        let Waiting::Attach { attach, key, awaits, sink, .. } = &mut pending.waiting else { continue };
        let session = attach.session;
        let found = listed.iter().find(|listing| listing.is(session, key.as_ref()));
        let why = match awaits {
            Awaits::Listing => match found {
                Some(listing) if key.is_some() => {
                    if listing.attached {
                        Some(format!("session {session} on {name} is shown by another Brindle"))
                    } else {
                        if let Some((cols, rows)) = listing.size {
                            (attach.cols, attach.rows) = (cols, rows);
                        }
                        *awaits = Awaits::Replay;
                        pending.sent = Sent::Queued;
                        decided.again.push((attach.clone(), listing.size.map(|size| (sink.clone(), size))));
                        None
                    }
                }
                Some(_) | None => Some(format!("session {session} on {name} has ended")),
            },
            Awaits::Replay => match found {
                Some(listing) if listing.attached => None,
                Some(_) => {
                    pending.sent = Sent::Queued;
                    decided.again.push((attach.clone(), None));
                    None
                }
                None => Some(format!("the terminal server no longer has session {session}")),
            },
        };
        if let Some(why) = why {
            decided.gone.extend(requests.remove(&req).map(|pending| (pending, why)));
        }
    }
    decided
}

/// A session the core started, once [`super::Session`] has it.
///
/// It acts only while its connection shows that session, by id and key
/// ([`State::shows`]). Once the session exited or was lost, its id can name
/// a session a fresh core started since, another tab's, which this one's
/// input, resizes, window, layout, detach and close must never reach. The
/// tab already says why it stopped (its exit or lost line), so the calls
/// do nothing.
pub struct Running {
    id: SessionId,
    pid: u32,
    /// `None` from a core that predates keys.
    key: Option<SessionKey>,
    conn: Arc<Connection>,
    /// The layout last sent with `SET_LAYOUT`, so an unchanged one isn't
    /// sent again (nor a too-large one complained about again).
    layout: Mutex<LayoutSent>,
    /// What bounds the session's waiting input; the [`super::Session`]'s
    /// own once it hands over ([`Running::with_budget`]).
    budget: Budget,
}

impl Running {
    fn new(id: SessionId, pid: u32, key: Option<SessionKey>, conn: Arc<Connection>) -> Self {
        Self { id, pid, key, conn, layout: Mutex::default(), budget: Budget::default() }
    }

    /// The handle, counting its input against `budget`, which input held
    /// while the session started was already counted in.
    pub(super) fn with_budget(self, budget: Budget) -> Self {
        Self { budget, ..self }
    }

    /// The program's pid.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn id(&self) -> SessionId {
        self.id
    }

    /// The session's key; `None` from a core that predates keys.
    pub fn key(&self) -> Option<&SessionKey> {
        self.key.as_ref()
    }

    /// Sends `messages` while the connection shows this session, and
    /// nothing otherwise. They are queued under the state lock, so the
    /// reader can't stop showing the session, or show another under its id,
    /// in between.
    fn send_while_shown(&self, messages: impl IntoIterator<Item = ToCore>) {
        let state = self.conn.state.lock();
        if state.shows(self.id, self.key.as_ref()) {
            for message in messages {
                self.conn.send(message);
            }
        }
    }

    /// Sends input for the program while the connection shows the session
    /// and its budget takes it; a call is taken or refused whole. A session
    /// no longer shown discards it, uncounted.
    pub fn input(&self, bytes: &[u8]) -> Delivery {
        let state = self.conn.state.lock();
        if !state.shows(self.id, self.key.as_ref()) {
            return Delivery::Discarded;
        }
        let offer = self.budget.lock().offer(bytes.len());
        match offer {
            Offer::Take => {
                self.queue_input(bytes);
                Delivery::Queued
            }
            Offer::Refuse(refusal) => Delivery::Refused(refusal),
        }
    }

    /// Sends input held while the session started, which its budget took
    /// then: it isn't offered again. If the session is no longer shown, it
    /// is only freed.
    pub(super) fn held_input(&self, bytes: &[u8]) {
        let state = self.conn.state.lock();
        if state.shows(self.id, self.key.as_ref()) {
            self.queue_input(bytes);
        } else {
            self.budget.lock().handled(bytes.len());
        }
    }

    /// Queues taken input as `DATA` frames, each counted until the writer
    /// is done with it. Called under the state lock.
    fn queue_input(&self, bytes: &[u8]) {
        for chunk in bytes.chunks(INPUT_CHUNK) {
            let message = ToCore::Data(Data { session: self.id, bytes: chunk.to_vec() });
            self.conn.queue(Outgoing { message, backlog: Some((self.budget.clone(), chunk.len())) });
        }
    }

    /// How many bytes of the session's input wait to be written.
    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.budget.lock().waiting()
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        self.send_while_shown([ToCore::Resize(Resize { session: self.id, cols, rows, cell_w, cell_h })]);
    }

    /// What the core last reported of the session's foreground program,
    /// once it has, while the session is shown.
    pub fn foreground(&self) -> Option<Foreground> {
        self.conn.state.lock().shown(self.id, self.key.as_ref()).and_then(|shown| shown.foreground.clone())
    }

    /// Makes the window `hwnd` the owner of the session's console window
    /// (`REPARENT`, which only a Windows core takes), and keeps it to send
    /// again after each reconnect's replay. A session that exited or was
    /// lost sends nothing, and neither does a relay's: its session runs on
    /// another machine.
    pub fn reparent(&self, hwnd: u64) {
        if self.conn.is_relay() {
            return;
        }
        let mut state = self.conn.state.lock();
        let Some(shown) = state.shown(self.id, self.key.as_ref()) else { return };
        shown.parent = Some(hwnd);
        self.conn.send(ToCore::Reparent(Reparent { session: self.id, hwnd }));
    }

    /// Ends the session (`CLOSE`). Once is enough; a session that already
    /// exited or was lost isn't closed again.
    pub fn close(&self) {
        if self.conn.forget(self.id, self.key.as_ref(), true) {
            self.conn.send(ToCore::Close(SessionRef { session: self.id }));
        }
    }

    /// Makes `layout` the session's layout on the core (`SET_LAYOUT`), when it
    /// differs from the one last sent, the current (or last) link speaks
    /// client protocol 3 ([`Negotiated::sets_layout`]) and the session is
    /// shown; a core below 3 gets it only with the `DETACH` on quit.
    pub fn set_layout(&self, layout: Option<serde_json::Value>) {
        let state = self.conn.state.lock();
        if !state.link.protocol().is_some_and(Negotiated::sets_layout) || !state.shows(self.id, self.key.as_ref()) {
            return;
        }
        let mut sent = self.layout.lock();
        if sent.layout() == &layout {
            return;
        }
        match layout_to_send(layout.clone()) {
            Ok(to_send) => {
                *sent = LayoutSent::Sent(to_send.clone());
                self.conn.send(ToCore::SetLayout(SetLayout { session: self.id, layout: to_send }));
            }
            Err(too_large) => {
                // Logged once: only a different layout is tried again.
                log::warn!("not sending session {}'s layout: {too_large}", self.id);
                *sent = LayoutSent::TooLarge(layout);
            }
        }
    }

    /// Leaves the session running with no client (`DETACH`), with the
    /// layout of the tab it was in for whichever client attaches it next,
    /// unless it already exited or was lost.
    pub fn detach(&self, layout: Option<serde_json::Value>) {
        if self.conn.forget(self.id, self.key.as_ref(), false) {
            // A layout the core would drop leaves the session its own tab
            // next time, as with none; the session still leaves.
            let layout = layout_to_send(layout).unwrap_or_else(|too_large| {
                log::warn!("detaching session {} without its layout: {too_large}", self.id);
                None
            });
            self.conn.send(ToCore::Detach(Detach { session: self.id, layout }));
        }
    }
}

/// In-memory relays for tests: a [`Dial`] that hands out the links a test
/// queues, each to a fake relay and the core behind it.
#[cfg(test)]
pub(super) mod fake {
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::remote::protocol::client::{Hello, Spawned, Welcome};

    /// Timings short enough for tests.
    pub fn quick() -> Timings {
        let ms = Duration::from_millis;
        Timings {
            heartbeat: ms(50),
            dead: ms(400),
            bootstrap: Duration::from_secs(5),
            first_retry: ms(40),
            retry_cap: ms(160),
            eager_cap: ms(320),
            // Long: most tests leave a connection unused for a while and
            // then use it again. The idle tests shorten it.
            idle: Duration::from_secs(60),
            // Long, as `idle`: the close limit's tests shorten it.
            close_limit: Duration::from_secs(60),
        }
    }

    pub struct Dialer {
        name: String,
        links: Mutex<mpsc::Receiver<Result<Link, String>>>,
        dialed: mpsc::Sender<Instant>,
    }

    /// What a test hands the dialer, and when it was asked for a link.
    pub struct Plan {
        pub links: mpsc::Sender<Result<Link, String>>,
        pub dialed: mpsc::Receiver<Instant>,
    }

    impl Plan {
        pub fn fail(&self, message: &str) {
            self.links.send(Err(message.to_string())).unwrap();
        }

        pub fn link(&self, link: Link) {
            self.links.send(Ok(link)).unwrap();
        }

        /// When the next dial came, waiting up to 10 s for it.
        pub fn next_dial(&self) -> Instant {
            self.dialed.recv_timeout(Duration::from_secs(10)).expect("a dial")
        }
    }

    pub fn dialer(name: &str) -> (Arc<Dialer>, Plan) {
        let (links, queued) = mpsc::channel();
        let (dialing, dialed) = mpsc::channel();
        let dialer = Dialer { name: name.to_string(), links: Mutex::new(queued), dialed: dialing };
        (Arc::new(dialer), Plan { links, dialed })
    }

    impl Dial for Dialer {
        fn name(&self) -> String {
            self.name.clone()
        }

        fn launch(&self) -> Result<Link> {
            self.dialed.send(Instant::now()).ok();
            let next = self.links.lock().recv();
            match next {
                Ok(Ok(link)) => Ok(link),
                Ok(Err(message)) => Err(anyhow!(message)),
                // The test is over: never connect again.
                Err(_) => loop {
                    std::thread::park();
                },
            }
        }
    }

    /// The far end's writer, which the link's `end()` cuts.
    #[derive(Clone)]
    pub struct Out {
        writer: Arc<Mutex<Option<io::PipeWriter>>>,
        cut: Arc<AtomicBool>,
        /// What the link's [`Wire::why`] says, as a relay's process would.
        why: Arc<Mutex<String>>,
    }

    impl Out {
        pub fn send(&self, message: ToClient) {
            self.frame(&message.encode().unwrap());
        }

        pub fn relay(&self, message: RelayMessage) {
            self.frame(&message.encode().unwrap());
        }

        fn frame(&self, frame: &Frame) {
            if let Some(writer) = &mut *self.writer.lock() {
                frame.write_to(writer).ok();
            }
        }

        /// Cuts the link both ways, as a dropped network does to a relay's
        /// process: the client's read ends and its writes fail.
        pub fn cut(&self) {
            self.cut.store(true, Ordering::Release);
            self.writer.lock().take();
        }

        pub fn is_cut(&self) -> bool {
            self.cut.load(Ordering::Acquire)
        }

        /// Sets what the wire says of why the link ended, before it is cut.
        pub fn set_why(&self, why: &str) {
            *self.why.lock() = why.to_string();
        }
    }

    struct PipeWire {
        pipe: Mutex<io::PipeWriter>,
        far: Out,
    }

    impl Wire for PipeWire {
        fn send(&self, frame: &Frame) -> io::Result<()> {
            if self.far.is_cut() {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            frame.write_to(&mut *self.pipe.lock())
        }

        fn end(&self) {
            self.far.cut();
        }

        fn why(&self) -> Option<String> {
            Some(self.far.why.lock().clone()).filter(|why| !why.is_empty())
        }
    }

    /// What the far end of a link saw.
    pub struct Far {
        pub out: Out,
        /// The core's messages after `HELLO`, heartbeats aside.
        pub got: mpsc::Receiver<ToCore>,
        pub hello: mpsc::Receiver<Hello>,
        /// The client's heartbeats, which the relay drops.
        pub heartbeats: Arc<AtomicUsize>,
        /// The next `LIST` is a test's round trip: the far end answers it
        /// with no sessions itself, without the link's `answer`.
        pub settle: Arc<AtomicBool>,
    }

    impl Far {
        pub fn next(&self) -> ToCore {
            self.got.recv_timeout(Duration::from_secs(10)).expect("a message")
        }

        pub fn hello(&self) -> Hello {
            self.hello.recv_timeout(Duration::from_secs(10)).expect("HELLO")
        }
    }

    /// A link to a fake relay, which says `RELAY_HOST` with `env` at once,
    /// and to the core behind it, which welcomes the client's `HELLO`, then
    /// hands each message to `answer` and reports it.
    pub fn link(env: &[(&str, &str)], mut answer: impl FnMut(&ToCore, &Out) + Send + 'static) -> (Link, Far) {
        let (client_reader, far_writer) = io::pipe().unwrap();
        let (mut far_reader, client_writer) = io::pipe().unwrap();
        let out = Out { writer: Arc::new(Mutex::new(Some(far_writer))), cut: Arc::default(), why: Arc::default() };
        out.relay(relay_host(env));
        let (got_tx, got) = mpsc::channel();
        let (hello_tx, hello) = mpsc::channel();
        let heartbeats = Arc::new(AtomicUsize::new(0));
        let settle = Arc::new(AtomicBool::new(false));
        let (far, counting, settling) = (out.clone(), heartbeats.clone(), settle.clone());
        std::thread::spawn(move || {
            let mut welcomed = false;
            while let Ok(frame) = Frame::read_from(&mut far_reader) {
                if is_relay_type(frame.msg_type) {
                    assert_eq!(RelayMessage::decode(&frame).unwrap(), RelayMessage::Heartbeat);
                    counting.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let message = ToCore::decode(&frame).unwrap();
                if !welcomed {
                    let ToCore::Hello(hello) = message else { panic!("not HELLO: {message:?}") };
                    let welcome = Welcome {
                        protocol: 2,
                        core_version: "0.0.0".into(),
                        core_build: "core".into(),
                        pid: 4242,
                        holder_protocols: vec![2],
                    };
                    far.send(ToClient::Welcome(welcome));
                    welcomed = true;
                    hello_tx.send(hello).ok();
                    continue;
                }
                match &message {
                    ToCore::List(list) if settling.swap(false, Ordering::AcqRel) => {
                        far.send(ToClient::Sessions(client::Sessions { req: list.req, sessions: Vec::new() }));
                    }
                    _ => answer(&message, &far),
                }
                got_tx.send(message).ok();
            }
        });
        let wire = Arc::new(PipeWire { pipe: Mutex::new(client_writer), far: out.clone() });
        (Link { reader: Box::new(client_reader), wire }, Far { out, got, hello, heartbeats, settle })
    }

    /// The fake relay's `RELAY_HOST`, with `env`.
    fn relay_host(env: &[(&str, &str)]) -> RelayMessage {
        RelayMessage::Host(RelayHost {
            version: "0.0.0".into(),
            build: "relay".into(),
            env: env.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect(),
            shell: "/bin/bash".into(),
            home: "/home/u".into(),
            server_dir: "/run/user/1000/brindle".into(),
            core_pid: 4242,
        })
    }

    /// A link to a fake relay whose core refuses this Brindle: it answers
    /// `HELLO` with an `ERROR` saying `why`, as a core that shares no
    /// protocol version with it does.
    pub fn refusing(why: &str) -> Link {
        let (client_reader, far_writer) = io::pipe().unwrap();
        let (mut far_reader, client_writer) = io::pipe().unwrap();
        let out = Out { writer: Arc::new(Mutex::new(Some(far_writer))), cut: Arc::default(), why: Arc::default() };
        out.relay(relay_host(&[]));
        let (far, why) = (out.clone(), why.to_string());
        std::thread::spawn(move || {
            let frame = Frame::read_from(&mut far_reader).unwrap();
            let ToCore::Hello(_) = ToCore::decode(&frame).unwrap() else { panic!("not HELLO") };
            far.send(ToClient::Error(client::ErrorMessage { req: None, session: None, message: why }));
        });
        let wire = Arc::new(PipeWire { pipe: Mutex::new(client_writer), far: out });
        Link { reader: Box::new(client_reader), wire }
    }

    /// Answers each `SPAWN` with session `10 + req`.
    pub fn spawning(message: &ToCore, out: &Out) {
        if let ToCore::Spawn(spawn) = message {
            let session = 10 + spawn.req as SessionId;
            let spawned = Spawned { req: spawn.req, session, pid: 1000 + session, key: format!("k{session}") };
            out.send(ToClient::Spawned(spawned));
        }
    }

    /// What a terminal saw.
    pub struct Recorder(pub mpsc::Sender<String>);

    impl SessionSink for Recorder {
        fn output(&mut self, bytes: &[u8]) {
            self.0.send(format!("output {}", String::from_utf8_lossy(bytes))).ok();
        }
        fn note(&mut self, text: &str) {
            self.0.send(format!("note {text}")).ok();
        }
        fn replay(&mut self, active: bool) {
            self.0.send(format!("replay {active}")).ok();
        }
        fn exited(&mut self, code: Option<i32>) {
            self.0.send(format!("exited {code:?}")).ok();
        }
        fn lost(&mut self, reason: &str) {
            self.0.send(format!("lost {reason}")).ok();
        }
        fn failed(&mut self, error: &anyhow::Error) {
            self.0.send(format!("failed {error:#}")).ok();
        }
        fn replay_size(&mut self, cols: u16, rows: u16) {
            self.0.send(format!("size {cols}x{rows}")).ok();
        }
        fn broken(&mut self, reason: &str) {
            self.0.send(format!("broken {reason}")).ok();
        }
    }

    pub fn sink() -> (Sink, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel();
        (Arc::new(Mutex::new(Box::new(Recorder(tx)))), rx)
    }

    /// A terminal whose emulator fails on output holding its marker, as
    /// `BRINDLE_TEST_SINK_PANIC` makes a real one fail; it records the rest.
    pub struct Failing {
        seen: Recorder,
        marker: &'static str,
    }

    impl SessionSink for Failing {
        fn output(&mut self, bytes: &[u8]) {
            if String::from_utf8_lossy(bytes).contains(self.marker) {
                panic!("the emulator failed on {}", self.marker);
            }
            self.seen.output(bytes);
        }
        fn note(&mut self, text: &str) {
            self.seen.note(text);
        }
        fn replay(&mut self, active: bool) {
            self.seen.replay(active);
        }
        fn exited(&mut self, code: Option<i32>) {
            self.seen.exited(code);
        }
        fn lost(&mut self, reason: &str) {
            self.seen.lost(reason);
        }
        fn failed(&mut self, error: &anyhow::Error) {
            self.seen.failed(error);
        }
        fn broken(&mut self, reason: &str) {
            self.seen.broken(reason);
        }
    }

    /// A [`Failing`] sink, failing on output that holds `marker`.
    pub fn failing_sink(marker: &'static str) -> (Sink, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel();
        (Arc::new(Mutex::new(Box::new(Failing { seen: Recorder(tx), marker }))), rx)
    }
}

#[cfg(test)]
mod tests {
    use std::thread::JoinHandle;

    use super::fake::{self, Plan};
    use super::*;
    use crate::remote::budget::Refusal;
    use crate::remote::protocol::client::Spawned;
    use crate::remote::protocol::relay::RelayError;
    use crate::test_support::wait::{limit, until};

    const MIB: usize = 1 << 20;

    fn seen(rx: &mpsc::Receiver<String>) -> String {
        rx.recv_timeout(limit(Duration::from_secs(10))).expect("an event")
    }

    /// A round trip through `far`'s link, the positive marker for "nothing
    /// more happened": the client sends a `LIST`, which the far end answers
    /// itself once it has read everything the client sent before, and the
    /// client takes the answer only after everything the far end sent
    /// before it. So once this returns, the tabs have been told all the far
    /// end caused, and `far` has had every message the client sent before:
    /// those are returned, the `LIST` left out.
    fn settled(connection: &Connection, far: &fake::Far) -> Vec<ToCore> {
        far.settle.store(true, Ordering::Release);
        assert!(connection.list().unwrap().is_empty(), "the round trip's LIST");
        let mut before = Vec::new();
        loop {
            match far.next() {
                ToCore::List(_) => return before,
                other => before.push(other),
            }
        }
    }

    /// The tab has been told nothing it hasn't been asked about. Only after
    /// a positive marker ([`settled`], or a later event in order).
    fn told_nothing_more(rx: &mpsc::Receiver<String>) {
        if let Ok(event) = rx.try_recv() {
            panic!("unexpected: {event:?}");
        }
    }

    /// Waits until every thread of the connection's own has returned, as a
    /// dead connection's do: only the test's `handles` hold it then. Nothing
    /// it does (a dial, a report) is still to come after that.
    fn threads_ended(connection: &Arc<Connection>, handles: usize) {
        until("the connection's threads end", Duration::from_secs(10), || {
            (Arc::strong_count(connection) == handles).then_some(())
        });
    }

    fn relay(name: &str) -> (Arc<Connection>, Plan) {
        let (dial, plan) = fake::dialer(name);
        (Connection::relay(dial, "tok", fake::quick()).unwrap(), plan)
    }

    fn spawn_request(env: &[(&str, &str)]) -> SpawnRequest {
        SpawnRequest {
            command: None,
            args: Vec::new(),
            cwd: Some("~/src".into()),
            env: env.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect(),
            cols: 80,
            rows: 24,
            cell_w: 8,
            cell_h: 16,
            scrollback: 1000,
            policy: Policy::Detach,
            profile: Some("A8".into()),
        }
    }

    /// A session just listed, with the key a core gives it (`k{id}`, as
    /// [`fake::spawning`]'s are).
    fn attach_request(session: SessionId) -> AttachRequest {
        AttachRequest {
            session,
            key: SessionKey::from_wire(&format!("k{session}")),
            when: Attaching::Now { cols: 100, rows: 30 },
        }
    }

    /// A session a restored zone recorded, whose size it doesn't know.
    fn recorded(session: SessionId) -> AttachRequest {
        AttachRequest { when: Attaching::Recorded, ..attach_request(session) }
    }

    /// Waits until `count` requests wait in the connection.
    fn waiting(connection: &Connection, count: usize) {
        eventually(&format!("{count} requests wait"), || connection.state.lock().requests.len() == count);
    }

    /// Waits, within a scaled 10 s, until `done` holds.
    fn eventually(what: &str, mut done: impl FnMut() -> bool) {
        until(what, Duration::from_secs(10), || done().then_some(()));
    }

    /// Waits until the idle thread has taken its decision at or after
    /// `idle` past `since`, when the connection became idle-eligible: had
    /// it ended the connection, it would have done so by then.
    fn looked_past(connection: &Connection, since: Instant, idle: Duration) {
        eventually("a look past the idle time", || {
            connection.state.lock().last_look.is_some_and(|look| look >= since + idle)
        });
    }

    fn in_background<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> JoinHandle<T> {
        std::thread::spawn(run)
    }

    /// Answers `ATTACH` with a replay, as a core does.
    fn replaying(message: &ToCore, out: &fake::Out) {
        if let ToCore::Attach(attach) = message {
            let session = attach.session;
            out.send(ToClient::ReplayStart(ReplayStart { req: Some(attach.req), session, pid: 99 }));
            out.send(ToClient::Data(Data { session, bytes: b"screen".to_vec() }));
            out.send(ToClient::ReplayEnd(SessionRef { session }));
        }
    }

    /// Answers `LIST` with no sessions.
    fn listing(message: &ToCore, out: &fake::Out) {
        if let ToCore::List(list) = message {
            out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions: Vec::new() }));
        }
    }

    #[test]
    fn a_relay_spawn_carries_the_endpoints_environment_under_its_own() {
        let spawn = ToCore::Spawn(client::Spawn {
            req: 3,
            command: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::from([("TERM".into(), "xterm-256color".into()), ("FOO".into(), "1".into())]),
            cols: 80,
            rows: 24,
            cell_w: 8,
            cell_h: 16,
            scrollback: 0,
            policy: Policy::Detach,
            profile: None,
        });
        let decoded = |frame: Frame| match ToCore::decode(&frame).unwrap() {
            ToCore::Spawn(spawn) => spawn,
            other => panic!("not SPAWN: {other:?}"),
        };
        // The local core gets the request's environment as it is.
        assert_eq!(decoded(encode(&spawn, None).unwrap()).env.len(), 2);
        let host = RelayHost {
            env: BTreeMap::from([("LANG".into(), "C.UTF-8".into()), ("TERM".into(), "dumb".into())]),
            ..Default::default()
        };
        let remote = decoded(encode(&spawn, Some(&host)).unwrap());
        let env: Vec<(&str, &str)> = remote.env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(env, [("FOO", "1"), ("LANG", "C.UTF-8"), ("TERM", "xterm-256color")]);
        assert_eq!((remote.req, remote.command), (3, None));
        // Other messages go as they are.
        let list = ToCore::List(Request { req: 4 });
        assert_eq!(encode(&list, Some(&host)).unwrap(), list.encode().unwrap());
    }

    /// A request's `SPAWN` on the wire: no command is `null`, which the
    /// core takes as the login shell, and a command goes as it is.
    #[test]
    fn a_spawn_request_encodes_its_command_as_the_wire_has_it() {
        let encoded = |command: Option<&str>| {
            let request = SpawnRequest { command: command.map(Into::into), ..spawn_request(&[("FOO", "1")]) };
            let frame = ToCore::Spawn(request.into_spawn(3)).encode().unwrap();
            assert_eq!(frame.msg_type, client::MSG_SPAWN);
            String::from_utf8(frame.payload).unwrap()
        };
        let rest = r#""args":[],"cwd":"~/src","env":{"FOO":"1"},"cols":80,"rows":24,"cell_w":8,"cell_h":16,"scrollback":1000,"policy":"detach","profile":"A8"}"#;
        assert_eq!(encoded(None), format!(r#"{{"req":3,"command":null,{rest}"#));
        assert_eq!(encoded(Some("htop")), format!(r#"{{"req":3,"command":"htop",{rest}"#));
    }

    /// 5.4: `HELLO` asks for linger and process info, and a `SPAWN` with
    /// no command carries the endpoint's environment with the request's.
    #[test]
    fn a_relay_says_hello_with_linger_and_spawns_in_the_endpoints_environment() {
        let (connection, plan) = relay("A8");
        let (link, far) = fake::link(&[("LANG", "en_GB.UTF-8"), ("TERM", "dumb")], fake::spawning);
        plan.link(link);
        let (sink, _tab) = fake::sink();
        let request = spawn_request(&[("TERM", "xterm-256color"), ("FOO", "1")]);
        let running = connection.spawn(request, sink, None).unwrap();
        assert_eq!((running.id(), running.pid(), running.key()), (11, 1011, Some(&key("k11"))));
        let hello = far.hello();
        assert_eq!((hello.linger_secs, hello.process_info, hello.client_token.as_str()), (300, true, "tok"));
        let ToCore::Spawn(spawn) = far.next() else { panic!("not SPAWN") };
        assert_eq!(spawn.command, None);
        assert_eq!(spawn.cwd.as_deref(), Some("~/src"));
        let env: Vec<(&str, &str)> = spawn.env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(env, [("FOO", "1"), ("LANG", "en_GB.UTF-8"), ("TERM", "xterm-256color")]);
    }

    /// A link that reads nothing for the dead-link limit is ended (its
    /// process killed) and replaced; the client's own heartbeats don't
    /// count, and the relay drops them.
    #[test]
    fn a_silent_relay_link_is_ended_and_replaced() {
        let timings = fake::quick();
        let (connection, plan) = relay("A8");
        let (silent, far) = fake::link(&[], |_, _| {});
        // The link's clock starts once it is up, which is after this.
        let linked = Instant::now();
        plan.link(silent);
        plan.next_dial();
        far.hello();
        let second = plan.next_dial();
        assert!(second - linked >= timings.dead, "{:?}", second - linked);
        assert!(far.out.is_cut(), "the dead link was ended");
        // The client beat at least once before the link was declared dead,
        // which its timings leave room for.
        assert!(timings.dead >= timings.heartbeat * 2);
        assert!(far.heartbeats.load(Ordering::Relaxed) >= 1, "{:?}", far.heartbeats);
        // Nothing but heartbeats went to the silent core.
        assert!(far.got.try_recv().is_err());
        // The next link is a reconnect: it asks which sessions are left.
        let (link, next) = fake::link(&[], |_, _| {});
        plan.link(link);
        assert!(matches!(next.next(), ToCore::List(_)));
        assert!(!connection.is_dead());
    }

    #[test]
    fn heartbeats_from_the_relay_keep_a_quiet_link() {
        let (_connection, plan) = relay("A8");
        let (link, far) = fake::link(&[], |_, _| {});
        plan.link(link);
        plan.next_dial();
        far.hello();
        // The relay beats every 100 ms until it is told to stop: a stimulus,
        // paced by the stop channel's wait.
        let (stop, stopped) = mpsc::channel::<()>();
        {
            let out = far.out.clone();
            std::thread::spawn(move || {
                while let Err(mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(Duration::from_millis(100)) {
                    out.relay(RelayMessage::Heartbeat);
                }
            });
        }
        // Three dead-link limits with only heartbeats: still the one link.
        assert!(plan.dialed.recv_timeout(fake::quick().dead * 3).is_err(), "redialed");
        assert!(!far.out.is_cut());
        stop.send(()).unwrap();
        plan.next_dial();
        assert!(far.out.is_cut());
    }

    /// Failed attempts pause 1x, 1x, 2x, 4x the first pause, up to the
    /// cap, while an attach waits; it is told the error once and attached
    /// when the endpoint comes back.
    #[test]
    fn relay_attempts_back_off_while_an_attach_waits() {
        let timings = fake::quick();
        let (connection, plan) = relay("A8");
        let (sink, tab) = fake::sink();
        let attaching = connection.clone();
        let attach = in_background(move || attaching.attach(attach_request(5), sink, None).map(|r| r.pid()));
        waiting(&connection, 1);
        let mut dials = vec![plan.next_dial()];
        for _ in 0..4 {
            plan.fail("ssh: connect to host A8 port 22: Connection refused");
            dials.push(plan.next_dial());
        }
        let gaps: Vec<Duration> = dials.windows(2).map(|pair| pair[1] - pair[0]).collect();
        for (failures, gap) in (1..).zip(&gaps) {
            let pause = relay::backoff(failures, timings.first_retry, timings.retry_cap);
            assert!(*gap >= pause, "gap {failures} {gap:?} < {pause:?} ({gaps:?})");
        }
        assert_eq!(
            seen(&tab),
            "output \r\n\x1b[2mcan't connect to A8: ssh: connect to host A8 port 22: Connection refused; retrying…\x1b[22m\r\n"
        );
        let (link, far) = fake::link(&[], replaying);
        plan.link(link);
        assert_eq!(attach.join().unwrap().unwrap(), 99);
        assert!(matches!(far.next(), ToCore::Attach(attach) if attach.session == 5));
        assert_eq!(seen(&tab), "replay true");
        assert_eq!(seen(&tab), "output screen");
        assert_eq!(seen(&tab), "replay false");
        settled(&connection, &far);
        told_nothing_more(&tab);
    }

    /// Requests made while the relay connects wait for it, with no limit,
    /// and go out once it is up.
    #[test]
    fn requests_wait_for_the_relay_and_go_out_once_it_is_up() {
        let (connection, plan) = relay("A8");
        plan.next_dial();
        let (sink, _tab) = fake::sink();
        let (spawning, asking) = (connection.clone(), connection.clone());
        let spawn = in_background(move || spawning.spawn(spawn_request(&[]), sink, None).map(|r| r.id()));
        waiting(&connection, 1);
        let list = in_background(move || asking.list().map(|sessions| sessions.len()));
        waiting(&connection, 2);
        // No link to finish on yet; their success once one comes shows they
        // waited for it.
        assert!(!spawn.is_finished() && !list.is_finished());
        let (link, far) = fake::link(&[], |message, out| {
            fake::spawning(message, out);
            listing(message, out);
        });
        plan.link(link);
        assert_eq!(spawn.join().unwrap().unwrap(), 11);
        assert_eq!(list.join().unwrap().unwrap(), 0);
        assert!(matches!(far.next(), ToCore::Spawn(_)));
        assert!(matches!(far.next(), ToCore::List(_)));
    }

    /// A failed first connect fails a new tab's spawn with its error, while
    /// an attach (a restored zone) keeps waiting through the retries. A new
    /// tab's spawn then tries at once, not after the pause.
    #[test]
    fn a_failed_first_connect_fails_a_spawn_and_keeps_an_attach() {
        let (dial, plan) = fake::dialer("dev@A8:2222");
        let timings = Timings { first_retry: Duration::from_secs(30), retry_cap: Duration::from_secs(30), ..fake::quick() };
        let connection = Connection::relay(dial, "tok", timings).unwrap();
        let (spawn_sink, spawn_tab) = fake::sink();
        let (attach_sink, attach_tab) = fake::sink();
        let (spawning, attaching) = (connection.clone(), connection.clone());
        let attach = in_background(move || attaching.attach(attach_request(5), attach_sink, None).map(|r| r.pid()));
        waiting(&connection, 1);
        let spawn = in_background(move || spawning.spawn(spawn_request(&[]), spawn_sink, None).map(|r| r.id()));
        waiting(&connection, 2);
        plan.next_dial();
        plan.fail("ssh: Could not resolve hostname A8");
        let error = spawn.join().unwrap().unwrap_err().to_string();
        assert_eq!(error, "can't connect to dev@A8:2222: ssh: Could not resolve hostname A8");
        assert!(seen(&attach_tab).contains("can't connect to dev@A8:2222"));
        // The next attempt would be 30 s away; a new tab's spawn asks for
        // one now. It fails too, quietly for the attach, which waits on.
        let (sink, _tab) = fake::sink();
        let spawning = connection.clone();
        let asked = Instant::now();
        let spawn = in_background(move || spawning.spawn(spawn_request(&[]), sink, None).map(|r| r.id()));
        plan.next_dial();
        assert!(asked.elapsed() < Duration::from_secs(10));
        plan.fail("ssh: Could not resolve hostname A8");
        assert!(spawn.join().unwrap().is_err());
        assert!(!attach.is_finished());
        // Once the endpoint answers, the attach and a new spawn go out.
        let (sink, _tab) = fake::sink();
        let spawning = connection.clone();
        let spawn = in_background(move || spawning.spawn(spawn_request(&[]), sink, None).map(|r| r.id()));
        plan.next_dial();
        let (link, far) = fake::link(&[], |message, out| {
            fake::spawning(message, out);
            replaying(message, out);
        });
        plan.link(link);
        assert_eq!(attach.join().unwrap().unwrap(), 99);
        assert!(spawn.join().unwrap().is_ok());
        // The attach's next event is its replay: the second failure told it
        // nothing.
        assert_eq!(seen(&attach_tab), "replay true");
        settled(&connection, &far);
        told_nothing_more(&spawn_tab);
    }

    /// With nothing waiting, a failed first connect ends the connection,
    /// and later requests get its error; an eager one keeps trying.
    #[test]
    fn a_failed_first_connect_ends_the_connection_unless_it_is_eager() {
        let (connection, plan) = relay("A8");
        plan.next_dial();
        plan.fail("docker: no such container: devbox");
        eventually("the connection ends", || connection.is_dead());
        let error = connection.list().unwrap_err().to_string();
        assert_eq!(error, "can't connect to A8: docker: no such container: devbox");
        threads_ended(&connection, 1);
        assert!(plan.dialed.try_recv().is_err(), "no retry");

        let (dial, plan) = fake::dialer("A8");
        let (eager, _queue) = Connection::new(Transport::Relay { dial, timings: fake::quick() }, "tok");
        eager.set_eager(true);
        let reading = eager.clone();
        std::thread::spawn(move || reading.run_relay());
        let first = plan.next_dial();
        plan.fail("docker: no such container: devbox");
        let second = plan.next_dial();
        assert!(!eager.is_dead());
        assert!(second - first >= fake::quick().first_retry);
    }

    /// 5.5: a lost relay link says so once in each terminal, input typed
    /// meanwhile goes out after the reconnect, and only the session the
    /// reconnected core's `LIST` lacks is lost.
    #[test]
    fn a_lost_relay_link_says_so_and_only_unlisted_sessions_are_lost() {
        let (connection, plan) = relay("A8");
        let (link, first) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (kept_sink, kept_tab) = fake::sink();
        let (gone_sink, gone_tab) = fake::sink();
        let kept = connection.spawn(spawn_request(&[]), kept_sink, None).unwrap();
        let gone = connection.spawn(spawn_request(&[]), gone_sink, None).unwrap();
        assert_eq!((kept.id(), gone.id()), (11, 12));
        first.next();
        first.next();

        first.out.cut();
        let line = "output \r\n\x1b[2mconnection to A8 lost (the relay closed the connection); reconnecting…\x1b[22m\r\n";
        assert_eq!(seen(&kept_tab), line);
        assert_eq!(seen(&gone_tab), line);
        kept.input(b"ls\r");

        let kept_id = kept.id();
        let (link, second) = fake::link(&[], move |message, out| {
            if let ToCore::List(list) = message {
                // The core replays what it still has before it answers.
                out.send(ToClient::ReplayStart(ReplayStart { req: None, session: kept_id, pid: 1011 }));
                out.send(ToClient::Data(Data { session: kept_id, bytes: b"screen".to_vec() }));
                out.send(ToClient::ReplayEnd(SessionRef { session: kept_id }));
                let info =
                    SessionInfo { session: kept_id, key: format!("k{kept_id}"), attached: true, ..Default::default() };
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions: vec![info] }));
            }
        });
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        assert!(matches!(second.next(), ToCore::Data(data) if data.session == kept_id && data.bytes == b"ls\r"));
        assert_eq!(seen(&kept_tab), "replay true");
        assert_eq!(seen(&kept_tab), "output screen");
        assert_eq!(seen(&kept_tab), "replay false");
        assert_eq!(seen(&gone_tab), "lost the terminal server no longer has this session");
        settled(&connection, &second);
        told_nothing_more(&kept_tab);
        told_nothing_more(&gone_tab);
        assert!(!connection.is_dead());
    }

    /// A relay connection whose links don't die of silence while a test
    /// checks that nothing more happens.
    fn steady_relay(name: &str) -> (Arc<Connection>, Plan) {
        let (dial, plan) = fake::dialer(name);
        let timings = Timings { dead: Duration::from_secs(30), ..fake::quick() };
        (Connection::relay(dial, "tok", timings).unwrap(), plan)
    }

    /// The core's replay of `session`: one with a `req` answers an
    /// `ATTACH`, one without is a reconnect's.
    fn replay(out: &fake::Out, session: SessionId, req: Option<RequestId>) {
        out.send(ToClient::ReplayStart(ReplayStart { req, session, pid: 1000 + session }));
        out.send(ToClient::Data(Data { session, bytes: b"screen".to_vec() }));
        out.send(ToClient::ReplayEnd(SessionRef { session }));
    }

    /// Attaches that went out just before their link dropped stay pending,
    /// and the reconnect's replays answer them: one the core sends before
    /// it answers `LIST`, and one it sends after (a new core rebuilding the
    /// session's screen first).
    #[test]
    fn attaches_cut_off_with_their_link_are_answered_by_the_reconnects_replays() {
        let (connection, plan) = steady_relay("A8");
        let (link, first) = fake::link(&[], |_, _| {});
        plan.link(link);
        let (seven_sink, seven_tab) = fake::sink();
        let (eight_sink, eight_tab) = fake::sink();
        let (seven, eight) = (connection.clone(), connection.clone());
        let seven = in_background(move || seven.attach(attach_request(7), seven_sink, None).map(|r| (r.id(), r.pid())));
        let eight = in_background(move || eight.attach(attach_request(8), eight_sink, None).map(|r| (r.id(), r.pid())));
        let mut sent: Vec<SessionId> = (0..2)
            .map(|_| match first.next() {
                ToCore::Attach(attach) => attach.session,
                other => panic!("not ATTACH: {other:?}"),
            })
            .collect();
        sent.sort_unstable();
        assert_eq!(sent, [7, 8]);
        first.out.cut();

        let (link, second) = fake::link(&[], |message, out| {
            if let ToCore::List(list) = message {
                replay(out, 7, None);
                let attached = |session: SessionId| SessionInfo {
                    session,
                    key: format!("k{session}"),
                    attached: true,
                    ..Default::default()
                };
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions: vec![attached(7), attached(8)] }));
                replay(out, 8, None);
            }
        });
        plan.link(link);
        assert_eq!(seven.join().unwrap().unwrap(), (7, 1007));
        assert_eq!(eight.join().unwrap().unwrap(), (8, 1008));
        for tab in [&seven_tab, &eight_tab] {
            assert_eq!(seen(tab), "replay true");
            assert_eq!(seen(tab), "output screen");
            assert_eq!(seen(tab), "replay false");
        }
        // Neither is asked for again, and nothing is detached or closed.
        assert!(matches!(second.next(), ToCore::List(_)));
        assert_eq!(settled(&connection, &second), []);
        told_nothing_more(&seven_tab);
        told_nothing_more(&eight_tab);
    }

    /// An attach cut off with its link whose session the reconnected core
    /// lists without a client (it never got the `ATTACH`, or a new core
    /// adopted the session under its old owner) is sent again after
    /// `WELCOME`. One whose session isn't listed fails.
    #[test]
    fn a_cut_off_attach_the_core_didnt_give_back_is_sent_again() {
        let (connection, plan) = steady_relay("A8");
        let (link, first) = fake::link(&[], |_, _| {});
        plan.link(link);
        let (sink, tab) = fake::sink();
        let (gone_sink, gone_tab) = fake::sink();
        let (attaching, losing) = (connection.clone(), connection.clone());
        let attach = in_background(move || attaching.attach(attach_request(7), sink, None).map(|r| (r.id(), r.pid())));
        let ToCore::Attach(sent) = first.next() else { panic!("not ATTACH") };
        let gone = in_background(move || losing.attach(attach_request(9), gone_sink, None).map(|r| r.pid()));
        assert!(matches!(first.next(), ToCore::Attach(attach) if attach.session == 9));
        first.out.cut();

        let (link, second) = fake::link(&[], |message, out| match message {
            ToCore::List(list) => {
                let detached = SessionInfo { session: 7, key: "k7".into(), owned: true, ..Default::default() };
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions: vec![detached] }));
            }
            other => replaying(other, out),
        });
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        assert_eq!(second.next(), ToCore::Attach(sent));
        assert_eq!(attach.join().unwrap().unwrap(), (7, 99));
        assert_eq!(gone.join().unwrap().unwrap_err().to_string(), "the terminal server no longer has session 9");
        assert_eq!(seen(&tab), "replay true");
        assert_eq!(seen(&tab), "output screen");
        assert_eq!(seen(&tab), "replay false");
        assert_eq!(settled(&connection, &second), []);
        told_nothing_more(&tab);
        told_nothing_more(&gone_tab);
    }

    /// A fresh core (no holder survived) restarts ids at 1, so a reconnect
    /// can list a shown session's id for another session, with another key
    /// and a client (another Brindle's). That tab is lost, and its handle
    /// reaches nothing; the session listed with its own key keeps its
    /// replay and its input.
    #[test]
    fn a_session_whose_id_the_reconnected_core_gave_another_is_lost() {
        let (connection, plan) = steady_relay("A8");
        let (link, first) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (reused_sink, reused_tab) = fake::sink();
        let (kept_sink, kept_tab) = fake::sink();
        let reused = connection.spawn(spawn_request(&[]), reused_sink, None).unwrap();
        let kept = connection.spawn(spawn_request(&[]), kept_sink, None).unwrap();
        first.next();
        first.next();
        first.out.cut();
        assert!(seen(&reused_tab).contains("connection to A8 lost"));
        assert!(seen(&kept_tab).contains("connection to A8 lost"));

        let (reused_id, kept_id) = (reused.id(), kept.id());
        let (link, second) = fake::link(&[], move |message, out| {
            if let ToCore::List(list) = message {
                replay(out, kept_id, None);
                let sessions = vec![
                    SessionInfo { session: reused_id, key: "another".into(), attached: true, ..Default::default() },
                    SessionInfo { session: kept_id, key: format!("k{kept_id}"), attached: true, ..Default::default() },
                ];
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions }));
            }
        });
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        assert_eq!(seen(&kept_tab), "replay true");
        assert_eq!(seen(&kept_tab), "output screen");
        assert_eq!(seen(&kept_tab), "replay false");
        assert_eq!(seen(&reused_tab), "lost the terminal server no longer has this session");
        assert_eq!(settled(&connection, &second), []);
        told_nothing_more(&reused_tab);
        told_nothing_more(&kept_tab);
        // The lost tab's input never goes out: the next message is the
        // kept one's.
        reused.input(b"lost\r");
        kept.input(b"kept\r");
        assert!(matches!(second.next(), ToCore::Data(data) if data.session == kept_id && data.bytes == b"kept\r"));
    }

    /// An attach cut off with its link, whose id the reconnected core lists
    /// for another session (another key, no client), is never sent again:
    /// it fails as one whose session isn't listed does, and its tab shows
    /// nothing of the other session.
    #[test]
    fn a_cut_off_attach_whose_id_has_another_key_fails_and_is_not_sent_again() {
        let (connection, plan) = steady_relay("A8");
        let (link, first) = fake::link(&[], |_, _| {});
        plan.link(link);
        let (sink, tab) = fake::sink();
        let attaching = connection.clone();
        let attach = in_background(move || attaching.attach(attach_request(7), sink, None).map(|r| r.pid()));
        assert!(matches!(first.next(), ToCore::Attach(attach) if attach.session == 7));
        first.out.cut();

        let (link, second) = fake::link(&[], |message, out| match message {
            ToCore::List(list) => {
                let other = SessionInfo { session: 7, key: "another".into(), owned: true, ..Default::default() };
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions: vec![other] }));
            }
            other => replaying(other, out),
        });
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        assert_eq!(attach.join().unwrap().unwrap_err().to_string(), "the terminal server no longer has session 7");
        assert_eq!(settled(&connection, &second), [], "the ATTACH went out again");
        told_nothing_more(&tab);
        assert!(connection.state.lock().sessions.is_empty());
    }

    /// The handle of a session lost after a reconnect, whose id a fresh core
    /// then gave a new tab's session, acts on neither: its input, resize,
    /// close and detach write nothing and leave the new session shown, whose
    /// own close still goes out.
    #[test]
    fn a_lost_session_handle_never_reaches_the_session_that_reused_its_id() {
        let (connection, plan) = steady_relay("A8");
        let (link, first) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (lost_sink, lost_tab) = fake::sink();
        let lost = connection.spawn(spawn_request(&[]), lost_sink, None).unwrap();
        assert!(matches!(first.next(), ToCore::Spawn(_)));
        first.out.cut();
        assert!(seen(&lost_tab).contains("connection to A8 lost"));

        // A fresh core: it has no session, and gives the next one the lost
        // one's id under another key.
        let id = lost.id();
        let (link, second) = fake::link(&[], move |message, out| match message {
            ToCore::List(list) => {
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions: Vec::new() }));
            }
            ToCore::Spawn(spawn) => {
                out.send(ToClient::Spawned(Spawned { req: spawn.req, session: id, pid: 2000, key: "fresh".into() }));
            }
            _ => {}
        });
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        assert_eq!(seen(&lost_tab), "lost the terminal server no longer has this session");
        let (fresh_sink, fresh_tab) = fake::sink();
        let fresh = connection.spawn(spawn_request(&[]), fresh_sink, None).unwrap();
        assert!(matches!(second.next(), ToCore::Spawn(_)));
        assert_eq!((fresh.id(), fresh.key()), (id, Some(&key("fresh"))));

        lost.input(b"leaked\r");
        lost.resize(100, 30, 8, 16);
        lost.close();
        lost.detach(None);
        assert!(connection.state.lock().shows(id, fresh.key()), "the new session isn't shown");
        // The next message written is the new session's own CLOSE.
        fresh.close();
        assert_eq!(second.next(), ToCore::Close(SessionRef { session: id }));
        assert_eq!(settled(&connection, &second), []);
        told_nothing_more(&lost_tab);
        told_nothing_more(&fresh_tab);
    }

    /// A server `ERROR` about a shown session, with no request, reaches its
    /// tab as a notice, once.
    #[test]
    fn a_session_error_is_noticed_once_in_its_tab() {
        let (connection, plan) = steady_relay("A8");
        let (link, far) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (sink, tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        assert!(matches!(far.next(), ToCore::Spawn(_)));

        let error = |session, message: &str| {
            ToClient::Error(client::ErrorMessage { req: None, session: Some(session), message: message.into() })
        };
        far.out.send(error(running.id(), "its holder failed on this input"));
        assert_eq!(seen(&tab), "note the terminal server: its holder failed on this input");
        assert_eq!(settled(&connection, &far), []);
        told_nothing_more(&tab);
    }

    /// An `ERROR` about a session this client doesn't show goes to no tab.
    #[test]
    fn an_error_for_a_session_not_shown_is_not_delivered() {
        let (connection, plan) = steady_relay("A8");
        let (link, far) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (sink, tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        assert!(matches!(far.next(), ToCore::Spawn(_)));

        far.out.send(ToClient::Error(client::ErrorMessage {
            req: None,
            session: Some(running.id() + 1),
            message: "another session's failure".into(),
        }));
        assert_eq!(settled(&connection, &far), []);
        told_nothing_more(&tab);
    }

    /// A layout over `MAX_LAYOUT` isn't sent with `SET_LAYOUT` (once, however
    /// often it is recorded), and a `DETACH` leaves without it.
    #[test]
    fn a_too_large_layout_is_not_sent_and_detach_still_goes() {
        let (connection, plan) = steady_relay("A8");
        let (link, far) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (sink, _tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        assert!(matches!(far.next(), ToCore::Spawn(_)));
        connection.state.lock().link = LinkPhase::Up(Negotiated(3));

        let big = serde_json::json!({ "tab": "x".repeat(client::MAX_LAYOUT + 1) });
        let small = serde_json::json!({ "tab": "x" });
        running.set_layout(Some(big.clone()));
        running.set_layout(Some(big.clone()));
        assert_eq!(settled(&connection, &far), []);
        // A different one is tried again.
        running.set_layout(Some(small.clone()));
        assert_eq!(far.next(), ToCore::SetLayout(SetLayout { session: running.id(), layout: Some(small) }));
        running.set_layout(Some(big.clone()));
        assert_eq!(settled(&connection, &far), []);

        running.detach(Some(big));
        assert_eq!(far.next(), ToCore::Detach(Detach { session: running.id(), layout: None }));
    }

    /// A relay's loss says why in each tab: the process's own account.
    #[test]
    fn a_remote_loss_names_its_cause() {
        let (connection, plan) = relay("A8");
        let (link, first) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (sink, tab) = fake::sink();
        connection.spawn(spawn_request(&[]), sink, None).unwrap();
        first.next();

        first.out.set_why("ssh exited with 255: connect to host a8 port 22: Connection timed out");
        first.out.cut();
        assert_eq!(
            seen(&tab),
            "output \r\n\x1b[2mconnection to A8 lost (ssh exited with 255: connect to host a8 port 22: Connection timed out); reconnecting…\x1b[22m\r\n"
        );
    }

    /// What the notice line and the lost line say, as plain functions.
    #[test]
    fn the_last_error_is_named() {
        assert_eq!(
            reconnect_gave_up(Duration::from_secs(15), Some("starting the terminal server: no such file")),
            "no terminal server came back within 15 s: starting the terminal server: no such file"
        );
    }

    #[test]
    fn only_timeouts_keep_todays_text() {
        assert_eq!(reconnect_gave_up(Duration::from_secs(15), None), "no terminal server came back within 15 s");
    }

    #[test]
    fn the_lost_line_carries_the_first_line_of_the_reason() {
        assert_eq!(
            lost_line("A8", "ssh exited with 255: boom\nsecond line"),
            "\r\n\x1b[2mconnection to A8 lost (ssh exited with 255: boom); reconnecting…\x1b[22m\r\n"
        );
        // Controls in what a host printed can't escape the dim line.
        assert_eq!(
            lost_line("A8", "boom\x1b]0;x\x07"),
            "\r\n\x1b[2mconnection to A8 lost (boom]0;x); reconnecting…\x1b[22m\r\n"
        );
        assert_eq!(lost_line("A8", ""), "\r\n\x1b[2mconnection to A8 lost; reconnecting…\x1b[22m\r\n");
    }

    #[test]
    fn a_layout_over_max_layout_is_too_large() {
        let small = serde_json::json!({ "tab": "x" });
        assert_eq!(layout_to_send(None), Ok(None));
        assert_eq!(layout_to_send(Some(small.clone())), Ok(Some(small)));
        let big = serde_json::json!({ "tab": "x".repeat(client::MAX_LAYOUT) });
        let size = serde_json::to_vec(&big).unwrap().len();
        assert!(size > client::MAX_LAYOUT);
        assert_eq!(layout_to_send(Some(big)), Err(LayoutTooLarge { size }));
    }

    /// What [`fake::failing_sink`]s fail on, as `BRINDLE_TEST_SINK_PANIC`
    /// would name it.
    const MARKER: &str = "brindle-test-sink-panic";

    /// Two sessions on a steady relay, the first one's sink failing on
    /// [`MARKER`], and the core's `DATA` for it carrying the marker: its
    /// tab has been told it is broken and the session detached.
    #[allow(clippy::type_complexity)]
    fn broken_by_data() -> (Arc<Connection>, Plan, fake::Far, Running, mpsc::Receiver<String>, Running, mpsc::Receiver<String>)
    {
        let (connection, plan) = steady_relay("A8");
        let (link, far) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (failing_sink, failing_tab) = fake::failing_sink(MARKER);
        let (fine_sink, fine_tab) = fake::sink();
        let failing = connection.spawn(spawn_request(&[]), failing_sink, None).unwrap();
        let fine = connection.spawn(spawn_request(&[]), fine_sink, None).unwrap();
        assert!(matches!(far.next(), ToCore::Spawn(_)));
        assert!(matches!(far.next(), ToCore::Spawn(_)));

        let bytes = format!("before {MARKER} after").into_bytes();
        far.out.send(ToClient::Data(Data { session: failing.id(), bytes }));
        assert_eq!(seen(&failing_tab), format!("broken the emulator failed on {MARKER}"));
        assert_eq!(far.next(), ToCore::Detach(Detach { session: failing.id(), layout: None }));
        (connection, plan, far, failing, failing_tab, fine, fine_tab)
    }

    /// 4.1: a sink that panics on `DATA` is told it is broken and its
    /// session detached: the program runs on, only Brindle's view of it
    /// failed. The reader carries on, so the endpoint's other session still
    /// gets its output, and the broken one gets nothing more.
    #[test]
    fn a_sink_that_panics_is_told_broken_and_its_session_detached() {
        let (connection, _plan, far, failing, failing_tab, fine, fine_tab) = broken_by_data();
        assert!(!connection.state.lock().shows(failing.id(), failing.key()));
        assert!(connection.state.lock().shows(fine.id(), fine.key()));

        far.out.send(ToClient::Data(Data { session: failing.id(), bytes: b"more".to_vec() }));
        far.out.send(ToClient::Data(Data { session: fine.id(), bytes: b"later".to_vec() }));
        assert_eq!(seen(&fine_tab), "output later");
        settled(&connection, &far);
        told_nothing_more(&failing_tab);
        assert!(!connection.is_dead());
    }

    /// 4.1: a sink that panics on the lost line, written while the link is
    /// down, is detached once a link is up again: the writer holds the
    /// `DETACH` like any other message, and it goes out after the `LIST`.
    /// The other session is kept, and takes input.
    #[test]
    fn a_sink_that_panics_while_the_link_is_down_is_detached_after_the_reconnect() {
        let (connection, plan) = steady_relay("A8");
        let (link, first) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (failing_sink, failing_tab) = fake::failing_sink("connection to A8 lost");
        let (fine_sink, fine_tab) = fake::sink();
        let failing = connection.spawn(spawn_request(&[]), failing_sink, None).unwrap();
        let fine = connection.spawn(spawn_request(&[]), fine_sink, None).unwrap();
        first.next();
        first.next();

        first.out.cut();
        assert_eq!(seen(&failing_tab), "broken the emulator failed on connection to A8 lost");
        assert!(seen(&fine_tab).contains("connection to A8 lost"));

        let (failing_id, fine_id) = (failing.id(), fine.id());
        let (link, second) = fake::link(&[], listing_attached(vec![failing_id, fine_id]));
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        assert_eq!(second.next(), ToCore::Detach(Detach { session: failing_id, layout: None }));
        settled(&connection, &second);
        told_nothing_more(&fine_tab);
        told_nothing_more(&failing_tab);
        fine.input(b"ls\r");
        assert!(matches!(second.next(), ToCore::Data(data) if data.session == fine_id && data.bytes == b"ls\r"));
    }

    /// 4.1: once its sink broke, a session's handle acts on nothing: its
    /// input, resize, close and detach write nothing, while the other
    /// session's close still goes out.
    #[test]
    fn a_broken_sessions_running_handle_writes_nothing() {
        let (connection, _plan, far, failing, failing_tab, fine, _fine_tab) = broken_by_data();
        failing.input(b"typed\r");
        failing.resize(100, 30, 8, 16);
        failing.close();
        failing.detach(None);
        fine.close();
        assert_eq!(far.next(), ToCore::Close(SessionRef { session: fine.id() }));
        assert_eq!(settled(&connection, &far), []);
        told_nothing_more(&failing_tab);
    }

    /// Answers `LIST` with `sessions`, each with its key and a client, as a
    /// core does for a client that came back within its linger.
    fn listing_attached(sessions: Vec<SessionId>) -> impl FnMut(&ToCore, &fake::Out) + Send + 'static {
        move |message, out| {
            if let ToCore::List(list) = message {
                let sessions = sessions
                    .iter()
                    .map(|&session| SessionInfo {
                        session,
                        key: format!("k{session}"),
                        attached: true,
                        ..Default::default()
                    })
                    .collect();
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions }));
            }
        }
    }

    /// The next `total` bytes of input the far end gets for `session`, in
    /// order; anything else first is a failure.
    fn written(far: &fake::Far, session: SessionId, total: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        while bytes.len() < total {
            match far.next() {
                ToCore::Data(data) if data.session == session => bytes.extend(data.bytes),
                other => panic!("not its DATA: {other:?}"),
            }
        }
        bytes
    }

    /// A session spawned on a steady relay whose link was then cut: its
    /// tab has the lost line, so the link is down for what comes next.
    fn spawned_then_cut() -> (Arc<Connection>, Plan, Running, mpsc::Receiver<String>) {
        let (connection, plan) = steady_relay("A8");
        let (link, first) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (sink, tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        assert!(matches!(first.next(), ToCore::Spawn(_)));
        first.out.cut();
        assert!(seen(&tab).contains("connection to A8 lost"));
        (connection, plan, running, tab)
    }

    /// 5.1: while the link is down, a session's input waits up to the high
    /// mark; past it, input is refused (the crossing's first refusal is
    /// `First`, the rest `Again`), and nothing refused is written once the
    /// link is back.
    #[test]
    fn input_past_four_mib_is_refused_while_the_link_is_down() {
        let (_connection, plan, running, _tab) = spawned_then_cut();
        let held = vec![b'a'; 4 * MIB];
        assert_eq!(running.input(&held), Delivery::Queued);
        assert_eq!(running.input(b"refused"), Delivery::Refused(Refusal::First));
        assert_eq!(running.input(b"refused again"), Delivery::Refused(Refusal::Again));

        let (link, far) = fake::link(&[], listing_attached(vec![running.id()]));
        plan.link(link);
        assert!(matches!(far.next(), ToCore::List(_)));
        assert_eq!(written(&far, running.id(), held.len()), held);
        assert!(far.got.recv_timeout(Duration::from_millis(300)).is_err(), "nothing refused is written");
    }

    /// 5.1: input held while the link is down goes out in order once it is
    /// back, and the writer frees what it wrote: input is taken again.
    #[test]
    fn held_input_is_written_in_order_after_the_reconnect_and_frees_the_budget() {
        let (_connection, plan, running, _tab) = spawned_then_cut();
        let (first, second) = (vec![b'a'; 2 * MIB], vec![b'b'; 2 * MIB]);
        assert_eq!(running.input(&first), Delivery::Queued);
        assert_eq!(running.input(&second), Delivery::Queued);
        assert_eq!(running.input(b"c"), Delivery::Refused(Refusal::First));
        assert_eq!(running.waiting(), 4 * MIB);

        let (link, far) = fake::link(&[], listing_attached(vec![running.id()]));
        plan.link(link);
        assert!(matches!(far.next(), ToCore::List(_)));
        assert_eq!(written(&far, running.id(), 4 * MIB), [first, second].concat());
        // Each frame is freed once the writer is done with it, just after
        // the far end has it.
        eventually("the written input is freed", || running.waiting() == 0);
        assert_eq!(running.input(b"d"), Delivery::Queued);
        assert_eq!(written(&far, running.id(), 1), b"d");
    }

    /// 5.1: a call is taken whole while less than the high mark waits, so
    /// a paste bigger than the mark into a connected tab is written whole.
    #[test]
    fn a_paste_past_the_high_mark_on_a_live_link_is_written_whole() {
        let (connection, plan) = steady_relay("A8");
        let (link, far) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (sink, _tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        assert!(matches!(far.next(), ToCore::Spawn(_)));
        let paste: Vec<u8> = (0..6 * MIB).map(|at| (at % 251) as u8).collect();
        assert_eq!(running.input(&paste), Delivery::Queued);
        assert_eq!(written(&far, running.id(), paste.len()), paste);
        eventually("the paste is freed", || running.waiting() == 0);
    }

    /// 5.1: only input counts against the budget: past the high mark, a
    /// resize and a detach still go out, behind the input queued first.
    #[test]
    fn resize_and_detach_are_never_refused() {
        let (_connection, plan, running, _tab) = spawned_then_cut();
        let held = vec![b'a'; 4 * MIB];
        assert_eq!(running.input(&held), Delivery::Queued);
        assert_eq!(running.input(b"x"), Delivery::Refused(Refusal::First));
        running.resize(100, 30, 8, 16);
        running.detach(None);
        // No longer the tab's: neither counted nor sent.
        assert_eq!(running.input(b"y"), Delivery::Discarded);

        let (link, far) = fake::link(&[], listing_attached(vec![running.id()]));
        plan.link(link);
        assert!(matches!(far.next(), ToCore::List(_)));
        assert_eq!(written(&far, running.id(), held.len()), held);
        assert!(matches!(far.next(), ToCore::Resize(resize) if (resize.cols, resize.rows) == (100, 30)));
        assert_eq!(far.next(), ToCore::Detach(Detach { session: running.id(), layout: None }));
    }

    /// A core that predates keys gives a session none, and lists it with
    /// none: after a reconnect it is kept by its id, as there is nothing
    /// better to compare, and its handle still acts.
    #[test]
    fn a_session_from_a_core_without_keys_is_kept_by_its_id() {
        let (connection, plan) = steady_relay("A8");
        let (link, first) = fake::link(&[], |message, out| {
            if let ToCore::Spawn(spawn) = message {
                out.send(ToClient::Spawned(Spawned { req: spawn.req, session: 3, pid: 1003, key: String::new() }));
            }
        });
        plan.link(link);
        let (sink, tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        assert_eq!((running.id(), running.key()), (3, None));
        assert!(matches!(first.next(), ToCore::Spawn(_)));
        first.out.cut();
        assert!(seen(&tab).contains("connection to A8 lost"));

        let (link, second) = fake::link(&[], |message, out| {
            if let ToCore::List(list) = message {
                replay(out, 3, None);
                let keyless = SessionInfo { session: 3, attached: true, ..Default::default() };
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions: vec![keyless] }));
            }
        });
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        assert_eq!(seen(&tab), "replay true");
        assert_eq!(seen(&tab), "output screen");
        assert_eq!(seen(&tab), "replay false");
        settled(&connection, &second);
        told_nothing_more(&tab);
        running.input(b"x");
        assert!(matches!(second.next(), ToCore::Data(data) if data.session == 3 && data.bytes == b"x"));
    }

    /// A replay of a session that no tab here shows or waits for (its tab
    /// closed, or its attach gave up) leaves the session detached: only a
    /// user's close ends a program.
    #[test]
    fn a_replay_nobody_here_waits_for_leaves_the_session_detached() {
        let (connection, plan) = steady_relay("A8");
        let (link, far) = fake::link(&[], |_, _| {});
        plan.link(link);
        far.hello();
        replay(&far.out, 5, None);
        assert_eq!(far.next(), ToCore::Detach(Detach { session: 5, layout: None }));
        replay(&far.out, 6, Some(77));
        assert_eq!(far.next(), ToCore::Detach(Detach { session: 6, layout: None }));
        assert_eq!(settled(&connection, &far), []);
        assert!(!connection.is_dead());
    }

    /// A tab that lets go of a request still waiting for the relay
    /// withdraws it: it is never sent.
    #[test]
    fn an_abandoned_request_is_withdrawn_and_never_sent() {
        let (connection, plan) = relay("A8");
        plan.next_dial();
        let abandon = Arc::new(Abandon::default());
        let (sink, _tab) = fake::sink();
        let (attaching, letting_go) = (connection.clone(), abandon.clone());
        let attach =
            in_background(move || attaching.attach(attach_request(5), sink, Some(&letting_go)).map(|r| r.pid()));
        waiting(&connection, 1);
        abandon.abandon();
        assert_eq!(attach.join().unwrap().unwrap_err().to_string(), "closed before it started");
        waiting(&connection, 0);
        // The ATTACH queued before the LIST never goes out.
        let (link, far) = fake::link(&[], listing);
        plan.link(link);
        assert!(connection.list().unwrap().is_empty());
        assert!(matches!(far.next(), ToCore::List(_)));
        assert_eq!(settled(&connection, &far), []);

        // Let go of before it was asked: withdrawn at once.
        let abandon = Abandon::default();
        abandon.abandon();
        let (sink, _tab) = fake::sink();
        let error = connection.attach(attach_request(6), sink, Some(&abandon)).err().expect("withdrawn");
        assert_eq!(error.to_string(), "closed before it started");
        assert_eq!(settled(&connection, &far), []);
    }

    /// With its attach let go, a relay that can't connect stops trying.
    #[test]
    fn a_relay_stops_trying_once_nothing_waits() {
        let (connection, plan) = relay("A8");
        let abandon = Arc::new(Abandon::default());
        let (sink, _tab) = fake::sink();
        let (attaching, letting_go) = (connection.clone(), abandon.clone());
        let attach =
            in_background(move || attaching.attach(attach_request(5), sink, Some(&letting_go)).map(|r| r.pid()));
        waiting(&connection, 1);
        plan.next_dial();
        plan.fail("offline");
        plan.next_dial();
        assert!(!connection.is_dead());
        abandon.abandon();
        assert!(attach.join().unwrap().is_err());
        plan.fail("offline");
        eventually("the connection ends", || connection.is_dead());
    }

    /// A relay that reached no core says why, which is the error a new tab
    /// shows.
    #[test]
    fn a_relay_error_is_what_a_new_tab_shows() {
        struct Plain(Mutex<io::PipeWriter>);
        impl Wire for Plain {
            fn send(&self, frame: &Frame) -> io::Result<()> {
                frame.write_to(&mut *self.0.lock())
            }
            fn end(&self) {}
        }
        let (connection, plan) = relay("A8");
        let (sink, _tab) = fake::sink();
        let spawning = connection.clone();
        let spawn = in_background(move || spawning.spawn(spawn_request(&[]), sink, None).map(|r| r.id()));
        waiting(&connection, 1);
        let (client_reader, mut far_writer) = io::pipe().unwrap();
        let (_far_reader, client_writer) = io::pipe().unwrap();
        let message = "terminal server unreachable: no server directory; its log is /tmp/b/core.log";
        RelayMessage::Error(RelayError { message: message.into() }).encode().unwrap().write_to(&mut far_writer).unwrap();
        drop(far_writer);
        plan.link(Link { reader: Box::new(client_reader), wire: Arc::new(Plain(Mutex::new(client_writer))) });
        assert_eq!(spawn.join().unwrap().unwrap_err().to_string(), format!("can't connect to A8: {message}"));
    }

    /// 6.1: a relay's session runs on another machine, so asking to parent
    /// its console to a window sends nothing, before or after a
    /// reconnect's replay.
    #[test]
    fn a_remote_session_never_sends_reparent() {
        let (connection, plan) = steady_relay("A8");
        let (link, first) = fake::link(&[], fake::spawning);
        plan.link(link);
        let (sink, tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        assert!(matches!(first.next(), ToCore::Spawn(_)));
        running.reparent(0x1234);
        first.out.cut();
        assert!(seen(&tab).contains("connection to A8 lost"));

        let id = running.id();
        let (link, second) = fake::link(&[], move |message, out| {
            if let ToCore::List(list) = message {
                replay(out, id, None);
                let info = SessionInfo { session: id, key: format!("k{id}"), attached: true, ..Default::default() };
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions: vec![info] }));
            }
        });
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        assert_eq!(seen(&tab), "replay true");
        assert_eq!(seen(&tab), "output screen");
        assert_eq!(seen(&tab), "replay false");
        running.reparent(0x5678);
        // Input still goes out: the link is up, and REPARENT never went.
        running.input(b"x");
        assert!(matches!(second.next(), ToCore::Data(data) if data.bytes == b"x"));
        assert_eq!(settled(&connection, &second), []);
        assert!(connection.state.lock().sessions.values().all(|shown| shown.parent.is_none()));
    }

    /// 6.1: `PROCESS` read right behind `SPAWNED` (the core sends one at
    /// once) is the session's, though its spawner may not have its answer
    /// yet. One for a session not shown here is dropped.
    #[test]
    fn process_right_after_spawned_is_kept_for_the_session() {
        let (connection, plan) = steady_relay("A8");
        let (link, far) = fake::link(&[], |message, out| {
            fake::spawning(message, out);
            if let ToCore::Spawn(spawn) = message {
                let session = 10 + spawn.req as SessionId;
                out.send(ToClient::Process(client::Process { session, name: Some("bash".into()), cwd: Some("/home/u".into()) }));
                out.send(ToClient::Process(client::Process { session: 99, name: Some("other".into()), cwd: None }));
            }
        });
        plan.link(link);
        let (sink, tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        let reported = Some(Foreground { name: Some("bash".into()), cwd: Some("/home/u".into()) });
        eventually("the PROCESS is kept", || running.foreground() == reported);
        settled(&connection, &far);
        told_nothing_more(&tab);
        assert!(!connection.state.lock().sessions.contains_key(&99), "nothing kept for session 99");
        running.close();
        assert_eq!(running.foreground(), None);
    }

    /// 6.3: a `CLOSE` made while the link is down keeps the connection
    /// reconnecting, past the idle time, until it is written within its
    /// limit; then, with nothing left in use, the connection ends and its
    /// relay is killed.
    #[test]
    fn a_close_queued_while_disconnected_keeps_the_relay_until_it_is_written() {
        let (dial, plan) = fake::dialer("A8");
        let idle = Duration::from_millis(250);
        let close_limit = Duration::from_secs(30);
        let timings = Timings { dead: Duration::from_secs(30), idle, close_limit, ..fake::quick() };
        let connection = Connection::relay(dial, "tok", timings).unwrap();
        let (link, first) = fake::link(&[], fake::spawning);
        plan.link(link);
        plan.next_dial();
        let (sink, _tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        assert!(matches!(first.next(), ToCore::Spawn(_)));
        first.out.cut();
        plan.next_dial();
        // Closed while reconnecting: the CLOSE waits for a link, and is all
        // that keeps the connection in use.
        running.close();
        looked_past(&connection, Instant::now(), idle);
        assert!(!connection.is_dead(), "ended with its CLOSE unwritten");

        let id = running.id();
        let (link, second) = fake::link(&[], listing);
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        let close = match second.next() {
            ToCore::Close(close) => close,
            other => panic!("not CLOSE: {other:?}"),
        };
        let written = (close.session, Instant::now());
        assert_eq!(written.0, id);
        eventually("the unused connection ends", || connection.is_dead());
        // Not before `idle` (less the idle thread's look, a fifth of it).
        assert!(written.1.elapsed() >= idle * 4 / 5, "{:?}", written.1.elapsed());
        eventually("its relay is ended", || second.out.is_cut());
        // It doesn't come back.
        threads_ended(&connection, 2);
        assert!(plan.dialed.try_recv().is_err());
    }

    /// 3.1: a `CLOSE` made while the link is down keeps the connection
    /// trying only for its limit. With still no link up, it is given up: the
    /// connection ends with a reason naming the session it left running, and
    /// dials no more.
    #[test]
    fn a_close_not_written_within_its_limit_is_given_up_and_the_relay_ends() {
        let (dial, plan) = fake::dialer("A8");
        let (idle, close_limit) = (Duration::from_millis(200), Duration::from_millis(600));
        let timings = Timings { dead: Duration::from_secs(30), idle, close_limit, ..fake::quick() };
        let connection = Connection::relay(dial, "tok", timings).unwrap();
        let (link, first) = fake::link(&[], fake::spawning);
        plan.link(link);
        plan.next_dial();
        let (sink, _tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        assert!(matches!(first.next(), ToCore::Spawn(_)));
        first.out.cut();
        // Reconnecting: the attempt waits on the test, so no link comes up.
        plan.next_dial();
        running.close();
        let closed = Instant::now();
        eventually("the connection ends", || connection.is_dead());
        assert!(closed.elapsed() >= close_limit, "ended {:?} after the close", closed.elapsed());
        let reason = format!("not ending session {} on A8: it couldn't be reached within 600 ms of the close", running.id());
        assert_eq!(connection.list().unwrap_err().to_string(), reason);
        // The attempt under way finds the connection ended; none follows.
        plan.fail("ssh: connect to host A8 port 22: Connection refused");
        threads_ended(&connection, 2);
        assert!(plan.dialed.try_recv().is_err(), "dialled again");
    }

    /// 3.1: an eager connection keeps trying past the close limit, but the
    /// `CLOSE` it gave up isn't written once a link is back: the program is
    /// left running, as when Brindle ends without quitting.
    #[test]
    fn a_close_given_up_is_never_written() {
        let (dial, plan) = fake::dialer("A8");
        let (idle, close_limit) = (Duration::from_millis(100), Duration::from_millis(250));
        let timings = Timings { dead: Duration::from_secs(30), idle, close_limit, ..fake::quick() };
        let connection = Connection::relay_with(dial, "tok", timings, true, None).unwrap();
        let (link, first) = fake::link(&[], fake::spawning);
        plan.link(link);
        plan.next_dial();
        let (sink, _tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        assert!(matches!(first.next(), ToCore::Spawn(_)));
        first.out.cut();
        plan.next_dial();
        running.close();
        eventually("the close is given up", || connection.state.lock().closing.is_empty());
        looked_past(&connection, Instant::now(), idle);
        assert!(!connection.is_dead(), "an eager connection ended");
        let (link, second) = fake::link(&[], listing);
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        assert_eq!(settled(&connection, &second), [], "the CLOSE given up went out");
    }

    /// 3.1: what a connection says of the closes it gave up, and its limit
    /// as reasons give it.
    #[test]
    fn closes_given_up_name_their_sessions() {
        let limit = Timings::default().close_limit;
        assert_eq!(
            not_ending(&[7], "A8", limit),
            "not ending session 7 on A8: it couldn't be reached within 5 minutes of the close"
        );
        assert_eq!(
            not_ending(&[7, 9], "dev@A8:2222", Duration::from_millis(600)),
            "not ending sessions 7, 9 on dev@A8:2222: it couldn't be reached within 600 ms of the close"
        );
        assert_eq!(span_text(Duration::from_secs(60)), "1 minute");
        assert_eq!(span_text(Duration::from_secs(90)), "90 s");
        assert_eq!(span_text(Duration::from_secs(30)), "30 s");
    }

    /// 6.3: an eager connection (a profile that connects at launch) stays
    /// with nothing using it.
    #[test]
    fn an_eager_relay_stays_unused() {
        let (dial, plan) = fake::dialer("A8");
        let idle = Duration::from_millis(100);
        let timings = Timings { dead: Duration::from_secs(30), idle, ..fake::quick() };
        let (eager, queue) = Connection::new(Transport::Relay { dial, timings }, "tok");
        eager.set_eager(true);
        let (writing, idling, reading) = (eager.clone(), eager.clone(), eager.clone());
        std::thread::spawn(move || writing.write_loop(queue));
        std::thread::spawn(move || idling.end_when_unused(idle, timings.close_limit));
        std::thread::spawn(move || reading.run_relay());
        let (link, far) = fake::link(&[], |_, _| {});
        plan.link(link);
        far.hello();
        looked_past(&eager, Instant::now(), idle);
        assert!(!eager.is_dead());
        assert!(!far.out.is_cut());
        // No longer eager: it ends.
        eager.set_eager(false);
        eventually("the unused connection ends", || eager.is_dead());
        eventually("its relay is ended", || far.out.is_cut());
    }

    /// A [`Report`] that hands each report to the receiver.
    fn reporting() -> (Report, mpsc::Receiver<(u64, LinkState)>) {
        let (tx, rx) = mpsc::channel();
        (Arc::new(move |id, state| _ = tx.send((id, state))), rx)
    }

    /// The next report, which must be from connection `id`.
    fn reported(rx: &mpsc::Receiver<(u64, LinkState)>, id: u64) -> LinkState {
        let (from, state) = rx.recv_timeout(Duration::from_secs(10)).expect("a report");
        assert_eq!(from, id, "{state:?} from another connection");
        state
    }

    /// 8.2: a relay connection reports connecting, connected once its link
    /// is up, offline with why once that link is lost and with each failed
    /// attempt's error after it (never connecting again), connected once a
    /// link is back, and ended once it is no longer eager and nothing uses
    /// it. Nothing follows the end.
    #[test]
    fn a_relay_reports_connecting_connected_offline_and_connected_again() {
        let (dial, plan) = fake::dialer("A8");
        let timings = Timings { dead: Duration::from_secs(30), idle: Duration::from_millis(200), ..fake::quick() };
        let (report, reports) = reporting();
        let connection = Connection::relay_with(dial, "tok", timings, true, Some(report)).unwrap();
        let id = connection.id;
        assert_eq!(reported(&reports, id), LinkState::Connecting);
        let (link, first) = fake::link(&[], |_, _| {});
        plan.link(link);
        assert_eq!(reported(&reports, id), LinkState::Connected);
        first.hello();

        first.out.cut();
        assert_eq!(
            reported(&reports, id),
            LinkState::Offline("connection lost: the relay closed the connection".into())
        );
        plan.fail("ssh: connect to host A8 port 22: Connection refused");
        assert_eq!(reported(&reports, id), LinkState::Offline("ssh: connect to host A8 port 22: Connection refused".into()));
        plan.fail("ssh: connect to host A8 port 22: Connection refused");
        let (link, second) = fake::link(&[], |_, _| {});
        plan.link(link);
        // The same error again says nothing new.
        assert_eq!(reported(&reports, id), LinkState::Connected);
        second.hello();
        settled(&connection, &second);
        assert!(reports.try_recv().is_err(), "nothing while connected");

        connection.set_eager(false);
        assert_eq!(reported(&reports, id), LinkState::Ended("the connection was no longer used".into()));
        eventually("its relay is ended", || second.out.is_cut());
        threads_ended(&connection, 1);
        assert!(reports.try_recv().is_err(), "nothing after the end");
    }

    /// An eager relay connection that reports its states, with the plan for
    /// its links, once it reports connecting.
    fn reporting_relay(name: &str, timings: Timings) -> (Arc<Connection>, Plan, mpsc::Receiver<(u64, LinkState)>) {
        let (dial, plan) = fake::dialer(name);
        let (report, reports) = reporting();
        let connection = Connection::relay_with(dial, "tok", timings, true, Some(report)).unwrap();
        assert_eq!(reported(&reports, connection.id), LinkState::Connecting);
        (connection, plan, reports)
    }

    /// 1.2: a link whose relay process ended says how: its status and what it
    /// wrote, in the offline report and in the failed requests.
    #[test]
    fn a_link_lost_to_its_process_ending_names_its_exit() {
        let timings = Timings { dead: Duration::from_secs(30), ..fake::quick() };
        let (connection, plan, reports) = reporting_relay("A8", timings);
        let (link, first) = fake::link(&[], |_, _| {});
        plan.link(link);
        assert_eq!(reported(&reports, connection.id), LinkState::Connected);
        first.hello();
        // A request written on the link, which the loss fails.
        let (sink, _tab) = fake::sink();
        let spawning = connection.clone();
        let spawn = in_background(move || spawning.spawn(spawn_request(&[]), sink, None).map(|r| r.id()));
        assert!(matches!(first.next(), ToCore::Spawn(_)));

        let why = "ssh exited with 255: Timeout, server A8 not responding.";
        first.out.set_why(why);
        first.out.cut();
        assert_eq!(reported(&reports, connection.id), LinkState::Offline(format!("connection lost: {why}")));
        assert_eq!(spawn.join().unwrap().unwrap_err().to_string(), format!("the terminal server went away ({why})"));
    }

    /// 1.2: a link the watchdog ended for silence says so, not what the
    /// kill it caused looks like (the wire's `why` is ignored).
    #[test]
    fn a_link_ended_by_the_watchdog_says_nothing_was_read() {
        let timings = Timings { dead: Duration::from_millis(400), ..fake::quick() };
        let (connection, plan, reports) = reporting_relay("A8", timings);
        let (link, first) = fake::link(&[], |_, _| {});
        // What a killed process would report.
        first.out.set_why("ssh ended: signal: 9 (SIGKILL)");
        plan.link(link);
        assert_eq!(reported(&reports, connection.id), LinkState::Connected);
        assert_eq!(
            reported(&reports, connection.id),
            LinkState::Offline("connection lost: nothing read from A8 for 400 ms".into())
        );
        assert!(first.out.is_cut());
    }

    /// 1.2: how a link's reason is chosen, whichever the order the facts
    /// come in.
    #[test]
    fn a_relay_links_reason_is_the_silence_then_the_process_then_the_stream() {
        // A process that said nothing gives no account.
        let said = |ending: &Ending, why: &str| {
            relay_link_ended("A8", ending, Some(why.to_string()).filter(|why| !why.is_empty()).map(RelayAccount::Said))
        };
        let failed = Ending::Failed(io::Error::new(io::ErrorKind::ConnectionReset, "reset"));
        // Silence wins, whatever the stream did (the process isn't asked:
        // its status would only report the kill).
        let silent = Some(RelayAccount::Silent(Duration::from_secs(30)));
        assert_eq!(relay_link_ended("A8", &Ending::Closed, silent.clone()), "nothing read from A8 for 30 s");
        assert_eq!(relay_link_ended("A8", &failed, silent), "nothing read from A8 for 30 s");
        // The process's own account.
        assert_eq!(said(&Ending::Closed, "ssh exited with 255: boom"), "ssh exited with 255: boom");
        assert_eq!(said(&failed, "ssh exited with 255: boom"), "ssh exited with 255: boom");
        // Nothing from the process: how the stream ended.
        assert_eq!(said(&Ending::Closed, ""), "the relay closed the connection");
        assert_eq!(said(&failed, ""), "reading from the relay failed: reset");
        // The local core's text is its own.
        assert_eq!(Ending::Closed.local_text(), "the terminal server closed the connection");
        assert_eq!(failed.local_text(), "reading from the terminal server failed: reset");
    }

    /// 8.2: a connection made eager keeps trying after its first attempt
    /// fails, with nothing waiting on it, and says it is offline; one that
    /// isn't eager ends with the error. Each connection reports with an id
    /// of its own.
    #[test]
    fn an_eager_relay_is_offline_until_its_first_link_and_a_lazy_one_ends() {
        let (dial, plan) = fake::dialer("devbox");
        let (report, reports) = reporting();
        let eager = Connection::relay_with(dial, "tok", fake::quick(), true, Some(report.clone())).unwrap();
        assert_eq!(reported(&reports, eager.id), LinkState::Connecting);
        plan.fail("Error response from daemon: container devbox is not running");
        let offline = LinkState::Offline("Error response from daemon: container devbox is not running".into());
        assert_eq!(reported(&reports, eager.id), offline);
        plan.next_dial();
        plan.next_dial();
        assert!(!eager.is_dead());

        let (dial, plan) = fake::dialer("devbox");
        let lazy = Connection::relay_with(dial, "tok", fake::quick(), false, Some(report)).unwrap();
        assert!(lazy.id > eager.id);
        assert_eq!(reported(&reports, lazy.id), LinkState::Connecting);
        plan.fail("Error response from daemon: container devbox is not running");
        assert_eq!(
            reported(&reports, lazy.id),
            LinkState::Ended("can't connect to devbox: Error response from daemon: container devbox is not running".into())
        );
        eventually("the connection ends", || lazy.is_dead());
    }

    /// 3.3: an eager connection whose core refuses this Brindle keeps
    /// trying: it reports the refusal offline, fails a new tab's spawn with
    /// it, and is connected once a core welcomes it. One that isn't eager
    /// ends on a refusal (`a_relay_error_is_what_a_new_tab_shows` covers its
    /// end with the error).
    #[test]
    fn an_eager_relay_retries_after_a_refusal() {
        let timings = Timings { dead: Duration::from_secs(30), ..fake::quick() };
        let (connection, plan, reports) = reporting_relay("A8", timings);
        let (sink, _tab) = fake::sink();
        let spawning = connection.clone();
        let spawn = in_background(move || spawning.spawn(spawn_request(&[]), sink, None).map(|r| r.id()));
        waiting(&connection, 1);
        let why = "no common protocol version (offered 1–2, supported 3–4)";
        plan.link(fake::refusing(why));
        let refusal = format!("{:#}", super::super::refused(why));
        assert_eq!(reported(&reports, connection.id), LinkState::Offline(refusal.clone()));
        assert_eq!(spawn.join().unwrap().unwrap_err().to_string(), format!("can't connect to A8: {refusal}"));
        assert!(!connection.is_dead());
        // The next attempt finds a core that welcomes it.
        plan.next_dial();
        let (link, far) = fake::link(&[], |_, _| {});
        plan.link(link);
        assert_eq!(reported(&reports, connection.id), LinkState::Connected);
        far.hello();

        // Not eager: a refusal ends it.
        let (dial, plan) = fake::dialer("B9");
        let lazy = Connection::relay(dial, "tok", timings).unwrap();
        plan.link(fake::refusing(why));
        eventually("the connection ends", || lazy.is_dead());
        assert_eq!(lazy.list().unwrap_err().to_string(), format!("can't connect to B9: {refusal}"));
    }

    /// 8.3: a listing given a limit fails once it passes, with the relay
    /// down, and the `LIST` it queued is never sent.
    #[test]
    fn a_limited_listing_gives_up_and_is_never_sent() {
        let (connection, plan) = steady_relay("A8");
        let (link, first) = fake::link(&[], |_, _| {});
        plan.link(link);
        first.hello();
        // Listed only once the connection has lost the link it had: a `LIST`
        // the writer is still trying on the cut link is one the reader's
        // loss fails as "went away". (Before the link is up the slot is
        // empty too, so up comes first.)
        eventually("the link is up", || connection.slot.lock().link.is_some());
        first.out.cut();
        eventually("the link is down", || connection.slot.lock().link.is_none());
        let error = connection.list_within(Some(Duration::from_millis(200))).unwrap_err().to_string();
        assert_eq!(error, "the terminal server didn't list its sessions within 0.2 s");
        assert!(connection.state.lock().requests.is_empty());
        let (link, second) = fake::link(&[], |_, _| {});
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)), "the reconnect's own");
        assert_eq!(settled(&connection, &second), [], "not the one given up on");
    }

    /// 7.1: a session a restored zone recorded waits, through failed
    /// attempts, for the endpoint's first link; a `LIST` goes out first,
    /// and once it has the session with its key and no client, the `ATTACH`
    /// goes out at the listed size, which the sink is told before the
    /// replay.
    #[test]
    fn a_recorded_attach_waits_for_a_listing_and_goes_out_at_the_listed_size() {
        let (connection, plan) = steady_relay("A8");
        let (sink, tab) = fake::sink();
        let attaching = connection.clone();
        let attach = in_background(move || {
            attaching.attach(recorded(5), sink, None).map(|r| (r.id(), r.pid(), r.key().cloned()))
        });
        waiting(&connection, 1);
        plan.next_dial();
        plan.fail("ssh: connect to host A8 port 22: Connection refused");
        assert!(seen(&tab).contains("Connection refused; retrying…"));
        plan.next_dial();
        assert!(!connection.is_dead());
        let (link, far) = fake::link(&[], |message, out| match message {
            ToCore::List(list) => {
                let info = SessionInfo { session: 5, key: "k5".into(), cols: 132, rows: 43, owned: true, ..Default::default() };
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions: vec![info] }));
            }
            other => replaying(other, out),
        });
        plan.link(link);
        assert!(matches!(far.next(), ToCore::List(_)));
        let ToCore::Attach(sent) = far.next() else { panic!("not ATTACH") };
        assert_eq!((sent.session, sent.cols, sent.rows), (5, 132, 43));
        assert_eq!(attach.join().unwrap().unwrap(), (5, 99, Some(key("k5"))));
        assert_eq!(seen(&tab), "size 132x43");
        assert_eq!(seen(&tab), "replay true");
        assert_eq!(seen(&tab), "output screen");
        assert_eq!(seen(&tab), "replay false");
        assert_eq!(settled(&connection, &far), []);
        told_nothing_more(&tab);
    }

    /// 7.1: a recorded session the listing lacks, lists with another key
    /// (its id was reused), or lists with a client is never attached: each
    /// attach fails with why. Attaches made while the link is up ask for a
    /// listing themselves.
    #[test]
    fn a_recorded_attach_fails_when_its_session_ended_was_replaced_or_is_shown() {
        let (connection, plan) = steady_relay("A8");
        let (link, far) = fake::link(&[], |message, out| {
            if let ToCore::List(list) = message {
                let sessions = vec![
                    SessionInfo { session: 6, key: "another".into(), ..Default::default() },
                    SessionInfo { session: 7, key: "k7".into(), attached: true, ..Default::default() },
                ];
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions }));
            }
        });
        plan.link(link);
        far.hello();
        let failures: Vec<String> = [5, 6, 7]
            .map(|id| {
                let attaching = connection.clone();
                let (sink, _tab) = fake::sink();
                in_background(move || attaching.attach(recorded(id), sink, None).map(|r| r.pid()))
            })
            .into_iter()
            .map(|attach| attach.join().unwrap().unwrap_err().to_string())
            .collect();
        assert_eq!(
            failures,
            ["session 5 on A8 has ended", "session 6 on A8 has ended", "session 7 on A8 is shown by another Brindle"]
        );
        // Only listings went out.
        for message in settled(&connection, &far) {
            assert!(matches!(message, ToCore::List(_)), "{message:?}");
        }
        assert!(connection.state.lock().requests.is_empty());
    }

    /// 7.2: a launch's listing hook gets the endpoint's first listing,
    /// keeping a relay that can't connect yet trying, as an attach does,
    /// until it is withdrawn (`a_withdrawn_listing_hook_lets_the_relay_end`).
    /// Once a link is up, a hook asks for a listing of its own. A connection
    /// that ended gives the hook back.
    #[test]
    fn a_listing_hook_gets_the_first_listing_and_keeps_the_relay_connecting() {
        let (got, hooked) = mpsc::channel();
        let hook = |got: &mpsc::Sender<Vec<SessionId>>| -> ListingHook {
            let got = got.clone();
            Box::new(move |sessions: Vec<SessionInfo>| {
                got.send(sessions.iter().map(|s| s.session).collect()).ok();
            })
        };
        let (connection, plan) = steady_relay("A8");
        assert!(connection.on_listing(hook(&got)).is_ok());
        plan.next_dial();
        plan.fail("offline");
        plan.next_dial();
        assert!(!connection.is_dead(), "the hook keeps it trying");
        let (link, far) = fake::link(&[], |message, out| {
            if let ToCore::List(list) = message {
                let sessions = vec![SessionInfo { session: 3, ..Default::default() }];
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions }));
            }
        });
        plan.link(link);
        assert!(matches!(far.next(), ToCore::List(_)));
        assert_eq!(hooked.recv_timeout(Duration::from_secs(10)).unwrap(), [3]);
        assert!(connection.on_listing(hook(&got)).is_ok());
        assert!(matches!(far.next(), ToCore::List(_)));
        assert_eq!(hooked.recv_timeout(Duration::from_secs(10)).unwrap(), [3]);
        settled(&connection, &far);
        assert!(hooked.try_recv().is_err(), "each runs once");
        assert!(!connection.state.lock().in_use(), "nothing waits any more");

        let (ended, plan) = relay("B9");
        plan.next_dial();
        plan.fail("offline");
        eventually("the connection ends", || ended.is_dead());
        assert!(ended.on_listing(hook(&got)).is_err());
    }

    /// 3.2: a listing hook alone keeps a relay that can't connect trying,
    /// past its idle time. Withdrawn (the window's last tab on the endpoint
    /// went), it is dropped without a listing, and with nothing else using
    /// it the connection ends and dials no more.
    #[test]
    fn a_withdrawn_listing_hook_lets_the_relay_end() {
        let (dial, plan) = fake::dialer("A8");
        let idle = Duration::from_millis(200);
        let timings = Timings { dead: Duration::from_secs(30), idle, ..fake::quick() };
        let connection = Connection::relay(dial, "tok", timings).unwrap();
        let (got, hooked) = mpsc::channel::<Vec<SessionInfo>>();
        // The hook holds the only sender: once it is dropped, the receiver
        // hears so.
        assert!(connection.on_listing(Box::new(move |sessions| _ = got.send(sessions))).is_ok());
        plan.next_dial();
        plan.fail("ssh: connect to host A8 port 22: Connection refused");
        // The next attempt waits on the test.
        plan.next_dial();
        looked_past(&connection, Instant::now(), idle);
        assert!(!connection.is_dead(), "the hook keeps it trying");

        let withdrawn = Instant::now();
        assert!(connection.stop_listing(), "no hook was waiting");
        assert!(!connection.stop_listing(), "withdrawn twice");
        let heard = hooked.recv_timeout(Duration::from_secs(10)).map(|sessions| sessions.len());
        assert_eq!(heard, Err(mpsc::RecvTimeoutError::Disconnected), "the hook wasn't dropped");
        eventually("the connection ends", || connection.is_dead());
        assert!(withdrawn.elapsed() >= idle * 4 / 5, "ended {:?} after the withdrawal", withdrawn.elapsed());
        assert_eq!(connection.list().unwrap_err().to_string(), "the connection was no longer used");
        plan.fail("ssh: connect to host A8 port 22: Connection refused");
        threads_ended(&connection, 1);
        assert!(plan.dialed.try_recv().is_err(), "dialled again");
    }

    /// A link that holds every frame while it is shut, as a stalled one
    /// does, and keeps the type of each frame it passed on.
    struct Gate {
        inner: Arc<dyn Wire>,
        shut: Mutex<bool>,
        opened: Condvar,
        /// Each frame's type, and when it went through.
        passed: Mutex<Vec<(u8, Instant)>>,
    }

    impl Gate {
        fn around(link: Link) -> (Link, Arc<Self>) {
            let gate = Arc::new(Self {
                inner: link.wire,
                shut: Mutex::new(false),
                opened: Condvar::new(),
                passed: Mutex::new(Vec::new()),
            });
            (Link { reader: link.reader, wire: gate.clone() }, gate)
        }

        fn shut(&self, shut: bool) {
            *self.shut.lock() = shut;
            self.opened.notify_all();
        }

        /// Whether a frame of `message`'s type has gone through.
        fn passed(&self, message: &ToCore) -> bool {
            self.passed_at(message).is_some()
        }

        /// When the first frame of `message`'s type went through.
        fn passed_at(&self, message: &ToCore) -> Option<Instant> {
            let kind = message.encode().unwrap().msg_type;
            self.passed.lock().iter().find(|(passed, _)| *passed == kind).map(|&(_, at)| at)
        }
    }

    impl Wire for Gate {
        fn send(&self, frame: &Frame) -> io::Result<()> {
            let mut shut = self.shut.lock();
            while *shut {
                self.opened.wait(&mut shut);
            }
            drop(shut);
            self.inner.send(frame)?;
            self.passed.lock().push((frame.msg_type, Instant::now()));
            Ok(())
        }

        fn end(&self) {
            self.inner.end();
        }
    }

    /// What an exiting Brindle waits for: a flush returns once the
    /// messages sent before it are written, not when they are only queued
    /// (a quit's `DETACH` that the exit cut off left its session an owner,
    /// so the next launch didn't open it as a tab). It gives up at its
    /// deadline on a stalled link, and at once on a link that is down.
    #[test]
    fn a_flush_returns_once_what_was_sent_is_written() {
        let (connection, plan) = steady_relay("A8");
        let (link, far) = fake::link(&[], fake::spawning);
        let (link, gate) = Gate::around(link);
        plan.link(link);
        let (sink, _tab) = fake::sink();
        let running = connection.spawn(spawn_request(&[]), sink, None).unwrap();
        assert!(matches!(far.next(), ToCore::Spawn(_)));
        assert!(connection.flush(Instant::now()), "nothing is left to write");

        // Stalled: the DETACH is queued, not written, by the deadline.
        let detach = ToCore::Detach(Detach { session: running.id(), layout: None });
        gate.shut(true);
        running.detach(None);
        let deadline = Duration::from_millis(200);
        let started = Instant::now();
        assert!(!connection.flush(started + deadline));
        // Not before its deadline, which the flush guarantees.
        assert!(started.elapsed() >= deadline, "gave up after {:?}", started.elapsed());
        assert!(!gate.passed(&detach));

        // Written while a flush waits: the flush returns, with it written,
        // and only after it was written.
        let flushing = connection.clone();
        let flush = in_background(move || {
            let flushed = flushing.flush(Instant::now() + limit(Duration::from_secs(10)));
            (flushed, Instant::now())
        });
        gate.shut(false);
        let (flushed, returned) = flush.join().unwrap();
        assert!(flushed);
        let written = gate.passed_at(&detach).expect("the DETACH written");
        assert!(written <= returned, "returned before the DETACH was written");
        assert_eq!(far.next(), detach);

        // Down: nothing can be written until a next link, which an exiting
        // process won't wait for, whatever its deadline.
        far.out.cut();
        eventually("the link is down", || connection.slot.lock().link.is_none());
        connection.send(ToCore::Detach(Detach { session: 99, layout: None }));
        let flushing = connection.clone();
        let flush = in_background(move || flushing.flush(Instant::now() + Duration::from_secs(3600)));
        eventually("the flush gives up", || flush.is_finished());
        assert!(!flush.join().unwrap());
    }

    fn key(text: &str) -> SessionKey {
        SessionKey::from_wire(text).expect("a key")
    }

    /// A listing's entry; `key` as the wire has it (empty: a core that
    /// predates keys), with no size.
    fn entry(session: SessionId, key: &str, attached: bool) -> ListedSession {
        ListedSession { session, key: SessionKey::from_wire(key), attached, size: None }
    }

    /// A state showing each of `shown` (id and key), with the sinks in the
    /// same order.
    fn showing(shown: &[(SessionId, Option<&str>)]) -> (State, Vec<Sink>) {
        let mut state = State::default();
        let sinks = shown
            .iter()
            .map(|&(session, wire_key)| {
                let (sink, _tab) = fake::sink();
                assert!(state.show(session, Shown::new(sink.clone(), wire_key.map(key))).is_none());
                sink
            })
            .collect();
        (state, sinks)
    }

    fn shown_ids(state: &State) -> Vec<SessionId> {
        let mut ids: Vec<SessionId> = state.sessions.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Whether the sinks taken out are `expected`, in any order.
    fn same_sinks(taken: Vec<(SessionId, Sink)>, expected: &[&Sink]) -> bool {
        let mut sinks: Vec<Sink> = taken.into_iter().map(|(_, sink)| sink).collect();
        sinks.len() == expected.len()
            && expected.iter().all(|wanted| {
                let found = sinks.iter().position(|sink| Arc::ptr_eq(sink, wanted));
                found.map(|at| sinks.swap_remove(at)).is_some()
            })
    }

    /// After a lost link, a session shown here is kept only when the
    /// listing has its id with its key, and a client (this one, given back).
    #[test]
    fn a_shown_session_is_kept_only_when_listed_with_its_key_and_a_client() {
        let (mut state, sinks) =
            showing(&[(1, Some("k1")), (2, Some("k2")), (3, Some("k3")), (4, Some("k4")), (5, Some("k5"))]);
        let listed = [
            entry(1, "k1", true),
            // The id is another session's now.
            entry(2, "another", true),
            // No client has it: not given back to this one.
            entry(3, "k3", false),
            // 4 isn't listed.
            // A core without keys didn't give us ours.
            entry(5, "", true),
        ];
        let lost = state.keep_listed(&listed);
        assert!(same_sinks(lost, &[&sinks[1], &sinks[2], &sinks[3], &sinks[4]]));
        assert_eq!(shown_ids(&state), [1]);
        assert!(state.keep_listed(&listed).is_empty(), "taken out once");
    }

    /// A session from a core that predates keys has nothing but its id to
    /// be matched by, as before keys.
    #[test]
    fn a_keyless_shown_session_is_kept_by_id() {
        let (mut state, sinks) = showing(&[(1, None), (2, None), (3, None)]);
        let listed = [entry(1, "", true), entry(2, "k2", true), entry(3, "", false)];
        let lost = state.keep_listed(&listed);
        assert!(same_sinks(lost, &[&sinks[2]]));
        assert_eq!(shown_ids(&state), [1, 2]);
    }

    /// A session's window and foreground program are kept in its entry, so
    /// they go with it: a session shown later under its id starts with none.
    #[test]
    fn a_pruned_session_takes_its_window_and_foreground_with_it() {
        let (mut state, _sinks) = showing(&[(1, Some("k1"))]);
        let shown = state.sessions.get_mut(&1).unwrap();
        shown.parent = Some(0x1234);
        shown.foreground = Some(Foreground { name: Some("bash".into()), cwd: None });
        assert_eq!(state.keep_listed(&[entry(1, "another", true)]).len(), 1);
        assert!(state.sessions.is_empty());

        let (sink, _tab) = fake::sink();
        assert!(state.show(1, Shown::new(sink, Some(key("another")))).is_none());
        let shown = &state.sessions[&1];
        assert_eq!((shown.parent, shown.foreground.clone()), (None, None));
    }

    /// The entry a session shown under an id it already has displaces comes
    /// back, so its tab can be told it is lost.
    #[test]
    fn showing_a_session_under_a_shown_id_returns_the_one_it_displaced() {
        let (mut state, sinks) = showing(&[(1, Some("k1"))]);
        let (newer, _tab) = fake::sink();
        let displaced = state.show(1, Shown::new(newer.clone(), Some(key("k1b")))).expect("displaced");
        assert!(Arc::ptr_eq(&displaced.sink, &sinks[0]));
        assert_eq!(displaced.key, Some(key("k1")));
        assert!(Arc::ptr_eq(&state.sessions[&1].sink, &newer));
        assert!(state.shows(1, Some(&key("k1b"))));
    }

    /// A handle forgets only its own session: one shown under its id with
    /// another key stays.
    #[test]
    fn forget_leaves_a_session_shown_under_another_key() {
        let (mut state, sinks) = showing(&[(1, Some("k1b"))]);
        assert!(state.forget(1, Some(&key("k1"))).is_none());
        assert!(state.forget(1, None).is_none());
        assert!(state.forget(2, Some(&key("k1b"))).is_none());
        assert_eq!(shown_ids(&state), [1]);
        let forgotten = state.forget(1, Some(&key("k1b"))).expect("its own");
        assert!(Arc::ptr_eq(&forgotten.sink, &sinks[0]));
        assert!(state.sessions.is_empty());
        assert!(state.forget(1, Some(&key("k1b"))).is_none(), "once");
    }

    #[test]
    fn closes_are_given_up_once_the_oldest_waited_the_limit_unlinked() {
        let limit = Duration::from_secs(300);
        let queued = Instant::now();
        let mut state = State::default();
        state.closing(7, queued);
        state.closing(3, queued + Duration::from_secs(10));
        assert!(state.in_use());
        assert_eq!(state.closes_due(limit), Some(queued + limit));
        // None before the limit.
        assert!(state.give_up_closes(limit, queued + limit - Duration::from_millis(1)).is_empty());
        // None while a link is up, however long they waited.
        state.link = LinkPhase::Up(Negotiated(2));
        assert_eq!(state.closes_due(limit), None);
        assert!(state.give_up_closes(limit, queued + limit * 2).is_empty());
        // All of them, oldest first, once the oldest waited the limit.
        state.link = LinkPhase::Down(Negotiated(2));
        assert_eq!(state.give_up_closes(limit, queued + limit), [7, 3]);
        assert!(!state.in_use());
        assert!(state.give_up_closes(limit, queued + limit * 2).is_empty(), "once");
    }

    #[test]
    fn shows_needs_the_same_id_and_key() {
        let (state, _sinks) = showing(&[(1, Some("k1")), (2, None)]);
        assert!(state.shows(1, Some(&key("k1"))));
        assert!(!state.shows(1, Some(&key("another"))));
        assert!(!state.shows(1, None));
        assert!(state.shows(2, None));
        assert!(!state.shows(2, Some(&key("k2"))));
        assert!(!state.shows(3, None));
        assert!(!state.shows(3, Some(&key("k1"))));
    }

    /// The attaches waiting on a listing, as [`Connection::attach`] makes
    /// them, all carried (cut off with a link, or recorded and not sent).
    struct Carried {
        requests: HashMap<RequestId, Pending>,
        sinks: HashMap<RequestId, Sink>,
    }

    impl Carried {
        fn new(attaches: impl IntoIterator<Item = (RequestId, AttachRequest)>) -> Self {
            let mut carried = Self { requests: HashMap::new(), sinks: HashMap::new() };
            for (req, request) in attaches {
                let (sink, _tab) = fake::sink();
                let (reply, _answer) = mpsc::sync_channel(1);
                let (waiting, _message) = request.waiting(req, sink.clone(), reply);
                carried.requests.insert(req, Pending { waiting, sent: Sent::Carried });
                carried.sinks.insert(req, sink);
            }
            carried
        }

        /// What `listed` decides, with the failures by request.
        fn decide(&mut self, listed: &[ListedSession]) -> (Vec<Again>, Vec<(RequestId, String)>) {
            let decided = carried_attaches(&mut self.requests, listed, "A8");
            let mut gone: Vec<(RequestId, String)> =
                decided.gone.into_iter().map(|(pending, why)| (req_of(&pending), why)).collect();
            gone.sort();
            (decided.again, gone)
        }

        fn awaits(&self, req: RequestId) -> Option<Awaits> {
            match &self.requests.get(&req)?.waiting {
                Waiting::Attach { awaits, .. } => Some(*awaits),
                Waiting::Spawn { .. } | Waiting::List(_) | Waiting::Done(_) => None,
            }
        }
    }

    fn req_of(pending: &Pending) -> RequestId {
        match &pending.waiting {
            Waiting::Attach { attach, .. } => attach.req,
            Waiting::Spawn { .. } | Waiting::List(_) | Waiting::Done(_) => panic!("not an attach"),
        }
    }

    fn recorded_request(session: SessionId, wire_key: &str) -> AttachRequest {
        AttachRequest { session, key: SessionKey::from_wire(wire_key), when: Attaching::Recorded }
    }

    fn listed_at(session: SessionId, key: &str, size: Option<(u16, u16)>) -> ListedSession {
        ListedSession { size, ..entry(session, key, false) }
    }

    /// A recorded attach listed with its key and no client goes out at the
    /// listed size, which its sink is told; with no size listed (a core that
    /// predates sizes), it goes out at the request's and its sink is told
    /// nothing. Either now waits for its replay.
    #[test]
    fn a_recorded_attach_goes_out_at_the_listed_size_when_listed_with_its_key_and_no_client() {
        let mut carried = Carried::new([(1, recorded_request(5, "k5")), (2, recorded_request(6, "k6"))]);
        let (mut again, gone) = carried.decide(&[listed_at(5, "k5", Some((132, 43))), listed_at(6, "k6", None)]);
        assert!(gone.is_empty(), "{gone:?}");
        again.sort_by_key(|(attach, _)| attach.req);
        let [(at_listed, told), (as_requested, untold)] = <[_; 2]>::try_from(again).ok().expect("two attaches");
        assert_eq!(at_listed, client::Attach { req: 1, session: 5, cols: 132, rows: 43 });
        let (sink, size) = told.expect("its sink is told the size");
        assert!(Arc::ptr_eq(&sink, &carried.sinks[&1]));
        assert_eq!(size, (132, 43));
        // The request's size: none, which the core takes as the session's.
        assert_eq!(as_requested, client::Attach { req: 2, session: 6, cols: 0, rows: 0 });
        assert!(untold.is_none());
        for req in [1, 2] {
            assert_eq!(carried.awaits(req), Some(Awaits::Replay));
            assert_eq!(carried.requests[&req].sent, Sent::Queued);
        }
    }

    #[test]
    fn a_recorded_attach_shown_by_another_client_fails() {
        let mut carried = Carried::new([(1, recorded_request(7, "k7"))]);
        let (again, gone) = carried.decide(&[entry(7, "k7", true)]);
        assert!(again.is_empty());
        assert_eq!(gone, [(1, "session 7 on A8 is shown by another Brindle".to_string())]);
        assert!(carried.requests.is_empty());
    }

    /// A recorded attach needs its key: without one, or listed with another
    /// (the id was given to another session) or not at all, its session
    /// ended.
    #[test]
    fn a_recorded_attach_without_a_key_or_with_another_has_ended() {
        let mut carried = Carried::new([
            (1, recorded_request(5, "")),
            (2, recorded_request(6, "k6")),
            (3, recorded_request(7, "k7")),
            (4, recorded_request(8, "k8")),
        ]);
        let listed = [entry(5, "k5", false), entry(6, "another", false), entry(7, "", false)];
        let (again, gone) = carried.decide(&listed);
        assert!(again.is_empty());
        assert_eq!(
            gone,
            [
                (1, "session 5 on A8 has ended".to_string()),
                (2, "session 6 on A8 has ended".to_string()),
                (3, "session 7 on A8 has ended".to_string()),
                (4, "session 8 on A8 has ended".to_string()),
            ]
        );
        assert!(carried.requests.is_empty());
    }

    /// An attach cut off with its link whose session the core gave back to
    /// this client waits for that replay: nothing is sent again.
    #[test]
    fn a_carried_attach_listed_with_its_key_and_a_client_waits_for_its_replay() {
        let mut carried = Carried::new([(1, attach_request(7)), (2, AttachRequest { key: None, ..attach_request(8) })]);
        // The keyless one, from a core that predates keys, by its id.
        let (again, gone) = carried.decide(&[entry(7, "k7", true), entry(8, "", true)]);
        assert!(again.is_empty());
        assert!(gone.is_empty(), "{gone:?}");
        for req in [1, 2] {
            assert_eq!(carried.awaits(req), Some(Awaits::Replay));
            assert_eq!(carried.requests[&req].sent, Sent::Carried, "still decided by the next listing");
        }
    }

    /// One the core lists with its key and no client (it never got the
    /// `ATTACH`) is sent again as it was, with no size to tell.
    #[test]
    fn a_carried_attach_listed_with_its_key_and_no_client_is_sent_again() {
        let mut carried = Carried::new([(1, attach_request(7))]);
        let (again, gone) = carried.decide(&[listed_at(7, "k7", Some((132, 43)))]);
        assert!(gone.is_empty(), "{gone:?}");
        let [(attach, told)] = <[_; 1]>::try_from(again).ok().expect("one attach");
        assert_eq!(attach, client::Attach { req: 1, session: 7, cols: 100, rows: 30 });
        assert!(told.is_none());
        assert_eq!(carried.awaits(1), Some(Awaits::Replay));
        assert_eq!(carried.requests[&1].sent, Sent::Queued);
    }

    /// Its id is another session's now: never answered with that one.
    #[test]
    fn a_carried_attach_whose_id_has_another_key_fails() {
        let mut carried = Carried::new([(1, attach_request(7)), (2, attach_request(8))]);
        let (again, gone) = carried.decide(&[entry(7, "another", true), entry(8, "", false)]);
        assert!(again.is_empty());
        assert_eq!(
            gone,
            [
                (1, "the terminal server no longer has session 7".to_string()),
                (2, "the terminal server no longer has session 8".to_string()),
            ]
        );
        assert!(carried.requests.is_empty());
    }

    #[test]
    fn a_carried_attach_not_listed_fails() {
        let mut carried = Carried::new([(1, attach_request(7)), (2, AttachRequest { key: None, ..attach_request(9) })]);
        let (again, gone) = carried.decide(&[entry(8, "k7", false)]);
        assert!(again.is_empty());
        assert_eq!(
            gone,
            [
                (1, "the terminal server no longer has session 7".to_string()),
                (2, "the terminal server no longer has session 9".to_string()),
            ]
        );
    }

    /// A request's kind, for assertions.
    fn kind(waiting: &Waiting) -> &'static str {
        match waiting {
            Waiting::Spawn { .. } => "spawn",
            Waiting::Attach { .. } => "attach",
            Waiting::List(_) => "list",
            Waiting::Done(_) => "done",
        }
    }

    /// A spawn's request, the sink it shows its session with, and where its
    /// answer goes.
    fn spawn_waiting() -> (Waiting, Sink, mpsc::Receiver<std::result::Result<Started, String>>) {
        let (sink, _tab) = fake::sink();
        let (reply, answer) = mpsc::sync_channel(1);
        (Waiting::Spawn { sink: sink.clone(), reply }, sink, answer)
    }

    fn list_waiting() -> Waiting {
        let (reply, _answer) = mpsc::sync_channel(1);
        Waiting::List(reply)
    }

    fn done_waiting() -> Waiting {
        let (reply, _answer) = mpsc::sync_channel(1);
        Waiting::Done(reply)
    }

    /// `request`'s attach under `req`, and the sink it shows its session
    /// with.
    fn attach_waiting(req: RequestId, request: AttachRequest) -> (Waiting, Sink) {
        let (sink, _tab) = fake::sink();
        let (reply, _answer) = mpsc::sync_channel(1);
        let (waiting, _message) = request.waiting(req, sink.clone(), reply);
        (waiting, sink)
    }

    /// The requests waiting, by id: their kinds and where their messages
    /// stand.
    fn requests(state: &State) -> Vec<(RequestId, &'static str, Sent)> {
        let mut requests: Vec<(RequestId, &'static str, Sent)> =
            state.requests.iter().map(|(&req, pending)| (req, kind(&pending.waiting), pending.sent)).collect();
        requests.sort_by_key(|&(req, ..)| req);
        requests
    }

    /// The kinds of `pending`, sorted.
    fn kinds(pending: &[Pending]) -> Vec<&'static str> {
        let mut kinds: Vec<&'static str> = pending.iter().map(|pending| kind(&pending.waiting)).collect();
        kinds.sort_unstable();
        kinds
    }

    fn hook() -> ListingHook {
        Box::new(|_| {})
    }

    /// 4.1: a connection's links are never up, then up, then down, and one
    /// that went down keeps its protocol until the next is up.
    #[test]
    fn a_links_phase_goes_from_never_up_to_up_and_down_keeping_its_protocol() {
        let never = LinkPhase::default();
        assert_eq!(never, LinkPhase::NeverUp);
        assert_eq!((never.is_up(), never.has_been_up(), never.protocol()), (false, false, None));
        assert_eq!(never.down(), LinkPhase::NeverUp, "nothing to lose before the first link");
        let up = LinkPhase::Up(Negotiated(3));
        assert_eq!((up.is_up(), up.has_been_up(), up.protocol()), (true, true, Some(Negotiated(3))));
        let down = up.down();
        assert_eq!(down, LinkPhase::Down(Negotiated(3)));
        assert_eq!((down.is_up(), down.has_been_up(), down.protocol()), (false, true, Some(Negotiated(3))));
        assert_eq!(down.down(), down, "down stays down");
    }

    /// 4.1: `SET_LAYOUT` goes only to a core on client protocol 3 or later,
    /// the current link's or, while it is down, the last one's.
    #[test]
    fn only_client_protocol_3_and_later_takes_set_layout() {
        let sets = |link: LinkPhase| link.protocol().is_some_and(Negotiated::sets_layout);
        assert!(!sets(LinkPhase::NeverUp));
        for (protocol, expected) in [(1, false), (2, false), (3, true), (4, true)] {
            assert_eq!(Negotiated(protocol).sets_layout(), expected, "protocol {protocol}");
            assert_eq!(sets(LinkPhase::Up(Negotiated(protocol))), expected, "up on {protocol}");
            assert_eq!(sets(LinkPhase::Down(Negotiated(protocol))), expected, "down from {protocol}");
        }
    }

    /// 4.2: a request is queued, then written on the link, and a failed
    /// write leaves it queued for the next link. Only a recorded attach
    /// starts carried, writing never carries one, and a carried attach whose
    /// write failed stays carried.
    #[test]
    fn a_requests_message_is_queued_written_or_carried() {
        let (spawn, ..) = spawn_waiting();
        assert_eq!(Pending::queued(spawn).sent, Sent::Queued);
        let (recorded, _) = attach_waiting(1, recorded(5));
        assert_eq!(Pending::recorded(recorded).sent, Sent::Carried);
        let cases = [
            (Sent::Queued, true, Sent::Written),
            (Sent::Queued, false, Sent::Queued),
            (Sent::Written, true, Sent::Written),
            (Sent::Written, false, Sent::Queued),
            (Sent::Carried, true, Sent::Written),
            (Sent::Carried, false, Sent::Carried),
        ];
        for (before, written, after) in cases {
            let mut pending = Pending { waiting: list_waiting(), sent: before };
            pending.mark_written(written);
            assert_eq!(pending.sent, after, "{before:?} marked written {written}");
        }
    }

    /// 4.2, 4.3: a lost link fails the requests written on it and carries the
    /// attaches written on it over to the next listing. Nothing else
    /// changes, and the listing waited for goes with the link.
    #[test]
    fn a_lost_link_fails_written_requests_and_carries_written_attaches() {
        let mut state = State {
            link: LinkPhase::Up(Negotiated(2)),
            list: Some(Listing { req: 99, after_loss: false }),
            ..State::default()
        };
        let (spawn, ..) = spawn_waiting();
        state.requests.insert(1, Pending { waiting: spawn, sent: Sent::Written });
        let (attach, _) = attach_waiting(2, attach_request(7));
        state.requests.insert(2, Pending { waiting: attach, sent: Sent::Written });
        let (spawn, ..) = spawn_waiting();
        state.requests.insert(3, Pending::queued(spawn));
        let (attach, _) = attach_waiting(4, attach_request(8));
        state.requests.insert(4, Pending::queued(attach));
        let (attach, _) = attach_waiting(5, recorded(9));
        state.requests.insert(5, Pending::recorded(attach));
        state.requests.insert(6, Pending { waiting: list_waiting(), sent: Sent::Written });
        state.requests.insert(7, Pending { waiting: done_waiting(), sent: Sent::Written });

        assert_eq!(kinds(&state.link_lost()), ["done", "list", "spawn"]);
        assert_eq!(
            requests(&state),
            [
                (2, "attach", Sent::Carried),
                (3, "spawn", Sent::Queued),
                (4, "attach", Sent::Queued),
                (5, "attach", Sent::Carried),
            ]
        );
        assert_eq!(state.link, LinkPhase::Down(Negotiated(2)));
        assert!(state.list.is_none(), "the listing waited for went with the link");
        // Nothing was written since: a second loss changes nothing.
        assert!(state.link_lost().is_empty());
        assert_eq!(requests(&state).len(), 4);
    }

    /// 4.3: before the first link, a failed attempt fails new tabs' requests
    /// and tells each waiting attach, recorded or not, once; it keeps trying
    /// while one waits.
    #[test]
    fn a_failed_first_attempt_fails_new_tabs_and_tells_attaches_once() {
        let message = "can't connect to A8: boom";
        let mut state = State { hurry: true, ..State::default() };
        let (spawn, ..) = spawn_waiting();
        state.requests.insert(1, Pending::queued(spawn));
        state.requests.insert(2, Pending::queued(list_waiting()));
        state.requests.insert(3, Pending::queued(done_waiting()));
        let (now, now_sink) = attach_waiting(4, attach_request(7));
        state.requests.insert(4, Pending::queued(now));
        let (recorded, recorded_sink) = attach_waiting(5, recorded(8));
        state.requests.insert(5, Pending::recorded(recorded));

        let ConnectFailed { failed, told, next } = state.connect_failed(false, false, message);
        assert_eq!(kinds(&failed), ["done", "list", "spawn"]);
        let mut sessions: Vec<SessionId> = told.iter().map(|&(session, _)| session).collect();
        sessions.sort_unstable();
        assert_eq!(sessions, [7, 8]);
        assert!(same_sinks(told, &[&now_sink, &recorded_sink]));
        assert_eq!(next, Next::Retry);
        assert!(!state.hurry, "this attempt answered the tabs that hurried it");
        assert_eq!(state.ended, None);
        assert_eq!(requests(&state), [(4, "attach", Sent::Queued), (5, "attach", Sent::Carried)]);

        // Once is enough.
        let again = state.connect_failed(false, false, message);
        assert!(again.failed.is_empty() && again.told.is_empty());
        assert_eq!(again.next, Next::Retry);
    }

    /// 4.3: whether a failed attempt before the first link keeps trying, by
    /// what waits, whether the connection is eager and whether the core
    /// refused Brindle. One that stops has ended, with the message.
    #[test]
    fn a_failed_first_attempt_retries_only_for_an_attach_a_hook_or_eagerness() {
        #[derive(Clone, Copy, Debug)]
        enum Waits {
            Nothing,
            Spawn,
            Attach,
            Hook,
        }
        let cases = [
            (Waits::Nothing, false, false, Next::End),
            (Waits::Spawn, false, false, Next::End),
            (Waits::Attach, false, false, Next::Retry),
            (Waits::Hook, false, false, Next::Retry),
            (Waits::Nothing, true, false, Next::Retry),
            (Waits::Spawn, true, false, Next::Retry),
            // A refusal: only eagerness keeps it trying.
            (Waits::Attach, false, true, Next::End),
            (Waits::Hook, false, true, Next::End),
            (Waits::Attach, true, true, Next::Retry),
        ];
        let message = "can't connect to A8: boom";
        for (waits, eager, refused, expected) in cases {
            let mut state = State::default();
            match waits {
                Waits::Nothing => {}
                Waits::Spawn => {
                    let (spawn, ..) = spawn_waiting();
                    state.requests.insert(1, Pending::queued(spawn));
                }
                Waits::Attach => {
                    let (attach, _) = attach_waiting(1, attach_request(7));
                    state.requests.insert(1, Pending::queued(attach));
                }
                Waits::Hook => state.on_listing.push(hook()),
            }
            let failed = state.connect_failed(refused, eager, message);
            let case = format!("{waits:?}, eager {eager}, refused {refused}");
            assert_eq!(failed.next, expected, "{case}");
            let ended = match expected {
                Next::Retry => None,
                Next::End => Some(message),
            };
            assert_eq!(state.ended.as_deref(), ended, "{case}");
        }
    }

    /// 4.3: once a link has been up, a failed attempt changes nothing and
    /// retries, unless the core refused Brindle: then new tabs' requests
    /// fail with the refusal and it ends unless it is eager. Attaches
    /// carried over the lost link aren't told: their tabs said it was lost.
    #[test]
    fn a_failed_attempt_after_a_link_fails_new_tabs_only_on_a_refusal() {
        let message = "can't connect to A8: the terminal server refused Brindle";
        let after_a_link = || {
            let mut state = State { link: LinkPhase::Down(Negotiated(2)), hurry: true, ..State::default() };
            let (spawn, ..) = spawn_waiting();
            state.requests.insert(1, Pending::queued(spawn));
            let (attach, _) = attach_waiting(2, attach_request(7));
            state.requests.insert(2, Pending { waiting: attach, sent: Sent::Carried });
            state
        };

        let mut state = after_a_link();
        let failed = state.connect_failed(false, false, message);
        assert!(failed.failed.is_empty() && failed.told.is_empty());
        assert_eq!(failed.next, Next::Retry);
        assert!(state.hurry, "untouched");
        assert_eq!(requests(&state), [(1, "spawn", Sent::Queued), (2, "attach", Sent::Carried)]);

        for (eager, expected) in [(false, Next::End), (true, Next::Retry)] {
            let mut state = after_a_link();
            let failed = state.connect_failed(true, eager, message);
            assert_eq!(kinds(&failed.failed), ["spawn"]);
            assert!(failed.told.is_empty(), "eager {eager}");
            assert_eq!(failed.next, expected, "eager {eager}");
            assert_eq!(requests(&state), [(2, "attach", Sent::Carried)]);
        }
    }

    /// 4.3: a relay out of use ends `idle` after the first look that found it
    /// so, as no longer used. One in use, or eager, never does, and its idle
    /// time starts over once it is out of use again. It looks five times per
    /// `idle`, and at least a millisecond apart.
    #[test]
    fn an_unused_relay_ends_idle_after_it_was_last_in_use() {
        let (idle, close_limit) = (Duration::from_secs(60), Duration::from_secs(300));
        let (t0, ms) = (Instant::now(), Duration::from_millis(1));
        let wait = |wait: Duration| IdleStep { given_up: Vec::new(), then: Then::Wait(wait) };
        let look = wait(Duration::from_secs(12));
        let mut state = State::default();
        let step = |state: &mut State, at: Duration, eager: bool| state.idle_step(t0 + at, eager, idle, close_limit, "A8");

        assert_eq!(step(&mut state, Duration::ZERO, false), look);
        assert_eq!(state.idle, Idle::Since { at: t0, given_up: None });
        assert_eq!(step(&mut state, idle - ms, false), look);
        // In use: the time starts over once it isn't.
        let (sink, _tab) = fake::sink();
        state.show(1, Shown::new(sink, None));
        assert_eq!(step(&mut state, idle, false), look);
        assert_eq!(state.idle, Idle::InUse);
        state.sessions.clear();
        let unused = idle + Duration::from_secs(1);
        assert_eq!(step(&mut state, unused, false), look);
        assert_eq!(state.idle, Idle::Since { at: t0 + unused, given_up: None });
        // Eager: in use, whatever uses it.
        assert_eq!(step(&mut state, unused + idle, true), look);
        assert_eq!(state.idle, Idle::InUse);
        let unused = unused + idle + Duration::from_secs(1);
        assert_eq!(step(&mut state, unused, false), look);
        assert_eq!(state.ended, None);
        let why = "the connection was no longer used";
        assert_eq!(step(&mut state, unused + idle, false), IdleStep { given_up: Vec::new(), then: Then::End(why.into()) });
        assert_eq!(state.ended.as_deref(), Some(why));

        // The looks are a millisecond apart at least.
        let mut state = State::default();
        assert_eq!(state.idle_step(t0, false, Duration::from_millis(2), close_limit, "A8"), wait(ms));
    }

    /// 4.3: with no link up, the `CLOSE`s are given up `close_limit` after
    /// the oldest was queued, and that is why an unused relay then ends. The
    /// look comes when they are due, if that is sooner.
    #[test]
    fn closes_given_up_are_why_an_unused_relay_ends() {
        let (idle, close_limit) = (Duration::from_secs(60), Duration::from_secs(300));
        let secs = Duration::from_secs;
        let t0 = Instant::now();
        let mut state = State::default();
        state.closing(7, t0);
        state.closing(3, t0 + secs(10));
        // In use while the closes wait, looking when they are due.
        let step = state.idle_step(t0 + secs(295), false, idle, close_limit, "A8");
        assert_eq!(step, IdleStep { given_up: Vec::new(), then: Then::Wait(secs(5)) });
        let why = "not ending sessions 7, 3 on A8: it couldn't be reached within 5 minutes of the close";
        let step = state.idle_step(t0 + secs(300), false, idle, close_limit, "A8");
        assert_eq!(step, IdleStep { given_up: vec![7, 3], then: Then::Wait(secs(12)) });
        assert_eq!(state.idle, Idle::Since { at: t0 + secs(300), given_up: Some(why.into()) });
        let step = state.idle_step(t0 + secs(360), false, idle, close_limit, "A8");
        assert_eq!(step, IdleStep { given_up: Vec::new(), then: Then::End(why.into()) });
        assert_eq!(state.ended.as_deref(), Some(why));
    }

    /// 4.3: closes given up while the relay is in use, or eager, aren't why
    /// it ends later: it then ends as no longer used.
    #[test]
    fn closes_given_up_while_in_use_are_not_why_it_ends_later() {
        let (idle, close_limit) = (Duration::from_secs(60), Duration::from_secs(300));
        let secs = Duration::from_secs;
        let t0 = Instant::now();
        let why = "not ending session 7 on A8: it couldn't be reached within 5 minutes of the close";
        for eager in [false, true] {
            let mut state = State::default();
            state.closing(7, t0);
            let (sink, _tab) = fake::sink();
            if !eager {
                // Used again after the closes were given up.
                state.show(1, Shown::new(sink, None));
            }
            let step = state.idle_step(t0 + secs(300), eager, idle, close_limit, "A8");
            assert_eq!(step.given_up, [7], "eager {eager}");
            assert_eq!(state.idle, Idle::InUse, "eager {eager}");
            state.sessions.clear();
            state.idle_step(t0 + secs(301), false, idle, close_limit, "A8");
            let step = state.idle_step(t0 + secs(361), false, idle, close_limit, "A8");
            assert_eq!(step.then, Then::End("the connection was no longer used".into()), "eager {eager}");
        }
        // Given up while unused, then used again: forgotten too.
        let mut state = State::default();
        state.closing(7, t0);
        state.idle_step(t0 + secs(300), false, idle, close_limit, "A8");
        assert_eq!(state.idle, Idle::Since { at: t0 + secs(300), given_up: Some(why.into()) });
        let (sink, _tab) = fake::sink();
        state.show(1, Shown::new(sink, None));
        state.idle_step(t0 + secs(310), false, idle, close_limit, "A8");
        state.sessions.clear();
        state.idle_step(t0 + secs(320), false, idle, close_limit, "A8");
        let step = state.idle_step(t0 + secs(380), false, idle, close_limit, "A8");
        assert_eq!(step.then, Then::End("the connection was no longer used".into()));
    }

    /// 4.3: an eager relay that nothing waits on pauses up to its eager cap
    /// between attempts; any other, up to the retry cap. A listing hook or a
    /// close isn't waited on.
    #[test]
    fn only_an_eager_relay_nothing_waits_on_pauses_up_to_the_eager_cap() {
        let timings = fake::quick();
        let cases = [
            (false, false, timings.retry_cap),
            (false, true, timings.retry_cap),
            (true, false, timings.retry_cap),
            (true, true, timings.eager_cap),
        ];
        for (eager, nothing_waits, cap) in cases {
            assert_eq!(retry_cap(eager, nothing_waits, &timings), cap, "eager {eager}, nothing waits {nothing_waits}");
        }
        let mut state = State::default();
        state.on_listing.push(hook());
        state.closing(1, Instant::now());
        assert!(state.nothing_waits());
        state.requests.insert(1, Pending::queued(list_waiting()));
        assert!(!state.nothing_waits());
        state.requests.clear();
        let (sink, _tab) = fake::sink();
        state.show(1, Shown::new(sink, None));
        assert!(!state.nothing_waits());
    }

    /// 4.3: `SPAWNED` for a spawn that waits shows its session with the
    /// spawn's sink and the key decoded, displacing what had its id. Else
    /// the session is closed, and a request of another kind under its `req`
    /// fails.
    #[test]
    fn spawned_is_shown_for_its_spawn_and_closed_otherwise() {
        let mut state = State::default();
        let (old, _old_tab) = fake::sink();
        state.show(11, Shown::new(old.clone(), Some(key("old"))));
        let (spawn, sink, _answer) = spawn_waiting();
        state.requests.insert(1, Pending { waiting: spawn, sent: Sent::Written });
        let spawned = client::Spawned { req: 1, session: 11, pid: 1011, key: "k11".into() };
        let Routed::Shown { sink: shown, answer, displaced, .. } = state.route_spawned(&spawned) else {
            panic!("not shown")
        };
        assert!(Arc::ptr_eq(&shown, &sink));
        assert_eq!(answer, Started { session: 11, pid: 1011, key: Some(key("k11")) });
        assert!(displaced.is_some_and(|displaced| Arc::ptr_eq(&displaced.sink, &old)));
        assert!(state.shows(11, Some(&key("k11"))));
        assert!(state.requests.is_empty());

        // A core that predates keys sends none.
        let (spawn, ..) = spawn_waiting();
        state.requests.insert(2, Pending::queued(spawn));
        let spawned = client::Spawned { req: 2, session: 12, pid: 1012, key: String::new() };
        let Routed::Shown { answer, displaced, .. } = state.route_spawned(&spawned) else { panic!("not shown") };
        assert_eq!((answer.key, displaced.is_none()), (None, true));
        assert!(state.shows(12, None));

        // Its spawner gave up.
        let spawned = client::Spawned { req: 3, session: 13, pid: 1013, key: "k13".into() };
        let Routed::Close { session: 13, fail: None } = state.route_spawned(&spawned) else { panic!("not closed") };
        // Another kind of request under its `req`.
        state.requests.insert(4, Pending::queued(list_waiting()));
        let spawned = client::Spawned { req: 4, session: 14, pid: 1014, key: "k14".into() };
        let Routed::Close { session, fail: Some((waiting, why)) } = state.route_spawned(&spawned) else {
            panic!("not closed with a failure")
        };
        assert_eq!((session, kind(&waiting), why), (14, "list", "answered with SPAWNED"));
        assert!(state.requests.is_empty());
        assert_eq!(shown_ids(&state), [11, 12]);
    }

    /// 4.3: the `REPLAY_START` answering an attach that waits shows the
    /// session with the attach's sink and key. Else the session is left
    /// detached, and a request of another kind under its `req` fails.
    #[test]
    fn a_replay_start_is_shown_for_its_attach_and_detached_otherwise() {
        let mut state = State::default();
        let (attach, sink) = attach_waiting(1, attach_request(7));
        state.requests.insert(1, Pending { waiting: attach, sent: Sent::Written });
        let Routed::Shown { sink: shown, answer, displaced, .. } = state.route_replay_start(1, 7, 99) else {
            panic!("not shown")
        };
        assert!(Arc::ptr_eq(&shown, &sink));
        assert_eq!(answer, 99);
        assert!(displaced.is_none());
        assert!(state.shows(7, Some(&key("k7"))), "with the attach's key");

        // Its attacher gave up.
        let Routed::Detach { session: 8, fail: None } = state.route_replay_start(2, 8, 100) else {
            panic!("not detached")
        };
        // Another kind of request under its `req`.
        let (spawn, ..) = spawn_waiting();
        state.requests.insert(3, Pending::queued(spawn));
        let Routed::Detach { session, fail: Some((waiting, why)) } = state.route_replay_start(3, 9, 101) else {
            panic!("not detached with a failure")
        };
        assert_eq!((session, kind(&waiting), why), (9, "spawn", "answered with REPLAY_START"));
        assert!(state.requests.is_empty());
        assert_eq!(shown_ids(&state), [7]);
    }

    /// 4.4: a connection is in use while it shows a session, a request
    /// waits, a listing hook waits, or a `CLOSE` isn't written.
    #[test]
    fn a_connection_is_in_use_while_anything_needs_it() {
        assert!(!State::default().in_use());
        let mut shown = State::default();
        let (sink, _tab) = fake::sink();
        shown.show(1, Shown::new(sink, None));
        let mut requested = State::default();
        requested.requests.insert(1, Pending::queued(list_waiting()));
        let mut hooked = State::default();
        hooked.on_listing.push(hook());
        let mut closing = State::default();
        closing.closing(1, Instant::now());
        for (what, state) in [("shown", shown), ("requested", requested), ("hooked", hooked), ("closing", closing)] {
            assert!(state.in_use(), "{what}");
        }
    }

    /// 4.4: the `CLOSE`s are due `limit` after the oldest was queued, while
    /// no link is up.
    #[test]
    fn closes_are_due_after_the_oldest_only_with_no_link_up() {
        let (limit, queued) = (Duration::from_secs(300), Instant::now());
        let mut state = State::default();
        assert_eq!(state.closes_due(limit), None, "no close");
        state.closing(7, queued);
        state.closing(3, queued + Duration::from_secs(10));
        let cases = [
            (LinkPhase::NeverUp, Some(queued + limit)),
            (LinkPhase::Up(Negotiated(2)), None),
            (LinkPhase::Down(Negotiated(2)), Some(queued + limit)),
        ];
        for (link, due) in cases {
            state.link = link;
            assert_eq!(state.closes_due(limit), due, "{link:?}");
        }
    }

    /// 4.4: a recorded attach not sent yet and a listing hook wait for the
    /// core's listing; no other request does.
    #[test]
    fn only_recorded_attaches_and_listing_hooks_wait_for_a_listing() {
        let mut state = State::default();
        assert!(!state.waits_for_listing());
        let (spawn, ..) = spawn_waiting();
        state.requests.insert(1, Pending::queued(spawn));
        let (attach, _) = attach_waiting(2, attach_request(7));
        state.requests.insert(2, Pending::queued(attach));
        state.requests.insert(3, Pending::queued(list_waiting()));
        state.requests.insert(4, Pending::queued(done_waiting()));
        assert!(!state.waits_for_listing());
        let (recorded, _) = attach_waiting(5, recorded(8));
        state.requests.insert(5, Pending::recorded(recorded));
        assert!(state.waits_for_listing());
        state.requests.remove(&5);
        state.on_listing.push(hook());
        assert!(state.waits_for_listing());
    }

    /// 4.4: a stamp's deadline is `limit` after its last touch, in whole
    /// milliseconds from its epoch, rounded down; a held one has none until
    /// the next touch, and one too far for its milliseconds is the furthest
    /// short of held. It is expired only once the watchdog says so.
    #[test]
    fn a_stamps_deadline_follows_its_touches_and_holds() {
        let secs = Duration::from_secs;
        let epoch = Instant::now();
        let stamp = Stamp::new(epoch, secs(5));
        assert_eq!(stamp.deadline(), Some(epoch + secs(5)));
        stamp.touch(epoch + secs(2), secs(30));
        assert_eq!(stamp.deadline(), Some(epoch + secs(32)));
        stamp.touch(epoch + Duration::from_micros(1500), Duration::ZERO);
        assert_eq!(stamp.deadline(), Some(epoch + Duration::from_millis(1)));
        stamp.hold();
        assert_eq!(stamp.deadline(), None);
        stamp.touch(epoch + secs(3), secs(1));
        assert_eq!(stamp.deadline(), Some(epoch + secs(4)));
        stamp.touch(epoch, secs(u64::MAX / 2));
        assert_eq!(stamp.deadline(), Some(epoch + Duration::from_millis(Stamp::HELD - 1)));
        assert!(!stamp.expired());
        stamp.expire();
        assert!(stamp.expired());
    }
}
