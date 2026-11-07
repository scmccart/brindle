//! `brindle-remote serve`: the core, which clients talk to.
//!
//! The core owns the client socket and drives the holders, which own the
//! PTYs. It can restart at any time without ending a session: a new core
//! adopts the running holders and their sessions.
//!
//! - **Single instance:** `core.lock` is flocked for the core's whole life,
//!   and only then is a stale `core.sock` unlinked and bound. A second
//!   `serve` that can't take the lock waits until the winner accepts
//!   connections, tells its starter it is ready, and exits 0.
//! - **Clients** ([`clients`]): the peer check before reading anything, the
//!   `HELLO`/`WELCOME` handshake, and per-client queues. Past
//!   [`CLIENT_QUEUE_HIGH`] queued to a client, its sessions are paused in
//!   their holders until the queue is under [`CLIENT_QUEUE_LOW`], so a
//!   stalled client blocks only its own programs.
//! - **Holders** ([`holders`]): found by their `holder-*.sock` sockets, live
//!   ones told from stale ones by their lock files, and adopted (`HELLO`,
//!   then `WELCOME` with the sessions, checkpoints and tails). Clients are
//!   accepted once the holders found at startup are adopted, so session ids
//!   continue from the largest adopted one and returning clients find their
//!   sessions. A holder of the core's own build is started when a session
//!   needs one.
//! - **Sessions** ([`sessions`]): spawned through a holder, with their I/O,
//!   the close policies and, after adopting sessions, the
//!   [`RECONNECT_GRACE`] for their owners to come back. Each has an
//!   emulator ([`screen`]) fed with all of its output.
//! - **Exit:** after [`UNUSED_LIMIT`] with no sessions and no clients, or
//!   once `STOP` has ended every session. The socket is removed first.

mod clients;
mod holders;
mod screen;
mod sessions;

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use brindle_remote_protocol::client::{Request, ToClient};
use brindle_remote_protocol::{RequestId, SessionId};

use self::clients::{Client, Pending};
use self::holders::{Holder, HolderStart, WaitingSpawn};
use self::sessions::Session;
use super::event_loop::{Event, EventLoop, Interest, PollLoop, Token};
use super::{build_id, daemon, dir};

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

/// Runs `brindle-remote serve [--ready-fd N]`. Without `--ready-fd` it stays
/// in the foreground and logs to stderr.
pub fn run(args: &[String]) -> Result<(), String> {
    let ready_fd = daemon::parse_ready_fd("serve", args)?;
    let ready = ready_fd
        .map(daemon::Ready::from_fd)
        .transpose()
        .map_err(|error| error.to_string())?;
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
    let ev = PollLoop::new().map_err(|error| error.to_string())?;
    let core = Core::new(ev, dir, paths, lock, build, dir::own_uid()).map_err(|error| {
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

/// The core's files in the server directory.
struct Paths {
    lock: PathBuf,
    socket: PathBuf,
    log: PathBuf,
}

impl Paths {
    fn new(dir: &Path) -> io::Result<Self> {
        let socket = dir.join("core.sock");
        // sun_path holds 108 bytes with the terminating NUL.
        if socket.as_os_str().len() > 107 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "the socket path {} is too long for a Unix socket; set a shorter BRINDLE_SERVER_DIR",
                    socket.display()
                ),
            ));
        }
        Ok(Self {
            lock: dir.join("core.lock"),
            socket,
            log: dir.join("core.log"),
        })
    }
}

enum Claim {
    /// This process holds `core.lock` and becomes the core.
    Core(File),
    /// Another core holds it and accepts connections.
    Running,
}

/// Takes `core.lock`, or waits up to [`LOSER_WAIT`] for the core that holds
/// it to accept connections (it may still be starting, or exiting).
fn claim(paths: &Paths) -> io::Result<Claim> {
    let deadline = Instant::now() + LOSER_WAIT;
    loop {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&paths.lock)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(Claim::Core(lock));
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::WouldBlock {
            return Err(error);
        }
        if UnixStream::connect(&paths.socket).is_ok() {
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
    let mut bytes = [0u8; 8];
    let read = unsafe { libc::getrandom(bytes.as_mut_ptr().cast(), bytes.len(), 0) };
    if read == bytes.len() as isize {
        return u64::from_ne_bytes(bytes);
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

/// A `STOP` in progress: the client and request to answer once every
/// session has ended.
struct Stopping {
    client: Token,
    req: RequestId,
}

pub(super) struct Core<L: EventLoop> {
    ev: L,
    dir: PathBuf,
    paths: Paths,
    _lock: File,
    listener: UnixListener,
    /// Clients are accepted once the startup adoption is done.
    accepting: bool,
    uid: u32,
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
    stopping: Option<Stopping>,
}

impl<L: EventLoop> Core<L> {
    fn new(
        mut ev: L,
        dir: PathBuf,
        paths: Paths,
        lock: File,
        build: String,
        uid: u32,
    ) -> io::Result<Self> {
        // The lock is ours, so any socket at this path is stale.
        match fs::remove_file(&paths.socket) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&paths.socket).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("can't listen on {}: {error}", paths.socket.display()),
            )
        })?;
        listener.set_nonblocking(true)?;
        ev.register(listener.as_raw_fd(), LISTENER, Interest::NONE)?;
        log!(
            "listening on {} (version {}, build {build})",
            paths.socket.display(),
            env!("CARGO_PKG_VERSION")
        );
        Ok(Self {
            ev,
            dir,
            paths,
            _lock: lock,
            listener,
            accepting: false,
            uid,
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
        if self.stopping.is_some() && self.sessions.is_empty() {
            return true;
        }
        self.idle_since
            .is_some_and(|since| now.duration_since(since) >= UNUSED_LIMIT)
    }

    fn shutdown(mut self) {
        // Nothing can find this core once the socket is gone.
        let _ = fs::remove_file(&self.paths.socket);
        let stopped = self.stopping.take();
        if let Some(stopping) = &stopped {
            self.send_client(stopping.client, ToClient::Ok(Request { req: stopping.req }));
        }
        for client in self.clients.values_mut() {
            let _ = client.conn.flush_for(EXIT_FLUSH_LIMIT);
        }
        for holder in self.holders.values_mut() {
            let _ = holder.conn.flush_for(EXIT_FLUSH_LIMIT);
        }
        if stopped.is_some() {
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
    use crate::server::peer::tests::{closed_unread, send_hello};

    /// A core in a fresh directory that runs, as far as its peer check is
    /// concerned, as `uid`.
    fn core_as(temp: &TempDir, uid: u32) -> Core<PollLoop> {
        let paths = Paths::new(&temp.0).unwrap();
        let Claim::Core(lock) = claim(&paths).unwrap() else {
            panic!("another core holds the lock");
        };
        let ev = PollLoop::new().unwrap();
        Core::new(ev, temp.0.clone(), paths, lock, "test".into(), uid).unwrap()
    }

    #[test]
    fn a_foreign_uid_is_closed_without_being_read() {
        let temp = TempDir::new("core-peer");
        // As if this core ran as someone else.
        let mut core = core_as(&temp, dir::own_uid().wrapping_add(1));
        let mut stream = UnixStream::connect(temp.0.join("core.sock")).unwrap();
        send_hello(&mut stream);
        core.accept();
        assert!(core.pending.is_empty() && core.clients.is_empty());
        assert!(closed_unread(&mut stream));
        drop(core);

        // The same connection from its own user waits for its HELLO.
        let temp = TempDir::new("core-peer-own");
        let mut core = core_as(&temp, dir::own_uid());
        let mut stream = UnixStream::connect(temp.0.join("core.sock")).unwrap();
        send_hello(&mut stream);
        core.accept();
        assert_eq!(core.pending.len(), 1);
    }
}
