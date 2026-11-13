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
//!   stalled client blocks only its own programs.
//! - **Holders** ([`holders`]): found by their `holder-*.sock` sockets (on
//!   Windows by their lock files, which name their pipes), live ones told
//!   from stale ones by their lock files, and adopted (`HELLO`,
//!   then `WELCOME` with the sessions, checkpoints and tails). Clients are
//!   accepted once the holders found at startup are adopted, so session ids
//!   continue from the largest adopted one and returning clients find their
//!   sessions. A holder of the core's own build is started when a session
//!   needs one, and holders of other builds are told to `DRAIN`.
//! - **Management and upgrades** ([`manage`]): `LIST`, `STATUS`, and
//!   `STEP_DOWN`, after which the core exits for the next one.
//! - **Sessions** ([`sessions`]): spawned through a holder, with their I/O,
//!   the close policies and, after adopting sessions, the
//!   [`RECONNECT_GRACE`] for their owners to come back. Each has an
//!   emulator ([`screen`]) fed with all of its output.
//! - **Exit:** after [`UNUSED_LIMIT`] with no sessions and no clients, once
//!   `STOP` has ended every session, or after `STEP_DOWN`. On Linux the
//!   socket is removed first.
//! - **Windows:** a client's `REPARENT` goes to its session's holder, and
//!   at its start the core removes the helper copies of other builds that
//!   nothing runs from any more ([`prune_copies`]).

mod clients;
mod holders;
mod manage;
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
use self::sessions::Session;
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
/// Past this much queued to a client, its sessions are paused...
pub const CLIENT_QUEUE_HIGH: usize = 1024 * 1024;
/// ...until its queue is back under this.
pub const CLIENT_QUEUE_LOW: usize = 256 * 1024;
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
    let dir = dir::from_env().map_err(|error| error.to_string())?;
    let paths = Paths::new(&dir).map_err(|error| error.to_string())?;
    let lock = match claim(&paths).map_err(|error| error.to_string())? {
        Claim::Core(lock) => lock,
        Claim::Running => {
            // Another core serves this directory; its starter can connect.
            if let Some(ready) = ready {
                ready.signal().map_err(|error| error.to_string())?;
            }
            return Ok(());
        }
    };
    if ready.is_some() {
        let log = daemon::open_log(&paths.log)
            .map_err(|error| format!("can't open {}: {error}", paths.log.display()))?;
        daemon::daemonize(&log).map_err(|error| error.to_string())?;
    }
    let build = build_id::current().map_err(|error| format!("can't read the build id: {error}"))?;
    prune_copies(&build);
    let ev = Loop::new().map_err(|error| error.to_string())?;
    let core = Core::new(ev, dir, paths, lock, build, Identity::own()).map_err(|error| {
        log!("can't start: {error}");
        error.to_string()
    })?;
    if let Some(ready) = ready
        && let Err(error) = ready.signal()
    {
        log!("couldn't tell the starter it is listening: {error}");
    }
    core.run().map_err(|error| {
        log!("stopped: {error}");
        error.to_string()
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

/// Removes the per-build helper copies of other builds, where nothing runs
/// from them any more (Windows runs every role from a copy, since a running
/// image can't be replaced). A copy still in use stays for a later core.
#[cfg(windows)]
fn prune_copies(build: &str) {
    use brindle_remote_protocol::{server_dir, win};

    let server_dir = std::env::var_os("BRINDLE_SERVER_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from);
    let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|dir| !dir.is_empty()) else {
        return;
    };
    let copies = server_dir::helper_copies(server_dir.as_deref(), Path::new(&local));
    win::prune_helper_copies(&copies, build);
}

/// Only Windows runs its roles from per-build copies.
#[cfg(not(windows))]
fn prune_copies(_build: &str) {}

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

pub(super) struct Core<L: EventLoop> {
    ev: L,
    dir: PathBuf,
    paths: Paths,
    _lock: Lock,
    /// Closed when the core exits, before its last messages go out.
    listener: Option<Listener>,
    /// Clients are accepted once the startup adoption is done.
    accepting: bool,
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
    /// While the owners of adopted sessions may still come back.
    grace_until: Option<Instant>,
    idle_since: Option<Instant>,
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
            accepting: false,
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
            grace_until: None,
            idle_since: None,
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

    fn flush_all(&mut self) {
        let clients: Vec<Token> = self.clients.keys().copied().collect();
        for client in clients {
            self.flush_client(client);
        }
        let holders: Vec<Token> = self.holders.keys().copied().collect();
        for holder in holders {
            self.flush_holder(holder);
        }
    }

    fn update_interests(&mut self) {
        let listener = if self.accepting {
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

    fn tick(&mut self, now: Instant) {
        self.expire_pending(now);
        if !self.accepting && self.startup_adopted(now) {
            self.accepting = true;
            if self.sessions.values().any(|session| session.awaiting_owner) {
                self.grace_until = Some(now + RECONNECT_GRACE);
                log!(
                    "adopted {} session(s); their owners have {}s to come back",
                    self.sessions.len(),
                    RECONNECT_GRACE.as_secs()
                );
            }
        }
        if self.grace_until.is_some_and(|until| now >= until) {
            self.grace_until = None;
            self.end_grace();
        }
        self.check_holder_start(now);
        let idle = self.sessions.is_empty()
            && self.clients.is_empty()
            && self.pending.is_empty()
            && self.waiting.is_empty()
            && self.holder_start.is_none();
        if !idle {
            self.idle_since = None;
        } else if self.idle_since.is_none() {
            self.idle_since = Some(now);
        }
    }

    fn should_exit(&self, now: Instant) -> bool {
        if self.stepping_down.is_some() || (self.stopping.is_some() && self.sessions.is_empty()) {
            return true;
        }
        self.idle_since
            .is_some_and(|since| now.duration_since(since) >= UNUSED_LIMIT)
    }

    /// Closes these holders' and clients' connections gracefully, all at
    /// once, within `limit`: each gets its queue and the connection's end,
    /// and the core reads until the peer closes too, since closing with
    /// unread input resets the connection and loses what the peer hadn't
    /// read yet. Holders' output and exits still go to their clients
    /// meanwhile. Each closed connection is dropped; the result lists those
    /// that failed or ran out of time.
    fn close_peers(
        &mut self,
        holders: &[Token],
        clients: &[Token],
        limit: Duration,
    ) -> Vec<(Token, io::Error)> {
        let deadline = Instant::now() + limit;
        let mut open: Vec<(Token, bool)> = holders
            .iter()
            .map(|&token| (token, true))
            .chain(clients.iter().map(|&token| (token, false)))
            .collect();
        let mut failed = Vec::new();
        loop {
            let mut still = Vec::new();
            for (token, holder) in open {
                let step = if holder {
                    let Some(entry) = self.holders.get_mut(&token) else {
                        continue;
                    };
                    let step = entry.conn.close_step();
                    self.closing_holder_frames(token);
                    step
                } else {
                    let Some(entry) = self.clients.get_mut(&token) else {
                        continue;
                    };
                    let step = entry.conn.close_step();
                    entry.conn.discard_input();
                    step
                };
                match step {
                    Ok(false) => still.push((token, holder)),
                    Ok(true) => self.forget_peer(token, holder),
                    Err(error) => {
                        failed.push((token, error));
                        self.forget_peer(token, holder);
                    }
                }
            }
            open = still;
            if open.is_empty() {
                return failed;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let conns: Vec<&conn::Conn> = open
                .iter()
                .filter_map(|&(token, holder)| {
                    if holder {
                        self.holders.get(&token).map(|entry| &entry.conn)
                    } else {
                        self.clients.get(&token).map(|entry| &entry.conn)
                    }
                })
                .collect();
            if remaining.is_zero() {
                failed.extend(
                    open.iter()
                        .zip(&conns)
                        .map(|(&(token, _), conn)| (token, conn.unfinished())),
                );
                return failed;
            }
            conn::wait(&conns, remaining);
        }
    }

    /// Drops a peer's connection while the core exits.
    fn forget_peer(&mut self, token: Token, holder: bool) {
        self.drop_source(token);
        if holder {
            self.holders.remove(&token);
        } else {
            self.clients.remove(&token);
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
            let holders: Vec<Token> = self.holders.keys().copied().collect();
            for (_, error) in self.close_peers(&holders, &[], STEP_DOWN_FLUSH_LIMIT) {
                log!("a holder didn't take its checkpoints: {error}");
            }
        }
        let stopped = self.stopping.take();
        if let Some(answer) = stopped.as_ref().or(stepped_down.as_ref()) {
            self.send_client(answer.client, ToClient::Ok(Request { req: answer.req }));
        }
        let holders: Vec<Token> = self.holders.keys().copied().collect();
        let clients: Vec<Token> = self.clients.keys().copied().collect();
        for (token, error) in self.close_peers(&holders, &clients, EXIT_FLUSH_LIMIT) {
            if holders.contains(&token) {
                log!("holder {token} didn't take its last messages: {error}");
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
}
