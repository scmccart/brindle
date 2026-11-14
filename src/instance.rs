//! One Brindle per state directory. The process holding the session lock is
//! the instance: it listens on a socket (a named pipe on Windows) named
//! after the state directory. A later launch that can't take the lock sends
//! its request there as one JSON line and exits once the instance answers
//! `ok`; the instance raises its window and opens what was asked for.

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

    /// The activation token this process was started with, if any.
    pub fn launch_token() -> Option<String> {
        ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|token| !token.is_empty())
    }
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

/// Handles one request on the listener's thread, returning once the
/// instance has taken it (or refused it).
pub type Handler = Box<dyn Fn(Request) -> Result<(), String> + Send + 'static>;

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::{UnixListener, UnixStream};

    /// `$XDG_RUNTIME_DIR/<name>.sock`, else under `~/.cache/brindle/run`.
    pub fn socket_path(state_dir: &Path) -> PathBuf {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("brindle/run"));
        dir.join(format!("{}.sock", name(state_dir)))
    }

    pub fn listen(state_dir: &Path, handler: Handler) -> io::Result<PathBuf> {
        let path = socket_path(state_dir);
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

    fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
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
        let path = socket_path(state_dir);
        let deadline = Instant::now() + FORWARD_LIMIT;
        let stream = loop {
            match UnixStream::connect(&path) {
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

    /// `\\.\pipe\<name>-<user>`. The pipe's default security lets only its
    /// owner (and administrators) write to it.
    pub fn pipe_name(state_dir: &Path) -> String {
        let user: String = std::env::var("USERNAME")
            .unwrap_or_default()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        format!(r"\\.\pipe\{}-{user}", name(state_dir))
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

    #[cfg(unix)]
    #[test]
    fn a_forwarded_request_reaches_the_handler() {
        let dir = std::env::temp_dir().join(format!("brindle-instance-test-{}", std::process::id()));
        // A private runtime directory for the socket.
        let runtime = dir.join("run");
        std::fs::create_dir_all(&runtime).unwrap();
        // SAFETY: the only test that reads XDG_RUNTIME_DIR.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &runtime) };
        let (tx, rx) = std::sync::mpsc::channel();
        listen(
            &dir,
            Box::new(move |request: Request| {
                let refuse = request.attach == Some(0);
                tx.send(request).unwrap();
                if refuse { Err("no such session".into()) } else { Ok(()) }
            }),
        )
        .unwrap();
        let request = Request { profile: Some("p".into()), ..Default::default() };
        assert_eq!(forward(&dir, &request), Ok(()));
        assert_eq!(rx.recv().unwrap(), request);
        assert_eq!(forward(&dir, &Request { attach: Some(0), ..Default::default() }), Err("no such session".into()));
        std::fs::remove_dir_all(&dir).ok();
    }
}
