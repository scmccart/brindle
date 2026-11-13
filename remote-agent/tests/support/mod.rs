//! Shared pieces for the terminal-server tests: a temp server directory,
//! starting a server process with its ready pipe, a test core speaking H1, a
//! test client speaking protocol 1, and process checks. What differs between
//! the platforms (connections, starting, processes) is in `unix` and
//! `windows`, with the same items.

#![allow(dead_code)]

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use brindle_remote_protocol::client::{self, ToClient};
use brindle_remote_protocol::endpoint::Endpoints;
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
/// paths fit). Dropping it kills the servers it saw and removes it.
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
        private_dir(&path);
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

    /// Where the holders the directory shows listen
    /// ([`Endpoints::holders`]): sockets on Linux, pipe names on Windows.
    pub fn holder_endpoints(&self) -> Vec<PathBuf> {
        let endpoints = Endpoints::of(&self.path).unwrap();
        let holders = endpoints.holders().unwrap();
        holders.into_iter().map(|holder| holder.endpoint).collect()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let mut pids = self.pids.clone();
        pids.extend(server_processes(&self.path, "serve"));
        pids.extend(server_processes(&self.path, "hold"));
        for pid in pids {
            kill_server(pid);
        }
        let _ = fs::remove_dir_all(&self.path);
    }
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

/// A core for tests: H1 over a holder's endpoint, read on a thread.
pub struct TestCore {
    stream: Stream,
    reader: Option<JoinHandle<()>>,
    incoming: Receiver<io::Result<ToCore>>,
    backlog: VecDeque<ToCore>,
    closed: bool,
}

impl TestCore {
    pub fn connect(endpoint: &Path) -> Self {
        let stream = connect(endpoint)
            .unwrap_or_else(|error| panic!("connect {}: {error}", endpoint.display()));
        let mut reader = stream.try_clone().unwrap();
        let (sender, incoming) = mpsc::channel();
        let reader = thread::spawn(move || {
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
            reader: Some(reader),
            incoming,
            backlog: VecDeque::new(),
            closed: false,
        }
    }

    /// Connects to the directory's only holder.
    pub fn connect_to(dir: &TempDir) -> Self {
        let endpoints = dir.holder_endpoints();
        assert_eq!(endpoints.len(), 1, "holder endpoints: {endpoints:?}");
        Self::connect(&endpoints[0])
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

    /// Whether the holder sends nothing for `duration`. What it does send
    /// is kept for later.
    pub fn quiet_for(&mut self, duration: Duration) -> bool {
        if !self.backlog.is_empty() {
            return false;
        }
        match self.incoming.recv_timeout(duration) {
            Ok(Ok(message)) => {
                self.backlog.push_back(message);
                false
            }
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
                self.closed = true;
                false
            }
            Err(RecvTimeoutError::Timeout) => true,
        }
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
    /// reader thread's clone would otherwise keep it open. The thread ends
    /// with the connection and is waited for, so the holder is told of the
    /// close before the test goes on.
    fn drop(&mut self) {
        close(&self.stream);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
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

#[cfg(unix)]
pub fn base_env() -> BTreeMap<String, String> {
    BTreeMap::from([(
        "PATH".to_string(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
    )])
}

/// What Windows programs need to start, and a `PATH` short enough for one
/// line of a terminal: the system's own programs.
#[cfg(windows)]
pub fn base_env() -> BTreeMap<String, String> {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    BTreeMap::from([
        ("PATH".to_string(), format!(r"{root}\System32")),
        ("SystemRoot".to_string(), root),
    ])
}

/// A `sh -c script` session.
#[cfg(unix)]
pub fn shell(session: SessionId, script: &str) -> Spawn {
    spawn(session, "/bin/sh", &["-c", script])
}

/// A `cmd /d /v:on /c script` session: no AutoRun commands, and `!name!`
/// reads a variable when its command runs.
#[cfg(windows)]
pub fn shell(session: SessionId, script: &str) -> Spawn {
    spawn(session, "cmd.exe", &["/d", "/v:on", "/c", script])
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

/// Where the core of `dir` listens: a socket on Linux, a pipe name on
/// Windows.
pub fn core_endpoint(dir: &Path) -> PathBuf {
    Endpoints::of(dir).unwrap().core()
}

/// Whether a core accepts connections for `dir`.
pub fn core_listens(dir: &Path) -> bool {
    connect(&core_endpoint(dir)).is_ok()
}

/// Whether nothing is left of the core's endpoint for `dir`: no socket
/// file on Linux, no pipe of that name on Windows.
pub fn core_endpoint_gone(dir: &Path) -> bool {
    let endpoint = core_endpoint(dir);
    #[cfg(unix)]
    return !endpoint.exists();
    #[cfg(windows)]
    return connect(&endpoint).is_err_and(|error| error.kind() == io::ErrorKind::NotFound);
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
pub const SUPERVISED: (&str, &str) = ("BRINDLE_TEST_HOLDER_CORE_ARGS", SERVE_ARGS);

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

/// A client for tests: protocol 1 over the core's endpoint, read on the
/// calling thread (so a test can stop reading, or hand the connection to
/// another process).
pub struct TestClient {
    stream: Stream,
    input: Vec<u8>,
    backlog: VecDeque<ToClient>,
    closed: bool,
    next_req: RequestId,
    pub welcome: Option<client::Welcome>,
}

impl TestClient {
    pub fn connect(dir: &Path) -> Self {
        let endpoint = core_endpoint(dir);
        let stream = connect(&endpoint)
            .unwrap_or_else(|error| panic!("connect {}: {error}", endpoint.display()));
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

    /// Collects `session`'s output until it contains `needle` and then
    /// until none came for `quiet`: everything it printed, when the program
    /// waits after `needle`. Other messages are kept for later.
    pub fn output_until_quiet(
        &mut self,
        session: SessionId,
        needle: &[u8],
        quiet: Duration,
    ) -> Vec<u8> {
        let mut output = self.output_until(session, needle);
        let mut kept = Vec::new();
        loop {
            match self.next(quiet) {
                Recv::Message(ToClient::Data(client::Data { session: s, bytes }))
                    if s == session =>
                {
                    output.extend(bytes)
                }
                Recv::Message(other) => kept.push(other),
                Recv::TimedOut => break,
                Recv::Closed => panic!("the core closed the connection"),
            }
        }
        kept.extend(self.backlog.drain(..));
        self.backlog = kept.into();
        output
    }

    /// Hands the connection to another process (a `sleep` on Linux, as its
    /// fd 3; a `ping` on Windows, as an inherited handle) and closes this
    /// side, so killing that process is a client dying.
    pub fn into_process(self) -> std::process::Child {
        let child = hand_to_process(&self.stream);
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
#[cfg(unix)]
pub fn client_shell(script: &str) -> client::Spawn {
    client_spawn("/bin/sh", &["-c", script])
}

/// A client `cmd /d /v:on /c script` session (see [`shell`]).
#[cfg(windows)]
pub fn client_shell(script: &str) -> client::Spawn {
    client_spawn("cmd.exe", &["/d", "/v:on", "/c", script])
}

/// This process's environment with `extra` over it, for programs that need
/// more than the system's own (PowerShell).
#[cfg(windows)]
pub fn inherited_env(extra: &[(&str, &str)]) -> BTreeMap<String, String> {
    let set = |name: &str| extra.iter().any(|(set, _)| set.eq_ignore_ascii_case(name));
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, _)| !set(name))
        .chain(extra.iter().map(|(name, value)| (name.to_string(), value.to_string())))
        .collect()
}

/// Line `n` of a file of numbered lines, without its line end.
fn numbered_line(n: usize) -> String {
    format!("#{n:07}# {}", "x".repeat(60))
}

/// Writes a file of `lines` numbered lines into `dir`: output that is
/// recognised in what a pseudoconsole makes of it. `cmd`'s `type` takes
/// its path unquoted, so it has no space.
pub fn numbered_lines(dir: &Path, lines: usize) -> PathBuf {
    let path = dir.join(format!("lines-{lines}.txt"));
    assert!(!path.to_str().unwrap().contains(' '), "{path:?}");
    let text: String = (0..lines).map(|n| numbered_line(n) + "\r\n").collect();
    fs::write(&path, text).unwrap();
    path
}

/// Fails unless `output` has every one of `lines` numbered lines, in
/// order.
pub fn check_lines(output: &[u8], lines: usize) {
    let output = String::from_utf8_lossy(output);
    let mut from = 0;
    for n in 0..lines {
        let line = numbered_line(n);
        match output[from..].find(&line) {
            Some(at) => from += at + line.len(),
            None => panic!("line {n} is missing after byte {from} of {}", output.len()),
        }
    }
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
#[cfg(unix)]
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
