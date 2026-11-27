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

use crate::agent::model::LaunchChoice;
use crate::{AgentLaunch, Command, FirstPrompt, PromptArg, TabKind, TerminalLaunch};

/// What an instance answers a request of the `agent` kind without its
/// `agent` fields.
const NO_AGENT_FIELDS: &str = "an agent request needs its agent fields";

/// How long a launch waits for the instance to take its request.
pub const FORWARD_LIMIT: Duration = Duration::from_secs(5);

/// The longest request line read, its line end included; anything longer
/// is refused.
const MAX_REQUEST: u64 = 1 << 20;

/// What a launch asks the instance for: one JSON line, never versioned or
/// negotiated, so fields are only ever added, each omitted when unset, and
/// unknown ones are ignored (the `process R4` exception in
/// `docs/architecture/README.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Request {
    /// The subcommand's kind of tab (`terminal`), omitted by a bare launch
    /// and by launches from before subcommands. A string on the wire, so a
    /// kind this Brindle doesn't know reaches [`Request::asked`] by name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// `brindle terminal PROFILE`: a profile name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// `brindle terminal -d`, made absolute by the launch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// `brindle terminal -e`: the program and its arguments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// `brindle agent`'s fields, with `kind` `agent`. An agent request
    /// fills none of `profile`, `cwd` and `command`, so an instance that
    /// reads only those never takes it for a terminal launch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentRequest>,
    /// `--attach`: a terminal server session id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attach: Option<u32>,
    /// The launching process's working directory, where a `-e` command
    /// starts without `-d`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_dir: Option<PathBuf>,
    /// The launcher's activation token (`XDG_ACTIVATION_TOKEN`, else
    /// `DESKTOP_STARTUP_ID`), which lets the instance take focus on Wayland.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_token: Option<String>,
}

/// `brindle agent [PROFILE] [-d DIR] [-p PROMPT]` in a request: the prompt
/// is always text by then (the launch read `-p -`), and the directory is
/// made absolute by the launch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// `-c` or `-r` (OpenSpec change `agents-session-list` decision 8),
    /// sent under [`AGENT_CHOOSE_KIND`] so an older Brindle refuses it
    /// rather than open a new conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choose: Option<AgentChoose>,
}

/// The kind of a request that continues an earlier conversation: an agent
/// request with its `choose` field.
pub const AGENT_CHOOSE_KIND: &str = "agent-choose";

/// What an agent request's `choose` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentChoose {
    /// `-c`: the latest conversation.
    Last,
    /// `-r`: the list.
    List,
}

impl AgentChoose {
    pub fn of(choice: LaunchChoice) -> Self {
        match choice {
            LaunchChoice::Last => Self::Last,
            LaunchChoice::List => Self::List,
        }
    }

    fn launch(self) -> LaunchChoice {
        match self {
            Self::Last => LaunchChoice::Last,
            Self::List => LaunchChoice::List,
        }
    }
}

/// The words a launch prints when the running Brindle refused its request
/// with `why`: for a continue (`kind` [`AGENT_CHOOSE_KIND`]) refused by
/// kind, that the running Brindle is older and can't continue one, with
/// its answer; else the answer itself.
pub fn refusal_text(kind: Option<&str>, why: &str) -> String {
    match kind {
        Some(AGENT_CHOOSE_KIND) if why.contains(&format!("can't open {AGENT_CHOOSE_KIND} tabs")) => {
            format!("the running Brindle is older and can't continue a conversation ({why})")
        }
        Some(_) | None => why.to_string(),
    }
}

impl AgentRequest {
    /// The launch the request carries, its prompt checked again
    /// ([`FirstPrompt::new`]).
    fn launch(&self) -> Result<AgentLaunch, String> {
        let prompt = match &self.prompt {
            Some(text) => Some(PromptArg::Text(
                FirstPrompt::new(text.clone()).map_err(|why| format!("the agent request's prompt: {why}"))?,
            )),
            None => None,
        };
        let choose = self.choose.map(AgentChoose::launch);
        Ok(AgentLaunch { profile: self.profile.clone(), cwd: self.cwd.clone(), prompt, choose })
    }
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

    /// What the request asks for, from this version's launches and older
    /// and newer ones: `attach` wins (an older launch may send it with a
    /// profile); the `terminal` kind, or no kind with a profile, directory
    /// or command (an older launch), is a terminal tab; the `agent` kind,
    /// with its `agent` fields, an agent tab; no kind and none of those
    /// only raises; any other kind is refused by name.
    pub fn asked(&self) -> Result<Forward, String> {
        if let Some(session) = self.attach {
            return Ok(Forward::Attach(session));
        }
        let terminal = || {
            Forward::Terminal(TerminalLaunch {
                profile: self.profile.clone(),
                cwd: self.cwd.clone(),
                command: self.command.as_deref().and_then(Command::from_argv),
            })
        };
        match self.kind.as_deref() {
            Some(kind) if kind == TabKind::Terminal.word() => Ok(terminal()),
            // This version reads a continue as an agent request with its
            // choice.
            Some(kind) if kind == TabKind::Agent.word() || kind == AGENT_CHOOSE_KIND => match &self.agent {
                Some(agent) => agent.launch().map(Forward::Agent),
                None => Err(NO_AGENT_FIELDS.into()),
            },
            Some(kind) => Err(format!(
                "this Brindle ({}) can't open {kind} tabs; quit it and launch again",
                env!("CARGO_PKG_VERSION")
            )),
            None if self.profile.is_some() || self.cwd.is_some() || self.command.is_some() => Ok(terminal()),
            None => Ok(Forward::Raise),
        }
    }
}

/// What a later launch's request opens ([`Request::asked`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Forward {
    /// Nothing: a plain launch raises the window.
    Raise,
    /// A terminal tab.
    Terminal(TerminalLaunch),
    /// An agent tab; its prompt, if any, is [`PromptArg::Text`].
    Agent(AgentLaunch),
    /// `--attach`: a tab on that detached session.
    Attach(u32),
}

/// The instance's answer to one request, one line: `ok`, `error: TEXT` or
/// `usage: TEXT`. A launch before `usage:` existed reads that line as an
/// error, prints it and exits 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The instance took the request.
    Accepted,
    /// It didn't, for this reason; the launch exits 1.
    Refused(String),
    /// The launch's arguments are wrong for this instance (an unknown
    /// profile); the launch exits 2, as for any usage error.
    Usage(String),
}

impl Answer {
    /// The answer's line.
    pub fn line(&self) -> String {
        match self {
            Self::Accepted => "ok\n".to_string(),
            Self::Refused(why) => format!("error: {}\n", why.replace('\n', " ")),
            Self::Usage(why) => format!("usage: {}\n", why.replace('\n', " ")),
        }
    }
}

/// Whether the encoded request `line`, its line end included, is within
/// what a running Brindle reads; else why not.
pub fn fits(line: &str) -> Result<(), String> {
    if line.len() as u64 <= MAX_REQUEST {
        Ok(())
    } else {
        Err(format!("the launch is {} bytes, over the 1 MiB a running Brindle takes", line.len()))
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

/// Writes `reply` as the answer to one request.
fn answer(mut stream: impl Write, reply: &Answer) -> io::Result<()> {
    stream.write_all(reply.line().as_bytes())?;
    stream.flush()
}

/// Reads one request line from `stream`: a line cut off at
/// [`MAX_REQUEST`] is refused as over the limit, not as malformed.
fn read_request(stream: impl Read) -> Result<Request, String> {
    let mut line = Vec::new();
    BufReader::new(stream.take(MAX_REQUEST))
        .read_until(b'\n', &mut line)
        .map_err(|e| format!("reading the request: {e}"))?;
    if line.len() as u64 == MAX_REQUEST && line.last() != Some(&b'\n') {
        return Err("the request is over the 1 MiB limit".into());
    }
    let line = String::from_utf8(line).map_err(|e| format!("bad request: {e}"))?;
    Request::decode(&line)
}

/// Sends `request` over `stream` and reads the answer; `Err` when the
/// exchange itself failed.
fn exchange(mut stream: impl Read + Write, request: &Request) -> Result<Answer, String> {
    stream.write_all(request.encode().as_bytes()).map_err(|e| format!("sending the request: {e}"))?;
    stream.flush().map_err(|e| format!("sending the request: {e}"))?;
    let mut line = String::new();
    BufReader::new(stream.take(MAX_REQUEST)).read_line(&mut line).map_err(|e| format!("waiting for an answer: {e}"))?;
    Ok(parse_answer(&line))
}

fn parse_answer(line: &str) -> Answer {
    match line.trim_end() {
        "ok" => Answer::Accepted,
        "" => Answer::Refused("the running Brindle closed the connection without answering".into()),
        other => match other.strip_prefix("usage: ") {
            Some(why) => Answer::Usage(why.to_string()),
            None => Answer::Refused(other.strip_prefix("error: ").unwrap_or(other).to_string()),
        },
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
pub type Handler = Box<dyn Fn(Request) -> Answer + Send + 'static>;

/// The answer to one request read from a stream: the handler's, or a
/// refusal of a request that couldn't be read.
fn reply_to(read: Result<Request, String>, handler: &Handler) -> Answer {
    match read {
        Ok(request) => {
            log::info!("a launch asked for {request:?}");
            handler(request)
        }
        Err(why) => Answer::Refused(why),
    }
}

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
        let reply = reply_to(read_request(&stream), handler);
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

    /// Sends `request` to the instance and returns its answer; `Err` when it
    /// couldn't be reached or the exchange failed.
    pub fn forward(state_dir: &Path, request: &Request) -> Result<Answer, String> {
        forward_to(&socket_path(state_dir), request)
    }

    /// [`forward`] to the socket at `path`. Before anything is sent, the
    /// socket must be served by this user ([`same_user`]): the instance
    /// checks who connects, and the launch checks who it connected to.
    pub(super) fn forward_to(path: &Path, request: &Request) -> Result<Answer, String> {
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
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
    use windows::core::HSTRING;

    use crate::remote::protocol::win;

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
                let reply = reply_to(read_request(&pipe), &handler);
                // Closing a pipe drops what the client hasn't read yet, so
                // wait until it has (FlushFileBuffers).
                if let Err(e) = answer(&pipe, &reply).and_then(|()| pipe.sync_all()) {
                    log::warn!("answering a launch: {e}");
                }
            }
        })?;
        Ok(PathBuf::from(name))
    }

    /// Sends `request` to the instance's pipe. The pipe is opened so that
    /// its server can identify this launch but not act as it
    /// (`SECURITY_IDENTIFICATION`), and before anything is sent its server
    /// must run as this user at this integrity level
    /// ([`win::check_server`]): a pipe of that name made by another
    /// process, or by this user's Brindle at another level, gets nothing.
    /// `Err` when it couldn't be reached or the exchange failed.
    pub fn forward(state_dir: &Path, request: &Request) -> Result<Answer, String> {
        let name = pipe_name(state_dir);
        let deadline = Instant::now() + FORWARD_LIMIT;
        let pipe = loop {
            let opened = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .security_qos_flags(SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0)
                .open(&name);
            match opened {
                Ok(pipe) => break pipe,
                Err(e) if Instant::now() < deadline => {
                    log::debug!("connecting to {name}: {e}");
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(format!("could not reach the running Brindle at {name}: {e}")),
            }
        };
        win::check_server(&pipe).map_err(|why| not_sent(&name, why))?;
        // Windows lets a process take the foreground only when the one that
        // has it allows it: this launch has it, so it lets the instance,
        // once it is known to be this user's own.
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
            kind: Some("terminal".into()),
            profile: Some("work".into()),
            cwd: Some("/srv".into()),
            command: Some(vec!["htop".into(), "-d".into(), "10".into()]),
            agent: Some(AgentRequest {
                profile: Some("Fake".into()),
                cwd: Some("/src".into()),
                prompt: Some("hi".into()),
                choose: Some(AgentChoose::Last),
            }),
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

    /// `process R6`: this version's request lines, byte for byte.
    #[test]
    fn this_versions_requests_are_pinned() {
        assert_eq!(Request::default().encode(), "{}\n", "a bare launch");
        assert_eq!(Request { kind: Some("terminal".into()), ..Default::default() }.encode(), "{\"kind\":\"terminal\"}\n");
        let launch = Request {
            kind: Some("terminal".into()),
            profile: Some("Work".into()),
            cwd: Some("/srv".into()),
            command: Some(vec!["htop".into()]),
            launch_dir: Some("/srv".into()),
            ..Default::default()
        };
        assert_eq!(
            launch.encode(),
            "{\"kind\":\"terminal\",\"profile\":\"Work\",\"cwd\":\"/srv\",\"command\":[\"htop\"],\"launch_dir\":\"/srv\"}\n",
            "brindle terminal Work -d /srv -e htop, launched in /srv"
        );
    }

    /// `process R6`: the lines launches from before subcommands send keep
    /// opening what they asked for.
    #[test]
    fn older_requests_keep_decoding() {
        let asked = |line: &str| Request::decode(line).and_then(|request| request.asked());
        let terminal = |profile: Option<&str>, cwd: Option<&str>, command: Option<&str>| {
            Ok(Forward::Terminal(TerminalLaunch {
                profile: profile.map(Into::into),
                cwd: cwd.map(Into::into),
                command: command.map(|program| Command { program: program.into(), args: Vec::new() }),
            }))
        };
        assert_eq!(asked("{\"profile\":\"work\"}\n"), terminal(Some("work"), None, None));
        assert_eq!(
            asked("{\"cwd\":\"/srv\",\"command\":[\"htop\"],\"launch_dir\":\"/srv\"}\n"),
            terminal(None, Some("/srv"), Some("htop"))
        );
        assert_eq!(asked("{\"attach\":7,\"profile\":\"x\"}\n"), Ok(Forward::Attach(7)), "attach wins");
        assert_eq!(asked("{}\n"), Ok(Forward::Raise));
        assert_eq!(asked("{\"launch_dir\":\"/home/u\"}\n"), Ok(Forward::Raise));
        assert_eq!(asked("{\"kind\":\"terminal\"}\n"), terminal(None, None, None), "brindle terminal alone");
        assert_eq!(asked("{\"future\":1,\"profile\":\"w\"}\n"), terminal(Some("w"), None, None), "unknown fields are ignored");
    }

    #[test]
    fn a_kind_this_brindle_cant_open_is_refused_by_name() {
        let refused = Request::decode("{\"kind\":\"chat\",\"chat\":{}}\n").and_then(|request| request.asked());
        let why = refused.unwrap_err();
        assert!(why.contains("can't open chat tabs") && why.contains(env!("CARGO_PKG_VERSION")), "{why}");
    }

    fn agent_launch(profile: Option<&str>, cwd: Option<&str>, prompt: Option<&str>) -> Result<Forward, String> {
        Ok(Forward::Agent(AgentLaunch {
            profile: profile.map(Into::into),
            cwd: cwd.map(Into::into),
            prompt: prompt.map(|text| PromptArg::Text(FirstPrompt::new(text.into()).unwrap())),
            choose: None,
        }))
    }

    /// `agents-session-list` 6.1: a continue's line, byte for byte, under
    /// its own kind; this version reads it as an agent request with its
    /// choice; an older one's refusal of the kind is worded for the launch.
    #[test]
    fn a_continue_goes_under_its_own_kind() {
        let request = |choose| Request {
            kind: Some(AGENT_CHOOSE_KIND.into()),
            agent: Some(AgentRequest { profile: Some("Fake".into()), choose: Some(choose), ..Default::default() }),
            ..Default::default()
        };
        let line = "{\"kind\":\"agent-choose\",\"agent\":{\"profile\":\"Fake\",\"choose\":\"last\"}}\n";
        assert_eq!(request(AgentChoose::Last).encode(), line);
        assert_eq!(Request::decode(line), Ok(request(AgentChoose::Last)));
        let launch = |choose| {
            Ok(Forward::Agent(AgentLaunch { profile: Some("Fake".into()), choose: Some(choose), ..Default::default() }))
        };
        assert_eq!(request(AgentChoose::Last).asked(), launch(LaunchChoice::Last));
        assert_eq!(request(AgentChoose::List).asked(), launch(LaunchChoice::List));
        assert!(request(AgentChoose::List).encode().contains("\"choose\":\"list\""));
        assert_eq!(AgentChoose::of(LaunchChoice::List), AgentChoose::List);

        // An older Brindle refuses the kind by name.
        let older = "this Brindle (0.40.0) can't open agent-choose tabs; quit it and launch again";
        assert_eq!(
            refusal_text(Some(AGENT_CHOOSE_KIND), older),
            format!("the running Brindle is older and can't continue a conversation ({older})")
        );
        assert_eq!(refusal_text(Some("agent"), "Brindle is quitting"), "Brindle is quitting");
        assert_eq!(refusal_text(Some(AGENT_CHOOSE_KIND), "Brindle is quitting"), "Brindle is quitting");
    }

    /// `process R6`: an agent request's line, byte for byte, with each
    /// field and with none, and what it decodes to.
    #[test]
    fn agent_requests_are_pinned() {
        let full = Request {
            kind: Some("agent".into()),
            agent: Some(AgentRequest {
                profile: Some("Fake".into()),
                cwd: Some("/src".into()),
                prompt: Some("run the tests".into()),
                choose: None,
            }),
            launch_dir: Some("/home/u/src".into()),
            ..Default::default()
        };
        let line = "{\"kind\":\"agent\",\"agent\":{\"profile\":\"Fake\",\"cwd\":\"/src\",\"prompt\":\"run the tests\"},\
                    \"launch_dir\":\"/home/u/src\"}\n";
        assert_eq!(full.encode(), line, "brindle agent Fake -d /src -p 'run the tests', launched in /home/u/src");
        assert_eq!(Request::decode(line), Ok(full.clone()));
        assert_eq!(full.asked(), agent_launch(Some("Fake"), Some("/src"), Some("run the tests")));
        let bare = Request { kind: Some("agent".into()), agent: Some(AgentRequest::default()), ..Default::default() };
        assert_eq!(bare.encode(), "{\"kind\":\"agent\",\"agent\":{}}\n", "brindle agent alone");
        assert_eq!(bare.asked(), agent_launch(None, None, None));
        let asked = |line: &str| Request::decode(line).and_then(|request| request.asked());
        assert_eq!(asked("{\"kind\":\"agent\",\"agent\":{\"profile\":\"F\"}}\n"), agent_launch(Some("F"), None, None));
        assert_eq!(asked("{\"kind\":\"agent\",\"agent\":{\"prompt\":\"hi\"}}\n"), agent_launch(None, None, Some("hi")));
        assert_eq!(asked("{\"kind\":\"agent\",\"agent\":{\"cwd\":\"/srv\"}}\n"), agent_launch(None, Some("/srv"), None));
        assert_eq!(
            asked("{\"kind\":\"agent\",\"agent\":{\"profile\":\"F\",\"future\":1}}\n"),
            agent_launch(Some("F"), None, None),
            "unknown fields inside the agent fields are ignored"
        );
    }

    #[test]
    fn an_agent_request_without_its_fields_or_with_a_bad_prompt_is_refused() {
        let asked = |line: &str| Request::decode(line).and_then(|request| request.asked());
        assert_eq!(asked("{\"kind\":\"agent\"}\n"), Err("an agent request needs its agent fields".into()));
        assert_eq!(
            asked("{\"kind\":\"agent\",\"profile\":\"Fake\"}\n"),
            Err("an agent request needs its agent fields".into()),
            "the terminal fields don't stand in for them"
        );
        assert_eq!(
            asked("{\"kind\":\"agent\",\"agent\":{\"prompt\":\" \"}}\n"),
            Err("the agent request's prompt: the prompt is blank".into())
        );
        assert_eq!(
            asked("{\"attach\":7,\"kind\":\"agent\"}\n"),
            Ok(Forward::Attach(7)),
            "attach wins, as for any request"
        );
    }

    /// An agent request goes through `fits`, the one size check: 256 KiB
    /// of ordinary text fits; 256 KiB of control characters, each escaped
    /// as six bytes, doesn't, and is refused before connecting.
    #[test]
    fn an_agent_requests_size_is_checked_by_fits() {
        let with_prompt = |prompt: String| Request {
            kind: Some("agent".into()),
            agent: Some(AgentRequest { profile: Some("Fake".into()), cwd: None, prompt: Some(prompt), choose: None }),
            launch_dir: Some("/home/u".into()),
            ..Default::default()
        };
        let plain = with_prompt("x".repeat(crate::PROMPT_LIMIT));
        assert_eq!(fits(&plain.encode()), Ok(()));
        assert_eq!(Request::decode(&plain.encode()).map(|r| r.asked().is_ok()), Ok(true));
        let controls = with_prompt("\u{1}".repeat(crate::PROMPT_LIMIT));
        assert!(FirstPrompt::new("\u{1}".repeat(crate::PROMPT_LIMIT)).is_ok(), "a prompt the launch took");
        assert_eq!(controls.agent.as_ref().and_then(|a| a.prompt.as_ref()).map(String::len), Some(crate::PROMPT_LIMIT));
        let line = controls.encode();
        assert!(line.len() > 1 << 20, "{} bytes", line.len());
        let why = fits(&line).unwrap_err();
        assert!(why.contains("over the 1 MiB a running Brindle takes"), "{why}");
    }

    /// `process R6`: the three answer lines, written and read back.
    #[test]
    fn answers_round_trip() {
        let written = |reply: &Answer| {
            let mut out = Vec::new();
            answer(&mut out, reply).unwrap();
            String::from_utf8(out).unwrap()
        };
        assert_eq!(written(&Answer::Accepted), "ok\n");
        assert_eq!(written(&Answer::Refused("x".into())), "error: x\n");
        assert_eq!(written(&Answer::Usage("x".into())), "usage: x\n");
        assert_eq!(written(&Answer::Refused("two\nlines".into())), "error: two lines\n");
        for reply in [Answer::Accepted, Answer::Refused("no window".into()), Answer::Usage("no profile".into())] {
            assert_eq!(parse_answer(&written(&reply)), reply);
        }
        assert!(matches!(parse_answer(""), Answer::Refused(_)), "a closed connection");
        assert_eq!(parse_answer("something else\n"), Answer::Refused("something else".into()));
    }

    #[test]
    fn a_request_fits_up_to_one_mib() {
        let limit = MAX_REQUEST as usize;
        assert_eq!(fits(&"x".repeat(limit)), Ok(()));
        let why = fits(&"x".repeat(limit + 1)).unwrap_err();
        assert!(why.contains("1 MiB") && why.contains(&(limit + 1).to_string()), "{why}");
    }

    #[test]
    fn a_request_over_the_limit_is_refused_as_such() {
        let line = "x".repeat(2 << 20);
        assert_eq!(read_request(line.as_bytes()), Err("the request is over the 1 MiB limit".into()));
        // Exactly at the limit, line end included, is read.
        let mut at_limit = format!("{{\"profile\":\"{}\"}}", "p".repeat(MAX_REQUEST as usize - 15));
        at_limit.push('\n');
        assert_eq!(at_limit.len() as u64, MAX_REQUEST);
        assert!(read_request(at_limit.as_bytes()).is_ok());
        assert!(read_request("nope\n".as_bytes()).unwrap_err().starts_with("bad request: "));
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

    /// A launch reaches an instance of its own user and level: the pipe's
    /// check of its server, made before the request is written, lets this
    /// process's own listener through, and the answer comes back.
    #[cfg(windows)]
    #[test]
    fn a_forwarded_request_reaches_the_handler_through_the_checked_pipe() {
        let state = std::env::temp_dir().join(format!("brindle-instance-win-test-{}", std::process::id()));
        let (tx, rx) = std::sync::mpsc::channel();
        imp::listen(
            &state,
            Box::new(move |request: Request| {
                let refuse = request.attach == Some(0);
                tx.send(request).unwrap();
                if refuse { Answer::Refused("no such session".into()) } else { Answer::Accepted }
            }),
        )
        .unwrap();
        let request = Request { profile: Some("p".into()), ..Default::default() };
        assert_eq!(forward(&state, &request), Ok(Answer::Accepted));
        assert_eq!(rx.recv().unwrap(), request);
        assert_eq!(
            forward(&state, &Request { attach: Some(0), ..Default::default() }),
            Ok(Answer::Refused("no such session".into()))
        );
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
                let reply = match (request.attach, request.profile.as_deref()) {
                    (Some(0), _) => Answer::Refused("no such session".into()),
                    (_, Some("wrk")) => Answer::Usage("no profile is named \"wrk\"".into()),
                    _ => Answer::Accepted,
                };
                tx.send(request).unwrap();
                reply
            }),
        )
        .unwrap();
        let request = Request { profile: Some("p".into()), ..Default::default() };
        assert_eq!(imp::forward_to(&socket, &request), Ok(Answer::Accepted));
        assert_eq!(rx.recv().unwrap(), request);
        assert_eq!(
            imp::forward_to(&socket, &Request { attach: Some(0), ..Default::default() }),
            Ok(Answer::Refused("no such session".into()))
        );
        assert_eq!(
            imp::forward_to(&socket, &Request { profile: Some("wrk".into()), ..Default::default() }),
            Ok(Answer::Usage("no profile is named \"wrk\"".into()))
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
