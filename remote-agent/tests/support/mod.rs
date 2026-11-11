//! Shared pieces for the terminal-server tests: a temp server directory,
//! starting a server process with `--ready-fd`, a test core speaking H1, a
//! test client speaking protocol 1, and process checks.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
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
use brindle_term_core::alacritty_terminal::grid::Dimensions;
use brindle_term_core::alacritty_terminal::index::{Column, Line};
use brindle_term_core::alacritty_terminal::term::cell::Cell;
pub use brindle_term_core::emulator::Emulator;

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

/// For a forked child, before its exec: makes `fd` the child's fd `target`,
/// open in the program it runs. `dup2` does that when the numbers differ.
/// When they already match it does nothing, so `fd` would stay close-on-exec:
/// the flag is cleared instead.
///
/// Makes only async-signal-safe calls, as `pre_exec` needs.
pub fn pass_fd_as(fd: RawFd, target: RawFd) -> io::Result<()> {
    let result = unsafe {
        if fd == target {
            match libc::fcntl(fd, libc::F_GETFD) {
                -1 => -1,
                flags => libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC),
            }
        } else {
            libc::dup2(fd, target)
        }
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
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
        .env("BRINDLE_NO_SCOPE", "1")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || pass_fd_as(write_fd, 3));
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
        assert_eq!(
            sockets.len(),
            1,
            "holder sockets: {sockets:?}; logs:\n{}",
            dir.logs()
        );
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

    /// Hangs `session` up and waits for its `EXITED`, whatever the status.
    ///
    /// For a program blocked reading the terminal, the status isn't the
    /// holder's to decide. Closing the PTY makes the kernel fail that read
    /// (EIO) and then send `SIGHUP`. Nearly always the signal kills the
    /// program (no status), but now and then it gets through "the read
    /// failed, exit 1" first, as `cat` and `sh`'s `read` do. Either way the
    /// session ended, which is what a hang-up is for.
    pub fn hang_up_and_wait(&mut self, session: SessionId) {
        self.hang_up(session);
        self.exit_code(session);
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

/// How often `pid` has been switched off a CPU, `None` once it is gone. The
/// count stands still while a process stays blocked.
pub fn context_switches(pid: u32) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let count = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name)?.trim().parse::<u64>().ok())
    };
    Some(count("voluntary_ctxt_switches:")? + count("nonvoluntary_ctxt_switches:")?)
}

/// How long [`stays_blocked`] watches a writer do nothing.
const BLOCKED_FOR: Duration = Duration::from_millis(250);

/// Waits until `writer` is blocked for good on output that `reader` no
/// longer takes. Returns `false` if `writer` ends instead, or isn't blocked
/// within [`TIMEOUT`].
///
/// A writer faster than its reader sleeps too, but only until the reader
/// takes some more. So it counts as blocked once, for [`BLOCKED_FOR`], it
/// never ran and both were asleep at every look: nobody was about to move
/// the output on. How long the output took to get that far doesn't matter,
/// as it would with a fixed sleep.
pub fn stays_blocked(writer: u32, reader: u32) -> bool {
    let asleep = |pid| state(pid) == Some('S');
    let deadline = Instant::now() + TIMEOUT;
    // Since when it has been quiet, and the writer's switch count then.
    let mut quiet: Option<(Instant, u64)> = None;
    while alive(writer) && Instant::now() < deadline {
        let switches = context_switches(writer).filter(|_| asleep(writer) && asleep(reader));
        quiet = match (switches, quiet) {
            (Some(now), Some((since, before))) if now == before => {
                if since.elapsed() >= BLOCKED_FOR {
                    return true;
                }
                quiet
            }
            (Some(now), _) => Some((Instant::now(), now)),
            (None, _) => None,
        };
        thread::sleep(Duration::from_millis(20));
    }
    false
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

/// Runs `brindle-remote args…` (a management command, say) for `dir`.
pub fn cli(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .env("BRINDLE_SERVER_DIR", dir)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Holders start a real core of their own once theirs is gone.
pub const SUPERVISED: (&str, &str) = (
    "BRINDLE_TEST_HOLDER_CORE_ARGS",
    "serve --ready-fd {ready_fd}",
);

/// Waits until `pid` is gone.
pub fn gone(pid: u32) -> bool {
    eventually(TIMEOUT, || !alive(pid))
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

    /// Sends `ATTACH` and returns the replay's `REPLAY_START` and bytes, or
    /// the refusal. Output that arrives before the replay starts is
    /// dropped; messages for other sessions are kept for later.
    pub fn attach(
        &mut self,
        session: SessionId,
        cols: u16,
        rows: u16,
    ) -> Result<(client::ReplayStart, Vec<u8>), client::ErrorMessage> {
        let req = self.take_req();
        self.send(client::ToCore::Attach(client::Attach {
            req,
            session,
            cols,
            rows,
        }));
        let mut kept = Vec::new();
        let start = loop {
            match self.recv() {
                ToClient::ReplayStart(start) if start.req == Some(req) => break start,
                ToClient::Error(error) if error.req == Some(req) => {
                    kept.extend(self.backlog.drain(..));
                    self.backlog = kept.into();
                    return Err(error);
                }
                ToClient::Data(data) if data.session == session => {}
                other => kept.push(other),
            }
        };
        let bytes = self.replay_body(session, &mut kept);
        kept.extend(self.backlog.drain(..));
        self.backlog = kept.into();
        Ok((start, bytes))
    }

    /// The replay's bytes after its `REPLAY_START`, up to `REPLAY_END`.
    fn replay_body(&mut self, session: SessionId, kept: &mut Vec<ToClient>) -> Vec<u8> {
        let mut bytes = Vec::new();
        loop {
            match self.recv() {
                ToClient::Data(data) if data.session == session => bytes.extend(data.bytes),
                ToClient::ReplayEnd(end) if end.session == session => return bytes,
                other => kept.push(other),
            }
        }
    }

    /// The next replay without a request (a reconnect's), for any session:
    /// its `REPLAY_START` and bytes. Other messages are kept for later.
    pub fn reconnect_replay(&mut self) -> (client::ReplayStart, Vec<u8>) {
        let mut kept = Vec::new();
        let start = loop {
            match self.recv() {
                ToClient::ReplayStart(start) if start.req.is_none() => break start,
                other => kept.push(other),
            }
        };
        let bytes = self.replay_body(start.session, &mut kept);
        kept.extend(self.backlog.drain(..));
        self.backlog = kept.into();
        (start, bytes)
    }

    pub fn detach(&mut self, session: SessionId) {
        self.send(client::ToCore::Detach(client::SessionRef { session }));
    }

    /// Sends a request built with a fresh `req` and returns its reply.
    pub fn request(&mut self, message: impl FnOnce(RequestId) -> client::ToCore) -> ToClient {
        let req = self.take_req();
        self.send(message(req));
        self.reply(req)
    }

    /// `STEP_DOWN`, expecting `OK`.
    pub fn step_down(&mut self) {
        match self.request(|req| client::ToCore::StepDown(client::Request { req })) {
            ToClient::Ok(_) => {}
            other => panic!("expected OK, got {other:?}"),
        }
    }

    /// `LIST`'s sessions.
    pub fn list(&mut self) -> Vec<client::SessionInfo> {
        match self.request(|req| client::ToCore::List(client::Request { req })) {
            ToClient::Sessions(sessions) => sessions.sessions,
            other => panic!("expected SESSIONS, got {other:?}"),
        }
    }

    /// `STATUS`'s answer.
    pub fn status(&mut self) -> client::ServerStatus {
        match self.request(|req| client::ToCore::Status(client::Request { req })) {
            ToClient::ServerStatus(status) => status,
            other => panic!("expected SERVER_STATUS, got {other:?}"),
        }
    }

    /// Attaches to `session` (at 80×24) until the replay, fed to a fresh
    /// emulator, satisfies `done`; between tries it detaches again. For
    /// sessions whose output arrives while no client is attached.
    pub fn attach_until(
        &mut self,
        session: SessionId,
        scrollback: usize,
        mut done: impl FnMut(&Emulator) -> bool,
    ) -> (client::ReplayStart, Emulator) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let (start, bytes) = self
                .attach(session, 80, 24)
                .unwrap_or_else(|error| panic!("attach refused: {}", error.message));
            let screen = replayed(&bytes, 80, 24, scrollback);
            if done(&screen) {
                return (start, screen);
            }
            assert!(
                Instant::now() < deadline,
                "the replay never matched; last:\n{}",
                snapshot(&screen)
            );
            self.detach(session);
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Hands the connection to a `sleep` process (as its fd 3) and closes
    /// this side, so killing that process is a client dying.
    pub fn into_process(self) -> std::process::Child {
        let fd = self.stream.as_raw_fd();
        let mut command = Command::new("sleep");
        command.arg("1000").stdin(Stdio::null());
        unsafe {
            command.pre_exec(move || pass_fd_as(fd, 3));
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
        profile: None,
    }
}

/// A client `sh -c script` session.
pub fn client_shell(script: &str) -> client::Spawn {
    client_spawn("/bin/sh", &["-c", script])
}

/// A fresh emulator of `cols` × `rows`, fed `bytes` (a replay, say).
pub fn replayed(bytes: &[u8], cols: u16, rows: u16, scrollback: usize) -> Emulator {
    let mut screen = Emulator::new(cols, rows, scrollback);
    screen.feed(bytes);
    screen
}

/// Every row's text, scrollback first, without trailing blanks.
pub fn rows(screen: &Emulator) -> Vec<String> {
    let grid = screen.term().grid();
    (grid.topmost_line().0..=grid.bottommost_line().0)
        .map(|line| {
            let row = &grid[Line(line)];
            let text: String = (0..grid.columns()).map(|col| row[Column(col)].c).collect();
            text.trim_end().to_string()
        })
        .collect()
}

/// Everything a replay must reproduce, as text: every row with each
/// cell's character and attributes, the cursor, the modes, the title, the
/// status records and the prompt marks.
pub fn snapshot(screen: &Emulator) -> String {
    use std::fmt::Write as _;
    let term = screen.term();
    let grid = term.grid();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "size {}x{} history {}",
        grid.columns(),
        grid.screen_lines(),
        grid.history_size()
    );
    for line in grid.topmost_line().0..=grid.bottommost_line().0 {
        let row = &grid[Line(line)];
        let _ = write!(out, "{line:5}|");
        for col in 0..grid.columns() {
            let cell = &row[Column(col)];
            let _ = write!(out, "{}", cell.c);
            if (cell.fg, cell.bg) != (Cell::default().fg, Cell::default().bg)
                || !cell.flags.is_empty()
            {
                let _ = write!(out, "[{:?} {:?} {:?}]", cell.fg, cell.bg, cell.flags);
            }
        }
        out.push('\n');
    }
    let cursor = &grid.cursor;
    let _ = writeln!(
        out,
        "cursor {:?} wrap {} style {:?}",
        cursor.point,
        cursor.input_needs_wrap,
        term.cursor_style()
    );
    let _ = writeln!(out, "mode {:?}", term.mode());
    let _ = writeln!(out, "title {:?}", screen.title());
    for (id, record) in screen.records().iter() {
        let _ = writeln!(out, "record {id:?} {}", record.encode(id));
    }
    let marks = screen.marks();
    for command in marks.commands() {
        let _ = writeln!(
            out,
            "command prompt {} input {:?} output {:?} end {:?} exit {:?}",
            marks.line(command.prompt),
            command.input.map(|pos| (marks.line(pos), pos.col)),
            command.output.map(|pos| (marks.line(pos), pos.col)),
            command.end.map(|pos| (marks.line(pos), pos.col)),
            command.exit
        );
    }
    out
}

/// Writes `bytes` to a file in `dir` and returns a `detach` session spawn
/// that prints the file unchanged (raw mode: no `\n` → `\r\n`), then waits.
pub fn fixture_spawn(dir: &TempDir, name: &str, bytes: &[u8], scrollback: u32) -> client::Spawn {
    let path = dir.path.join(name);
    fs::write(&path, bytes).unwrap();
    let mut spawn = client_shell(&format!(
        "stty raw -echo; cat '{}'; exec sleep 1000",
        path.display()
    ));
    spawn.scrollback = scrollback;
    spawn.policy = client::Policy::Detach;
    spawn
}

/// An `htop`-like full-screen program's output: a line on the primary
/// screen, then the alternate screen with mouse modes, colours, a hidden
/// cursor and a title.
pub fn full_screen_fixture(frame: u32) -> Vec<u8> {
    let mut out = b"before the program\r\n\x1b]2;htop-like\x07".to_vec();
    out.extend_from_slice(b"\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?1h\x1b=\x1b[?25l");
    out.extend_from_slice(b"\x1b[H\x1b[2J\x1b[1;37;44m  PID USER      CPU%  MEM%  COMMAND\x1b[K\x1b[0m");
    for row in 0..20u32 {
        let pid = 1000 + row * 7 + frame;
        let cpu = (row * 13 + frame) % 100;
        out.extend_from_slice(
            format!(
                "\x1b[{};1H\x1b[38;5;{}m{pid:5}\x1b[0m user     \x1b[48;2;{};40;60m{cpu:4}.0\x1b[0m  \x1b[1m{:4}.{}\x1b[0m  proc-{row}",
                row + 2,
                (row * 11) % 256,
                (row * 12) % 256,
                row % 10,
                frame % 10,
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(
        format!("\x1b[24;1H\x1b[30;46mF1\x1b[0mHelp  \x1b[30;46mF10\x1b[0mQuit frame {frame}\x1b[12;30H")
            .as_bytes(),
    );
    out
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
