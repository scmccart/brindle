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
//!   sessions it doesn't have: those are lost, and their tabs say so.
//!   Requests written on a link that ended fail, since their answers are
//!   gone, except attaches (restored zones), which stay pending: the
//!   reconnect's replay of the session answers one, and one whose session
//!   the core lists without a client is sent again. The others wait for
//!   the next link.
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
//! - **Listings:** a session a restored zone recorded ([`AttachRequest`]'s
//!   `recorded`) and a launch's [`Connection::on_listing`] both wait for the
//!   core's `LIST`, sent on the first link when either waits (and at once
//!   when the link is up). The listing must have the recorded session with
//!   its key and no client; then its `ATTACH` goes out at the listed size,
//!   which its replay is rendered at. Both keep a relay connecting, as an
//!   attach does; a hook only until it is withdrawn.
//!
//! The reader holds the state lock only to look a sink up; each sink has
//! its own lock, taken without the state lock, so parsing never waits on
//! the map and closing a tab never waits on parsing.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use parking_lot::{Condvar, Mutex};

use super::link::{Link, Wire};
use super::relay::{self, Dial, Timings};
use super::{Connected, Endpoint, Refused};
use crate::remote::protocol::client::{
    self, Data, Kill, KillTarget, Policy, Reparent, ReplayStart, Request, Resize, SessionInfo,
    Detach, SessionRef, Spawned, ToClient, ToCore,
};
use crate::remote::hosts::LinkState;
use crate::remote::protocol::relay::{RelayHost, RelayMessage, is_relay_type};
use crate::remote::protocol::{Frame, RequestId, SessionId};

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

/// Where a relay connection tells its changes of state: its id (each
/// connection's own, increasing) and its new state. Called with the state
/// lock held, so it must not block.
pub type Report = Arc<dyn Fn(u64, LinkState) + Send + Sync>;

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

/// Where a session's output and events go: its terminal.
pub trait SessionSink: Send {
    /// Output from the program, or from a replay.
    fn output(&mut self, bytes: &[u8]);
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

/// What a new session runs.
pub struct SpawnRequest {
    pub command: String,
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

/// Which detached session to attach, and the tab's size.
pub struct AttachRequest {
    pub session: SessionId,
    /// The key the session was listed with, kept for the tab's state.
    pub key: String,
    pub cols: u16,
    pub rows: u16,
    /// A session a restored zone recorded, not one just listed: the core's
    /// `LIST` must still have it, with this key and no client, before its
    /// `ATTACH` goes out, at the size listed instead of `cols` and `rows`.
    pub recorded: bool,
}

type Reply<T> = mpsc::SyncSender<Result<T, String>>;

/// What a request waits for.
enum Waiting {
    /// `SPAWNED`, whose sink then joins the session map.
    Spawn { sink: Sink, reply: Reply<Spawned> },
    /// The `REPLAY_START` answering an `ATTACH` (its pid), whose sink joins
    /// the map before the replay's first `DATA` is read. A reconnect's
    /// replay of the session answers it too, and `attach` is sent again
    /// when the reconnected core didn't give the session back. `check` is
    /// a recorded session's key, until a listing has confirmed it: until
    /// then nothing is sent.
    Attach { sink: Sink, reply: Reply<u32>, attach: client::Attach, check: Option<String> },
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

struct Pending {
    waiting: Waiting,
    /// Its request went out on the current link: if that ends, the answer
    /// is lost with it.
    written: bool,
    /// An attach waiting for a relay's first link was told why it can't
    /// connect yet.
    told: bool,
    /// An attach that went out on a link that ended before its answer, or
    /// a recorded one not sent yet. The next `LIST` says what becomes of
    /// it.
    carried: bool,
}

impl Pending {
    fn is_attach(&self) -> bool {
        matches!(self.waiting, Waiting::Attach { .. })
    }

    /// A recorded attach that waits for a listing to confirm its session.
    fn is_check(&self) -> bool {
        matches!(self.waiting, Waiting::Attach { check: Some(_), .. })
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

#[derive(Default)]
struct State {
    sessions: HashMap<SessionId, Sink>,
    /// The window each session's console is parented to (Windows), sent
    /// again after a reconnect's replay: a core doesn't keep it.
    parents: HashMap<SessionId, u64>,
    /// The last `PROCESS` of each session shown.
    foreground: HashMap<SessionId, Foreground>,
    requests: HashMap<RequestId, Pending>,
    /// The `LIST` sent after a link's `WELCOME`, or for a recorded attach
    /// or a listing hook.
    list: Option<Listing>,
    /// What the next listing goes to ([`Connection::on_listing`]).
    on_listing: Vec<ListingHook>,
    /// A link has been up.
    connected: bool,
    /// A link is up now.
    linked: bool,
    /// A new tab's request wants a relay attempt now, not after the pause.
    hurry: bool,
    /// The sessions whose `CLOSE`s went to the writer and aren't written
    /// yet, with when each was queued, oldest first. The writer skips one
    /// that is no longer here: it was given up ([`State::give_up_closes`]).
    closing: Vec<(SessionId, Instant)>,
    /// Since when a relay connection has been out of use ([`State::in_use`]).
    idle_since: Option<Instant>,
    /// The connection ended for good, and why; the next tab makes a new one.
    ended: Option<String>,
    /// Run once when it ends ([`Connection::on_end`]).
    on_end: Option<Box<dyn FnOnce() + Send>>,
    /// What was last reported ([`Connection::report`]).
    reported: Option<LinkState>,
}

impl State {
    /// Stops showing `session`: its sink, window and foreground program go.
    fn remove(&mut self, session: SessionId) -> Option<Sink> {
        self.parents.remove(&session);
        self.foreground.remove(&session);
        self.sessions.remove(&session)
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

    /// Queues `session`'s `CLOSE`, which keeps the connection in use.
    fn closing(&mut self, session: SessionId) {
        self.closing.push((session, Instant::now()));
    }

    /// The `CLOSE`s given up, taken out: all of them once the oldest was
    /// queued `limit` ago with no link up, else none.
    fn give_up_closes(&mut self, limit: Duration) -> Vec<SessionId> {
        match self.closing.first() {
            Some(&(_, queued)) if !self.linked && queued.elapsed() >= limit => {
                self.closing.drain(..).map(|(session, _)| session).collect()
            }
            _ => Vec::new(),
        }
    }

    /// When the `CLOSE`s are given up if no link comes up first.
    fn closes_due(&self, limit: Duration) -> Option<Instant> {
        let (_, queued) = self.closing.first().filter(|_| !self.linked)?;
        Some(*queued + limit)
    }

    /// A recorded attach or a listing hook waits for the core's `LIST`.
    fn waits_for_listing(&self) -> bool {
        !self.on_listing.is_empty() || self.requests.values().any(Pending::is_check)
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

    fn new(limit: Duration) -> Self {
        let stamp = Self { epoch: Instant::now(), deadline: AtomicU64::new(0), expired: AtomicBool::new(false) };
        stamp.touch(limit);
        stamp
    }

    /// Dead if nothing more is read within `limit`.
    fn touch(&self, limit: Duration) {
        let at = (self.epoch.elapsed() + limit).as_millis();
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
    outgoing: mpsc::Sender<ToCore>,
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
    /// This connection's own, for its reports.
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
    fn new(transport: Transport, token: &str) -> (Arc<Self>, mpsc::Receiver<ToCore>) {
        Self::made(transport, token, false, None)
    }

    fn made(
        transport: Transport,
        token: &str,
        eager: bool,
        report: Option<Report>,
    ) -> (Arc<Self>, mpsc::Receiver<ToCore>) {
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
            id: NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed),
            report,
        });
        (this, queue)
    }

    /// Starts the reader and writer threads on a negotiated local connection.
    pub fn start(endpoint: Endpoint, token: &str, connected: Connected) -> Result<Arc<Self>> {
        let link = Link::local(connected.stream)?;
        let (this, queue) = Self::new(Transport::Local(endpoint), token);
        this.up(link.wire, None)?;
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

    /// Takes the connection for another terminal; false once it has ended.
    /// A relay out of use starts its idle time over, so it can't end
    /// between this and the terminal's request.
    pub(super) fn reuse(&self) -> bool {
        let mut state = self.state.lock();
        if state.ended.is_some() {
            return false;
        }
        if state.idle_since.is_some() {
            state.idle_since = Some(Instant::now());
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
    /// in use ([`State::in_use`]), unless it is eager. With no link up, it
    /// gives the `CLOSE`s not written up `close_limit` after the oldest was
    /// queued, eager or not, and logs the sessions left running; that is
    /// then the end's reason, unless the connection is used again first. It
    /// looks five times per `idle`, and when the closes are due, so it ends
    /// at most a fifth of `idle` late.
    fn end_when_unused(&self, idle: Duration, close_limit: Duration) {
        let look = (idle / 5).max(Duration::from_millis(1));
        let mut given_up: Option<String> = None;
        let mut state = self.state.lock();
        loop {
            if state.ended.is_some() {
                return;
            }
            let sessions = state.give_up_closes(close_limit);
            if !sessions.is_empty() {
                let text = not_ending(&sessions, &self.name(), close_limit);
                log::warn!("{text}");
                given_up = Some(text);
            }
            if self.eager.load(Ordering::Acquire) || state.in_use() {
                state.idle_since = None;
                given_up = None;
            } else if state.idle_since.get_or_insert_with(Instant::now).elapsed() >= idle {
                // Under the lock, so no request slips in to wait on it.
                let reason = given_up.take().unwrap_or_else(|| "the connection was no longer used".to_string());
                state.ended = Some(reason.clone());
                drop(state);
                log::info!("closing the unused connection to {}", self.name());
                return self.ended(&reason);
            }
            let due = state.closes_due(close_limit).map(|due| due.saturating_duration_since(Instant::now()));
            let wait = due.map_or(look, |due| due.min(look)).max(Duration::from_millis(1));
            self.wake.wait_for(&mut state, wait);
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
        self.sent.fetch_add(1, Ordering::AcqRel);
        self.outgoing.send(message).ok();
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
        self.state.lock().closing(session);
        self.send(ToCore::Close(SessionRef { session }));
    }

    /// Stops showing `session`; says whether it was shown. With `closing`,
    /// the `CLOSE` the caller sends next is counted under the same lock,
    /// so the connection is never out of use in between.
    fn forget(&self, session: SessionId, closing: bool) -> bool {
        let mut state = self.state.lock();
        let shown = state.remove(session).is_some();
        if shown && closing {
            state.closing(session);
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
            if self.is_relay() && !state.connected && !matches!(waiting, Waiting::Attach { .. }) {
                state.hurry = true;
                self.wake.notify_all();
            }
            let listed = message.is_none();
            state.requests.insert(req, Pending { waiting, written: false, told: false, carried: listed });
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
                Some(pending) if !pending.written => state.requests.remove(&req),
                _ => None,
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
        let command = Some(request.command).filter(|command| !command.is_empty());
        let abandon = abandon.map(|abandon| (Arc::downgrade(self), abandon));
        let spawned = self.ask("start the session", abandon, None, |req, reply| {
            let message = Some(ToCore::Spawn(client::Spawn {
                req,
                command,
                args: request.args,
                cwd: request.cwd,
                env: request.env,
                cols: request.cols,
                rows: request.rows,
                cell_w: request.cell_w,
                cell_h: request.cell_h,
                scrollback: request.scrollback,
                policy: request.policy,
                profile: request.profile,
            }));
            (Waiting::Spawn { sink, reply }, message)
        })?;
        Ok(Running { id: spawned.session, pid: spawned.pid, key: spawned.key, conn: self.clone() })
    }

    /// Attaches a detached session, whose replay and then live output go to
    /// `sink`, and waits for its `REPLAY_START`. A recorded session is
    /// looked up in the core's listing first. Not on the UI thread.
    pub fn attach(self: &Arc<Self>, request: AttachRequest, sink: Sink, abandon: Option<&Abandon>) -> Result<Running> {
        let session = request.session;
        let abandon = abandon.map(|abandon| (Arc::downgrade(self), abandon));
        let check = request.recorded.then(|| request.key.clone());
        let pid = self.ask("attach the session", abandon, None, |req, reply| {
            let attach = client::Attach { req, session, cols: request.cols, rows: request.rows };
            let message = check.is_none().then(|| ToCore::Attach(attach.clone()));
            (Waiting::Attach { sink, reply, attach, check }, message)
        })?;
        Ok(Running { id: session, pid, key: request.key, conn: self.clone() })
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
        if !state.connected || state.list.is_some() {
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
                    let reason = relay_link_ended(&name, timings.dead, stamp.expired(), &ending, || wire.why());
                    log::warn!("connection to {name} ended ({reason}); reconnecting");
                    self.disconnected(&reason);
                    self.lost_link(&name);
                    self.report_now(LinkState::Offline(format!("connection lost: {reason}")));
                }
                Err(_) if self.is_dead() => return,
                Err(error) => {
                    let message = format!("can't connect to {name}: {error:#}");
                    log::warn!("{message}");
                    let refused = error.downcast_ref::<Refused>().is_some();
                    if !self.failed_to_connect(&message, refused) {
                        return self.ended(&message);
                    }
                    self.report_now(LinkState::Offline(format!("{error:#}")));
                    failures += 1;
                }
            }
            let cap = self.retry_cap(timings);
            if !self.pause(relay::backoff(failures, timings.first_retry, cap)) {
                return;
            }
        }
    }

    /// Starts a relay and greets the core behind it, with the watchdog
    /// ending a link that takes longer than [`Timings::bootstrap`].
    fn open_relay(
        &self,
        dial: &dyn Dial,
        timings: &Timings,
    ) -> Result<(BufReader<Box<dyn Read + Send>>, Arc<Stamp>, Arc<dyn Wire>)> {
        let Link { reader, wire } = dial.launch()?;
        let stamp = Arc::new(Stamp::new(timings.bootstrap));
        self.watch_link(&wire, &stamp);
        let mut reader = BufReader::with_capacity(READ_BUFFER, reader);
        let greeted = self.greet(&dial.name(), &mut reader, wire.as_ref());
        match greeted.and_then(|host| self.up(wire.clone(), Some(host))) {
            Ok(()) => {
                // Sooner than the bootstrap's deadline the watchdog waits for.
                stamp.touch(timings.dead);
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
    fn greet(&self, name: &str, reader: &mut impl BufRead, wire: &dyn Wire) -> Result<RelayHost> {
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
                    return Ok(host);
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
    fn up(&self, wire: Arc<dyn Wire>, host: Option<RelayHost>) -> Result<()> {
        let list = {
            let mut state = self.state.lock();
            if let Some(reason) = &state.ended {
                // It ended while this link was starting.
                bail!("{reason}");
            }
            let after_loss = state.connected;
            let wanted = after_loss || state.waits_for_listing();
            // Under the same lock: what registers after this asks itself.
            state.connected = true;
            state.linked = true;
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
                stamp.touch(dead);
            }
        }
    }

    fn handle(&self, message: ToClient) {
        match message {
            ToClient::Data(Data { session, bytes }) => {
                if let Some(sink) = self.sink(session) {
                    sink.lock().output(&bytes);
                }
            }
            ToClient::Spawned(spawned) => {
                let mut state = self.state.lock();
                match state.requests.remove(&spawned.req).map(|pending| pending.waiting) {
                    Some(Waiting::Spawn { sink, reply }) => {
                        state.sessions.insert(spawned.session, sink);
                        reply.try_send(Ok(spawned)).ok();
                    }
                    other => {
                        // Its spawner gave up: nobody shows this session.
                        drop(state);
                        if let Some(other) = other {
                            other.fail("answered with SPAWNED".into());
                        }
                        self.send_close(spawned.session);
                    }
                }
            }
            ToClient::Error(error) => {
                let pending = error.req.and_then(|req| self.state.lock().requests.remove(&req));
                match pending {
                    Some(pending) => pending.waiting.fail(error.message),
                    None => log::debug!("terminal server: {}", error.message),
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
                let mut state = self.state.lock();
                match state.requests.remove(&req).map(|pending| pending.waiting) {
                    Some(Waiting::Attach { sink, reply, .. }) => {
                        state.sessions.insert(session, sink.clone());
                        drop(state);
                        sink.lock().replay(true);
                        reply.try_send(Ok(pid)).ok();
                    }
                    other => {
                        // Its attacher gave up: leave the session detached,
                        // as it was.
                        drop(state);
                        if let Some(other) = other {
                            other.fail("answered with REPLAY_START".into());
                        }
                        self.send(ToCore::Detach(Detach { session, layout: None }));
                    }
                }
            }
            ToClient::Exit(exit) => {
                let sink = self.state.lock().remove(exit.session);
                if let Some(sink) = sink {
                    match exit.lost {
                        Some(reason) => sink.lock().lost(&format!("the terminal server lost it: {reason}")),
                        None => sink.lock().exited(exit.code),
                    }
                }
            }
            ToClient::ReplayStart(start) => {
                let session = start.session;
                if let Some(sink) = self.sink(session) {
                    sink.lock().replay(true);
                } else if let Some((sink, reply)) = self.attached_by_replay(session) {
                    sink.lock().replay(true);
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
                    sink.lock().replay(false);
                }
                // The core that replayed may be a new one, which doesn't
                // know the session's window.
                let parent = self.state.lock().parents.get(&end.session).copied();
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
                    // Only a lost link's sessions can be missing: on a link
                    // that stayed up, every session shown here is listed.
                    let lost = if listing.after_loss {
                        let kept: HashSet<SessionId> =
                            sessions.sessions.iter().filter(|s| s.attached).map(|s| s.session).collect();
                        state.parents.retain(|session, _| kept.contains(session));
                        state.foreground.retain(|session, _| kept.contains(session));
                        missing(&mut state.sessions, &kept)
                    } else {
                        Vec::new()
                    };
                    let decided = carried_attaches(&mut state.requests, &sessions.sessions, &name);
                    (lost, decided, std::mem::take(&mut state.on_listing))
                };
                for sink in lost {
                    sink.lock().lost("the terminal server no longer has this session");
                }
                for (pending, reason) in decided.gone {
                    pending.waiting.fail(reason);
                }
                for (attach, recorded) in decided.again {
                    // Told before the ATTACH goes out, so before its replay.
                    if let Some(sink) = recorded {
                        sink.lock().replay_size(attach.cols, attach.rows);
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
                    let Some(sink) = state.sessions.get(&process.session).cloned() else { return };
                    let foreground = Foreground { name: process.name, cwd: process.cwd };
                    let previous = state.foreground.insert(process.session, foreground.clone());
                    (previous.map(|previous| previous.name) != Some(foreground.name)).then_some(sink)
                };
                if let Some(sink) = renamed {
                    sink.lock().foreground();
                }
            }
            ToClient::Welcome(_) | ToClient::ServerStatus(_) => {}
        }
    }

    fn sink(&self, session: SessionId) -> Option<Sink> {
        self.state.lock().sessions.get(&session).cloned()
    }

    /// A replay without a `req` (a reconnect's) of a session an attach
    /// waits for answers that attach: its sink joins the map. A recorded
    /// attach not sent yet asked the core for nothing, so it isn't one.
    fn attached_by_replay(&self, session: SessionId) -> Option<(Sink, Reply<u32>)> {
        let mut state = self.state.lock();
        let req = state.requests.iter().find_map(|(&req, pending)| match &pending.waiting {
            Waiting::Attach { attach, check: None, .. } if attach.session == session => Some(req),
            _ => None,
        })?;
        let Some(Waiting::Attach { sink, reply, .. }) = state.requests.remove(&req).map(|pending| pending.waiting)
        else {
            unreachable!("an attach")
        };
        state.sessions.insert(session, sink.clone());
        Some((sink, reply))
    }

    /// The link ended: the writer holds its messages, and requests that
    /// went out on it fail, since their answers are gone. Attaches stay,
    /// carried over to the next link.
    fn disconnected(&self, reason: &str) {
        if let Some(current) = self.slot.lock().link.take() {
            current.wire.end();
        }
        self.unwatch_link();
        let failed = {
            let mut state = self.state.lock();
            state.list = None;
            state.linked = false;
            let lost: Vec<RequestId> =
                state.requests.iter().filter(|(_, pending)| pending.written).map(|(&req, _)| req).collect();
            let mut failed = Vec::new();
            for req in lost {
                let Some(pending) = state.requests.get_mut(&req) else { continue };
                if pending.is_attach() {
                    pending.written = false;
                    pending.carried = true;
                } else if let Some(pending) = state.requests.remove(&req) {
                    failed.push(pending);
                }
            }
            failed
        };
        for pending in failed {
            pending.waiting.fail(format!("the terminal server went away ({reason})"));
        }
    }

    /// A relay link was lost: each of the endpoint's terminals says so.
    fn lost_link(&self, name: &str) {
        let sinks: Vec<Sink> = self.state.lock().sessions.values().cloned().collect();
        let line = lost_line(name);
        for sink in sinks {
            sink.lock().output(line.as_bytes());
        }
    }

    /// A relay attempt failed with `message`; `refused` when the core
    /// refused this Brindle ([`Refused`]). Before the first link, new tabs'
    /// requests fail with it, and the attaches that wait on are told it once;
    /// after a refusal, new tabs' requests fail with it after a first link
    /// too, since retrying won't clear it soon. Says whether to keep trying:
    /// after a refusal only while the connection is eager; otherwise always
    /// once a link has been up, and before that while an attach or a listing
    /// hook waits or the connection is eager.
    fn failed_to_connect(&self, message: &str, refused: bool) -> bool {
        let (failed, told, keep) = {
            let mut state = self.state.lock();
            if state.connected && !refused {
                return true;
            }
            let new_tabs: Vec<RequestId> =
                state.requests.iter().filter(|(_, pending)| !pending.is_attach()).map(|(&req, _)| req).collect();
            let failed: Vec<Pending> = new_tabs.into_iter().filter_map(|req| state.requests.remove(&req)).collect();
            // This attempt answered them: only a later tab hurries the next.
            state.hurry = false;
            // Attaches carried over a lost link aren't told: their tabs say
            // the link was lost.
            let first = !state.connected;
            let told: Vec<Sink> = state
                .requests
                .values_mut()
                .filter(|_| first)
                .filter_map(|pending| match &pending.waiting {
                    Waiting::Attach { sink, .. } if !pending.told => {
                        pending.told = true;
                        Some(sink.clone())
                    }
                    _ => None,
                })
                .collect();
            let eager = self.eager.load(Ordering::Acquire);
            let keep = eager
                || (!refused && (state.requests.values().any(Pending::is_attach) || !state.on_listing.is_empty()));
            if !keep {
                // Under the same lock, so no attach slips in to wait on a
                // connection that has stopped trying.
                state.ended = Some(message.to_string());
            }
            (failed, told, keep)
        };
        for pending in failed {
            pending.waiting.fail(message.to_string());
        }
        let line = retrying_line(message);
        for sink in told {
            sink.lock().output(line.as_bytes());
        }
        keep
    }

    /// The longest pause between relay attempts: longer for an eager
    /// connection that nothing waits on.
    fn retry_cap(&self, timings: &Timings) -> Duration {
        let idle = {
            let state = self.state.lock();
            state.sessions.is_empty() && state.requests.is_empty()
        };
        if self.eager.load(Ordering::Acquire) && idle { timings.eager_cap } else { timings.retry_cap }
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
        loop {
            let elapsed = started.elapsed();
            if elapsed >= RECONNECT_LIMIT {
                return Err(format!("no terminal server came back within {} s", RECONNECT_LIMIT.as_secs()));
            }
            match super::try_connect(endpoint) {
                Ok(Some(stream)) => match super::hello(stream, &self.token) {
                    Ok(connected) => return self.resume_local(connected).map_err(|error| format!("{error:#}")),
                    Err(error) if error.downcast_ref::<Refused>().is_some() => return Err(format!("{error:#}")),
                    Err(error) => log::debug!("reconnecting: {error:#}"),
                },
                Ok(None) if elapsed >= next_start => {
                    next_start += START_AFTER;
                    if let Err(error) = super::start_core(endpoint) {
                        log::warn!("reconnecting: {error:#}");
                    }
                    continue;
                }
                Ok(None) => {}
                Err(error) => log::debug!("reconnecting: {error:#}"),
            }
            std::thread::sleep(RECONNECT_POLL);
        }
    }

    fn resume_local(&self, connected: Connected) -> Result<Box<dyn Read + Send>> {
        let Connected { stream, welcome } = connected;
        log::info!("reconnected to the terminal server (core pid {})", welcome.pid);
        let link = Link::local(stream)?;
        self.up(link.wire, None)?;
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
            state.parents.clear();
            state.foreground.clear();
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
        for sink in sessions.into_values() {
            sink.lock().lost(reason);
        }
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Sends queued messages in order, and on a relay a heartbeat after
    /// [`Timings::heartbeat`] without a write.
    fn write_loop(&self, queue: mpsc::Receiver<ToCore>) {
        let heartbeat = match &self.transport {
            Transport::Relay { timings, .. } => Some(timings.heartbeat),
            Transport::Local(_) => None,
        };
        let mut last = Instant::now();
        loop {
            let message = match heartbeat {
                None => match queue.recv() {
                    Ok(message) => message,
                    Err(_) => return,
                },
                Some(after) => match queue.recv_timeout(after.saturating_sub(last.elapsed())) {
                    Ok(message) => message,
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
            let open = self.write(&message);
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
                    if let Some(req) = request
                        && let Some(pending) = self.state.lock().requests.remove(&req)
                    {
                        pending.waiting.fail(format!("the request can't be sent: {error}"));
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
                pending.written = written;
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

/// Why a relay link ended, in the order that tells the most (design
/// decision 2): the watchdog ended it for silence (`expired`; the process
/// was killed, so what it says would only report that); else what the relay's
/// process says (`why`, asked only then, since it may wait for the exit: its
/// status and what it wrote to stderr); else how the stream ended. A
/// process that exited 0 with nothing to say is a relay that closed the
/// connection, as it does when its core connection closes.
fn relay_link_ended(
    name: &str,
    dead: Duration,
    expired: bool,
    ending: &Ending,
    why: impl FnOnce() -> String,
) -> String {
    if expired {
        return format!("nothing read from {name} for {}", span_text(dead));
    }
    let why = why();
    if !why.is_empty() {
        return why;
    }
    match ending {
        Ending::Closed => "the relay closed the connection".to_string(),
        Ending::Failed(error) => format!("reading from the relay failed: {error}"),
    }
}

/// The next frame of a relay link that is starting; its end says why.
fn next_frame(reader: &mut impl BufRead, wire: &dyn Wire) -> Result<Frame> {
    match Frame::read_from_or_eof(reader) {
        Ok(Some(frame)) => Ok(frame),
        Ok(None) => {
            let why = wire.why();
            if why.is_empty() {
                bail!("the relay ended before the terminal server answered");
            }
            bail!("the relay ended before the terminal server answered: {why}")
        }
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

/// What a relay's loss writes into each of its terminals: one dim line.
fn lost_line(name: &str) -> String {
    format!("\r\n\x1b[2mconnection to {name} lost; reconnecting…\x1b[22m\r\n")
}

/// What an attach waiting on a relay's first link is told once.
fn retrying_line(message: &str) -> String {
    format!("\r\n\x1b[2m{}; retrying…\x1b[22m\r\n", message.replace('\n', "\r\n"))
}

/// What a listing decides for the attaches that wait on it.
#[derive(Default)]
struct Decided {
    /// To send (again), with a recorded session's sink, which is told the
    /// listed size its replay is rendered at.
    again: Vec<(client::Attach, Option<Sink>)>,
    /// Taken out, with why.
    gone: Vec<(Pending, String)>,
}

/// What becomes of the attaches waiting on the core's `LIST` (`listed`),
/// at endpoint `name`:
/// - **Carried over a lost link:** the core gave this client back the
///   sessions it owns before it answered, so one listed with a client waits
///   for that replay. One listed without a client is sent again, and one not
///   listed is taken out.
/// - **Recorded (not sent yet):** sent at the listed size when the listing
///   has the session with its key and no client. Taken out when another
///   client has it, or when it isn't listed with its key (it ended, and the
///   id may be another session's now).
fn carried_attaches(requests: &mut HashMap<RequestId, Pending>, listed: &[SessionInfo], name: &str) -> Decided {
    let mut decided = Decided::default();
    let carried: Vec<RequestId> =
        requests.iter().filter(|(_, pending)| pending.carried).map(|(&req, _)| req).collect();
    for req in carried {
        let Some(pending) = requests.get_mut(&req) else { continue };
        let Waiting::Attach { attach, check, sink, .. } = &mut pending.waiting else { continue };
        let session = attach.session;
        let info = listed.iter().find(|info| info.session == session);
        let why = match (check.as_deref(), info) {
            (Some(key), Some(info)) if !key.is_empty() && info.key == key => {
                if info.attached {
                    Some(format!("session {session} on {name} is shown by another Brindle"))
                } else {
                    if info.cols > 0 && info.rows > 0 {
                        (attach.cols, attach.rows) = (info.cols, info.rows);
                    }
                    *check = None;
                    pending.carried = false;
                    decided.again.push((attach.clone(), Some(sink.clone())));
                    None
                }
            }
            (Some(_), _) => Some(format!("session {session} on {name} has ended")),
            (None, Some(info)) if info.attached => None,
            (None, Some(_)) => {
                pending.carried = false;
                decided.again.push((attach.clone(), None));
                None
            }
            (None, None) => Some(format!("the terminal server no longer has session {session}")),
        };
        if let Some(why) = why {
            decided.gone.extend(requests.remove(&req).map(|pending| (pending, why)));
        }
    }
    decided
}

/// Takes the sessions out of `sessions` that the core no longer holds for
/// this client (not in `kept`).
fn missing<T>(sessions: &mut HashMap<SessionId, T>, kept: &HashSet<SessionId>) -> Vec<T> {
    let gone: Vec<SessionId> = sessions.keys().filter(|id| !kept.contains(id)).copied().collect();
    gone.into_iter().filter_map(|id| sessions.remove(&id)).collect()
}

/// A session the core started, once [`super::Session`] has it.
pub struct Running {
    id: SessionId,
    pid: u32,
    key: String,
    conn: Arc<Connection>,
}

impl Running {
    /// The program's pid.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn id(&self) -> SessionId {
        self.id
    }

    /// The session's key; empty from a core that predates keys.
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn input(&self, bytes: &[u8]) {
        for chunk in bytes.chunks(INPUT_CHUNK) {
            self.conn.send(ToCore::Data(Data { session: self.id, bytes: chunk.to_vec() }));
        }
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        self.conn.send(ToCore::Resize(Resize { session: self.id, cols, rows, cell_w, cell_h }));
    }

    /// What the core last reported of the session's foreground program,
    /// once it has.
    pub fn foreground(&self) -> Option<Foreground> {
        self.conn.state.lock().foreground.get(&self.id).cloned()
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
        {
            let mut state = self.conn.state.lock();
            if !state.sessions.contains_key(&self.id) {
                return;
            }
            state.parents.insert(self.id, hwnd);
        }
        self.conn.send(ToCore::Reparent(Reparent { session: self.id, hwnd }));
    }

    /// Ends the session (`CLOSE`). Once is enough; a session that already
    /// exited or was lost isn't closed again.
    pub fn close(&self) {
        if self.conn.forget(self.id, true) {
            self.conn.send(ToCore::Close(SessionRef { session: self.id }));
        }
    }

    /// Leaves the session running with no client (`DETACH`), with the
    /// layout of the tab it was in for whichever client attaches it next,
    /// unless it already exited or was lost.
    pub fn detach(&self, layout: Option<serde_json::Value>) {
        if self.conn.forget(self.id, false) {
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
    use crate::remote::protocol::client::{Hello, Welcome};

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

        fn why(&self) -> String {
            self.far.why.lock().clone()
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
        let (far, counting) = (out.clone(), heartbeats.clone());
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
                answer(&message, &far);
                got_tx.send(message).ok();
            }
        });
        let wire = Arc::new(PipeWire { pipe: Mutex::new(client_writer), far: out.clone() });
        (Link { reader: Box::new(client_reader), wire }, Far { out, got, hello, heartbeats })
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
    }

    pub fn sink() -> (Sink, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel();
        (Arc::new(Mutex::new(Box::new(Recorder(tx)))), rx)
    }
}

#[cfg(test)]
mod tests {
    use std::thread::JoinHandle;

    use super::fake::{self, Plan};
    use super::*;
    use crate::remote::protocol::relay::RelayError;

    fn seen(rx: &mpsc::Receiver<String>) -> String {
        rx.recv_timeout(Duration::from_secs(10)).expect("an event")
    }

    fn quiet(rx: &mpsc::Receiver<String>) {
        if let Ok(event) = rx.recv_timeout(Duration::from_millis(300)) {
            panic!("unexpected: {event:?}");
        }
    }

    fn relay(name: &str) -> (Arc<Connection>, Plan) {
        let (dial, plan) = fake::dialer(name);
        (Connection::relay(dial, "tok", fake::quick()).unwrap(), plan)
    }

    fn spawn_request(env: &[(&str, &str)]) -> SpawnRequest {
        SpawnRequest {
            command: String::new(),
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

    fn attach_request(session: SessionId) -> AttachRequest {
        AttachRequest { session, key: format!("k{session}"), cols: 100, rows: 30, recorded: false }
    }

    /// A session a restored zone recorded, whose size it doesn't know.
    fn recorded(session: SessionId) -> AttachRequest {
        AttachRequest { cols: 80, rows: 24, recorded: true, ..attach_request(session) }
    }

    /// Waits until `count` requests wait in the connection.
    fn waiting(connection: &Connection, count: usize) {
        eventually(&format!("{count} requests wait"), || connection.state.lock().requests.len() == count);
    }

    fn eventually(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "never: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
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
        assert_eq!((running.id(), running.pid(), running.key()), (11, 1011, "k11"));
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
        assert!(far.heartbeats.load(Ordering::Relaxed) >= 2, "{:?}", far.heartbeats);
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
        let beating = Arc::new(AtomicBool::new(true));
        {
            let (out, beating) = (far.out.clone(), beating.clone());
            std::thread::spawn(move || {
                while beating.load(Ordering::Acquire) {
                    out.relay(RelayMessage::Heartbeat);
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
        }
        // Three dead-link limits with only heartbeats: still the one link.
        assert!(plan.dialed.recv_timeout(fake::quick().dead * 3).is_err(), "redialed");
        assert!(!far.out.is_cut());
        beating.store(false, Ordering::Release);
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
        quiet(&tab);
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
        std::thread::sleep(Duration::from_millis(300));
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
        quiet(&attach_tab);
        assert!(!attach.is_finished());
        // Once the endpoint answers, the attach and a new spawn go out.
        let (sink, _tab) = fake::sink();
        let spawning = connection.clone();
        let spawn = in_background(move || spawning.spawn(spawn_request(&[]), sink, None).map(|r| r.id()));
        plan.next_dial();
        let (link, _far) = fake::link(&[], |message, out| {
            fake::spawning(message, out);
            replaying(message, out);
        });
        plan.link(link);
        assert_eq!(attach.join().unwrap().unwrap(), 99);
        assert!(spawn.join().unwrap().is_ok());
        quiet(&spawn_tab);
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
        assert!(plan.dialed.recv_timeout(Duration::from_millis(300)).is_err(), "no retry");

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
        let line = "output \r\n\x1b[2mconnection to A8 lost; reconnecting…\x1b[22m\r\n";
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
                let info = SessionInfo { session: kept_id, attached: true, ..Default::default() };
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
        quiet(&kept_tab);
        quiet(&gone_tab);
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
                let attached = |session: SessionId| SessionInfo { session, attached: true, ..Default::default() };
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
            quiet(tab);
        }
        // Neither is asked for again, and nothing is detached or closed.
        assert!(matches!(second.next(), ToCore::List(_)));
        assert!(second.got.recv_timeout(Duration::from_millis(300)).is_err());
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
                let detached = SessionInfo { session: 7, owned: true, ..Default::default() };
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
        quiet(&tab);
        quiet(&gone_tab);
        assert!(second.got.recv_timeout(Duration::from_millis(300)).is_err());
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
        assert!(far.got.recv_timeout(Duration::from_millis(300)).is_err());
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
        assert!(far.got.recv_timeout(Duration::from_millis(300)).is_err());

        // Let go of before it was asked: withdrawn at once.
        let abandon = Abandon::default();
        abandon.abandon();
        let (sink, _tab) = fake::sink();
        let error = connection.attach(attach_request(6), sink, Some(&abandon)).err().expect("withdrawn");
        assert_eq!(error.to_string(), "closed before it started");
        assert!(far.got.recv_timeout(Duration::from_millis(300)).is_err());
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
                let info = SessionInfo { session: id, attached: true, ..Default::default() };
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
        assert!(second.got.recv_timeout(Duration::from_millis(300)).is_err());
        assert!(connection.state.lock().parents.is_empty());
    }

    /// 6.1: `PROCESS` read right behind `SPAWNED` (the core sends one at
    /// once) is the session's, though its spawner may not have its answer
    /// yet. One for a session not shown here is dropped.
    #[test]
    fn process_right_after_spawned_is_kept_for_the_session() {
        let (connection, plan) = steady_relay("A8");
        let (link, _far) = fake::link(&[], |message, out| {
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
        quiet(&tab);
        assert_eq!(connection.state.lock().foreground.len(), 1, "nothing kept for session 99");
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
        let idle = Duration::from_millis(300);
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
        // Closed while reconnecting: the CLOSE waits for a link.
        running.close();
        std::thread::sleep(idle * 3);
        assert!(!connection.is_dead(), "ended with its CLOSE unwritten");

        let id = running.id();
        let (link, second) = fake::link(&[], listing);
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        let written = loop {
            match second.next() {
                ToCore::Close(close) => break (close.session, Instant::now()),
                other => panic!("not CLOSE: {other:?}"),
            }
        };
        assert_eq!(written.0, id);
        eventually("the unused connection ends", || connection.is_dead());
        // Not before `idle` (less the idle thread's look, a fifth of it).
        assert!(written.1.elapsed() >= idle * 4 / 5, "{:?}", written.1.elapsed());
        eventually("its relay is ended", || second.out.is_cut());
        // It doesn't come back.
        assert!(plan.dialed.recv_timeout(Duration::from_millis(300)).is_err());
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
        assert!(plan.dialed.recv_timeout(Duration::from_millis(300)).is_err(), "dialled again");
    }

    /// 3.1: an eager connection keeps trying past the close limit, but the
    /// `CLOSE` it gave up isn't written once a link is back: the program is
    /// left running, as when Brindle ends without quitting.
    #[test]
    fn a_close_given_up_is_never_written() {
        let (dial, plan) = fake::dialer("A8");
        let (idle, close_limit) = (Duration::from_millis(100), Duration::from_millis(300));
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
        std::thread::sleep(idle * 3);
        assert!(!connection.is_dead(), "an eager connection ended");
        let (link, second) = fake::link(&[], listing);
        plan.link(link);
        assert!(matches!(second.next(), ToCore::List(_)));
        assert!(second.got.recv_timeout(Duration::from_millis(300)).is_err(), "the CLOSE given up went out");
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
        std::thread::sleep(idle * 5);
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
        assert!(reports.recv_timeout(Duration::from_millis(300)).is_err(), "nothing while connected");

        connection.set_eager(false);
        assert_eq!(reported(&reports, id), LinkState::Ended("the connection was no longer used".into()));
        eventually("its relay is ended", || second.out.is_cut());
        assert!(reports.recv_timeout(Duration::from_millis(300)).is_err(), "nothing after the end");
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
        let dead = Duration::from_secs(30);
        let ending = |ending: &Ending, expired: bool, why: &str| {
            relay_link_ended("A8", dead, expired, ending, || why.to_string())
        };
        let failed = Ending::Failed(io::Error::new(io::ErrorKind::ConnectionReset, "reset"));
        // Silence wins, and never asks the process (a kill's status).
        let never = || -> String { panic!("asked the process") };
        assert_eq!(relay_link_ended("A8", dead, true, &Ending::Closed, never), "nothing read from A8 for 30 s");
        // The process's own account.
        assert_eq!(ending(&Ending::Closed, false, "ssh exited with 255: boom"), "ssh exited with 255: boom");
        assert_eq!(ending(&failed, false, "ssh exited with 255: boom"), "ssh exited with 255: boom");
        // Nothing from the process: how the stream ended.
        assert_eq!(ending(&Ending::Closed, false, ""), "the relay closed the connection");
        assert_eq!(ending(&failed, false, ""), "reading from the relay failed: reset");
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
        assert!(second.got.recv_timeout(Duration::from_millis(300)).is_err(), "not the one given up on");
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
            attaching.attach(recorded(5), sink, None).map(|r| (r.id(), r.pid(), r.key().to_string()))
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
        assert_eq!(attach.join().unwrap().unwrap(), (5, 99, "k5".to_string()));
        assert_eq!(seen(&tab), "size 132x43");
        assert_eq!(seen(&tab), "replay true");
        assert_eq!(seen(&tab), "output screen");
        assert_eq!(seen(&tab), "replay false");
        quiet(&tab);
        assert!(far.got.recv_timeout(Duration::from_millis(300)).is_err());
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
        while let Ok(message) = far.got.recv_timeout(Duration::from_millis(300)) {
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
        assert!(hooked.recv_timeout(Duration::from_millis(300)).is_err(), "each runs once");
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
        std::thread::sleep(idle * 3);
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
        assert!(plan.dialed.recv_timeout(Duration::from_millis(300)).is_err(), "dialled again");
    }

    /// A link that holds every frame while it is shut, as a stalled one
    /// does, and keeps the type of each frame it passed on.
    struct Gate {
        inner: Arc<dyn Wire>,
        shut: Mutex<bool>,
        opened: Condvar,
        passed: Mutex<Vec<u8>>,
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
            let kind = message.encode().unwrap().msg_type;
            self.passed.lock().contains(&kind)
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
            self.passed.lock().push(frame.msg_type);
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
        let started = Instant::now();
        assert!(!connection.flush(started + Duration::from_millis(300)));
        assert!(started.elapsed() >= Duration::from_millis(300), "gave up after {:?}", started.elapsed());
        assert!(!gate.passed(&detach));

        // Written while a flush waits: the flush returns, with it written.
        let flushing = connection.clone();
        let flush = in_background(move || flushing.flush(Instant::now() + Duration::from_secs(10)));
        std::thread::sleep(Duration::from_millis(200));
        assert!(!flush.is_finished(), "returned before the DETACH was written");
        gate.shut(false);
        assert!(flush.join().unwrap());
        assert!(gate.passed(&detach), "returned before the DETACH was written");
        assert_eq!(far.next(), detach);

        // Down: nothing can be written until a next link, which an exiting
        // process won't wait for.
        far.out.cut();
        eventually("the link is down", || connection.slot.lock().link.is_none());
        connection.send(ToCore::Detach(Detach { session: 99, layout: None }));
        let started = Instant::now();
        assert!(!connection.flush(started + Duration::from_secs(10)));
        assert!(started.elapsed() < Duration::from_secs(1), "waited {:?}", started.elapsed());
    }

    #[test]
    fn sessions_the_core_no_longer_lists_are_missing() {
        let mut sessions: HashMap<SessionId, &str> = [(1, "a"), (2, "b"), (3, "c")].into();
        let mut lost = missing(&mut sessions, &[2].into());
        lost.sort();
        assert_eq!(lost, ["a", "c"]);
        assert_eq!(sessions.keys().copied().collect::<Vec<_>>(), [2]);
        assert!(missing(&mut sessions, &[2, 9].into()).is_empty());
    }
}
