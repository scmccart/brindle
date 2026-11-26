//! Brindle's client for the local terminal server.
//!
//! Local tabs run in sessions of the terminal server, `brindle-remote serve`
//! (the core) and the holders it drives. This module finds the helper
//! ([`super::find_server_helper`]), resolves the server directory as the
//! helper does ([`server_dir::resolve`], [`server_dir::resolve_windows`]),
//! starts the core when none is listening (it detaches itself and writes
//! one byte once it accepts connections), connects to it and negotiates
//! protocol 1 with this process's random `client_token`.
//!
//! What differs by platform is in one module each, with the same items
//! (`Stream`, `connect`, `address`, `start`, `image_of`, `kill`, and the
//! tests' `Listener`):
//! - **Linux** (`unix.rs`): the core listens on `core.sock` in the server
//!   directory, and is started as `serve --ready-fd 3`.
//! - **Windows** (`windows.rs`, `pipe.rs`): the core listens on a named pipe
//!   named from the directory, and is started from a per-build copy of the
//!   helper as `launch serve --ready-handle N`.
//!
//! Every failure is an error naming its reason and the fix, shown in the
//! tab that needed the server. Nothing is cached, so the next tab tries
//! again.
//!
//! The process keeps one [`Connection`] per endpoint ([`Host`]) for all
//! its tabs ([`shared`]). A tab's [`Session`] gets it, and its session, on
//! a thread of its own. A remote endpoint's connection goes through a
//! relay (`relay.rs`) instead of the local socket or pipe. The endpoints of
//! profiles that connect at launch are kept connected ([`keep_connected`]),
//! and each remote connection tells [`watch_hosts`]' callback how it
//! stands, for `remote::hosts::Hosts`.

mod agent;
mod connection;
mod endpoint_env;
mod link;
#[cfg(windows)]
mod pipe;
mod relay;
mod session;
#[cfg(unix)]
#[path = "local/unix.rs"]
mod sys;
#[cfg(windows)]
#[path = "local/windows.rs"]
mod sys;
mod upgrade;

pub use connection::{
    AttachRequest, Attaching, Connection, Foreground, ListingHook, Report, SessionSink, SpawnRequest,
};
pub use super::Host;
/// What `remote::client`'s tests of the relay's launch against real
/// endpoints start for themselves.
#[cfg(test)]
pub(super) use {
    link::Link,
    relay::{dial_with, start as start_relay},
};
pub use session::Session;

use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use parking_lot::Mutex;

use super::client::{HelperLookup, default_helper};
use super::hosts::LinkState;
use super::protocol::client::{self, Hello, ToClient, ToCore, Welcome};
use super::protocol::{Frame, server_dir};
pub use sys::Stream;

/// How long the starter waits for a new core's ready byte (design.md).
const START_LIMIT: Duration = Duration::from_secs(2);
/// How long a core may take to answer `HELLO`. A core that just started
/// accepts clients only once it has adopted the running holders (at most
/// 5 s), so this is longer than that.
const HELLO_LIMIT: Duration = Duration::from_secs(10);
/// How long connecting keeps retrying after a core said it was ready.
const CONNECT_RETRY: Duration = Duration::from_secs(1);
/// How long connecting waits for a core that is there but takes no client
/// yet (Windows: its one waiting pipe instance is taken until it accepts,
/// which a new core does only once it has adopted its holders). As long as
/// such a core may take to answer `HELLO`.
const BUSY_LIMIT: Duration = HELLO_LIMIT;
/// How long a remote endpoint's core keeps this client's `kill` sessions
/// after a relay link drops, for the reconnect to get them back.
const RELAY_LINGER_SECS: u32 = 300;

/// The client protocols Brindle offers: every one the codec knows, 1 to 4.
/// Protocol 4 adds agent sessions, which restore, the sweep, `--attach`
/// and the sessions palette tell apart from terminals (`agents-profiles`
/// design.md decision 6). With a core that can't go past 3, agent zones
/// say the server is too old (`agent_start`).
const BRINDLE_PROTOCOLS: client::VersionRange = client::PROTOCOLS;
/// How long an exiting Brindle waits, at most, for its connections to write
/// what its closing tabs sent ([`flush`]). Writing to a link that is up
/// takes far less.
pub const QUIT_FLUSH: Duration = Duration::from_millis(500);

#[cfg(not(windows))]
pub const HELPER_MISSING: &str =
    "terminal server helper not found: set BRINDLE_SERVER_HELPER or install brindle-remote beside brindle";
#[cfg(windows)]
pub const HELPER_MISSING: &str =
    "terminal server helper not found: set BRINDLE_SERVER_HELPER or install brindle-remote.exe beside brindle.exe";

/// This process's `client_token`: random, and the same for every
/// connection it makes, so a reconnect gets its sessions back.
pub fn client_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        let mut random = [0u8; 16];
        if getrandom::fill(&mut random).is_err() {
            // Unique enough within one boot, which is all a token needs.
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            random[..16].copy_from_slice(&nanos.to_le_bytes());
        }
        let hex: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        format!("brindle-{}-{hex}", std::process::id())
    })
}

/// The process's connections, one per endpoint.
static CONNECTIONS: LazyLock<Registry> = LazyLock::new(Registry::default);
/// The remote endpoints the process keeps connected ([`keep_connected`]).
static EAGER: Mutex<Vec<Host>> = Mutex::new(Vec::new());
/// Where remote connections report how they stand ([`watch_hosts`]).
type Watcher = Box<dyn Fn(&Host, u64, LinkState) + Send + Sync>;
static WATCHER: OnceLock<Watcher> = OnceLock::new();

/// The process's connection to `host`'s core. A local one is made
/// (starting a core if needed, and offering it an upgrade) when there is
/// none or the last one ended; it blocks, so it runs on a session's start
/// thread ([`Session::start`]), never on the UI thread. A remote one returns
/// at once and connects in the background, with requests waiting for it;
/// it is eager from the start when the process keeps `host` connected, and
/// reports to [`watch_hosts`]' callback.
pub fn shared(host: &Host) -> Result<Arc<Connection>> {
    shared_with(host, &default_helper())
}

/// [`shared`], where a remote endpoint's relay deploys the helper `helper`
/// finds (production's is [`default_helper`]: `BRINDLE_REMOTE_AGENT`, else
/// the one beside Brindle). Only a connection made now uses it: one the
/// process already has for `host` is returned as it is.
pub(crate) fn shared_with(host: &Host, helper: &HelperLookup) -> Result<Arc<Connection>> {
    CONNECTIONS.get(host, || match host {
        Host::Local => {
            let endpoint = Endpoint::from_env()?;
            let connected = connect(&endpoint, client_token())?;
            let offered = upgrade::offer(&endpoint, client_token(), connected)?;
            if let upgrade::Offered::Kept { why, .. } = &offered {
                log::warn!("keeping the running terminal server on {}: {why}", host.name());
            }
            Connection::start(endpoint, client_token(), offered.connected())
        }
        Host::Remote(target) => {
            let dial = relay::dial_with(host.name(), target, helper.clone());
            let eager = EAGER.lock().contains(host);
            let report = WATCHER.get().map(|_| {
                let host = host.clone();
                Arc::new(move |conn: u64, state: LinkState| {
                    if let Some(watch) = WATCHER.get() {
                        watch(&host, conn, state);
                    }
                }) as Report
            });
            let token = host_token(client_token(), host);
            Connection::relay_with(dial, &token, BRINDLE_PROTOCOLS, relay::Timings::default(), eager, report)
        }
    })
}

/// The process's connection to `host` if it has one that hasn't ended. It
/// never connects.
pub fn existing(host: &Host) -> Option<Arc<Connection>> {
    CONNECTIONS.existing(host)
}

/// Waits up to `limit` for every connection of the process to write what
/// was sent on it so far ([`Connection::flush`]); true when all did. Brindle
/// calls it as it exits, once its tabs are gone: quitting detaches their
/// sessions, and a `DETACH` still queued when the process ends is lost (on
/// Windows the exit ends every other thread at once). The core would then
/// see only the connection close, leave the sessions their owner, and the
/// next launch would leave them in the sessions palette for 5 minutes
/// instead of opening them as tabs.
pub fn flush(limit: Duration) -> bool {
    CONNECTIONS.flush(Instant::now() + limit)
}

/// Makes `watch` the callback every remote connection made from now on
/// reports its changes of state to, with its host and its id: once, before
/// any remote connection is made. It is called with a connection's lock
/// held, from its threads, so it must not block.
pub fn watch_hosts(watch: impl Fn(&Host, u64, LinkState) + Send + Sync + 'static) {
    if WATCHER.set(Box::new(watch)).is_err() {
        log::warn!("remote endpoints are already watched");
    }
}

/// Keeps the remote endpoints in `hosts` connected (profiles that connect at
/// launch): each one's connection is eager, never closing for want of use
/// and retrying for as long as the process runs, and one not connected yet
/// is connected to in the background. An endpoint no longer listed loses
/// that, so its connection closes once nothing uses it. Doesn't block.
pub fn keep_connected(hosts: Vec<Host>) {
    let hosts: Vec<Host> = hosts.into_iter().filter(|host| !host.is_local()).collect();
    let (added, removed) = {
        let mut eager = EAGER.lock();
        let change = eager_change(&eager, &hosts);
        *eager = hosts;
        change
    };
    // Not under `EAGER`: `shared` takes it under a registry slot's lock.
    for host in &removed {
        if let Some(connection) = existing(host) {
            connection.set_eager(false);
        }
    }
    if added.is_empty() {
        return;
    }
    let spawned = std::thread::Builder::new().name("eager-connect".into()).spawn(move || {
        for host in added {
            match shared(&host) {
                // One made just now is eager already; one a tab made isn't.
                // Under `EAGER`, so a later call that drops the host wins.
                Ok(connection) => {
                    let eager = EAGER.lock();
                    if eager.contains(&host) {
                        connection.set_eager(true);
                    }
                }
                Err(error) => report_offline(&host, &error),
            }
        }
    });
    if let Err(error) = spawned {
        log::warn!("not connecting at launch: {error}");
    }
}

/// Reports `host` offline to [`watch_hosts`]' callback, `error` being why
/// no connection to it could be made, under a connection id of its own, so
/// it is ordered with the reports of the connections before and after it
/// (`Hosts`). The endpoint's row in the sessions palette then says why. It
/// is logged too, for a process that watches no endpoint.
fn report_offline(host: &Host, error: &anyhow::Error) {
    let why = format!("connecting to {}: {error:#}", host.name());
    log::warn!("{why}");
    if let Some(watch) = WATCHER.get() {
        watch(host, connection::next_connection_id(), LinkState::Offline(why));
    }
}

/// Which of `new` aren't in `old` (to connect), and which of `old` aren't in
/// `new` (to let go), each in its list's order.
fn eager_change(old: &[Host], new: &[Host]) -> (Vec<Host>, Vec<Host>) {
    let added = new.iter().filter(|host| !old.contains(host)).cloned().collect();
    let removed = old.iter().filter(|host| !new.contains(host)).cloned().collect();
    (added, removed)
}

/// Hands `hook` the sessions `host`'s core lists once the process's
/// connection to it is up: at once if it is, else on its first link
/// (design.md decision 10). Until then the connection keeps trying, unless
/// the hook is withdrawn ([`stop_listing`]). For a remote host: a local one
/// would block here while connecting. Not on the UI thread.
pub fn when_listed(host: &Host, hook: ListingHook) {
    let mut hook = hook;
    // A connection that ended just now is replaced by the next `shared`.
    for _ in 0..3 {
        let connection = match shared(host) {
            Ok(connection) => connection,
            Err(error) => {
                report_offline(host, &error);
                return;
            }
        };
        match connection.on_listing(hook) {
            Ok(()) => return,
            Err(back) => hook = back,
        }
    }
    log::warn!("not listing {}'s sessions: its connection keeps ending", host.name());
}

/// Stops waiting for `host`'s first listing ([`when_listed`]): the hooks of
/// the process's connection to it are dropped without a listing, so they no
/// longer keep it trying (design.md decision 4). For the endpoint of a
/// window's last tab that went, when no profile connects to it at launch:
/// the launch then doesn't open its detached sessions as tabs. It never
/// connects, and does nothing for this machine.
pub fn stop_listing(host: &Host) {
    if host.is_local() {
        return;
    }
    if let Some(connection) = existing(host)
        && connection.stop_listing()
    {
        log::info!("not waiting for {}'s sessions any more: no tab is on it", host.name());
    }
}

/// The `client_token` of a connection to `host`, from the process's own
/// token `base` ([`client_token`]). Each remote endpoint gets one of its
/// own, the same for every reconnect: a core takes a connection's sessions
/// over from any other client with its token, and two endpoints can be one
/// core (`ssh localhost` beside local tabs, or two aliases of one machine),
/// which would otherwise take each other over.
fn host_token(base: &str, host: &Host) -> String {
    use std::hash::{BuildHasher as _, BuildHasherDefault, DefaultHasher};

    match host {
        Host::Local => base.to_string(),
        Host::Remote(_) => {
            let hash = BuildHasherDefault::<DefaultHasher>::default().hash_one(host);
            format!("{base}-{hash:016x}")
        }
    }
}

/// A host's connection, behind a lock of its own.
type Slot = Arc<Mutex<Option<Arc<Connection>>>>;

/// One connection per host, each made under a lock of its own, so a local
/// connect that blocks holds up no remote endpoint's. A connection that
/// ends (a relay no terminal used any more, or one that never connected)
/// is forgotten.
#[derive(Default)]
struct Registry(Arc<Mutex<HashMap<Host, Slot>>>);

impl Registry {
    /// `host`'s connection, made by `make` when there is none or it ended.
    fn get(&self, host: &Host, make: impl FnOnce() -> Result<Arc<Connection>>) -> Result<Arc<Connection>> {
        let slot = self.0.lock().entry(host.clone()).or_default().clone();
        let mut current = slot.lock();
        if let Some(connection) = current.as_ref().filter(|connection| connection.reuse()) {
            return Ok(connection.clone());
        }
        let connection = make()?;
        let (map, host, made) = (Arc::downgrade(&self.0), host.clone(), connection.id());
        connection.on_end(Box::new(move || {
            if let Some(map) = map.upgrade() {
                forget(&map, &host, made);
            }
        }));
        *current = Some(connection.clone());
        Ok(connection)
    }

    /// `host`'s connection, if one is made and hasn't ended.
    fn existing(&self, host: &Host) -> Option<Arc<Connection>> {
        let slot = self.0.lock().get(host).cloned()?;
        let current = slot.lock();
        current.as_ref().filter(|connection| !connection.is_dead()).cloned()
    }

    /// Flushes every connection ([`Connection::flush`]) by one `deadline`.
    /// A slot's lock is held while its connection is made, which blocks
    /// (connecting, starting a core), so it is waited for only until then.
    fn flush(&self, deadline: Instant) -> bool {
        let slots: Vec<Slot> = self.0.lock().values().cloned().collect();
        let mut flushed = true;
        for slot in slots {
            let Some(current) = slot.try_lock_until(deadline) else {
                flushed = false;
                continue;
            };
            let connection = current.clone();
            drop(current);
            if let Some(connection) = connection {
                flushed &= connection.flush(deadline);
            }
        }
        flushed
    }

    #[cfg(test)]
    fn has(&self, host: &Host) -> bool {
        self.0.lock().contains_key(host)
    }
}

/// Takes the connection whose [`Connection::id`] is `ended` out of `host`'s
/// slot, and the slot out of `map`, unless a newer connection has replaced
/// it. Locks the map, then the slot, as [`Registry::get`] does (it never
/// holds both).
fn forget(map: &Mutex<HashMap<Host, Slot>>, host: &Host, ended: u64) {
    let Some(slot) = map.lock().get(host).cloned() else { return };
    {
        let mut current = slot.lock();
        if !current.as_ref().is_some_and(|connection| connection.id() == ended) {
            return;
        }
        *current = None;
    }
    let mut map = map.lock();
    if map.get(host).is_some_and(|kept| Arc::ptr_eq(kept, &slot) && kept.lock().is_none()) {
        map.remove(host);
    }
}

/// The client token of this process's `n`th listing connection
/// ([`running_sessions_at`]): never the process's own token, and never
/// another listing's, since the core closes a connection whose token a newer
/// one says (`take_over`). A launch's listing and a forwarded `--attach`'s
/// can run at once.
fn list_token(client: &str, n: u64) -> String {
    format!("{client}-list-{n}")
}

/// The sessions of this process's environment's core, if it is already
/// running ([`running_sessions_at`]): none when no core is listening (it
/// never starts one). It blocks, so its callers run it off the UI thread: the
/// launch's `launch-list` thread and the instance listener's (`process R13`).
pub fn running_sessions(limit: Duration) -> Result<Vec<client::SessionInfo>> {
    running_sessions_at(&Endpoint::from_env()?, limit)
}

/// The sessions of `endpoint`'s core that is already running, attached or
/// not, for a launch to attach: none when no core is listening (it never
/// starts one). It blocks: on Windows up to `limit` for a busy pipe, then up
/// to `HELLO_LIMIT` for the `HELLO`, then about `limit` per read.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "railway R3 exception: frames other than this request's `Sessions` or `Error` are skipped"
)]
pub fn running_sessions_at(endpoint: &Endpoint, limit: Duration) -> Result<Vec<client::SessionInfo>> {
    static LISTINGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let Some(stream) = try_connect_within(endpoint, limit)? else {
        return Ok(Vec::new());
    };
    // A token of its own: this connection owns nothing and gets no replays.
    let token = list_token(client_token(), LISTINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let Connected { mut stream, .. } = hello(stream, &token)?;
    stream.set_read_timeout(Some(limit))?;
    stream.set_write_timeout(Some(limit))?;
    let req = 1;
    ToCore::List(client::Request { req }).encode()?.write_to(&mut stream)?;
    loop {
        let frame = Frame::read_from(&mut stream).context("listing the terminal server's sessions")?;
        match ToClient::decode(&frame)? {
            ToClient::Sessions(sessions) if sessions.req == req => {
                return Ok(sessions.sessions);
            }
            ToClient::Error(error) if error.req == Some(req) => bail!("listing sessions: {}", error.message),
            _ => {}
        }
    }
}

/// Where the server is and what starts it.
#[derive(Clone, Debug)]
pub struct Endpoint {
    /// The `brindle-remote` binary that starts a core.
    pub helper: PathBuf,
    /// The server directory.
    pub dir: PathBuf,
    /// Where the per-build copies of the helper go, which roles run from.
    #[cfg(windows)]
    copies: PathBuf,
    /// Whether `copies` is an elevated process's own (`server-elevated`),
    /// made for Administrators and SYSTEM only and held while a core is
    /// started from it.
    #[cfg(windows)]
    protect_copies: bool,
    /// Whether a core started here may run in a systemd scope of its own
    /// (`scope::current()` decides). Only the environment's server may:
    /// tests' stand-in cores never become units.
    #[cfg(not(windows))]
    pub(crate) scope: bool,
    /// The variables set on every role this endpoint starts, beside this
    /// process's environment, which the role inherits. Empty for the
    /// environment's own server, whose roles find everything there.
    role_env: Vec<(&'static str, OsString)>,
}

impl Endpoint {
    /// The helper and the server directory for this process's environment.
    pub fn from_env() -> Result<Self> {
        let helper = super::find_server_helper()?.context(HELPER_MISSING)?;
        let named = std::env::var_os("BRINDLE_SERVER_DIR");
        #[cfg(not(windows))]
        let dir = server_dir::resolve(named, std::env::var_os("XDG_RUNTIME_DIR"), dirs::home_dir());
        #[cfg(windows)]
        let (dir, copies, protect_copies) = {
            use super::protocol::win;

            let local = std::env::var_os("LOCALAPPDATA").unwrap_or_default();
            let elevated = win::is_elevated().context("finding the terminal server directory")?;
            let given = named.as_ref().is_some_and(|dir| !dir.is_empty());
            let dir = server_dir::resolve_windows(named, Some(local.clone()), elevated);
            // In a directory that was named, the copies are its own, used as
            // they are; else `server`, or an elevated Brindle's protected
            // `server-elevated`.
            let copies = dir.as_ref().ok().map(|dir| {
                server_dir::helper_copies(given.then_some(dir.as_path()), std::path::Path::new(&local), elevated)
            });
            (dir, copies.unwrap_or_default(), elevated && !given)
        };
        let dir = dir.context("finding the terminal server directory")?;
        Ok(Self {
            helper,
            dir,
            #[cfg(windows)]
            copies,
            #[cfg(windows)]
            protect_copies,
            #[cfg(not(windows))]
            scope: true,
            role_env: Vec::new(),
        })
    }

    /// The server in `dir`, as if `BRINDLE_SERVER_DIR` had named it. A core
    /// it starts never runs in a scope, whatever the environment says, and
    /// is told so: it gets `BRINDLE_SERVER_DIR=<dir>` and
    /// `BRINDLE_NO_SCOPE=1`, so it serves `dir` and starts its holders
    /// without scopes too.
    #[cfg(test)]
    pub fn at(helper: PathBuf, dir: PathBuf) -> Self {
        Self {
            helper,
            #[cfg(windows)]
            copies: server_dir::helper_copies(Some(&dir), std::path::Path::new(""), false),
            #[cfg(windows)]
            protect_copies: false,
            role_env: vec![
                ("BRINDLE_SERVER_DIR", dir.clone().into_os_string()),
                ("BRINDLE_NO_SCOPE", "1".into()),
            ],
            dir,
            #[cfg(not(windows))]
            scope: false,
        }
    }

    /// Where the core listens, for messages: `core.sock` in the directory
    /// on Linux, the pipe named from it on Windows.
    pub fn address(&self) -> String {
        sys::address(&self.dir)
    }

    pub fn log(&self) -> PathBuf {
        self.dir.join(server_dir::CORE_LOG)
    }
}

/// A connection past `WELCOME`.
#[derive(Debug)]
pub struct Connected {
    pub stream: Stream,
    pub welcome: Welcome,
    /// The client protocols its `HELLO` offered, which a connection made on
    /// it offers again when it reconnects.
    pub offer: client::VersionRange,
}

/// Connects to the core, starting one if none is listening, and negotiates
/// Brindle's protocols ([`BRINDLE_PROTOCOLS`]).
pub fn connect(endpoint: &Endpoint, token: &str) -> Result<Connected> {
    connect_offering(endpoint, token, BRINDLE_PROTOCOLS)
}

/// [`connect`], offering the client protocols `offer`: the tests' way to
/// reach protocol 4 while Brindle's own offer stops below it.
pub fn connect_offering(endpoint: &Endpoint, token: &str, offer: client::VersionRange) -> Result<Connected> {
    let stream = match try_connect(endpoint)? {
        Some(stream) => stream,
        None => {
            start_core(endpoint)?;
            connect_started(endpoint)?
        }
    };
    hello_offering(stream, token, offer)
}

/// Connects to a listening core; `None` when none is listening.
pub fn try_connect(endpoint: &Endpoint) -> Result<Option<Stream>> {
    try_connect_within(endpoint, BUSY_LIMIT)
}

/// [`try_connect`], waiting at most `busy` for a core that takes no client
/// yet.
fn try_connect_within(endpoint: &Endpoint, busy: Duration) -> Result<Option<Stream>> {
    match sys::connect(&endpoint.dir, busy) {
        Ok(stream) => Ok(Some(stream)),
        Err(error) if no_listener(&error) => Ok(None),
        Err(error) => Err(unreachable(endpoint, &error)),
    }
}

fn no_listener(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused)
}

fn unreachable(endpoint: &Endpoint, error: &io::Error) -> anyhow::Error {
    anyhow!(
        "terminal server unreachable: can't connect to {}: {error}; its log is {}",
        endpoint.address(),
        endpoint.log().display()
    )
}

/// Connects to a core that just said it was ready. A core that lost the
/// start race says so once the winner accepts, so this only retries briefly.
fn connect_started(endpoint: &Endpoint) -> Result<Stream> {
    let deadline = Instant::now() + CONNECT_RETRY;
    loop {
        match sys::connect(&endpoint.dir, BUSY_LIMIT) {
            Ok(stream) => return Ok(stream),
            Err(error) if no_listener(&error) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(unreachable(endpoint, &error)),
        }
    }
}

/// How starting a core went, the same on every platform.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Start {
    /// The core wrote its ready byte. On Linux that is all it wrote before
    /// its ready pipe ended (`protocol::ready`).
    Ready,
    /// The wait ended by the core's doing without its ready byte: it exited
    /// (with its starter), or on Linux wrote something else into the pipe.
    Exited {
        /// How the starter exited.
        status: String,
        /// What the starter and the core said before the core had its log.
        stderr: String,
    },
    /// Neither its ready byte nor its end came within the time limit.
    TimedOut {
        /// What the starter and the core said meanwhile.
        stderr: String,
    },
}

/// Why a start that waited up to `limit` failed, as the tab and the log say
/// it: what the starter and the core wrote, else how the start ended. `None`
/// for a core that is ready.
fn start_failure(start: &Start, limit: Duration) -> Option<String> {
    let (said, ended) = match start {
        Start::Ready => return None,
        Start::Exited { status, stderr } => (stderr, format!("it exited ({status})")),
        Start::TimedOut { stderr } => (stderr, format!("no ready signal within {} s", limit.as_secs())),
    };
    Some(match said.trim() {
        "" => ended,
        text => text.to_string(),
    })
}

/// Starts a core from the helper and waits up to [`START_LIMIT`] for its
/// ready byte. The core detaches itself, so it is never Brindle's child and
/// outlives it (Linux: `serve --ready-fd 3`, a double fork and `setsid`;
/// Windows: `launch serve --ready-handle N` from the per-build copy).
pub fn start_core(endpoint: &Endpoint) -> Result<()> {
    let start = sys::start(endpoint, START_LIMIT)?;
    let Some(reason) = start_failure(&start, START_LIMIT) else {
        return Ok(());
    };
    bail!(
        "terminal server didn't start ({} serve): {reason}; its log is {}",
        endpoint.helper.display(),
        endpoint.log().display()
    )
}

/// The `HELLO` this client says. Through a relay it asks the core to keep
/// its `kill` sessions for [`RELAY_LINGER_SECS`] after the link drops, and
/// for `PROCESS` reports, since this machine can't read the endpoint's
/// `/proc`.
fn hello_message(token: &str, relay: bool, offer: client::VersionRange) -> Hello {
    Hello {
        min: offer.min,
        max: offer.max,
        client_version: format!("brindle {}", env!("CARGO_PKG_VERSION")),
        client_token: token.to_string(),
        linger_secs: if relay { RELAY_LINGER_SECS } else { 0 },
        process_info: relay,
    }
}

/// Says `HELLO`, offering Brindle's protocols ([`BRINDLE_PROTOCOLS`]), and
/// waits for `WELCOME`.
pub fn hello(stream: Stream, token: &str) -> Result<Connected> {
    hello_offering(stream, token, BRINDLE_PROTOCOLS)
}

/// Says `HELLO` offering the client protocols `offer` and waits for
/// `WELCOME`. An `ERROR` (no common version) names both sides' ranges.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "railway R3 exception: any answer to HELLO but `Welcome` or `Error` fails it, naming the answer"
)]
pub fn hello_offering(mut stream: Stream, token: &str, offer: client::VersionRange) -> Result<Connected> {
    let hello = ToCore::Hello(hello_message(token, false, offer));
    let talk = |error: io::Error| anyhow!("terminal server unreachable: {error}");
    stream.set_read_timeout(Some(HELLO_LIMIT)).map_err(talk)?;
    hello.encode().and_then(|frame| frame.write_to(&mut stream)).map_err(talk)?;
    let frame = match Frame::read_from_or_eof(&mut stream) {
        Ok(Some(frame)) => frame,
        Ok(None) => bail!("terminal server unreachable: it closed the connection before answering"),
        Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
            bail!("terminal server unreachable: no answer within {} s", HELLO_LIMIT.as_secs())
        }
        Err(error) => return Err(talk(error)),
    };
    stream.set_read_timeout(None).map_err(talk)?;
    match ToClient::decode(&frame).map_err(talk)? {
        ToClient::Welcome(welcome) => Ok(Connected { stream, welcome, offer }),
        ToClient::Error(error) => Err(refused(&error.message)),
        other => bail!("terminal server answered HELLO with {other:?}"),
    }
}

/// The core answered `HELLO` with an `ERROR`: it shares no protocol
/// version with this Brindle. Retrying helps only once that core has exited
/// (a relay then starts this build's), so only an eager relay connection
/// retries it.
#[derive(Debug)]
pub struct Refused(String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the terminal server refused Brindle ({}); install a brindle-remote that matches it, \
             or stop it with `brindle-remote stop` (its sessions end)",
            self.0
        )
    }
}

impl std::error::Error for Refused {}

fn refused(message: &str) -> anyhow::Error {
    Refused(message.to_string()).into()
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::sync::mpsc;

    use super::sys::Listener;
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("brindle-local-{tag}-{}", std::process::id()));
            std::fs::remove_dir_all(&path).ok();
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        /// An executable shell script standing in for the helper.
        #[cfg(unix)]
        fn script(&self, body: &str) -> PathBuf {
            let path = self.0.join("brindle-remote");
            crate::test_support::write_executable(&path, &format!("#!/bin/sh\n{body}\n"));
            path
        }

        /// The server directory, which exists (the pipe names of a Windows
        /// server hash its canonical path).
        fn server(&self) -> PathBuf {
            let dir = self.0.join("server");
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    /// Two listings at once (the launch's and a forwarded `--attach`'s)
    /// never say one token, which would make the core close the first, and
    /// neither says the process's own, which owns its tabs' sessions.
    #[test]
    fn each_listing_has_a_token_of_its_own() {
        let client = "brindle-42-00ff";
        let (first, second) = (list_token(client, 0), list_token(client, 1));
        assert_ne!(first, second);
        assert_ne!(first, client);
        assert_ne!(second, client);
        assert_eq!(first, "brindle-42-00ff-list-0");
    }

    /// A connected pair: the client's end, and the stand-in core's.
    fn pair(temp: &TempDir) -> (Stream, Stream) {
        let dir = temp.server();
        let listener = Listener::bind(&dir);
        let client = sys::connect(&dir, Duration::from_secs(5)).unwrap();
        (client, listener.accept())
    }

    #[test]
    fn a_missing_helper_says_how_to_point_at_one() {
        assert!(HELPER_MISSING.contains("terminal server helper not found"));
        assert!(HELPER_MISSING.contains("BRINDLE_SERVER_HELPER"));
        assert!(HELPER_MISSING.contains("beside brindle"));
        assert_eq!(HELPER_MISSING.contains("brindle-remote.exe"), cfg!(windows));
    }

    #[test]
    fn no_core_is_not_an_error() {
        let temp = TempDir::new("nobody");
        // Neither a directory without a core, nor no directory at all.
        for dir in [temp.server(), temp.0.join("missing")] {
            let endpoint = Endpoint::at(temp.0.join("unused"), dir);
            assert!(try_connect(&endpoint).unwrap().is_none());
        }
    }

    #[test]
    fn a_blocked_read_does_not_delay_a_write() {
        let temp = TempDir::new("duplex");
        let (client, mut core) = pair(&temp);
        let (tx, read) = mpsc::channel();
        let mut reader = client.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut byte = [0u8; 1];
            let got = reader.read(&mut byte).map(|read| (read, byte[0]));
            tx.send(got.map_err(|error| error.to_string())).ok();
        });
        // The reader is waiting on an empty pipe by now.
        assert!(read.recv_timeout(Duration::from_millis(200)).is_err());
        let started = Instant::now();
        let message = [7u8; 4096];
        (&client).write_all(&message).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        let mut got = [0u8; 4096];
        core.read_exact(&mut got).unwrap();
        assert_eq!(got, message);
        // And it is still reading: it gets what the core sends.
        assert!(read.try_recv().is_err());
        core.write_all(b"r").unwrap();
        assert_eq!(read.recv_timeout(Duration::from_secs(10)).unwrap(), Ok((1, b'r')));
    }

    #[test]
    fn a_shutdown_ends_a_blocked_read_and_later_writes() {
        let temp = TempDir::new("shutdown");
        let (client, _core) = pair(&temp);
        let (tx, read) = mpsc::channel();
        let mut reader = client.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut byte = [0u8; 1];
            tx.send(reader.read(&mut byte).map_err(|error| error.to_string())).ok();
        });
        assert!(read.recv_timeout(Duration::from_millis(200)).is_err());
        client.shutdown(std::net::Shutdown::Both).unwrap();
        // The end of the stream, as when the core closes it.
        assert_eq!(read.recv_timeout(Duration::from_secs(10)).unwrap(), Ok(0));
        assert!((&client).write_all(b"late").is_err());
    }

    #[test]
    fn a_read_times_out_and_the_core_closing_ends_the_stream() {
        let temp = TempDir::new("timeout");
        let (mut client, mut core) = pair(&temp);
        client.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let mut byte = [0u8; 1];
        let error = client.read(&mut byte).unwrap_err();
        assert!(matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut), "{error}");
        // What the core wrote before it went is still read, then the end.
        client.set_read_timeout(None).unwrap();
        core.write_all(b"z").unwrap();
        drop(core);
        assert_eq!(client.read(&mut byte).unwrap(), 1);
        assert_eq!(byte, *b"z");
        assert_eq!(client.read(&mut byte).unwrap(), 0);
    }

    // The stand-in helper is a shell script, which Windows can't start as
    // the copy of an executable. There a real core covers starting
    // (`a_real_core_starts_from_a_copy_and_answers`).
    #[cfg(unix)]
    #[test]
    fn a_core_that_never_listens_is_unreachable() {
        let temp = TempDir::new("unreachable");
        // Says it is ready, but nothing listens on the socket.
        let helper = temp.script("printf R >&3");
        let endpoint = Endpoint::at(helper, temp.0.join("server"));
        let error = connect(&endpoint, "t").unwrap_err().to_string();
        assert!(error.starts_with("terminal server unreachable"), "{error}");
        assert!(error.contains("core.sock") && error.contains("core.log"), "{error}");
    }

    // As above: a shell script stands in for the helper.
    #[cfg(unix)]
    #[test]
    fn a_core_that_fails_to_start_shows_why() {
        let temp = TempDir::new("no-start");
        let helper = temp.script("echo 'serve: the directory is refused' >&2; exit 1");
        let endpoint = Endpoint::at(helper, temp.0.join("server"));
        let error = connect(&endpoint, "t").unwrap_err().to_string();
        assert!(error.starts_with("terminal server didn't start"), "{error}");
        assert!(error.contains("the directory is refused"), "{error}");
    }

    // As above. The script writes what a failed exec leaves in the ready
    // pipe when that pipe took the place of `Command`'s own for the error:
    // the errno and "NOEX". Their first byte once read as "ready", and the
    // failure then showed as a server that couldn't be reached.
    #[cfg(unix)]
    #[test]
    fn a_failed_execs_error_in_the_ready_pipe_is_a_core_that_didnt_start() {
        let temp = TempDir::new("exec-error");
        let helper = temp.script(r"printf '\000\000\000\002NOEX' >&3; exit 1");
        let endpoint = Endpoint::at(helper, temp.0.join("server"));
        let error = connect(&endpoint, "t").unwrap_err().to_string();
        assert!(error.starts_with("terminal server didn't start"), "{error}");
    }

    // As above. A core started for a given endpoint serves that endpoint's
    // directory and starts its holders without scopes, whatever this
    // process's environment says.
    #[cfg(unix)]
    #[test]
    fn a_given_endpoint_tells_the_core_its_directory_and_no_scope() {
        let temp = TempDir::new("role-env");
        let said = temp.0.join("said");
        let helper = temp.script(&format!(
            r#"printf '%s %s' "$BRINDLE_SERVER_DIR" "$BRINDLE_NO_SCOPE" > '{}'; exit 1"#,
            said.display()
        ));
        let dir = temp.0.join("server");
        let endpoint = Endpoint::at(helper, dir.clone());
        assert!(connect(&endpoint, "t").is_err());
        assert_eq!(std::fs::read_to_string(&said).unwrap(), format!("{} 1", dir.display()));
    }

    /// A failed start says what the starter and the core wrote, else how it
    /// ended: they exited (Windows: or the trampoline was still running), or
    /// nothing came within the limit, in whole seconds.
    #[test]
    fn a_failed_start_says_what_was_written_else_how_it_ended() {
        let limit = Duration::from_millis(10_500);
        let exited = |status: &str, stderr: &str| Start::Exited { status: status.into(), stderr: stderr.into() };
        let timed_out = |stderr: &str| Start::TimedOut { stderr: stderr.into() };
        let cases = [
            (Start::Ready, None),
            (exited("exit status: 1", ""), Some("it exited (exit status: 1)")),
            (exited("exit code: 3", " \n"), Some("it exited (exit code: 3)")),
            (exited("still running", ""), Some("it exited (still running)")),
            (exited("exit status: 1", "serve: the directory is refused\n"), Some("serve: the directory is refused")),
            (timed_out(""), Some("no ready signal within 10 s")),
            (timed_out("\r\n"), Some("no ready signal within 10 s")),
            (timed_out("  launch: breakaway refused\n"), Some("launch: breakaway refused")),
        ];
        for (start, expected) in cases {
            assert_eq!(start_failure(&start, limit).as_deref(), expected, "{start:?}");
        }
    }

    /// An ended connection takes itself out of its slot by its id, and
    /// leaves a slot that holds another connection alone.
    #[test]
    fn an_ended_connection_is_forgotten_only_from_its_own_slot() {
        use crate::config::RemoteTarget;
        use connection::fake;

        let host = Host::Remote(RemoteTarget::Ssh {
            host: "A8".into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        });
        let registry = Registry::default();
        let (dial, _plan) = fake::dialer("A8");
        let connection = registry.get(&host, || Connection::relay(dial, "tok", fake::quick())).unwrap();
        forget(&registry.0, &host, connection.id() + 1);
        assert!(Arc::ptr_eq(&registry.existing(&host).unwrap(), &connection), "another's id");
        forget(&registry.0, &host, connection.id());
        assert!(!registry.has(&host));
    }

    #[cfg(windows)]
    #[test]
    fn a_helper_that_is_not_there_shows_why() {
        let temp = TempDir::new("no-helper");
        let endpoint = Endpoint::at(temp.0.join("brindle-remote.exe"), temp.server());
        let error = format!("{:#}", connect(&endpoint, "t").unwrap_err());
        assert!(error.starts_with("terminal server didn't start: can't copy"), "{error}");
        assert!(error.contains("brindle-remote.exe"), "{error}");
    }

    /// Starts a real core in a private server directory from the helper
    /// `BRINDLE_TEST_SERVER_HELPER` names (a built `brindle-remote`), talks
    /// to it, and stops it. The endpoint tells the core its directory and to
    /// use no scopes, so the test leaves this process's environment alone
    /// and runs beside the others. Ignored only because it needs that
    /// helper: `cargo test -p brindle a_real_core -- --ignored`.
    #[test]
    #[ignore = "needs BRINDLE_TEST_SERVER_HELPER, a built brindle-remote"]
    fn a_real_core_starts_from_a_copy_and_answers() {
        let helper = PathBuf::from(std::env::var_os("BRINDLE_TEST_SERVER_HELPER").expect("the helper"));
        let temp = TempDir::new("real");
        // Left for the core to make: a Linux core refuses a directory that
        // isn't private. On Windows the per-build copy makes it first.
        let dir = temp.0.join("server");
        let endpoint = Endpoint::at(helper.clone(), dir.clone());
        assert!(try_connect(&endpoint).unwrap().is_none(), "no core yet");

        let connected = connect(&endpoint, "real-1").unwrap();
        assert_eq!(connected.welcome.protocol, BRINDLE_PROTOCOLS.max);
        let core = connected.welcome.pid;
        assert_ne!(core, std::process::id());
        // The core runs from its per-build copy, not from the helper.
        #[cfg(windows)]
        {
            let image = sys::image_of(core).unwrap();
            assert!(image.starts_with(dir.join("bin")), "{}", image.display());
            assert_ne!(std::fs::canonicalize(&image).unwrap(), std::fs::canonicalize(&helper).unwrap());
        }
        // The same build is no upgrade: the connection is kept.
        let offered = upgrade::offer(&endpoint, "real-1", connected).unwrap();
        assert!(matches!(offered, upgrade::Offered::Current(_)), "the same build is current");
        let connected = offered.connected();
        assert_eq!(connected.welcome.pid, core);
        // A second client finds the running core and starts none.
        let second = connect(&endpoint, "real-2").unwrap();
        assert_eq!(second.welcome.pid, core);
        assert!(running_sessions_at(&endpoint, Duration::from_secs(5)).unwrap().is_empty());

        // A newer build of the helper (other bytes, a later file) replaces
        // the core, which then runs from that build's copy. Only Windows
        // tells builds apart by their bytes; an ELF helper has a build id.
        #[cfg(windows)]
        let (endpoint, connected, core) = {
            let old_image = sys::image_of(core).unwrap();
            let newer = temp.0.join("newer").join("brindle-remote.exe");
            std::fs::create_dir_all(newer.parent().unwrap()).unwrap();
            let mut bytes = std::fs::read(&helper).unwrap();
            bytes.extend_from_slice(b"another build");
            std::fs::write(&newer, bytes).unwrap();
            let endpoint = Endpoint::at(newer.clone(), dir.clone());
            let offered = upgrade::offer(&endpoint, "real-1", connected).unwrap();
            assert!(matches!(offered, upgrade::Offered::Upgraded(_)), "a newer build upgrades");
            let connected = offered.connected();
            let core = connected.welcome.pid;
            let image = sys::image_of(core).unwrap();
            assert!(image.starts_with(dir.join("bin")), "{}", image.display());
            assert_ne!(image, old_image);
            (endpoint, connected, core)
        };

        // The shared connection's two threads, over the real transport.
        let connection = Connection::start(endpoint.clone(), "real-1", connected).unwrap();
        assert!(connection.list().unwrap().is_empty());
        assert!(sys::image_of(core).is_ok(), "the core runs until it is stopped");

        let stop = super::super::hide_window(&mut std::process::Command::new(&helper))
            .arg("stop")
            .env("BRINDLE_SERVER_DIR", &dir)
            .output()
            .unwrap();
        assert!(stop.status.success(), "{stop:?}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while try_connect(&endpoint).unwrap().is_some() {
            assert!(Instant::now() < deadline, "the core still listens");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn no_common_version_names_both() {
        let temp = TempDir::new("versions");
        let dir = temp.server();
        let listener = Listener::bind(&dir);
        let core = std::thread::spawn(move || {
            let mut stream = listener.accept();
            let frame = Frame::read_from(&mut stream).unwrap();
            let ToCore::Hello(hello) = ToCore::decode(&frame).unwrap() else { panic!("not HELLO") };
            assert_eq!(hello.client_token, "tok");
            let refusal = client::answer_hello(&hello, client::VersionRange::new(5, 6)).unwrap_err();
            ToClient::Error(refusal).encode().unwrap().write_to(&mut stream).unwrap();
        });
        let endpoint = Endpoint::at(temp.0.join("unused"), dir);
        let error = connect(&endpoint, "tok").unwrap_err().to_string();
        core.join().unwrap();
        assert!(error.contains("refused Brindle"), "{error}");
        assert!(error.contains("offered 1–4, supported 5–6"), "{error}");
        assert!(error.contains("brindle-remote stop"), "{error}");
    }

    /// 5.1: tabs of profiles with the same target share one connection;
    /// another user or port is another endpoint, and an ended connection is
    /// replaced.
    #[test]
    fn one_connection_per_host() {
        use crate::config::{Profile, RemoteTarget};

        let target = |user: &str, port: u16| RemoteTarget::Ssh {
            host: "A8".into(),
            user: Some(user.into()),
            port: Some(port),
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        };
        let profile = |name: &str, remote: RemoteTarget| Profile { name: name.into(), remote: Some(remote), ..Default::default() };
        let registry = Registry::default();
        let made = std::sync::atomic::AtomicUsize::new(0);
        let plans = Mutex::new(Vec::new());
        let make = || {
            made.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let (dial, plan) = connection::fake::dialer("A8");
            plans.lock().push(plan);
            Connection::relay(dial, "tok", connection::fake::quick())
        };
        let shell = registry.get(&Host::of(&profile("A8", target("dev", 22))), make).unwrap();
        let htop = registry.get(&Host::of(&profile("htop on A8", target("dev", 22))), make).unwrap();
        assert!(Arc::ptr_eq(&shell, &htop));
        assert_eq!(made.load(std::sync::atomic::Ordering::Relaxed), 1);
        let root = registry.get(&Host::of(&profile("root", target("root", 22))), make).unwrap();
        let port = registry.get(&Host::of(&profile("port", target("dev", 2222))), make).unwrap();
        assert!(!Arc::ptr_eq(&shell, &root) && !Arc::ptr_eq(&shell, &port) && !Arc::ptr_eq(&root, &port));
        assert_eq!(made.load(std::sync::atomic::Ordering::Relaxed), 3);

        // The first endpoint can't be reached and nothing waits: it ends,
        // and the next tab gets a new connection.
        plans.lock()[0].fail("ssh: connect to host A8 port 22: Connection refused");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !shell.is_dead() {
            assert!(Instant::now() < deadline, "the connection never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
        let again = registry.get(&Host::Remote(target("dev", 22)), make).unwrap();
        assert!(!Arc::ptr_eq(&shell, &again));
        assert_eq!(made.load(std::sync::atomic::Ordering::Relaxed), 4);
    }

    /// 6.3: a relay connection ends `idle` after its last session went (its
    /// `CLOSE` written): its relay is killed and the registry forgets it, so
    /// the next tab gets a new one. A tab taking it within that time keeps
    /// it, and starts the time over.
    #[test]
    fn a_relay_closes_after_its_last_session_goes() {
        use crate::config::RemoteTarget;
        use connection::fake;

        let host = Host::Remote(RemoteTarget::Ssh {
            host: "A8".into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        });
        let idle = Duration::from_secs(1);
        let registry = Registry::default();
        let plans = Mutex::new(Vec::new());
        let make = || {
            let (dial, plan) = fake::dialer("A8");
            plans.lock().push(plan);
            let timings = relay::Timings { dead: Duration::from_secs(30), idle, ..fake::quick() };
            Connection::relay(dial, "tok", timings)
        };
        let request = || SpawnRequest {
            command: None,
            args: Vec::new(),
            cwd: None,
            env: Default::default(),
            cols: 80,
            rows: 24,
            cell_w: 8,
            cell_h: 16,
            scrollback: 1000,
            policy: client::Policy::Detach,
            profile: Some("A8".into()),
        };
        let eventually = |what: &str, done: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done() {
                assert!(Instant::now() < deadline, "never: {what}");
                std::thread::sleep(Duration::from_millis(5));
            }
        };

        let connection = registry.get(&host, make).unwrap();
        let (link, far) = fake::link(&[], fake::spawning);
        plans.lock()[0].link(link);
        let (sink, _tab) = fake::sink();
        let running = connection.spawn(request(), sink, None).unwrap();
        assert!(matches!(far.next(), ToCore::Spawn(_)));
        // A session keeps it.
        std::thread::sleep(idle * 2);
        assert!(!connection.is_dead());

        running.close();
        assert!(matches!(far.next(), ToCore::Close(_)));
        // Taken again within `idle`: the same connection, its time started
        // over.
        std::thread::sleep(idle / 5);
        let taken = Instant::now();
        let again = registry.get(&host, make).unwrap();
        assert!(Arc::ptr_eq(&connection, &again));
        assert_eq!(plans.lock().len(), 1);

        eventually("the unused connection ends", &|| connection.is_dead());
        assert!(taken.elapsed() >= idle * 4 / 5, "ended {:?} after it was taken", taken.elapsed());
        // The ending thread cuts the relay and forgets the connection after
        // marking it ended, so a test thread can look in between.
        eventually("its relay is killed", &|| far.out.is_cut());
        eventually("it is forgotten", &|| !registry.has(&host));
        let next = registry.get(&host, make).unwrap();
        assert!(!Arc::ptr_eq(&connection, &next));
        assert_eq!(plans.lock().len(), 2);
    }

    /// An exiting Brindle's flush waits for a connection still being made
    /// (a local connect blocks while it starts a core) only until its
    /// deadline, and has nothing to wait for once it has failed.
    #[test]
    fn a_flush_waits_for_a_connection_being_made_only_until_its_deadline() {
        let registry = Registry::default();
        let making = Registry(registry.0.clone());
        let (started_making, making_started) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let maker = std::thread::spawn(move || {
            making.get(&Host::Local, || {
                started_making.send(()).unwrap();
                released.recv().ok();
                Err(anyhow!("no core"))
            })
        });
        making_started.recv_timeout(Duration::from_secs(10)).unwrap();
        let started = Instant::now();
        assert!(!registry.flush(started + Duration::from_millis(200)));
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(200) && waited < Duration::from_secs(5), "waited {waited:?}");
        release.send(()).unwrap();
        assert!(maker.join().unwrap().is_err());
        assert!(registry.flush(Instant::now() + Duration::from_secs(1)), "no connection is left to wait for");
    }

    /// The palette's lookup never makes a connection, and doesn't hand out
    /// one that has ended.
    #[test]
    fn looking_a_connection_up_never_makes_one() {
        use crate::config::RemoteTarget;
        use connection::fake;

        let host = Host::Remote(RemoteTarget::Ssh {
            host: "A8".into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        });
        let registry = Registry::default();
        assert!(registry.existing(&host).is_none());
        assert!(!registry.has(&host), "a lookup makes no slot");
        let (dial, plan) = fake::dialer("A8");
        let connection = registry.get(&host, || Connection::relay(dial, "tok", fake::quick())).unwrap();
        assert!(Arc::ptr_eq(&registry.existing(&host).unwrap(), &connection));
        // A lazy connection whose first attempt fails ends.
        plan.fail("ssh: Connection refused");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !connection.is_dead() {
            assert!(Instant::now() < deadline, "the connection never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(registry.existing(&host).is_none(), "an ended connection isn't handed out");
    }

    /// A reload's change to the endpoints kept connected: the ones to
    /// connect and the ones to let go, in order.
    #[test]
    fn the_eager_change_names_what_to_connect_and_let_go() {
        use crate::config::RemoteTarget;

        let ssh = |host: &str| {
            Host::Remote(RemoteTarget::Ssh {
                host: host.into(),
                user: None,
                port: None,
                identity_file: None,
                ssh_args: Vec::new(),
                connect_at_launch: false,
            })
        };
        let (a8, b9, c7) = (ssh("A8"), ssh("B9"), ssh("C7"));
        assert_eq!(eager_change(&[], &[a8.clone(), b9.clone()]), (vec![a8.clone(), b9.clone()], vec![]));
        assert_eq!(eager_change(&[a8.clone(), b9.clone()], &[c7.clone(), a8.clone()]), (vec![c7.clone()], vec![b9.clone()]));
        assert_eq!(eager_change(std::slice::from_ref(&a8), std::slice::from_ref(&a8)), (vec![], vec![]));
        assert_eq!(eager_change(&[a8.clone(), c7.clone()], &[]), (vec![], vec![a8, c7]));
    }

    #[test]
    fn each_remote_endpoint_has_a_token_of_its_own() {
        use crate::config::RemoteTarget;

        let ssh = |host: &str| {
            Host::Remote(RemoteTarget::Ssh {
                host: host.into(),
                user: None,
                port: None,
                identity_file: None,
                ssh_args: Vec::new(),
                connect_at_launch: false,
            })
        };
        assert_eq!(host_token("t", &Host::Local), "t");
        let (a8, localhost) = (host_token("t", &ssh("A8")), host_token("t", &ssh("localhost")));
        assert_eq!(host_token("t", &ssh("A8")), a8, "stable for reconnects");
        assert_ne!(a8, localhost);
        assert!(a8.starts_with("t-") && a8.len() == "t-".len() + 16, "{a8}");
    }

    #[test]
    fn only_a_relay_asks_for_linger_and_process_info() {
        let local = hello_message("tok", false, BRINDLE_PROTOCOLS);
        assert_eq!((local.linger_secs, local.process_info), (0, false));
        let relay = hello_message("tok", true, BRINDLE_PROTOCOLS);
        assert_eq!((relay.linger_secs, relay.process_info), (300, true));
        assert_eq!((relay.min, relay.max), (BRINDLE_PROTOCOLS.min, BRINDLE_PROTOCOLS.max));
        assert_eq!(relay.client_token, "tok");
    }

    /// Brindle offers every protocol the codec knows, agent sessions' 4
    /// included (`agents-profiles` task 6.5), down to 1 for the oldest
    /// cores; a connection made with another offer says that one.
    #[test]
    fn brindle_offers_every_protocol_up_to_agent_sessions() {
        assert_eq!((BRINDLE_PROTOCOLS.min, BRINDLE_PROTOCOLS.max), (1, 4));
        assert_eq!((BRINDLE_PROTOCOLS.min, BRINDLE_PROTOCOLS.max), (client::PROTOCOLS.min, client::PROTOCOLS.max));
        let hello = hello_message("tok", false, BRINDLE_PROTOCOLS);
        assert_eq!((hello.min, hello.max), (1, 4));
        let older = hello_message("tok", true, client::VersionRange::new(1, 3));
        assert_eq!((older.min, older.max), (1, 3));
    }

    #[test]
    fn the_token_is_random_and_stable() {
        let token = client_token();
        assert_eq!(client_token(), token);
        assert!(token.len() > 32, "{token}");
    }
}
