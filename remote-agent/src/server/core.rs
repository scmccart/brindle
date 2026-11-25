//! `brindle-remote serve`: the core, which clients talk to.
//!
//! The core owns the client endpoint (`core.sock` on Linux, a named pipe on
//! Windows; see `brindle_remote_protocol::endpoint`) and drives the
//! holders, which own the PTYs. It can restart at any time without ending a session: a new core
//! adopts the running holders and their sessions.
//!
//! - **Single instance:** `core.lock` is held for the core's whole life,
//!   and only then is a stale `core.sock` replaced (a pipe leaves nothing
//!   behind). A second
//!   `serve` that can't take the lock waits until the winner accepts
//!   connections, tells its starter it is ready, and exits 0.
//! - **Clients** ([`clients`]): the peer check before reading anything, the
//!   `HELLO`/`WELCOME` handshake, and per-client queues. Past
//!   [`CLIENT_QUEUE_HIGH`] queued to a client, its sessions are paused in
//!   their holders until the queue is under [`CLIENT_QUEUE_LOW`], so a
//!   stalled client blocks only its own programs. A client found gone by a
//!   failed write is dropped only once what it had sent is handled (up to
//!   [`DEAD_CLIENT_READ_LIMIT`]), so a `DETACH` sent just before it closed
//!   still counts.
//! - **Takeover:** a client that connects with the token of one still
//!   connected replaces it: the old connection, which stopped delivering
//!   without closing (a dropped network link), is closed, and its sessions
//!   go to the new one with their replays, with no close policy applied.
//! - **Holders** ([`holders`]): found by their `holder-*.sock` sockets (on
//!   Windows by their lock files, which name their pipes), live ones told
//!   from stale ones by their lock files, and adopted (`HELLO`,
//!   then `WELCOME` with the sessions, checkpoints and tails). Clients are
//!   accepted once the holders found at startup are adopted, so session ids
//!   continue from the largest adopted one and returning clients find their
//!   sessions. A holder of the core's own build is started when a session
//!   needs one, and holders of other builds are told to `DRAIN`.
//! - **Management and upgrades** ([`manage`]): `LIST` (whose detached
//!   sessions say whether their owner may come back, and since when they
//!   have had no client), `STATUS`, and `STEP_DOWN`, after which the core
//!   exits for the next one.
//! - **Sessions** ([`sessions`]): spawned through a holder, with their I/O,
//!   the close policies and, after adopting sessions, the
//!   [`RECONNECT_GRACE`] for their owners to come back. A client may ask
//!   for a linger of its own (`HELLO`'s `linger_secs`, at most
//!   [`MAX_LINGER`]): its `kill` sessions then wait that long for it after
//!   its connection ends. Each session has an emulator ([`screen`]) fed with
//!   all of its output.
//! - **Process info** ([`process`]): clients that negotiated protocol 2 and
//!   asked are told of their sessions' foreground programs (`PROCESS`),
//!   looked at once a second in sessions that had output or input. A
//!   Windows core reports none.
//! - **Exit:** after [`UNUSED_LIMIT`] with no sessions and no clients, once
//!   `STOP` has ended every session, or after `STEP_DOWN`. On Linux the
//!   socket is removed first.
//! - **Windows:** a client's `REPARENT` goes to its session's holder, and
//!   at its start the core removes the helper copies of other builds that
//!   nothing runs from any more, and an elevated core what was set aside
//!   for not being protected (`dir::prune_copies`), holding its own copies'
//!   directory for its life.

mod agents;
mod clients;
mod holders;
mod manage;
mod policy;
mod process;
mod screen;
mod sessions;

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use brindle_remote_protocol::client::{Request, ToClient};
use brindle_remote_protocol::endpoint::{self, Endpoints};
use brindle_remote_protocol::{RequestId, SessionId};

use self::clients::{Client, Pending};
use self::holders::{Holder, HolderStart, WaitingSpawn};
use self::policy::{Accept, Ending, Lifecycle, Seen};
use self::sessions::{Ownership, Session};
use super::event_loop::{Event, EventLoop, Interest, Loop, Token};
use super::platform::listen::{self, Identity, Listener};
use super::platform::lock::Lock;
use super::platform::sys;
use super::{build_id, conn, daemon, dir};

/// With no sessions and no clients for this long, the core exits.
pub const UNUSED_LIMIT: Duration = Duration::from_secs(5);
/// After a core starts with adopted sessions, their owners have this long to
/// come back before their `kill` sessions are hung up.
pub const RECONNECT_GRACE: Duration = Duration::from_secs(10);
/// The longest a client may ask its `kill` sessions to wait for it
/// (`HELLO`'s `linger_secs`).
pub const MAX_LINGER: Duration = Duration::from_secs(600);
/// Past this much queued to a client, its sessions are paused...
pub const CLIENT_QUEUE_HIGH: usize = 1024 * 1024;
/// ...until its queue is back under this.
pub const CLIENT_QUEUE_LOW: usize = 256 * 1024;
/// When a write to a client fails, the core still reads what that client
/// had sent, up to this much, before it drops the client (the holder's
/// `DEAD_CORE_READ_LIMIT`, for clients).
pub const DEAD_CLIENT_READ_LIMIT: usize = 16 * 1024 * 1024;
/// How long a losing `serve` waits for the winner to accept connections.
const LOSER_WAIT: Duration = Duration::from_secs(2);
/// How long clients wait, unaccepted, for the startup adoption.
const ADOPT_LIMIT: Duration = Duration::from_secs(5);
/// How long a flush may wait for a peer when the core exits.
const EXIT_FLUSH_LIMIT: Duration = Duration::from_secs(1);
/// How long a stepping-down core waits for each holder to take its
/// checkpoints.
const STEP_DOWN_FLUSH_LIMIT: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_millis(250);

macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!(
            "{} core[{}]: {}",
            crate::server::daemon::timestamp(),
            std::process::id(),
            format_args!($($arg)*)
        )
    };
}
pub(crate) use log;

/// Runs `brindle-remote serve [--ready-fd N]` (`--ready-handle N` on
/// Windows). Without it the core stays in the foreground and logs to stderr.
pub fn run(args: &[String]) -> Result<(), String> {
    let ready = daemon::ready_from_args("serve", args)?;
    // Held until the core exits: an elevated core's directory stays the one
    // it checked.
    let server_dir =
        dir::from_env().map_err(|error| format!("can't set up the server directory: {error}"))?;
    let dir = server_dir.path().to_path_buf();
    let paths = Paths::new(&dir)
        .map_err(|error| format!("can't name the core's endpoint in {}: {error}", dir.display()))?;
    let claimed = claim(&paths)
        .map_err(|error| format!("can't take the core's lock {}: {error}", paths.lock.display()))?;
    let lock = match claimed {
        Claim::Core(lock) => lock,
        Claim::Running => {
            // Another core serves this directory; its starter can connect.
            if let Some(ready) = ready {
                ready.signal().map_err(|error| {
                    format!("can't tell the starter that another core is listening: {error}")
                })?;
            }
            return Ok(());
        }
    };
    if ready.is_some() {
        let log = daemon::open_log(&paths.log)
            .map_err(|error| format!("can't open {}: {error}", paths.log.display()))?;
        daemon::daemonize(&log)
            .map_err(|error| format!("can't detach from the starter: {error}"))?;
    }
    let build = build_id::current().map_err(|error| format!("can't read the build id: {error}"))?;
    // Held until the core exits, as its server directory is.
    let pruned = dir::prune_copies(&build);
    for note in &pruned.notes {
        log!("{note}");
    }
    let ev = Loop::new().map_err(|error| format!("can't create the event loop: {error}"))?;
    let core = Core::new(ev, dir, paths, lock, build, Identity::own()).map_err(|error| {
        log!("can't start: {error}");
        format!("can't start the core: {error}")
    })?;
    if let Some(ready) = ready
        && let Err(error) = ready.signal()
    {
        log!("couldn't tell the starter it is listening: {error}");
    }
    core.run().map_err(|error| {
        log!("stopped: {error}");
        format!("the core stopped: {error}")
    })
}

/// Where the core listens, and its files in the server directory.
struct Paths {
    endpoints: Endpoints,
    lock: PathBuf,
    /// The socket on Linux, the pipe's name on Windows.
    endpoint: PathBuf,
    log: PathBuf,
}

impl Paths {
    fn new(dir: &Path) -> io::Result<Self> {
        let endpoints = Endpoints::of(dir)?;
        let endpoint = endpoints.core();
        listen::check_path(&endpoint)?;
        Ok(Self {
            endpoints,
            lock: dir.join("core.lock"),
            endpoint,
            log: dir.join("core.log"),
        })
    }
}

enum Claim {
    /// This process holds `core.lock` and becomes the core.
    Core(Lock),
    /// Another core holds it and accepts connections.
    Running,
}

/// Takes `core.lock`, or waits up to [`LOSER_WAIT`] for the core that holds
/// it to accept connections (it may still be starting, or exiting).
fn claim(paths: &Paths) -> io::Result<Claim> {
    let deadline = Instant::now() + LOSER_WAIT;
    loop {
        if let Some(lock) = Lock::create(&paths.lock)? {
            return Ok(Claim::Core(lock));
        }
        if listen::connect(&paths.endpoint).is_ok() {
            return Ok(Claim::Running);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!(
                "another core holds {} but doesn't accept connections",
                paths.lock.display()
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A random id for this core process, for the holders' logs and takeovers.
fn random_instance() -> u64 {
    if let Some(random) = sys::random_u64() {
        return random;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_nanos() as u64 ^ (u64::from(std::process::id()) << 32)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Listener,
    Pending,
    Client,
    Holder,
    HolderReady,
}

const LISTENER: Token = 0;

/// A `STOP` or `STEP_DOWN` in progress: the client and request to answer
/// when the core exits.
struct Answer {
    client: Token,
    req: RequestId,
}

/// A connection the core closes as it exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Peer {
    Holder(Token),
    Client(Token),
}

pub(super) struct Core<L: EventLoop> {
    ev: L,
    dir: PathBuf,
    paths: Paths,
    _lock: Lock,
    /// Closed when the core exits, before its last messages go out.
    listener: Option<Listener>,
    /// When clients are accepted (once the startup adoption is done), the
    /// grace period of adopted sessions' owners, and the idle time.
    lifecycle: Lifecycle,
    build: String,
    instance: u64,
    started: Instant,
    sources: HashMap<Token, Source>,
    next_token: Token,
    pending: HashMap<Token, Pending>,
    clients: HashMap<Token, Client>,
    holders: HashMap<Token, Holder>,
    holder_start: Option<HolderStart>,
    /// Holder starters that haven't been reaped yet.
    starters: Vec<Child>,
    /// Spawns waiting for a holder of this build to be ready.
    waiting: Vec<WaitingSpawn>,
    sessions: BTreeMap<SessionId, Session>,
    next_session: SessionId,
    /// When the core next looks at the sessions' foreground programs.
    next_look: Instant,
    /// Answered once every session has ended.
    stopping: Option<Answer>,
    /// Answered once the holders have their checkpoints and `GOODBYE`.
    stepping_down: Option<Answer>,
}

impl<L: EventLoop> Core<L> {
    fn new(
        mut ev: L,
        dir: PathBuf,
        paths: Paths,
        lock: Lock,
        build: String,
        identity: Identity,
    ) -> io::Result<Self> {
        // The lock is ours, so any socket at this path is stale.
        let listener = Listener::bind(&paths.endpoint, identity)?;
        ev.register(&listener.source(), LISTENER, Interest::NONE)?;
        log!(
            "listening on {} (version {}, build {build})",
            paths.endpoint.display(),
            env!("CARGO_PKG_VERSION")
        );
        Ok(Self {
            ev,
            dir,
            paths,
            _lock: lock,
            listener: Some(listener),
            lifecycle: Lifecycle::new(),
            build,
            instance: random_instance(),
            started: Instant::now(),
            sources: HashMap::from([(LISTENER, Source::Listener)]),
            next_token: LISTENER + 1,
            pending: HashMap::new(),
            clients: HashMap::new(),
            holders: HashMap::new(),
            holder_start: None,
            starters: Vec::new(),
            waiting: Vec::new(),
            sessions: BTreeMap::new(),
            next_session: 1,
            next_look: Instant::now() + process::PROCESS_LOOK,
            stopping: None,
            stepping_down: None,
        })
    }

    fn run(mut self) -> io::Result<()> {
        self.discover(true);
        let mut events: Vec<Event> = Vec::new();
        loop {
            self.turn(&mut events, TICK)?;
            if self.should_exit(Instant::now()) {
                break;
            }
        }
        self.shutdown();
        Ok(())
    }

    /// One pass of the loop: flush, wait up to `timeout`, handle what's
    /// ready, then the timers.
    fn turn(&mut self, events: &mut Vec<Event>, timeout: Duration) -> io::Result<()> {
        self.flush_all();
        self.update_interests();
        self.ev.wait(events, Some(timeout))?;
        for event in events.iter() {
            self.dispatch(*event);
        }
        self.reap_starters();
        self.tick(Instant::now());
        Ok(())
    }

    fn token(&mut self) -> Token {
        let token = self.next_token;
        self.next_token += 1;
        token
    }

    fn drop_source(&mut self, token: Token) {
        let _ = self.ev.deregister(token);
        self.sources.remove(&token);
    }

    fn dispatch(&mut self, event: Event) {
        match self.sources.get(&event.token).copied() {
            Some(Source::Listener) => self.accept(),
            Some(Source::Pending) => self.read_pending(event.token),
            Some(Source::Client) => {
                if event.writable {
                    self.flush_client(event.token);
                }
                if event.readable || event.closed {
                    self.read_client(event.token);
                }
            }
            Some(Source::Holder) => {
                if event.writable {
                    self.flush_holder(event.token);
                }
                if event.readable || event.closed {
                    self.read_holder(event.token);
                }
            }
            Some(Source::HolderReady) => self.read_holder_ready(),
            None => {}
        }
    }

    /// Holders before clients: what a client is told about a session (that
    /// it owns it, after an attach) must already be in the holder's socket,
    /// since the core can be killed between the two writes. A replay that
    /// waited for that goes out in the same turn.
    fn flush_all(&mut self) {
        let holders: Vec<Token> = self.holders.keys().copied().collect();
        for holder in holders {
            self.flush_holder(holder);
        }
        self.send_stored_replays();
        let clients: Vec<Token> = self.clients.keys().copied().collect();
        for client in clients {
            self.flush_client(client);
        }
    }

    fn update_interests(&mut self) {
        let listener = if self.lifecycle.accepting() {
            Interest::READABLE
        } else {
            Interest::NONE
        };
        let _ = self.ev.modify(LISTENER, listener);
        for (&token, client) in &mut self.clients {
            let want = Interest {
                readable: true,
                writable: client.conn.queued() > 0,
            };
            if want != client.interest && self.ev.modify(token, want).is_ok() {
                client.interest = want;
            }
        }
        for (&token, holder) in &mut self.holders {
            let want = Interest {
                readable: true,
                writable: holder.conn.queued() > 0,
            };
            if want != holder.interest && self.ev.modify(token, want).is_ok() {
                holder.interest = want;
            }
        }
    }

    fn reap_starters(&mut self) {
        self.starters
            .retain_mut(|child| matches!(child.try_wait(), Ok(None)));
    }

    /// The timers: what the turn gathers goes to the [`Lifecycle`], whose
    /// answer it applies (`fcis R2`).
    fn tick(&mut self, now: Instant) {
        self.expire_pending(now);
        self.end_lingering(now);
        self.tick_agents(now);
        if now >= self.next_look {
            self.next_look = now + process::PROCESS_LOOK;
            self.look_at_processes(now);
        }
        self.check_holder_start(now);
        let seen = Seen {
            startup_adopted: self.startup_adopted(now),
            awaiting_owner: self
                .sessions
                .values()
                .any(|session| match session.ownership {
                    Ownership::AwaitingOwner { .. } => true,
                    Ownership::Attached(_)
                    | Ownership::Lingering { .. }
                    | Ownership::Detached { .. } => false,
                }),
            busy: !(self.sessions.is_empty()
                && self.clients.is_empty()
                && self.pending.is_empty()
                && self.waiting.is_empty()
                && self.holder_start.is_none()),
        };
        let tick = self.lifecycle.tick(now, seen);
        match tick.accept {
            Accept::Now { grace: true } => log!(
                "adopted {} session(s); their owners have {}s to come back",
                self.sessions.len(),
                RECONNECT_GRACE.as_secs()
            ),
            Accept::Now { grace: false } | Accept::No => {}
        }
        if tick.end_grace {
            self.end_grace();
        }
    }

    /// What is ending the core, for [`Lifecycle::should_exit`].
    fn ending(&self) -> Ending {
        match (&self.stepping_down, &self.stopping) {
            (Some(_), _) => Ending::SteppingDown,
            (None, Some(_)) => Ending::Stopping {
                sessions_left: !self.sessions.is_empty(),
            },
            (None, None) => Ending::No,
        }
    }

    fn should_exit(&self, now: Instant) -> bool {
        self.lifecycle.should_exit(now, self.ending())
    }

    /// Closes these holders' and clients' connections gracefully, all at
    /// once, within `limit`: each gets its queue and the connection's end,
    /// and the core reads until the peer closes too, since closing with
    /// unread input resets the connection and loses what the peer hadn't
    /// read yet. Holders' output and exits still go to their clients
    /// meanwhile. Each closed connection is dropped; the result lists those
    /// that failed or ran out of time.
    fn close_peers(&mut self, peers: Vec<Peer>, limit: Duration) -> Vec<(Peer, io::Error)> {
        let deadline = Instant::now() + limit;
        let mut open = peers;
        let mut failed = Vec::new();
        loop {
            let mut still = Vec::new();
            for peer in open {
                let step = match peer {
                    Peer::Holder(token) => {
                        let Some(entry) = self.holders.get_mut(&token) else {
                            continue;
                        };
                        let step = entry.conn.close_step();
                        self.closing_holder_frames(token);
                        step
                    }
                    Peer::Client(token) => {
                        let Some(entry) = self.clients.get_mut(&token) else {
                            continue;
                        };
                        let step = entry.conn.close_step();
                        entry.conn.discard_input();
                        step
                    }
                };
                match step {
                    Ok(false) => still.push(peer),
                    Ok(true) => self.forget_peer(peer),
                    Err(error) => {
                        failed.push((peer, error));
                        self.forget_peer(peer);
                    }
                }
            }
            open = still;
            if open.is_empty() {
                return failed;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let conns: Vec<(Peer, &conn::Conn)> = open
                .iter()
                .filter_map(|&peer| {
                    let conn = match peer {
                        Peer::Holder(token) => self.holders.get(&token).map(|entry| &entry.conn),
                        Peer::Client(token) => self.clients.get(&token).map(|entry| &entry.conn),
                    };
                    conn.map(|conn| (peer, conn))
                })
                .collect();
            if remaining.is_zero() {
                failed.extend(conns.iter().map(|&(peer, conn)| (peer, conn.unfinished())));
                return failed;
            }
            let conns: Vec<&conn::Conn> = conns.into_iter().map(|(_, conn)| conn).collect();
            conn::wait(&conns, remaining);
        }
    }

    /// Drops a peer's connection while the core exits.
    fn forget_peer(&mut self, peer: Peer) {
        match peer {
            Peer::Holder(token) => {
                self.drop_source(token);
                self.holders.remove(&token);
            }
            Peer::Client(token) => {
                self.drop_source(token);
                self.clients.remove(&token);
            }
        }
    }

    fn shutdown(mut self) {
        // Nothing can find this core once it stops listening and its socket
        // is gone. (A pipe's name goes with the listener and the
        // connections it accepted.)
        self.drop_source(LISTENER);
        self.listener = None;
        let _ = endpoint::remove(&self.paths.endpoint);
        let stepped_down = self.stepping_down.take();
        if stepped_down.is_some() {
            // The checkpoints and GOODBYEs reach the holders before the
            // client hears OK, so the core it starts next finds them. What
            // the holders sent before their GOODBYE still reaches the
            // clients.
            let holders: Vec<Peer> = self.holders.keys().copied().map(Peer::Holder).collect();
            for (_, error) in self.close_peers(holders, STEP_DOWN_FLUSH_LIMIT) {
                log!("a holder didn't take its checkpoints: {error}");
            }
        }
        let stopped = self.stopping.take();
        if let Some(answer) = stopped.as_ref().or(stepped_down.as_ref()) {
            self.send_client(answer.client, ToClient::Ok(Request { req: answer.req }));
        }
        let peers: Vec<Peer> = (self.holders.keys().copied().map(Peer::Holder))
            .chain(self.clients.keys().copied().map(Peer::Client))
            .collect();
        for (peer, error) in self.close_peers(peers, EXIT_FLUSH_LIMIT) {
            match peer {
                Peer::Holder(token) => {
                    log!("holder {token} didn't take its last messages: {error}");
                }
                Peer::Client(_) => {}
            }
        }
        if stepped_down.is_some() {
            log!(
                "upgrade: stepped down after {}s; the next core takes the holders over",
                self.started.elapsed().as_secs()
            );
        } else if stopped.is_some() {
            log!("stopped; exiting");
        } else {
            log!(
                "unused for {}s; exiting after {}s",
                UNUSED_LIMIT.as_secs(),
                self.started.elapsed().as_secs()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::dir::tests::TempDir;

    /// A core in a fresh directory that runs, as far as its peer check is
    /// concerned, as `identity`.
    fn core_as(temp: &TempDir, identity: Identity) -> Core<Loop> {
        let paths = Paths::new(&temp.0).unwrap();
        let Claim::Core(lock) = claim(&paths).unwrap() else {
            panic!("another core holds the lock");
        };
        let ev = Loop::new().unwrap();
        Core::new(ev, temp.0.clone(), paths, lock, "test".into(), identity).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn a_foreign_uid_is_closed_without_being_read() {
        use crate::server::platform::listen::tests::TestPeer;

        let temp = TempDir::new("core-peer");
        // As if this core ran as someone else.
        let mut core = core_as(&temp, Identity::own().other());
        let mut peer = TestPeer::connect(&core.paths.endpoint);
        peer.send_hello();
        core.accept();
        assert!(core.pending.is_empty() && core.clients.is_empty());
        assert!(peer.closed_unread());
        drop(core);

        // The same connection from its own user waits for its HELLO.
        let temp = TempDir::new("core-peer-own");
        let mut core = core_as(&temp, Identity::own());
        let mut peer = TestPeer::connect(&core.paths.endpoint);
        peer.send_hello();
        core.accept();
        assert_eq!(core.pending.len(), 1);
    }

    /// The Windows form of the test above. A pipe has no reset, so a client
    /// can't tell "closed unread" from "closed"; that the listener closes a
    /// refused peer before reading is its own test's
    /// (`platform::windows::listen`). Here: the core keeps nothing of a
    /// peer that runs as another user, and that peer reads the end.
    #[cfg(windows)]
    #[test]
    fn a_foreign_user_is_closed_and_never_becomes_a_client() {
        use std::io::{Read, Write};

        let hello = [1u8, 0, 0, 0, 2, b'{', b'}'];

        let temp = TempDir::new("core-peer");
        // As if this core ran as someone else.
        let mut core = core_as(&temp, Identity::own().other());
        let mut peer = listen::connect(&core.paths.endpoint).unwrap();
        peer.write_all(&hello).unwrap();
        core.accept();
        assert!(core.pending.is_empty() && core.clients.is_empty());
        let mut rest = Vec::new();
        peer.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        assert_eq!(peer.read_to_end(&mut rest).unwrap(), 0, "closed, with no answer");
        drop(core);

        // The same connection from its own user waits for its HELLO.
        let temp = TempDir::new("core-peer-own");
        let mut core = core_as(&temp, Identity::own());
        let mut peer = listen::connect(&core.paths.endpoint).unwrap();
        peer.write_all(&hello).unwrap();
        core.accept();
        assert_eq!(core.pending.len(), 1);
    }

    /// A replay waits until its holder's socket has the session's new
    /// owner. Linux only: the stand-in holder is a locked file and a Unix
    /// socket, and on Windows `remote-agent/tests/core.rs`'s
    /// `an_attach_holds_when_the_core_is_killed_as_its_replay_starts`
    /// covers it.
    #[cfg(unix)]
    mod stored_before_told {
        use std::fs::File;
        use std::os::fd::AsRawFd as _;

        use super::*;

        use brindle_remote_protocol::Frame;
        use brindle_remote_protocol::client as c1;
        use brindle_remote_protocol::holder::{self as h1, SessionMeta, ToHolder};

        /// The other end of one of the core's connections, read and written
        /// without blocking, since the test turns the core's loop itself.
        struct Peer(conn::Conn);

        impl Peer {
            fn send(&mut self, frame: io::Result<Frame>) {
                self.0.queue(&frame.unwrap());
                self.0.flush().unwrap();
                assert_eq!(self.0.queued(), 0, "the core's socket was full");
            }

            /// The next frame the core has sent, if one is there.
            fn frame(&mut self) -> Option<Frame> {
                if let Some(frame) = self.0.next_frame().unwrap() {
                    return Some(frame);
                }
                let _ = self.0.read_available();
                self.0.next_frame().unwrap()
            }

            fn message(&mut self) -> Option<ToClient> {
                self.frame().map(|frame| ToClient::decode(&frame).unwrap())
            }
        }

        const SESSION: SessionId = 1;
        const ATTACH: RequestId = 7;

        /// A core that adopted session 1, which is A's, from a holder that then
        /// stopped reading with far more queued for it than its socket holds,
        /// and client B, whose `ATTACH` of the session the core has handled:
        /// the `SET_META` naming B is queued behind all that.
        struct WaitingAttach {
            _temp: TempDir,
            core: Core<Loop>,
            events: Vec<Event>,
            /// The holder's end of the core's connection, until a test ends it.
            holder: Option<Peer>,
            /// What makes it a live holder for the core: its lock and socket.
            holder_files: Option<(File, Listener)>,
            client: Peer,
        }

        impl WaitingAttach {
            fn holder(&mut self) -> &mut Peer {
                self.holder.as_mut().expect("the holder")
            }

            fn turn(&mut self) {
                self.core
                    .turn(&mut self.events, Duration::from_millis(5))
                    .unwrap();
            }

            /// Turns the core until `done`, for a few seconds at most.
            fn turn_until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
                let deadline = Instant::now() + Duration::from_secs(10);
                while !done(self) {
                    assert!(Instant::now() < deadline, "never happened: {what}");
                    self.turn();
                }
            }

            fn new(tag: &str) -> Self {
                let temp = TempDir::new(tag);
                // A live holder: its lock held, its socket listening.
                let lock = File::create(temp.0.join("holder-test-0.lock")).unwrap();
                assert_eq!(
                    unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
                    0
                );
                let listener =
                    Listener::bind(&temp.0.join("holder-test-0.sock"), Identity::own()).unwrap();
                let mut core = core_as(&temp, Identity::own());
                core.discover(true);
                let accepted = listener.accept().unwrap().expect("the core's connection");
                let holder = Peer(conn::Conn::new(accepted).unwrap());
                let client = listen::connect(&temp.0.join("core.sock")).unwrap();
                let mut test = Self {
                    _temp: temp,
                    core,
                    events: Vec::new(),
                    holder: Some(holder),
                    holder_files: Some((lock, listener)),
                    client: Peer(conn::Conn::new(client).unwrap()),
                };

                // The holder welcomes the core with A's session.
                let mut hello = None;
                test.turn_until("the core's HELLO", |test| {
                    hello = test.holder().frame();
                    hello.is_some()
                });
                assert!(matches!(
                    ToHolder::decode(&hello.unwrap()).unwrap(),
                    ToHolder::Hello(_)
                ));
                let meta = SessionMeta {
                    command: "cat".into(),
                    key: "key".into(),
                    owner: "a".into(),
                    policy: c1::Policy::Detach,
                    scrollback: 100,
                    ..SessionMeta::default()
                };
                test.holder().send(
                    h1::ToCore::Welcome(h1::Welcome {
                        protocol: h1::PROTOCOLS.max,
                        build: "test".into(),
                        pid: std::process::id(),
                        previous: h1::Previous::Crash,
                        sessions: vec![h1::HeldSession {
                            session: SESSION,
                            pid: 4242,
                            cols: 80,
                            rows: 24,
                            exit: None,
                            meta: meta.to_value(),
                            truncated: false,
                            mode: h1::SpawnMode::Pty,
                        }],
                    })
                    .encode(),
                );
                let blobs = h1::ToCore::checkpoint(SESSION, b"before")
                    .into_iter()
                    .chain(h1::ToCore::tail(SESSION, b""));
                for message in blobs {
                    test.holder().send(message.encode());
                }
                test.turn_until("accepting clients", |test| test.core.lifecycle.accepting());

                // B connects.
                test.client.send(
                    c1::ToCore::Hello(c1::Hello {
                        min: c1::PROTOCOLS.min,
                        max: c1::PROTOCOLS.max,
                        client_version: "test".into(),
                        client_token: "b".into(),
                        ..Default::default()
                    })
                    .encode(),
                );
                let mut welcome = None;
                test.turn_until("the client's WELCOME", |test| {
                    welcome = test.client.message();
                    welcome.is_some()
                });
                assert!(matches!(welcome, Some(ToClient::Welcome(_))), "{welcome:?}");

                // The holder reads nothing from here on, and its socket fills.
                let token = *test.core.holders.keys().next().unwrap();
                for _ in 0..8 {
                    test.core.send_holder(
                        token,
                        ToHolder::Input(h1::Data {
                            session: SESSION,
                            bytes: vec![b'x'; 1024 * 1024 - 64],
                        }),
                    );
                }
                test.turn();
                assert!(
                    test.core.holders[&token].conn.queued() > 0,
                    "the holder's socket took 8 MiB unread"
                );

                // B attaches, and the session prints while the replay waits.
                test.client.send(
                    c1::ToCore::Attach(c1::Attach {
                        req: ATTACH,
                        session: SESSION,
                        cols: 100,
                        rows: 30,
                    })
                    .encode(),
                );
                test.turn_until("the ATTACH is handled", |test| {
                    test.core.sessions[&SESSION].ownership.client().is_some()
                });
                test.holder().send(
                    h1::ToCore::Output(h1::Data {
                        session: SESSION,
                        bytes: b" and after".to_vec(),
                    })
                    .encode(),
                );
                for _ in 0..5 {
                    test.turn();
                    let told = test.client.message();
                    assert!(told.is_none(), "told before its meta was stored: {told:?}");
                }
                test
            }

            /// The rest of the replay whose `REPLAY_START` the client has, from
            /// what its socket already holds: the core isn't turned.
            fn replay_body(&mut self) -> Vec<u8> {
                let mut bytes = Vec::new();
                loop {
                    match self.client.message() {
                        Some(ToClient::Data(data)) => bytes.extend(data.bytes),
                        Some(ToClient::ReplayEnd(_)) => return bytes,
                        other => panic!("in the replay: {other:?}"),
                    }
                }
            }
        }

        fn is_replay_start(message: &ToClient) -> bool {
            matches!(
                message,
                ToClient::ReplayStart(start) if start.req == Some(ATTACH) && start.session == SESSION
            )
        }

        #[test]
        fn an_attachs_replay_waits_until_the_holders_socket_has_its_new_owner() {
            let mut test = WaitingAttach::new("core-stored");
            // The holder reads on, a part at a time.
            let mut start = None;
            test.turn_until("the replay", |test| {
                let _ = test.holder().0.read_available();
                start = test.client.message();
                start.is_some()
            });
            // Nothing came before it, and what the session printed meanwhile is
            // in it.
            assert!(start.as_ref().is_some_and(is_replay_start), "{start:?}");
            let replay = String::from_utf8(test.replay_body()).unwrap();
            assert!(replay.contains("before and after"), "{replay:?}");

            // The core is killed now, as far as the holder can tell: it reads
            // what its socket holds, then the end. B's ownership is in that.
            let WaitingAttach { core, holder, .. } = test;
            let mut holder = holder.expect("the holder");
            drop(core);
            while matches!(holder.0.read_available(), Ok(false)) {}
            let mut owner = None;
            while let Some(frame) = holder.0.next_frame().unwrap() {
                if let ToHolder::SetMeta(update) = ToHolder::decode(&frame).unwrap() {
                    assert_eq!(update.session, SESSION);
                    owner = Some(SessionMeta::from_value(&update.meta).unwrap().owner);
                }
            }
            assert_eq!(owner.as_deref(), Some("b"));
        }

        #[test]
        fn a_replay_waiting_for_its_holder_goes_out_when_the_holder_is_lost() {
            let mut test = WaitingAttach::new("core-stored-lost");
            // The holder goes, with the new owner never written to it.
            test.holder = None;
            test.holder_files = None;
            let mut start = None;
            test.turn_until("the replay", |test| {
                start = test.client.message();
                start.is_some()
            });
            // The replay answers the ATTACH, then the session is lost.
            assert!(start.as_ref().is_some_and(is_replay_start), "{start:?}");
            let replay = String::from_utf8(test.replay_body()).unwrap();
            assert!(replay.contains("before and after"), "{replay:?}");
            let mut exit = None;
            test.turn_until("the session's loss", |test| {
                exit = test.client.message();
                exit.is_some()
            });
            match exit {
                Some(ToClient::Exit(exit)) => {
                    assert_eq!((exit.session, exit.code), (SESSION, None));
                    assert!(exit.lost.is_some());
                }
                other => panic!("{other:?}"),
            }
            assert!(test.core.sessions.is_empty());
        }
    }
}
