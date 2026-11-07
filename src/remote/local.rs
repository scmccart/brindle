//! Brindle's client for the local terminal server (Linux).
//!
//! Local tabs run in sessions of the terminal server, `brindle-remote serve`
//! (the core) and the holders it drives. This module finds the helper (as
//! [`super::find_helper`] does for remote tabs), resolves the server
//! directory as the helper does ([`server_dir::resolve`]), starts the core
//! when none is listening (`serve --ready-fd 3`, which detaches itself and
//! writes one byte once it accepts connections), connects to `core.sock`
//! and negotiates protocol 1 with this process's random `client_token`.
//!
//! Every failure is an error naming its reason and the fix, shown in the
//! tab that needed the server. Nothing is cached, so the next tab tries
//! again.
//!
//! The process keeps one [`Connection`] for all its tabs ([`shared`]). A
//! tab's [`Session`] gets it, and its session, on a thread of its own.

mod connection;
mod session;
mod upgrade;

pub use connection::{Connection, SessionSink, SpawnRequest};
pub use session::Session;

use std::io::{self, Read as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use parking_lot::Mutex;

use super::protocol::client::{self, Hello, ToClient, ToCore, Welcome};
use super::protocol::{Frame, server_dir};

/// How long the starter waits for a new core's ready byte (design.md).
const START_LIMIT: Duration = Duration::from_secs(2);
/// How long a core may take to answer `HELLO`. A core that just started
/// accepts clients only once it has adopted the running holders (at most
/// 5 s), so this is longer than that.
const HELLO_LIMIT: Duration = Duration::from_secs(10);
/// How long connecting keeps retrying after a core said it was ready.
const CONNECT_RETRY: Duration = Duration::from_secs(1);

pub const HELPER_MISSING: &str =
    "terminal server helper not found: set BRINDLE_REMOTE_AGENT or install brindle-remote beside brindle";

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

/// Where the server is and what starts it.
#[derive(Clone, Debug)]
pub struct Endpoint {
    /// The `brindle-remote` binary that starts a core.
    pub helper: PathBuf,
    /// The server directory.
    pub dir: PathBuf,
}

impl Endpoint {
    /// The helper and the server directory for this process's environment.
    pub fn from_env() -> Result<Self> {
        let helper = super::find_helper()?.context(HELPER_MISSING)?;
        let dir = server_dir::resolve(
            std::env::var_os("BRINDLE_SERVER_DIR"),
            std::env::var_os("XDG_RUNTIME_DIR"),
            dirs::home_dir(),
        )
        .context("finding the terminal server directory")?;
        Ok(Self { helper, dir })
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.join(server_dir::CORE_SOCKET)
    }

    pub fn log(&self) -> PathBuf {
        self.dir.join(server_dir::CORE_LOG)
    }
}

/// A connection past `WELCOME`.
#[derive(Debug)]
pub struct Connected {
    pub stream: UnixStream,
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
pub fn try_connect(endpoint: &Endpoint) -> Result<Option<UnixStream>> {
    let socket = endpoint.socket();
    match UnixStream::connect(&socket) {
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
        endpoint.socket().display(),
        endpoint.log().display()
    )
}

/// Connects to a core that just said it was ready. A core that lost the
/// start race says so once the winner accepts, so this only retries briefly.
fn connect_started(endpoint: &Endpoint) -> Result<UnixStream> {
    let deadline = Instant::now() + CONNECT_RETRY;
    loop {
        match UnixStream::connect(endpoint.socket()) {
            Ok(stream) => return Ok(stream),
            Err(error) if no_listener(&error) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(unreachable(endpoint, &error)),
        }
    }
}

/// Runs `<helper> serve --ready-fd 3` and waits up to [`START_LIMIT`] for
/// its ready byte. The core detaches itself (double fork, `setsid`), so it
/// is never Brindle's child and outlives it.
pub fn start_core(endpoint: &Endpoint) -> Result<()> {
    let mut fds = [-1; 2];
    // SAFETY: `pipe2` fills both fds on success; they are owned below.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error()).context("starting the terminal server: pipe");
    }
    // SAFETY: freshly created and not owned elsewhere.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let write_fd = write.as_raw_fd();
    let mut command = Command::new(&endpoint.helper);
    command
        .args(["serve", "--ready-fd", "3"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(write_fd, 3) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().with_context(|| {
        format!("terminal server didn't start: can't run {}", endpoint.helper.display())
    })?;
    drop(write);
    let ready = wait_readable(&read, START_LIMIT);
    let mut byte = [0u8; 1];
    let got = ready && std::fs::File::from(read).read(&mut byte).ok() == Some(1);
    // The starter exits once the core has detached (or failed first).
    let output = child.wait_with_output().context("waiting for the terminal server to start")?;
    if got {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = match stderr.trim() {
        "" if !ready => format!("no ready signal within {} s", START_LIMIT.as_secs()),
        "" => format!("it exited ({})", output.status),
        text => text.to_string(),
    };
    bail!(
        "terminal server didn't start ({} serve): {reason}; its log is {}",
        endpoint.helper.display(),
        endpoint.log().display()
    )
}

/// Whether `fd` has data (or EOF) within `limit`.
fn wait_readable(fd: &OwnedFd, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        let mut pollfd = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let wait = deadline.saturating_duration_since(Instant::now()).as_millis() as libc::c_int;
        // SAFETY: one valid pollfd.
        match unsafe { libc::poll(&mut pollfd, 1, wait) } {
            1 => return true,
            0 => return false,
            _ if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => {}
            _ => return false,
        }
    }
}

/// Says `HELLO` and waits for `WELCOME`. An `ERROR` (no common version)
/// names both sides' ranges.
pub fn hello(mut stream: UnixStream, token: &str) -> Result<Connected> {
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
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::net::UnixListener;

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
        fn script(&self, body: &str) -> PathBuf {
            let path = self.0.join("brindle-remote");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn a_missing_helper_says_how_to_point_at_one() {
        assert!(HELPER_MISSING.contains("terminal server helper not found"));
        assert!(HELPER_MISSING.contains("BRINDLE_REMOTE_AGENT"));
        assert!(HELPER_MISSING.contains("beside brindle"));
    }

    #[test]
    fn a_core_that_never_listens_is_unreachable() {
        let temp = TempDir::new("unreachable");
        // Says it is ready, but nothing listens on the socket.
        let helper = temp.script("printf R >&3");
        let endpoint = Endpoint { helper, dir: temp.0.join("server") };
        let error = connect(&endpoint, "t").unwrap_err().to_string();
        assert!(error.starts_with("terminal server unreachable"), "{error}");
        assert!(error.contains("core.sock") && error.contains("core.log"), "{error}");
    }

    #[test]
    fn a_core_that_fails_to_start_shows_why() {
        let temp = TempDir::new("no-start");
        let helper = temp.script("echo 'serve: the directory is refused' >&2; exit 1");
        let endpoint = Endpoint { helper, dir: temp.0.join("server") };
        let error = connect(&endpoint, "t").unwrap_err().to_string();
        assert!(error.starts_with("terminal server didn't start"), "{error}");
        assert!(error.contains("the directory is refused"), "{error}");
    }

    #[test]
    fn no_common_version_names_both() {
        let temp = TempDir::new("versions");
        let dir = temp.0.join("server");
        std::fs::create_dir_all(&dir).unwrap();
        let listener = UnixListener::bind(dir.join("core.sock")).unwrap();
        let core = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let frame = Frame::read_from(&mut stream).unwrap();
            let ToCore::Hello(hello) = ToCore::decode(&frame).unwrap() else { panic!("not HELLO") };
            assert_eq!(hello.client_token, "tok");
            let refusal = client::answer_hello(&hello, client::VersionRange::new(2, 3)).unwrap_err();
            ToClient::Error(refusal).encode().unwrap().write_to(&mut stream).unwrap();
        });
        let endpoint = Endpoint { helper: temp.0.join("unused"), dir };
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
