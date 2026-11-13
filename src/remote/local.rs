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
//! The process keeps one [`Connection`] for all its tabs ([`shared`]). A
//! tab's [`Session`] gets it, and its session, on a thread of its own.

mod connection;
#[cfg(windows)]
mod pipe;
mod session;
#[cfg(unix)]
#[path = "local/unix.rs"]
mod sys;
#[cfg(windows)]
#[path = "local/windows.rs"]
mod sys;
mod upgrade;

pub use connection::{AttachRequest, Connection, SessionSink, SpawnRequest};
pub use session::Session;

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use parking_lot::Mutex;

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

/// The process's connection to the core, made (starting a core if needed,
/// and offering it an upgrade) when there is none or the last one ended.
/// It blocks, so it runs on a session's start thread ([`Session::start`]),
/// never on the UI thread.
pub fn shared() -> Result<Arc<Connection>> {
    static CURRENT: Mutex<Option<Arc<Connection>>> = Mutex::new(None);
    let mut current = CURRENT.lock();
    if let Some(connection) = current.as_ref().filter(|connection| !connection.is_dead()) {
        return Ok(connection.clone());
    }
    let endpoint = Endpoint::from_env()?;
    let connected = connect(&endpoint, client_token())?;
    let connected = upgrade::offer(&endpoint, client_token(), connected)?;
    let connection = Connection::start(endpoint, client_token(), connected)?;
    *current = Some(connection.clone());
    Ok(connection)
}

/// The sessions of a core that is already running, attached or not, for a
/// launch to attach: none when no core is listening (it never starts one).
/// It blocks, for about `limit` at most past the `HELLO`; it runs once,
/// before any window opens.
// Unused on Windows until sessions are listed and attached there (task 8.1
// of `terminal-server-windows`).
#[cfg_attr(windows, allow(dead_code))]
pub fn running_sessions(limit: Duration) -> Result<Vec<client::SessionInfo>> {
    let endpoint = Endpoint::from_env()?;
    let Some(stream) = try_connect_within(&endpoint, limit)? else {
        return Ok(Vec::new());
    };
    // A token of its own: this connection owns nothing and gets no replays.
    let token = format!("{}-list", client_token());
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
}

impl Endpoint {
    /// The helper and the server directory for this process's environment.
    pub fn from_env() -> Result<Self> {
        let helper = super::find_server_helper()?.context(HELPER_MISSING)?;
        let named = std::env::var_os("BRINDLE_SERVER_DIR");
        #[cfg(not(windows))]
        let dir = server_dir::resolve(named, std::env::var_os("XDG_RUNTIME_DIR"), dirs::home_dir());
        #[cfg(windows)]
        let (dir, copies) = {
            use super::protocol::win;

            let local = std::env::var_os("LOCALAPPDATA").unwrap_or_default();
            let elevated = win::is_elevated().context("finding the terminal server directory")?;
            let given = named.as_ref().is_some_and(|dir| !dir.is_empty());
            let dir = server_dir::resolve_windows(named, Some(local.clone()), elevated);
            // In a directory that was named, the copies are its own.
            let copies = dir.as_ref().ok().map(|dir| {
                server_dir::helper_copies(given.then_some(dir.as_path()), std::path::Path::new(&local))
            });
            (dir, copies.unwrap_or_default())
        };
        let dir = dir.context("finding the terminal server directory")?;
        Ok(Self {
            helper,
            dir,
            #[cfg(windows)]
            copies,
        })
    }

    /// The server in `dir`, as if `BRINDLE_SERVER_DIR` had named it.
    #[cfg(test)]
    pub fn at(helper: PathBuf, dir: PathBuf) -> Self {
        Self {
            helper,
            #[cfg(windows)]
            copies: server_dir::helper_copies(Some(&dir), std::path::Path::new("")),
            dir,
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
}

/// Connects to the core, starting one if none is listening, and negotiates.
pub fn connect(endpoint: &Endpoint, token: &str) -> Result<Connected> {
    let stream = match try_connect(endpoint)? {
        Some(stream) => stream,
        None => {
            start_core(endpoint)?;
            connect_started(endpoint)?
        }
    };
    hello(stream, token)
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

/// How starting a core went.
struct Start {
    /// The core wrote its ready byte.
    ready: bool,
    /// The wait ended by the core's doing (its byte, or its exit) and not
    /// by the time limit.
    signalled: bool,
    /// How the starter exited.
    status: String,
    /// What the starter and the core said before the core had its log.
    stderr: String,
}

/// Starts a core from the helper and waits up to [`START_LIMIT`] for its
/// ready byte. The core detaches itself, so it is never Brindle's child and
/// outlives it (Linux: `serve --ready-fd 3`, a double fork and `setsid`;
/// Windows: `launch serve --ready-handle N` from the per-build copy).
pub fn start_core(endpoint: &Endpoint) -> Result<()> {
    let start = sys::start(endpoint, START_LIMIT)?;
    if start.ready {
        return Ok(());
    }
    let reason = match start.stderr.trim() {
        "" if !start.signalled => format!("no ready signal within {} s", START_LIMIT.as_secs()),
        "" => format!("it exited ({})", start.status),
        text => text.to_string(),
    };
    bail!(
        "terminal server didn't start ({} serve): {reason}; its log is {}",
        endpoint.helper.display(),
        endpoint.log().display()
    )
}

/// Says `HELLO` and waits for `WELCOME`. An `ERROR` (no common version)
/// names both sides' ranges.
pub fn hello(mut stream: Stream, token: &str) -> Result<Connected> {
    let hello = ToCore::Hello(Hello {
        min: client::PROTOCOLS.min,
        max: client::PROTOCOLS.max,
        client_version: format!("brindle {}", env!("CARGO_PKG_VERSION")),
        client_token: token.to_string(),
    });
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
        ToClient::Welcome(welcome) => Ok(Connected { stream, welcome }),
        ToClient::Error(error) => Err(refused(&error.message)),
        other => bail!("terminal server answered HELLO with {other:?}"),
    }
}

/// The core answered `HELLO` with an `ERROR`: it shares no protocol
/// version with this Brindle. Retrying won't help.
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
            use std::os::unix::fs::PermissionsExt as _;

            let path = self.0.join("brindle-remote");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
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
    /// to it, and stops it. Ignored: it needs that helper, and it sets this
    /// process's environment, so it runs alone:
    /// `cargo test -p brindle a_real_core -- --ignored`.
    #[test]
    #[ignore = "needs BRINDLE_TEST_SERVER_HELPER, a built brindle-remote"]
    fn a_real_core_starts_from_a_copy_and_answers() {
        let helper = PathBuf::from(std::env::var_os("BRINDLE_TEST_SERVER_HELPER").expect("the helper"));
        let temp = TempDir::new("real");
        let dir = temp.server();
        // SAFETY: no other test runs beside this one (see above).
        unsafe {
            std::env::set_var("BRINDLE_SERVER_HELPER", &helper);
            std::env::set_var("BRINDLE_SERVER_DIR", &dir);
            std::env::set_var("BRINDLE_NO_SCOPE", "1");
        }
        let endpoint = Endpoint::from_env().unwrap();
        assert_eq!(endpoint.dir, dir);
        assert!(try_connect(&endpoint).unwrap().is_none(), "no core yet");

        let connected = connect(&endpoint, "real-1").unwrap();
        assert_eq!(connected.welcome.protocol, 1);
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
        let connected = upgrade::offer(&endpoint, "real-1", connected).unwrap();
        assert_eq!(connected.welcome.pid, core);
        // A second client finds the running core and starts none.
        let second = connect(&endpoint, "real-2").unwrap();
        assert_eq!(second.welcome.pid, core);
        assert!(running_sessions(Duration::from_secs(5)).unwrap().is_empty());

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
            // SAFETY: as above.
            unsafe { std::env::set_var("BRINDLE_SERVER_HELPER", &newer) };
            let endpoint = Endpoint::from_env().unwrap();
            let connected = upgrade::offer(&endpoint, "real-1", connected).unwrap();
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
            let refusal = client::answer_hello(&hello, client::VersionRange::new(2, 3)).unwrap_err();
            ToClient::Error(refusal).encode().unwrap().write_to(&mut stream).unwrap();
        });
        let endpoint = Endpoint::at(temp.0.join("unused"), dir);
        let error = connect(&endpoint, "tok").unwrap_err().to_string();
        core.join().unwrap();
        assert!(error.contains("refused Brindle"), "{error}");
        assert!(error.contains("offered 1, supported 2–3"), "{error}");
        assert!(error.contains("brindle-remote stop"), "{error}");
    }

    #[test]
    fn the_token_is_random_and_stable() {
        let token = client_token();
        assert_eq!(client_token(), token);
        assert!(token.len() > 32, "{token}");
    }
}
