//! Shared pieces for the terminal-server tests: a temp server directory,
//! starting a server process with `--ready-fd`, a test core speaking H1, a
//! test client speaking protocol 1, and process checks.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use brindle_remote_protocol::client::{self, ToClient};
use brindle_remote_protocol::holder::{
    self as h1, Assembler, Data, Hello, SessionRef, Spawn, ToCore, ToHolder, Welcome,
};
use brindle_remote_protocol::{Frame, RequestId, SessionId};

pub const BIN: &str = env!("CARGO_BIN_EXE_brindle-remote");
/// Long enough for a loaded CI machine; tests poll, so passing ones are fast.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// A fresh user-only directory under the system temp dir (short, so socket
/// paths fit). Dropping it kills the holders it saw and removes it.
pub struct TempDir {
    pub path: PathBuf,
    pids: Vec<u32>,
}

impl TempDir {
    pub fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("bt-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            path,
            pids: Vec::new(),
        }
    }

    /// Kills `pid` when the directory is dropped, if it still runs.
    pub fn track(&mut self, pid: u32) {
        self.pids.push(pid);
    }

    /// Every log in the directory, concatenated.
    pub fn logs(&self) -> String {
        let mut text = String::new();
        for entry in fs::read_dir(&self.path).unwrap().flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "log") {
                text.push_str(&fs::read_to_string(entry.path()).unwrap_or_default());
            }
        }
        text
    }

    pub fn holder_sockets(&self) -> Vec<PathBuf> {
        let mut sockets: Vec<PathBuf> = fs::read_dir(&self.path)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                let name = path.file_name().unwrap().to_string_lossy();
                name.starts_with("holder-") && name.ends_with(".sock")
            })
            .collect();
        sockets.sort();
        sockets
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let mut pids = self.pids.clone();
        pids.extend(server_processes(&self.path, "serve"));
        pids.extend(server_processes(&self.path, "hold"));
        for pid in pids {
            if alive(pid) {
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            }
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// The running `brindle-remote <role>` processes (`serve` or `hold`) whose
/// environment names `dir` as their server directory.
pub fn server_processes(dir: &Path, role: &str) -> Vec<u32> {
    let wanted = format!("BRINDLE_SERVER_DIR={}", dir.display());
    let mut pids: Vec<u32> = fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&pid| {
            let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) else {
                return false;
            };
            let is_role = cmdline.split(|&b| b == 0).nth(1) == Some(role.as_bytes());
            is_role
                && alive(pid)
                && fs::read(format!("/proc/{pid}/environ"))
                    .is_ok_and(|env| env.split(|&b| b == 0).any(|var| var == wanted.as_bytes()))
        })
        .collect();
    pids.sort_unstable();
    pids
}

/// Runs `bin <role> --ready-fd 3` for `dir` and waits for its ready byte.
/// Fails, with the server's stderr, if it exits before being ready.
pub fn start(bin: &Path, role: &str, dir: &Path, env: &[(&str, &str)]) -> io::Result<()> {
    let mut fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let write_fd = write.as_raw_fd();
    let mut command = Command::new(bin);
    command
        .args([role, "--ready-fd", "3"])
        .env("BRINDLE_SERVER_DIR", dir)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(write_fd, 3) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // A freshly written binary can be briefly busy: another test thread's
    // fork may still hold the writer's fd until it execs.
    let mut attempts = 0;
    let child = loop {
        match command.spawn() {
            Err(error) if error.kind() == io::ErrorKind::ExecutableFileBusy && attempts < 100 => {
                attempts += 1;
                thread::sleep(Duration::from_millis(20));
            }
            result => break result?,
        }
    };
    drop(write);
    let mut pollfd = libc::pollfd {
        fd: read.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let ready = unsafe { libc::poll(&mut pollfd, 1, TIMEOUT.as_millis() as libc::c_int) };
    let mut byte = [0u8; 1];
    let got =
        ready == 1 && unsafe { libc::read(read.as_raw_fd(), byte.as_mut_ptr().cast(), 1) } == 1;
    let output = child.wait_with_output()?;
    if !got {
        return Err(io::Error::other(format!(
            "{role} wasn't ready ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "the starter exited with {}",
            output.status
        )));
    }
    Ok(())
}

/// Starts a holder from the built binary. A core it starts on its own (once
/// a test's core has gone) only prints the version.
pub fn start_holder(dir: &Path) {
    start(
        Path::new(BIN),
        "hold",
        dir,
        &[("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version")],
    )
    .unwrap();
}

/// A blob per session, as WELCOME's followers deliver them.
#[derive(Debug, Default)]
pub struct Adopted {
    pub welcome: Option<Welcome>,
    pub checkpoints: HashMap<SessionId, Vec<u8>>,
    pub tails: HashMap<SessionId, Vec<u8>>,
}

impl Adopted {
    pub fn welcome(&self) -> &Welcome {
        self.welcome.as_ref().unwrap()
    }
}

/// A core for tests: H1 over a holder's socket, read on a thread.
pub struct TestCore {
    stream: UnixStream,
    incoming: Receiver<io::Result<ToCore>>,
    backlog: VecDeque<ToCore>,
    closed: bool,
}

impl TestCore {
    pub fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket)
            .unwrap_or_else(|error| panic!("connect {}: {error}", socket.display()));
        let mut reader = stream.try_clone().unwrap();
        let (sender, incoming) = mpsc::channel();
        thread::spawn(move || {
            loop {
                let message = match Frame::read_from_or_eof(&mut reader) {
                    Ok(Some(frame)) => ToCore::decode(&frame),
                    Ok(None) => Err(io::ErrorKind::UnexpectedEof.into()),
                    Err(error) => Err(error),
                };
                let stop = message.is_err();
                if sender.send(message).is_err() || stop {
                    return;
                }
            }
        });
        Self {
            stream,
            incoming,
            backlog: VecDeque::new(),
            closed: false,
        }
    }

    /// Connects to the directory's only holder.
    pub fn connect_to(dir: &TempDir) -> Self {
        let sockets = dir.holder_sockets();
        assert_eq!(sockets.len(), 1, "holder sockets: {sockets:?}");
        Self::connect(&sockets[0])
    }

    pub fn send(&mut self, message: ToHolder) {
        message
            .encode()
            .unwrap()
            .write_to(&mut self.stream)
            .unwrap();
    }

    /// The next message, or `None` once the holder closed the connection.
    pub fn try_recv(&mut self, timeout: Duration) -> Option<ToCore> {
        if let Some(message) = self.backlog.pop_front() {
            return Some(message);
        }
        if self.closed {
            return None;
        }
        match self.incoming.recv_timeout(timeout) {
            Ok(Ok(message)) => Some(message),
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
                self.closed = true;
                None
            }
            Err(RecvTimeoutError::Timeout) => {
                panic!("no message from the holder within {timeout:?}")
            }
        }
    }

    pub fn recv(&mut self) -> ToCore {
        self.try_recv(TIMEOUT)
            .expect("the holder closed the connection")
    }

    /// Whether the holder closes the connection within the timeout, skipping
    /// any messages before that.
    pub fn closes(&mut self) -> bool {
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            if self.try_recv(deadline - Instant::now()).is_none() {
                return true;
            }
        }
        false
    }

    /// Sends `HELLO` and collects `WELCOME` with every checkpoint and tail.
    pub fn hello(&mut self, instance: u64) -> Adopted {
        self.send(ToHolder::Hello(Hello {
            min: h1::PROTOCOLS.min,
            max: h1::PROTOCOLS.max,
            core_build: "test-core".into(),
            instance,
        }));
        let ToCore::Welcome(welcome) = self.recv() else {
            panic!("the first message wasn't WELCOME");
        };
        let mut adopted = Adopted::default();
        let mut checkpoints = Assembler::new(h1::MAX_CHECKPOINT_LEN);
        let mut tails = Assembler::new(h1::TAIL_LIMIT);
        let mut left = welcome.sessions.len();
        while left > 0 {
            match self.recv() {
                ToCore::CheckpointBegin(begin) => checkpoints.begin(begin).unwrap(),
                ToCore::CheckpointChunk(chunk) => checkpoints.chunk(&chunk).unwrap(),
                ToCore::CheckpointEnd(end) => {
                    adopted
                        .checkpoints
                        .insert(end.session, checkpoints.end(end).unwrap());
                }
                ToCore::TailBegin(begin) => tails.begin(begin).unwrap(),
                ToCore::TailChunk(chunk) => tails.chunk(&chunk).unwrap(),
                ToCore::TailEnd(end) => {
                    adopted.tails.insert(end.session, tails.end(end).unwrap());
                    left -= 1;
                }
                other => panic!("unexpected {other:?} while adopting"),
            }
        }
        adopted.welcome = Some(welcome);
        adopted
    }

    /// Sends a `SPAWN` and returns the program's pid.
    pub fn spawn(&mut self, spawn: Spawn) -> u32 {
        let session = spawn.session;
        self.send(ToHolder::Spawn(spawn));
        match self.recv_skipping_output() {
            ToCore::Spawned(spawned) if spawned.session == session => spawned.pid,
            other => panic!("expected SPAWNED for {session}, got {other:?}"),
        }
    }

    /// The next message that isn't `OUTPUT`.
    pub fn recv_skipping_output(&mut self) -> ToCore {
        loop {
            match self.recv() {
                ToCore::Output(_) => {}
                other => return other,
            }
        }
    }

    /// Collects `session`'s output until it contains `needle`. Other
    /// messages are kept for later.
    pub fn output_until(&mut self, session: SessionId, needle: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        let mut kept = Vec::new();
        let deadline = Instant::now() + TIMEOUT;
        // Where the search resumes, so a long stream isn't rescanned.
        let mut searched = 0;
        loop {
            if contains(&output[searched..], needle) {
                break;
            }
            searched = output.len().saturating_sub(needle.len());
            assert!(
                Instant::now() < deadline,
                "no {:?} in {:?}",
                String::from_utf8_lossy(needle),
                String::from_utf8_lossy(&output)
            );
            match self.try_recv(TIMEOUT) {
                Some(ToCore::Output(Data { session: s, bytes })) if s == session => {
                    output.extend(bytes)
                }
                Some(other) => kept.push(other),
                None => panic!(
                    "connection closed; output so far {:?}",
                    String::from_utf8_lossy(&output)
                ),
            }
        }
        self.backlog.extend(kept);
        output
    }

    /// Waits for `session`'s `EXITED`, skipping output, and returns its code.
    pub fn exit_code(&mut self, session: SessionId) -> Option<i32> {
        loop {
            match self.recv_skipping_output() {
                ToCore::Exited(exited) if exited.session == session => return exited.code,
                ToCore::Exited(_) | ToCore::Error(_) => {}
                other => panic!("expected EXITED for {session}, got {other:?}"),
            }
        }
    }

    /// All of `session`'s output up to its `EXITED`, and its code.
    pub fn output_and_exit(&mut self, session: SessionId) -> (Vec<u8>, Option<i32>) {
        let mut output = Vec::new();
        loop {
            match self.recv() {
                ToCore::Output(Data { session: s, bytes }) if s == session => output.extend(bytes),
                ToCore::Exited(exited) if exited.session == session => {
                    return (output, exited.code);
                }
                ToCore::Output(_) | ToCore::Exited(_) | ToCore::Error(_) => {}
                other => panic!("expected EXITED for {session}, got {other:?}"),
            }
        }
    }

    pub fn input(&mut self, session: SessionId, bytes: &[u8]) {
        self.send(ToHolder::Input(Data {
            session,
            bytes: bytes.to_vec(),
        }));
    }

    pub fn hang_up(&mut self, session: SessionId) {
        self.send(ToHolder::Hangup(SessionRef { session }));
    }
}

impl Drop for TestCore {
    /// Closes the connection, as a core exiting or being killed would; the
    /// reader thread's clone would otherwise keep it open.
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// A `SPAWN` of `command args` with a minimal environment of its own.
pub fn spawn(session: SessionId, command: &str, args: &[&str]) -> Spawn {
    Spawn {
        session,
        command: Some(command.into()),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        cwd: None,
        env: base_env(),
        cols: 80,
        rows: 24,
        cell_w: 0,
        cell_h: 0,
        meta: Default::default(),
    }
}

pub fn base_env() -> BTreeMap<String, String> {
    BTreeMap::from([(
        "PATH".to_string(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
    )])
}

/// A `sh -c script` session.
pub fn shell(session: SessionId, script: &str) -> Spawn {
    spawn(session, "/bin/sh", &["-c", script])
}

/// The state letter in `/proc/<pid>/stat`, `None` once the pid is gone.
pub fn state(pid: u32) -> Option<char> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat[stat.rfind(')')? + 1..].trim_start().chars().next()
}

/// Running, and not a zombie.
pub fn alive(pid: u32) -> bool {
    state(pid).is_some_and(|state| state != 'Z' && state != 'X')
}

pub fn parent(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat[stat.rfind(')')? + 1..]
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Children of `pid` named `comm`.
pub fn children_named(pid: u32, comm: &str) -> Vec<u32> {
    fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&child| {
            parent(child) == Some(pid)
                && fs::read_to_string(format!("/proc/{child}/comm"))
                    .is_ok_and(|name| name.trim_end() == comm)
        })
        .collect()
}

/// Polls `condition` every 20 ms until it holds or `timeout` passes.
pub fn eventually(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Starts a core for `dir` from the built binary. Its holders inherit
/// `env`; unless it says otherwise, a core a holder starts on its own (once
/// a test has killed the core) only prints the version.
pub fn start_core(dir: &Path, env: &[(&str, &str)]) -> io::Result<()> {
    let mut env = env.to_vec();
    if !env
        .iter()
        .any(|(key, _)| *key == "BRINDLE_TEST_HOLDER_CORE_ARGS")
    {
        env.push(("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version"));
    }
    start(Path::new(BIN), "serve", dir, &env)
}

pub fn core_socket(dir: &Path) -> PathBuf {
    dir.join("core.sock")
}

/// What [`TestClient::next`] got.
#[derive(Debug)]
pub enum Recv {
    Message(ToClient),
    Closed,
    TimedOut,
}

/// A client for tests: protocol 1 over the core's socket, read on the
/// calling thread (so a test can stop reading, or hand the connection to
/// another process).
pub struct TestClient {
    stream: UnixStream,
    input: Vec<u8>,
    backlog: VecDeque<ToClient>,
    closed: bool,
    next_req: RequestId,
    pub welcome: Option<client::Welcome>,
}

impl TestClient {
    pub fn connect(dir: &Path) -> Self {
        let socket = core_socket(dir);
        let stream = UnixStream::connect(&socket)
            .unwrap_or_else(|error| panic!("connect {}: {error}", socket.display()));
        Self {
            stream,
            input: Vec::new(),
            backlog: VecDeque::new(),
            closed: false,
            next_req: 1,
            welcome: None,
        }
    }

    /// Connects and says `HELLO` with `token`, expecting `WELCOME`.
    pub fn hello(dir: &Path, token: &str) -> Self {
        let mut client = Self::connect(dir);
        client.send(client::ToCore::Hello(client::Hello {
            min: client::PROTOCOLS.min,
            max: client::PROTOCOLS.max,
            client_version: "test".into(),
            client_token: token.into(),
        }));
        match client.recv() {
            ToClient::Welcome(welcome) => client.welcome = Some(welcome),
            other => panic!("expected WELCOME, got {other:?}"),
        }
        client
    }

    /// The core's pid, from `WELCOME`.
    pub fn core_pid(&self) -> u32 {
        self.welcome.as_ref().expect("WELCOME").pid
    }

    pub fn send(&mut self, message: client::ToCore) {
        message
            .encode()
            .unwrap()
            .write_to(&mut self.stream)
            .unwrap();
    }

    fn frame(&mut self) -> Option<Frame> {
        let header = self.input.first_chunk::<5>()?;
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if self.input.len() < 5 + len {
            return None;
        }
        let frame = Frame::new(header[0], self.input[5..5 + len].to_vec()).unwrap();
        self.input.drain(..5 + len);
        Some(frame)
    }

    /// The next message within `timeout`.
    pub fn next(&mut self, timeout: Duration) -> Recv {
        if let Some(message) = self.backlog.pop_front() {
            return Recv::Message(message);
        }
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(frame) = self.frame() {
                return Recv::Message(ToClient::decode(&frame).unwrap());
            }
            if self.closed {
                return Recv::Closed;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Recv::TimedOut;
            }
            self.stream
                .set_read_timeout(Some(remaining.max(Duration::from_millis(1))))
                .unwrap();
            let mut buffer = [0u8; 64 * 1024];
            match io::Read::read(&mut self.stream, &mut buffer) {
                Ok(0) => self.closed = true,
                Ok(read) => self.input.extend_from_slice(&buffer[..read]),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => self.closed = true,
            }
        }
    }

    pub fn recv(&mut self) -> ToClient {
        match self.next(TIMEOUT) {
            Recv::Message(message) => message,
            other => panic!("expected a message from the core, got {other:?}"),
        }
    }

    /// Whether the core closes the connection within the timeout, skipping
    /// any messages before that.
    pub fn closes(&mut self) -> bool {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match self.next(deadline.saturating_duration_since(Instant::now())) {
                Recv::Message(_) => {}
                Recv::Closed => return true,
                Recv::TimedOut => return false,
            }
        }
    }

    fn take_req(&mut self) -> RequestId {
        let req = self.next_req;
        self.next_req += 1;
        req
    }

    /// Sends a `SPAWN` and returns the reply (`SPAWNED` or `ERROR`). Other
    /// messages are kept for later.
    pub fn spawn_reply(&mut self, mut spawn: client::Spawn) -> ToClient {
        spawn.req = self.take_req();
        let req = spawn.req;
        self.send(client::ToCore::Spawn(spawn));
        self.reply(req)
    }

    /// The reply to request `req`, keeping other messages for later.
    pub fn reply(&mut self, req: RequestId) -> ToClient {
        let mut kept = Vec::new();
        let reply = loop {
            let message = self.recv();
            let matches = match &message {
                ToClient::Spawned(spawned) => spawned.req == req,
                ToClient::Error(error) => error.req == Some(req),
                ToClient::Ok(ok) => ok.req == req,
                ToClient::Sessions(sessions) => sessions.req == req,
                ToClient::ServerStatus(status) => status.req == req,
                _ => false,
            };
            if matches {
                break message;
            }
            kept.push(message);
        };
        kept.extend(self.backlog.drain(..));
        self.backlog = kept.into();
        reply
    }

    /// Spawns a session and returns its id and the program's pid.
    pub fn spawn(&mut self, spawn: client::Spawn) -> (SessionId, u32) {
        match self.spawn_reply(spawn) {
            ToClient::Spawned(spawned) => (spawned.session, spawned.pid),
            other => panic!("expected SPAWNED, got {other:?}"),
        }
    }

    pub fn input(&mut self, session: SessionId, bytes: &[u8]) {
        self.send(client::ToCore::Data(client::Data {
            session,
            bytes: bytes.to_vec(),
        }));
    }

    /// Collects `session`'s output until it contains `needle`. Other
    /// messages are kept for later.
    pub fn output_until(&mut self, session: SessionId, needle: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        let mut kept = Vec::new();
        let deadline = Instant::now() + TIMEOUT;
        let mut searched = 0;
        while !contains(&output[searched..], needle) {
            searched = output.len().saturating_sub(needle.len());
            match self.next(deadline.saturating_duration_since(Instant::now())) {
                Recv::Message(ToClient::Data(client::Data { session: s, bytes }))
                    if s == session =>
                {
                    output.extend(bytes)
                }
                Recv::Message(other) => kept.push(other),
                other => panic!(
                    "{other:?} before {:?}; output so far {:?}",
                    String::from_utf8_lossy(needle),
                    String::from_utf8_lossy(&output)
                ),
            }
        }
        kept.extend(self.backlog.drain(..));
        self.backlog = kept.into();
        output
    }

    /// Waits for `session`'s `EXIT`, skipping output, and returns its code.
    pub fn exit_code(&mut self, session: SessionId) -> Option<i32> {
        self.output_and_exit(session).1
    }

    /// `session`'s output up to its `EXIT`, and its code. Other messages are
    /// kept for later.
    pub fn output_and_exit(&mut self, session: SessionId) -> (Vec<u8>, Option<i32>) {
        let mut output = Vec::new();
        let mut kept = Vec::new();
        let result = loop {
            match self.recv() {
                ToClient::Data(client::Data { session: s, bytes }) if s == session => {
                    output.extend(bytes)
                }
                ToClient::Exit(exit) if exit.session == session => break (output, exit.code),
                ToClient::Data(_) => {}
                other => kept.push(other),
            }
        };
        kept.extend(self.backlog.drain(..));
        self.backlog = kept.into();
        result
    }

    /// Hands the connection to a `sleep` process (as its fd 3) and closes
    /// this side, so killing that process is a client dying.
    pub fn into_process(self) -> std::process::Child {
        let fd = self.stream.as_raw_fd();
        let mut command = Command::new("sleep");
        command.arg("1000").stdin(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                let result = if fd == 3 {
                    libc::fcntl(fd, libc::F_SETFD, 0)
                } else {
                    libc::dup2(fd, 3)
                };
                if result == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        drop(self);
        child
    }
}

/// A client `SPAWN` of `command args` with a minimal environment of its own.
pub fn client_spawn(command: &str, args: &[&str]) -> client::Spawn {
    client::Spawn {
        req: 0,
        command: Some(command.into()),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        cwd: None,
        env: base_env(),
        cols: 80,
        rows: 24,
        cell_w: 0,
        cell_h: 0,
        scrollback: 1000,
        policy: client::Policy::Kill,
    }
}

/// A client `sh -c script` session.
pub fn client_shell(script: &str) -> client::Spawn {
    client_spawn("/bin/sh", &["-c", script])
}

/// The pid in a `pid=N.` line of `output`.
pub fn reported_pid(output: &[u8], label: &str) -> u32 {
    let text = String::from_utf8_lossy(output);
    text.split(&format!("{label}="))
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .and_then(|pid| pid.parse().ok())
        .unwrap_or_else(|| panic!("no {label}= in {text:?}"))
}
