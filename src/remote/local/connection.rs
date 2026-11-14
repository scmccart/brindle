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
//!   timed out) isn't sent.
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
//! - **A relay's first connect:** until a link has been up, a failed attempt
//!   fails new tabs' requests (spawns, lists, kills) with its error, while
//!   attaches (restored zones) keep waiting through the retries and are told
//!   the error once. With no attach waiting, the connection ends, unless it
//!   is eager (connects at launch). A new tab's request made while it
//!   retries asks for an attempt at once.
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
    SessionRef, Spawned, ToClient, ToCore,
};
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
}

type Reply<T> = mpsc::SyncSender<Result<T, String>>;

/// What a request waits for.
enum Waiting {
    /// `SPAWNED`, whose sink then joins the session map.
    Spawn { sink: Sink, reply: Reply<Spawned> },
    /// The `REPLAY_START` answering an `ATTACH` (its pid), whose sink joins
    /// the map before the replay's first `DATA` is read. A reconnect's
    /// replay of the session answers it too, and `attach` is sent again
    /// when the reconnected core didn't give the session back.
    Attach { sink: Sink, reply: Reply<u32>, attach: client::Attach },
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
    /// An attach that went out on a link that ended before its answer.
    /// The reconnect's `LIST` says what became of it.
    carried: bool,
}

impl Pending {
    fn is_attach(&self) -> bool {
        matches!(self.waiting, Waiting::Attach { .. })
    }
}

#[derive(Default)]
struct State {
    sessions: HashMap<SessionId, Sink>,
    /// The window each session's console is parented to (Windows), sent
    /// again after a reconnect's replay: a core doesn't keep it.
    parents: HashMap<SessionId, u64>,
    requests: HashMap<RequestId, Pending>,
    /// The `LIST` sent after a reconnect's `WELCOME`.
    list: Option<RequestId>,
    /// A link has been up.
    connected: bool,
    /// A new tab's request wants a relay attempt now, not after the pause.
    hurry: bool,
    /// The connection ended for good, and why; the next tab makes a new one.
    ended: Option<String>,
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
}

/// When a relay link counts as dead, as the watchdog reads it: a deadline
/// in milliseconds from `epoch`, or held while a frame is handled.
struct Stamp {
    epoch: Instant,
    deadline: AtomicU64,
}

impl Stamp {
    const HELD: u64 = u64::MAX;

    fn new(limit: Duration) -> Self {
        let stamp = Self { epoch: Instant::now(), deadline: AtomicU64::new(0) };
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
    slot: Mutex<Slot>,
    slot_ready: Condvar,
    watch: Mutex<Watch>,
    watch_changed: Condvar,
    next_req: AtomicU64,
    transport: Transport,
    token: String,
    /// Keep connecting while nothing waits (a profile connects at launch).
    eager: AtomicBool,
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
        let (outgoing, queue) = mpsc::channel();
        let this = Arc::new(Self {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            outgoing,
            slot: Mutex::new(Slot::default()),
            slot_ready: Condvar::new(),
            watch: Mutex::new(Watch::default()),
            watch_changed: Condvar::new(),
            next_req: AtomicU64::new(1),
            transport,
            token: token.to_string(),
            eager: AtomicBool::new(false),
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
    /// wait for it.
    pub fn relay(dial: Arc<dyn Dial>, token: &str, timings: Timings) -> Result<Arc<Self>> {
        let name = dial.name();
        let (this, queue) = Self::new(Transport::Relay { dial, timings }, token);
        let writing = this.clone();
        std::thread::Builder::new()
            .name("relay-writer".into())
            .spawn(move || writing.write_loop(queue))?;
        let watching = this.clone();
        std::thread::Builder::new()
            .name("relay-watch".into())
            .spawn(move || watching.watchdog(&name, timings.dead))?;
        let reading = this.clone();
        std::thread::Builder::new()
            .name("relay-reader".into())
            .spawn(move || reading.run_relay())?;
        Ok(this)
    }

    pub fn is_dead(&self) -> bool {
        self.state.lock().ended.is_some()
    }

    /// Makes the connection keep connecting while nothing waits for it, as
    /// for a profile that connects at launch.
    // Task 8.2 sets it for eager endpoints.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_eager(&self, eager: bool) {
        self.eager.store(eager, Ordering::Release);
    }

    fn is_relay(&self) -> bool {
        matches!(self.transport, Transport::Relay { .. })
    }

    fn send(&self, message: ToCore) {
        self.outgoing.send(message).ok();
    }

    fn req(&self) -> RequestId {
        self.next_req.fetch_add(1, Ordering::Relaxed)
    }

    /// Sends the request `make` builds and waits for its answer: up to
    /// [`REQUEST_LIMIT`] on the local core, as long as it takes on a relay.
    /// Not on the UI thread: this blocks.
    fn ask<T>(
        &self,
        what: &str,
        abandon: Option<(Weak<Self>, &Abandon)>,
        make: impl FnOnce(RequestId, Reply<T>) -> (Waiting, ToCore),
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
            state.requests.insert(req, Pending { waiting, written: false, told: false, carried: false });
        }
        if let Some((connection, abandon)) = abandon
            && !abandon.asked(connection, req)
        {
            self.withdraw(req);
        }
        self.send(message);
        let answer = if self.is_relay() {
            answer.recv().map_err(|_| format!("the terminal server didn't {what}"))
        } else {
            answer
                .recv_timeout(REQUEST_LIMIT)
                .map_err(|_| format!("the terminal server didn't {what} within {} s", REQUEST_LIMIT.as_secs()))
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
        let spawned = self.ask("start the session", abandon, |req, reply| {
            let message = ToCore::Spawn(client::Spawn {
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
            });
            (Waiting::Spawn { sink, reply }, message)
        })?;
        Ok(Running { id: spawned.session, pid: spawned.pid, key: spawned.key, conn: self.clone() })
    }

    /// Attaches a detached session, whose replay and then live output go to
    /// `sink`, and waits for its `REPLAY_START`. Not on the UI thread.
    pub fn attach(self: &Arc<Self>, request: AttachRequest, sink: Sink, abandon: Option<&Abandon>) -> Result<Running> {
        let session = request.session;
        let abandon = abandon.map(|abandon| (Arc::downgrade(self), abandon));
        let pid = self.ask("attach the session", abandon, |req, reply| {
            let attach = client::Attach { req, session, cols: request.cols, rows: request.rows };
            (Waiting::Attach { sink, reply, attach: attach.clone() }, ToCore::Attach(attach))
        })?;
        Ok(Running { id: session, pid, key: request.key, conn: self.clone() })
    }

    /// The core's sessions, this client's and detached ones. Blocks.
    pub fn list(&self) -> Result<Vec<SessionInfo>> {
        self.ask("list its sessions", None, |req, reply| {
            (Waiting::List(reply), ToCore::List(Request { req }))
        })
    }

    /// Ends a session whatever its policy or client. Blocks.
    pub fn kill(&self, session: SessionId) -> Result<()> {
        self.ask("end the session", None, |req, reply| {
            let target = KillTarget::Session(session);
            (Waiting::Done(reply), ToCore::Kill(Kill { req, target }))
        })
    }

    /// Reads each local link in turn, reconnecting when one ends.
    fn run_local(self: Arc<Self>, mut reader: Box<dyn Read + Send>) {
        let Transport::Local(endpoint) = &self.transport else { unreachable!("a local connection") };
        loop {
            let reason = self.read(&mut BufReader::with_capacity(READ_BUFFER, reader), None);
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
        loop {
            match self.open_relay(dial.as_ref(), timings) {
                Ok((mut reader, stamp)) => {
                    failures = 0;
                    let reason = self.read(&mut reader, Some((&stamp, timings.dead)));
                    log::warn!("connection to {name} ended ({reason}); reconnecting");
                    self.disconnected(&reason);
                    self.lost_link(&name);
                }
                Err(error) => {
                    let message = format!("can't connect to {name}: {error:#}");
                    log::warn!("{message}");
                    if error.downcast_ref::<Refused>().is_some() || !self.failed_to_connect(&message) {
                        return self.ended(&message);
                    }
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
    ) -> Result<(BufReader<Box<dyn Read + Send>>, Arc<Stamp>)> {
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
                Ok((reader, stamp))
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
    /// sessions the core no longer has; then the writer gets the link.
    fn up(&self, wire: Arc<dyn Wire>, host: Option<RelayHost>) -> Result<()> {
        if self.state.lock().connected {
            let req = self.req();
            self.state.lock().list = Some(req);
            wire.send(&ToCore::List(Request { req }).encode()?)?;
        }
        self.state.lock().connected = true;
        self.slot.lock().link = Some(Current { wire, host: host.map(Arc::new) });
        self.slot_ready.notify_all();
        Ok(())
    }

    /// Reads one link until it ends, saying why. On a relay, `liveness`'s
    /// stamp is held while each frame is handled and then touched.
    fn read(&self, reader: &mut impl BufRead, liveness: Option<(&Stamp, Duration)>) -> String {
        loop {
            let frame = match Frame::read_from_or_eof(reader) {
                Ok(Some(frame)) => frame,
                Ok(None) => return "the terminal server closed the connection".to_string(),
                Err(error) => return format!("reading from the terminal server failed: {error}"),
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
                        self.send(ToCore::Close(SessionRef { session: spawned.session }));
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
            ToClient::Sessions(sessions) if self.state.lock().list != Some(sessions.req) => {
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
                        self.send(ToCore::Detach(SessionRef { session }));
                    }
                }
            }
            ToClient::Exit(exit) => {
                let sink = {
                    let mut state = self.state.lock();
                    state.parents.remove(&exit.session);
                    state.sessions.remove(&exit.session)
                };
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
                    self.send(ToCore::Detach(SessionRef { session }));
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
                let (lost, (again, gone)) = {
                    let mut state = self.state.lock();
                    if state.list != Some(sessions.req) {
                        return;
                    }
                    state.list = None;
                    let kept: HashSet<SessionId> =
                        sessions.sessions.iter().filter(|s| s.attached).map(|s| s.session).collect();
                    state.parents.retain(|session, _| kept.contains(session));
                    let listed: HashMap<SessionId, bool> =
                        sessions.sessions.iter().map(|s| (s.session, s.attached)).collect();
                    (missing(&mut state.sessions, &kept), carried_attaches(&mut state.requests, &listed))
                };
                for sink in lost {
                    sink.lock().lost("the terminal server no longer has this session");
                }
                for (session, pending) in gone {
                    pending.waiting.fail(format!("the terminal server no longer has session {session}"));
                }
                for attach in again {
                    self.send(ToCore::Attach(attach));
                }
            }
            // Task 6.1 keeps `PROCESS` for remote sessions' titles.
            ToClient::Welcome(_) | ToClient::ServerStatus(_) | ToClient::Process(_) => {}
        }
    }

    fn sink(&self, session: SessionId) -> Option<Sink> {
        self.state.lock().sessions.get(&session).cloned()
    }

    /// A replay without a `req` (a reconnect's) of a session an attach
    /// waits for answers that attach: its sink joins the map.
    fn attached_by_replay(&self, session: SessionId) -> Option<(Sink, Reply<u32>)> {
        let mut state = self.state.lock();
        let req = state.requests.iter().find_map(|(&req, pending)| match &pending.waiting {
            Waiting::Attach { attach, .. } if attach.session == session => Some(req),
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

    /// A relay attempt failed with `message`. Before the first link, new
    /// tabs' requests fail with it, and the attaches that wait on are told
    /// it once. Says whether to keep trying: always once a link has been
    /// up, else while an attach waits or the connection is eager.
    fn failed_to_connect(&self, message: &str) -> bool {
        let (failed, told, keep) = {
            let mut state = self.state.lock();
            if state.connected {
                return true;
            }
            let new_tabs: Vec<RequestId> =
                state.requests.iter().filter(|(_, pending)| !pending.is_attach()).map(|(&req, _)| req).collect();
            let failed: Vec<Pending> = new_tabs.into_iter().filter_map(|req| state.requests.remove(&req)).collect();
            // This attempt answered them: only a later tab hurries the next.
            state.hurry = false;
            let told: Vec<Sink> = state
                .requests
                .values_mut()
                .filter_map(|pending| match &pending.waiting {
                    Waiting::Attach { sink, .. } if !pending.told => {
                        pending.told = true;
                        Some(sink.clone())
                    }
                    _ => None,
                })
                .collect();
            let keep = self.eager.load(Ordering::Acquire) || state.requests.values().any(Pending::is_attach);
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
    /// request fails.
    fn ended(&self, reason: &str) {
        let (sessions, requests) = {
            let mut state = self.state.lock();
            state.ended.get_or_insert_with(|| reason.to_string());
            state.parents.clear();
            (std::mem::take(&mut state.sessions), std::mem::take(&mut state.requests))
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
        {
            let mut watch = self.watch.lock();
            watch.closed = true;
            watch.link = None;
        }
        self.watch_changed.notify_all();
        for pending in requests.into_values() {
            pending.waiting.fail(reason.to_string());
        }
        for sink in sessions.into_values() {
            sink.lock().lost(reason);
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
            if !self.write(&message) {
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
                Ok(()) => return true,
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
        }
    }

    /// Marks a request written or not; false when nobody waits for it.
    fn mark_written(&self, req: RequestId, written: bool) -> bool {
        match self.state.lock().requests.get_mut(&req) {
            Some(pending) => {
                pending.written = written;
                true
            }
            None => false,
        }
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

/// What becomes of the attaches carried over a lost link, from the
/// reconnected core's `LIST` (`listed`: each session and whether a client
/// has it). The core gave this client back the sessions it owns before it
/// answered, so one listed with a client waits for that replay. One listed
/// without a client is sent again (returned first, no longer carried), and
/// one not listed is taken out (returned second, with its session).
fn carried_attaches(
    requests: &mut HashMap<RequestId, Pending>,
    listed: &HashMap<SessionId, bool>,
) -> (Vec<client::Attach>, Vec<(SessionId, Pending)>) {
    let (mut again, mut gone) = (Vec::new(), Vec::new());
    let carried: Vec<RequestId> =
        requests.iter().filter(|(_, pending)| pending.carried).map(|(&req, _)| req).collect();
    for req in carried {
        let Some(pending) = requests.get_mut(&req) else { continue };
        let Waiting::Attach { attach, .. } = &pending.waiting else { continue };
        let session = attach.session;
        match listed.get(&session) {
            Some(true) => {}
            Some(false) => {
                again.push(attach.clone());
                pending.carried = false;
            }
            None => gone.extend(requests.remove(&req).map(|pending| (session, pending))),
        }
    }
    (again, gone)
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

    /// Makes the window `hwnd` the owner of the session's console window
    /// (`REPARENT`, which only a Windows core takes), and keeps it to send
    /// again after each reconnect's replay. A session that exited or was
    /// lost sends nothing.
    pub fn reparent(&self, hwnd: u64) {
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
        if self.forget() {
            self.conn.send(ToCore::Close(SessionRef { session: self.id }));
        }
    }

    /// Leaves the session running with no client (`DETACH`), unless it
    /// already exited or was lost.
    pub fn detach(&self) {
        if self.forget() {
            self.conn.send(ToCore::Detach(SessionRef { session: self.id }));
        }
    }

    /// Stops showing the session; says whether it was still shown.
    fn forget(&self) -> bool {
        let mut state = self.conn.state.lock();
        state.parents.remove(&self.id);
        state.sessions.remove(&self.id).is_some()
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
        let out = Out { writer: Arc::new(Mutex::new(Some(far_writer))), cut: Arc::default() };
        out.relay(RelayMessage::Host(RelayHost {
            version: "0.0.0".into(),
            build: "relay".into(),
            env: env.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect(),
            shell: "/bin/bash".into(),
            home: "/home/u".into(),
            server_dir: "/run/user/1000/brindle".into(),
            core_pid: 4242,
        }));
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
        AttachRequest { session, key: format!("k{session}"), cols: 100, rows: 30 }
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
        assert_eq!(far.next(), ToCore::Detach(SessionRef { session: 5 }));
        replay(&far.out, 6, Some(77));
        assert_eq!(far.next(), ToCore::Detach(SessionRef { session: 6 }));
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
