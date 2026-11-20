//! One Brindle per state directory. The process holding the session lock is
//! the instance: it listens on a socket (a named pipe on Windows) named
//! after the state directory. A later launch that can't take the lock sends
//! its request there as one JSON line and exits once the instance answers
//! `ok`; the instance raises its window and opens what was asked for.

use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// How long a launch waits for the instance to take its request.
pub const FORWARD_LIMIT: Duration = Duration::from_secs(5);

/// The longest request line read; anything longer is refused.
const MAX_REQUEST: u64 = 1 << 20;

/// What a launch asks the instance for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Request {
    /// `-p`: a profile name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// `-d`, made absolute by the launch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// `-e`: the program and its arguments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// `--attach`: a terminal server session id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attach: Option<u32>,
    /// The launching process's working directory, where `-e` starts
    /// without `-d`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_dir: Option<PathBuf>,
    /// The launcher's activation token (`XDG_ACTIVATION_TOKEN`, else
    /// `DESKTOP_STARTUP_ID`), which lets the instance take focus on Wayland.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_token: Option<String>,
}

impl Request {
    pub fn encode(&self) -> String {
        let mut line = serde_json::to_string(self).expect("a request always serializes");
        line.push('\n');
        line
    }

    pub fn decode(line: &str) -> Result<Self, String> {
        serde_json::from_str(line.trim_end()).map_err(|e| format!("bad request: {e}"))
    }
}

/// The activation token a launch was started with, from the values of
/// `XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID`: the first that is set
/// and not empty.
pub fn launch_token(xdg: Option<String>, startup_id: Option<String>) -> Option<String> {
    [xdg, startup_id].into_iter().flatten().find(|token| !token.is_empty())
}

/// FNV-1a, so every Brindle version names the same directory's socket the
/// same way (std's hasher may change between Rust releases).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

/// The instance's name for `state_dir`.
pub fn name(state_dir: &Path) -> String {
    format!("brindle-instance-{:016x}", fnv1a(state_dir.as_os_str().as_encoded_bytes()))
}

/// The instance's socket on Unix: `<name>.sock` in `runtime_dir`
/// (`$XDG_RUNTIME_DIR`, unset when empty), else in `brindle/run` under
/// `cache_dir`, else under `temp_dir`.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn socket_path_in(
    runtime_dir: Option<OsString>,
    cache_dir: Option<PathBuf>,
    temp_dir: PathBuf,
    state_dir: &Path,
) -> PathBuf {
    let dir = runtime_dir
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| cache_dir.unwrap_or(temp_dir).join("brindle/run"));
    dir.join(format!("{}.sock", name(state_dir)))
}

/// The instance's pipe on Windows: `\\.\pipe\<name>-<user>`, where each
/// character of `user` (`%USERNAME%`) that isn't an ASCII letter or digit
/// becomes `_`, and a missing user leaves that part empty.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn pipe_name_for(user: Option<String>, state_dir: &Path) -> String {
    let user: String =
        user.unwrap_or_default().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!(r"\\.\pipe\{}-{user}", name(state_dir))
}

/// Writes `reply` (`ok`, or `error: …`) as the answer to one request.
fn answer(mut stream: impl Write, reply: &Result<(), String>) -> io::Result<()> {
    let line = match reply {
        Ok(()) => "ok\n".to_string(),
        Err(why) => format!("error: {}\n", why.replace('\n', " ")),
    };
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

/// Reads one request line from `stream`.
fn read_request(stream: impl Read) -> Result<Request, String> {
    let mut line = String::new();
    BufReader::new(stream.take(MAX_REQUEST))
        .read_line(&mut line)
        .map_err(|e| format!("reading the request: {e}"))?;
    Request::decode(&line)
}

/// Sends `request` over `stream` and reads the answer.
fn exchange(mut stream: impl Read + Write, request: &Request) -> Result<(), String> {
    stream.write_all(request.encode().as_bytes()).map_err(|e| format!("sending the request: {e}"))?;
    stream.flush().map_err(|e| format!("sending the request: {e}"))?;
    let mut line = String::new();
    BufReader::new(stream.take(MAX_REQUEST)).read_line(&mut line).map_err(|e| format!("waiting for an answer: {e}"))?;
    parse_answer(&line)
}

fn parse_answer(line: &str) -> Result<(), String> {
    match line.trim_end() {
        "ok" => Ok(()),
        "" => Err("the running Brindle closed the connection without answering".into()),
        other => Err(other.strip_prefix("error: ").unwrap_or(other).to_string()),
    }
}

/// What a launch says when it won't hand its request to whatever serves
/// `address` (the instance's socket or pipe), and `why`: nothing was sent.
/// A request names the launch's command, its directory and its activation
/// token, which are only for this user's own Brindle.
fn not_sent(address: &str, why: impl std::fmt::Display) -> String {
    format!("not sending the launch to {address}: {why}")
}

/// Whether the instance a launch reached on a Unix socket is this user's:
/// `own` is the launch's effective uid, and `peer` the uid serving the
/// socket (`SO_PEERCRED`), or why it can't be told, which refuses it too.
/// The `Err` says why not.
#[cfg_attr(not(unix), allow(dead_code))]
fn same_user(own: u32, peer: io::Result<u32>) -> Result<(), String> {
    match peer {
        Ok(peer) if peer == own => Ok(()),
        Ok(peer) => Err(format!("it is served by another user (uid {peer}, not {own})")),
        Err(error) => Err(format!("the user serving it can't be told: {error}")),
    }
}

/// Handles one request on the listener's thread, returning once the
/// instance has taken it (or refused it).
pub type Handler = Box<dyn Fn(Request) -> Result<(), String> + Send + 'static>;

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::{UnixListener, UnixStream};

    /// `$XDG_RUNTIME_DIR/<name>.sock`, else under `~/.cache/brindle/run`
    /// ([`socket_path_in`]).
    pub fn socket_path(state_dir: &Path) -> PathBuf {
        socket_path_in(std::env::var_os("XDG_RUNTIME_DIR"), dirs::cache_dir(), std::env::temp_dir(), state_dir)
    }

    pub fn listen(state_dir: &Path, handler: Handler) -> io::Result<PathBuf> {
        listen_at(socket_path(state_dir), handler)
    }

    /// [`listen`] on the socket at `path`.
    pub(super) fn listen_at(path: PathBuf, handler: Handler) -> io::Result<PathBuf> {
        if let Some(dir) = path.parent() {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        // The caller holds the session lock, so a socket left here belongs to
        // an instance that is gone.
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let uid = unsafe { libc::getuid() };
        std::thread::Builder::new().name("instance".into()).spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => serve(stream, uid, &handler),
                    Err(e) => log::warn!("instance socket: {e}"),
                }
            }
        })?;
        Ok(path)
    }

    fn serve(stream: UnixStream, uid: u32, handler: &Handler) {
        match peer_uid(&stream) {
            Ok(peer) if peer == uid => {}
            Ok(peer) => {
                log::warn!("refused an instance request from uid {peer}");
                return;
            }
            Err(e) => {
                log::warn!("refused an instance request whose user can't be told: {e}");
                return;
            }
        }
        stream.set_read_timeout(Some(FORWARD_LIMIT)).ok();
        stream.set_write_timeout(Some(FORWARD_LIMIT)).ok();
        let reply = read_request(&stream).and_then(|request| {
            log::info!("a launch asked for {request:?}");
            handler(request)
        });
        if let Err(e) = answer(&stream, &reply) {
            log::warn!("answering a launch: {e}");
        }
    }

    /// The uid at the other end of `stream` (`SO_PEERCRED`): for a client,
    /// the user whose process listens on the socket.
    pub(super) fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
        #[cfg(target_os = "linux")]
        {
            let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
            let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
            // SAFETY: `cred` and `len` are valid for the call.
            let result = unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut cred as *mut libc::ucred).cast(),
                    &mut len,
                )
            };
            if result == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(cred.uid)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let (mut uid, mut gid) = (0, 0);
            // SAFETY: plain out parameters.
            if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(uid)
        }
    }

    pub fn forward(state_dir: &Path, request: &Request) -> Result<(), String> {
        forward_to(&socket_path(state_dir), request)
    }

    /// [`forward`] to the socket at `path`. Before anything is sent, the
    /// socket must be served by this user ([`same_user`]): the instance
    /// checks who connects, and the launch checks who it connected to.
    pub(super) fn forward_to(path: &Path, request: &Request) -> Result<(), String> {
        let deadline = Instant::now() + FORWARD_LIMIT;
        let stream = loop {
            match UnixStream::connect(path) {
                Ok(stream) => break stream,
                // The instance may still be starting: it takes the lock before
                // it binds.
                Err(e) if Instant::now() < deadline => {
                    log::debug!("connecting to {}: {e}", path.display());
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(format!("could not reach the running Brindle at {}: {e}", path.display())),
            }
        };
        // SAFETY: takes no argument and can't fail.
        let own = unsafe { libc::geteuid() };
        same_user(own, peer_uid(&stream)).map_err(|why| not_sent(&path.display().to_string(), why))?;
        let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_secs(1));
        stream.set_read_timeout(Some(left)).ok();
        stream.set_write_timeout(Some(left)).ok();
        exchange(&stream, request)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::fs::File;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
    use windows::core::HSTRING;

    /// `\\.\pipe\<name>-<user>` ([`pipe_name_for`]). The pipe's default
    /// security lets only its owner (and administrators) write to it.
    pub fn pipe_name(state_dir: &Path) -> String {
        pipe_name_for(std::env::var("USERNAME").ok(), state_dir)
    }

    fn create(name: &HSTRING, first: bool) -> io::Result<File> {
        let mut mode = PIPE_ACCESS_DUPLEX;
        if first {
            // Fails when another process already made this pipe.
            mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        // SAFETY: plain arguments; the handle is owned by the File below.
        let handle = unsafe {
            CreateNamedPipeW(
                name,
                mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                None,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a fresh handle nothing else owns.
        Ok(unsafe { File::from_raw_handle(handle.0) })
    }

    pub fn listen(state_dir: &Path, handler: Handler) -> io::Result<PathBuf> {
        let name = pipe_name(state_dir);
        let wide = HSTRING::from(name.as_str());
        let mut next = Some(create(&wide, true)?);
        std::thread::Builder::new().name("instance".into()).spawn(move || {
            loop {
                let pipe = match next.take().map(Ok).unwrap_or_else(|| create(&wide, false)) {
                    Ok(pipe) => pipe,
                    Err(e) => {
                        log::warn!("instance pipe: {e}");
                        return;
                    }
                };
                // SAFETY: the pipe handle is live.
                match unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) } {
                    Ok(()) => {}
                    Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
                    Err(e) => {
                        log::warn!("instance pipe: {e}");
                        continue;
                    }
                }
                let reply = read_request(&pipe).and_then(|request| {
                    log::info!("a launch asked for {request:?}");
                    handler(request)
                });
                // Closing a pipe drops what the client hasn't read yet, so
                // wait until it has (FlushFileBuffers).
                if let Err(e) = answer(&pipe, &reply).and_then(|()| pipe.sync_all()) {
                    log::warn!("answering a launch: {e}");
                }
            }
        })?;
        Ok(PathBuf::from(name))
    }

    pub fn forward(state_dir: &Path, request: &Request) -> Result<(), String> {
        let name = pipe_name(state_dir);
        let deadline = Instant::now() + FORWARD_LIMIT;
        let pipe = loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(&name) {
                Ok(pipe) => break pipe,
                Err(e) if Instant::now() < deadline => {
                    log::debug!("connecting to {name}: {e}");
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(format!("could not reach the running Brindle at {name}: {e}")),
            }
        };
        // Windows lets a process take the foreground only when the one that
        // has it allows it: this launch has it, so it lets the instance.
        let mut pid = 0;
        // SAFETY: the pipe handle is live and `pid` is a plain out parameter.
        if unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }.is_ok() {
            // SAFETY: plain argument.
            if let Err(e) = unsafe { AllowSetForegroundWindow(pid) } {
                log::debug!("AllowSetForegroundWindow({pid}): {e}");
            }
        }
        exchange(&pipe, request)
    }
}

pub use imp::{forward, listen};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_as_one_line() {
        let request = Request {
            profile: Some("work".into()),
            cwd: Some("/srv".into()),
            command: Some(vec!["htop".into(), "-d".into(), "10".into()]),
            attach: Some(7),
            launch_dir: Some("/home/u".into()),
            activation_token: Some("tok".into()),
        };
        let line = request.encode();
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1, "{line:?}");
        assert_eq!(Request::decode(&line), Ok(request));
        // A plain launch is an empty object, and unknown fields are ignored.
        assert_eq!(Request::default().encode(), "{}\n");
        assert_eq!(Request::decode("{\"future\": 1}\n"), Ok(Request::default()));
        assert!(Request::decode("nope").is_err());
    }

    #[test]
    fn the_launch_token_is_the_first_one_set() {
        let some = |token: &str| Some(token.to_string());
        assert_eq!(launch_token(some("xdg"), some("startup")), some("xdg"));
        assert_eq!(launch_token(None, some("startup")), some("startup"));
        // An empty one counts as unset.
        assert_eq!(launch_token(some(""), some("startup")), some("startup"));
        assert_eq!(launch_token(some("xdg"), some("")), some("xdg"));
        assert_eq!(launch_token(some(""), some("")), None);
        assert_eq!(launch_token(None, None), None);
    }

    #[test]
    fn answers_parse() {
        assert_eq!(parse_answer("ok\n"), Ok(()));
        assert_eq!(parse_answer("error: no window\n"), Err("no window".into()));
        assert!(parse_answer("").is_err());
        let mut out = Vec::new();
        answer(&mut out, &Err("two\nlines".into())).unwrap();
        assert_eq!(out, b"error: two lines\n");
    }

    #[test]
    fn names_depend_only_on_the_state_directory() {
        let a = name(Path::new("/home/u/.local/state/brindle"));
        assert_eq!(a, name(Path::new("/home/u/.local/state/brindle")));
        assert_ne!(a, name(Path::new("/tmp/e2e/state")));
        // Pinned: another Brindle version must find the same socket.
        assert_eq!(name(Path::new("")), "brindle-instance-cbf29ce484222325");
        assert!(a.len() < 40);
    }

    #[test]
    fn the_socket_is_in_the_runtime_dir_else_the_cache_else_temp() {
        let state = Path::new("/home/u/.local/state/brindle");
        let socket = format!("{}.sock", name(state));
        let at = |runtime: Option<&str>, cache: Option<&str>| {
            socket_path_in(runtime.map(OsString::from), cache.map(PathBuf::from), PathBuf::from("/tmp"), state)
        };
        assert_eq!(at(Some("/run/user/1000"), Some("/home/u/.cache")), PathBuf::from("/run/user/1000").join(&socket));
        assert_eq!(at(None, Some("/home/u/.cache")), PathBuf::from("/home/u/.cache").join("brindle/run").join(&socket));
        assert_eq!(at(None, None), PathBuf::from("/tmp").join("brindle/run").join(&socket));
    }

    #[test]
    fn an_empty_runtime_dir_counts_as_unset() {
        let state = Path::new("/home/u/.local/state/brindle");
        assert_eq!(
            socket_path_in(Some(OsString::new()), Some(PathBuf::from("/home/u/.cache")), PathBuf::from("/tmp"), state),
            socket_path_in(None, Some(PathBuf::from("/home/u/.cache")), PathBuf::from("/tmp"), state)
        );
    }

    #[test]
    fn the_pipe_names_its_user_in_letters_digits_and_underscores() {
        let state = Path::new(r"C:\Users\me\AppData\Local\brindle\state");
        assert_eq!(pipe_name_for(Some("me".into()), state), format!(r"\\.\pipe\{}-me", name(state)));
        assert_eq!(pipe_name_for(Some("J. Doe-2\\é".into()), state), format!(r"\\.\pipe\{}-J__Doe_2__", name(state)));
    }

    #[test]
    fn a_missing_user_leaves_the_user_part_empty() {
        let state = Path::new(r"C:\Users\me\AppData\Local\brindle\state");
        assert_eq!(pipe_name_for(None, state), format!(r"\\.\pipe\{}-", name(state)));
        assert_eq!(pipe_name_for(Some(String::new()), state), pipe_name_for(None, state));
    }

    /// A launch hands its request only to an instance of its own user:
    /// the refusal names where it wasn't sent and why.
    #[test]
    fn a_launch_is_sent_only_to_an_instance_of_the_same_user() {
        assert_eq!(same_user(1000, Ok(1000)), Ok(()));
        assert_eq!(same_user(0, Ok(0)), Ok(()));
        assert_eq!(same_user(1000, Ok(0)), Err("it is served by another user (uid 0, not 1000)".into()));
        assert_eq!(same_user(0, Ok(1000)), Err("it is served by another user (uid 1000, not 0)".into()));
        // Not knowing who serves it refuses it too.
        let unknown = io::Error::new(io::ErrorKind::PermissionDenied, "not permitted");
        assert_eq!(same_user(1000, Err(unknown)), Err("the user serving it can't be told: not permitted".into()));

        let socket = "/run/user/1000/brindle-instance-cbf29ce484222325.sock";
        assert_eq!(
            not_sent(socket, same_user(1000, Ok(0)).unwrap_err()),
            "not sending the launch to /run/user/1000/brindle-instance-cbf29ce484222325.sock: \
             it is served by another user (uid 0, not 1000)"
        );
        // A pipe's refusal reads the same way.
        assert_eq!(
            not_sent(r"\\.\pipe\brindle-instance-cbf29ce484222325-me", "the server runs at medium integrity, and this process at high"),
            r"not sending the launch to \\.\pipe\brindle-instance-cbf29ce484222325-me: the server runs at medium integrity, and this process at high"
        );
    }

    /// A socket this user may connect to that another user serves, if this
    /// machine has one: the system bus's, which its bus daemon's or
    /// systemd's user listens on.
    #[cfg(target_os = "linux")]
    fn socket_of_another_user() -> Option<PathBuf> {
        // SAFETY: takes no argument and can't fail.
        let own = unsafe { libc::geteuid() };
        ["/run/dbus/system_bus_socket", "/var/run/dbus/system_bus_socket"].into_iter().map(PathBuf::from).find(|path| {
            std::os::unix::net::UnixStream::connect(path)
                .and_then(|stream| imp::peer_uid(&stream))
                .is_ok_and(|peer| peer != own)
        })
    }

    /// The launch refuses a socket another user serves before it sends
    /// anything: what comes back is the refusal, not an answer (or the
    /// lack of one) to a request.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_socket_served_by_another_user_is_refused_before_anything_is_sent() {
        let Some(socket) = socket_of_another_user() else {
            eprintln!(
                "skipped: a_socket_served_by_another_user_is_refused_before_anything_is_sent needs a socket \
                 another user serves, and found none (the system bus at /run/dbus/system_bus_socket, \
                 served by another user than this one)"
            );
            return;
        };
        let request = Request { command: Some(vec!["secret".into()]), ..Default::default() };
        let error = imp::forward_to(&socket, &request).unwrap_err();
        let expected = format!("not sending the launch to {}: it is served by another user (uid ", socket.display());
        assert!(error.starts_with(&expected), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_forwarded_request_reaches_the_handler() {
        let dir = std::env::temp_dir().join(format!("brindle-instance-test-{}", std::process::id()));
        // A private socket, given by path: setting XDG_RUNTIME_DIR would
        // change it for every test in this process, and the terminal
        // server's scope decision reads it.
        let socket = dir.join("run").join("instance.sock");
        let (tx, rx) = std::sync::mpsc::channel();
        imp::listen_at(
            socket.clone(),
            Box::new(move |request: Request| {
                let refuse = request.attach == Some(0);
                tx.send(request).unwrap();
                if refuse { Err("no such session".into()) } else { Ok(()) }
            }),
        )
        .unwrap();
        let request = Request { profile: Some("p".into()), ..Default::default() };
        assert_eq!(imp::forward_to(&socket, &request), Ok(()));
        assert_eq!(rx.recv().unwrap(), request);
        assert_eq!(imp::forward_to(&socket, &Request { attach: Some(0), ..Default::default() }), Err("no such session".into()));
        std::fs::remove_dir_all(&dir).ok();
    }
}
