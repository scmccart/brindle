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

pub mod fake_agent;
pub mod programs;
#[cfg(unix)]
pub mod relay;
mod wait;
pub use wait::*;

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
use brindle_remote_protocol::relay::{MSG_RELAY_HEARTBEAT, is_relay_type};
use brindle_remote_protocol::endpoint::Endpoints;
use brindle_remote_protocol::holder::{
    self as h1, Assembler, Data, Hello, SessionRef, Spawn, SpawnMode, SpawnPipes, ToCore,
    ToHolder, Welcome,
};
use brindle_remote_protocol::{Frame, RequestId, SessionId};
use brindle_term_core::alacritty_terminal::grid::Dimensions;
use brindle_term_core::alacritty_terminal::index::{Column, Line};
use brindle_term_core::alacritty_terminal::term::cell::Cell;
pub use brindle_term_core::emulator::Emulator;
use brindle_term_core::shell_integration::PathStyle;
pub use brindle_transcript::{Event, Transcript};

pub const BIN: &str = env!("CARGO_BIN_EXE_brindle-remote");
/// The fake ACP agent (`src/fake_acp/main.rs`), which cargo builds with the
/// helper for these tests; [`fake_agent::AgentScript`] writes what it plays.
pub const FAKE_AGENT: &str = env!("CARGO_BIN_EXE_brindle-fake-acp-agent");
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
    start_holder_with(dir, &[]);
}

/// [`start_holder`], with `env` set for the holder too (a fault switch,
/// say).
pub fn start_holder_with(dir: &Path, env: &[(&str, &str)]) {
    let mut env = env.to_vec();
    env.push(("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version"));
    start(Path::new(BIN), "hold", dir, &env).unwrap();
}

/// What a refused peer sends after its opening: 64 KiB past what a server
/// reads from a connection in one turn (1 MiB, `conn.rs`'s `READ_BUDGET`),
/// so some of it is still unread when the server refuses, whenever the
/// server wakes.
const UNREAD: usize = 1024 * 1024 + 64 * 1024;

/// Connects to `endpoint`, sends `opening`, which the server refuses, then
/// [`UNREAD`] bytes it never handles, and returns the frame it answers with,
/// checking that the end of the connection follows: a refused peer reads
/// the server's `ERROR`, then the end, not a reset (`process R10`).
///
/// The bytes go out on a thread of their own, while this one reads. Where
/// the server can read them to the end ([`REFUSED_WRITES_ARRIVE`]), their
/// write must succeed too: a server that closed after reading at most a
/// turn's worth fails it.
pub fn refused(endpoint: &Path, opening: &Frame) -> Frame {
    use std::io::Write as _;

    let mut stream = connect(endpoint).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    opening.write_to(&mut stream).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let rest = thread::spawn(move || writer.write_all(&vec![0u8; UNREAD]));
    let answer = match Frame::read_from_or_eof(&mut stream) {
        Ok(Some(frame)) => frame,
        other => panic!("expected the server's answer, got {other:?}"),
    };
    let end = Frame::read_from_or_eof(&mut stream);
    let written = rest.join().unwrap();
    match end {
        Ok(None) => {}
        other => panic!("expected the end after the answer, got {other:?}"),
    }
    if REFUSED_WRITES_ARRIVE {
        written.expect("the server stopped reading before the end of what was sent");
    }
    answer
}

/// A blob per session, as WELCOME's followers deliver them.
#[derive(Debug, Default)]
pub struct Adopted {
    pub welcome: Option<Welcome>,
    pub checkpoints: HashMap<SessionId, Vec<u8>>,
    pub tails: HashMap<SessionId, Vec<u8>>,
    /// The `STDERR_TAIL` of each pipe session that ended with no core, each
    /// right after that session's `TAIL_END`.
    pub stderr_tails: HashMap<SessionId, Vec<u8>>,
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
        assert_eq!(
            endpoints.len(),
            1,
            "holder endpoints: {endpoints:?}; logs:\n{}",
            dir.logs()
        );
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
        self.hello_up_to(instance, h1::PROTOCOLS.max)
    }

    /// [`TestCore::hello`] from a core that speaks H1 up to `max`: one
    /// below 3 stands for an older core, which isn't shown pipe sessions.
    pub fn hello_up_to(&mut self, instance: u64, max: u32) -> Adopted {
        self.send(ToHolder::Hello(Hello {
            min: h1::PROTOCOLS.min,
            max,
            core_build: "test-core".into(),
            instance,
        }));
        let ToCore::Welcome(welcome) = self.recv() else {
            panic!("the first message wasn't WELCOME");
        };
        let mut adopted = Adopted::default();
        let mut checkpoints = Assembler::new(h1::MAX_CHECKPOINT_LEN);
        let mut tails = Assembler::new(h1::TAIL_LIMIT);
        // A pipe session that ended has its stderr tail after its tail.
        let ended_on_pipes = |id: SessionId| {
            welcome.sessions.iter().any(|held| {
                held.session == id && held.mode == SpawnMode::Pipes && held.exit.is_some()
            })
        };
        let mut left = welcome.sessions.len()
            + welcome
                .sessions
                .iter()
                .filter(|held| ended_on_pipes(held.session))
                .count();
        let mut last_tail = None;
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
                    last_tail = Some(end.session);
                    adopted.tails.insert(end.session, tails.end(end).unwrap());
                    left -= 1;
                }
                ToCore::StderrTail(Data { session, bytes }) => {
                    assert_eq!(
                        last_tail,
                        Some(session),
                        "session {session}'s STDERR_TAIL doesn't follow its TAIL_END"
                    );
                    assert!(ended_on_pipes(session), "STDERR_TAIL for {session}");
                    adopted.stderr_tails.insert(session, bytes);
                    left -= 1;
                }
                other => panic!("unexpected {other:?} while adopting"),
            }
        }
        adopted.welcome = Some(welcome);
        adopted
    }

    /// Sends a `SPAWN` and returns the program's pid. Output that arrives
    /// meanwhile is dropped, other sessions' too: a test that waits for an
    /// earlier session's output does so before the next spawn.
    pub fn spawn(&mut self, spawn: Spawn) -> u32 {
        let session = spawn.session;
        self.send(ToHolder::Spawn(spawn));
        match self.recv_skipping_output() {
            ToCore::Spawned(spawned) if spawned.session == session => spawned.pid,
            other => panic!("expected SPAWNED for {session}, got {other:?}"),
        }
    }

    /// Sends a `SPAWN_PIPES` and returns the program's pid, dropping output
    /// as [`TestCore::spawn`] does.
    pub fn spawn_pipes(&mut self, spawn: SpawnPipes) -> u32 {
        let session = spawn.session;
        self.send(ToHolder::SpawnPipes(spawn));
        match self.recv_skipping_output() {
            ToCore::Spawned(spawned) if spawned.session == session => spawned.pid,
            other => panic!("expected SPAWNED for {session}, got {other:?}"),
        }
    }

    /// A pipe session's output up to its end, its `STDERR_TAIL` (which
    /// must come once, just before its `EXITED`) and its exit code. Other
    /// sessions' messages are skipped.
    pub fn pipes_end(&mut self, session: SessionId) -> (Vec<u8>, Vec<u8>, Option<i32>) {
        let mut output = Vec::new();
        let mut stderr = None;
        loop {
            match self.recv() {
                ToCore::Output(Data { session: s, bytes }) if s == session => {
                    assert!(stderr.is_none(), "output after STDERR_TAIL");
                    output.extend(bytes);
                }
                ToCore::StderrTail(Data { session: s, bytes }) if s == session => {
                    assert!(stderr.is_none(), "a second STDERR_TAIL");
                    stderr = Some(bytes);
                }
                ToCore::Exited(exited) if exited.session == session => {
                    let stderr = stderr.expect("no STDERR_TAIL before EXITED");
                    return (output, stderr, exited.code);
                }
                ToCore::Output(_) | ToCore::StderrTail(_) | ToCore::Exited(_) | ToCore::Error(_) => {
                    assert!(stderr.is_none(), "messages between STDERR_TAIL and EXITED");
                }
                other => panic!("expected EXITED for {session}, got {other:?}"),
            }
        }
    }

    /// Every message the holder sends until it closes the connection.
    pub fn until_closed(&mut self) -> Vec<ToCore> {
        let mut messages = Vec::new();
        let deadline = Instant::now() + TIMEOUT;
        while let Some(message) = self.try_recv(deadline.saturating_duration_since(Instant::now())) {
            messages.push(message);
        }
        messages
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

    /// Hangs `session` up and waits for its `EXITED`, checking that the
    /// hang-up ended it ([`assert_hung_up`]).
    pub fn hang_up_and_wait(&mut self, session: SessionId) {
        self.hang_up(session);
        self.hung_up(session);
    }

    /// Waits for the `EXITED` of a session that was hung up, checking that
    /// the hang-up ended it ([`assert_hung_up`]).
    pub fn hung_up(&mut self, session: SessionId) {
        let code = self.exit_code(session);
        assert_hung_up(session, code);
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

/// Checks the exit `code` of a session that was hung up. On Linux the
/// holder signals before it closes the terminal, so `SIGHUP` ends the
/// program, which then has no code. On Windows it has the code the
/// console's end or the holder ended it with.
#[track_caller]
pub fn assert_hung_up(session: SessionId, code: Option<i32>) {
    #[cfg(unix)]
    assert_eq!(code, None, "session {session} wasn't ended by a signal");
    #[cfg(windows)]
    assert!(code.is_some(), "session {session} ended with no code");
}

/// The records of an agent checkpoint of format 2 (`BACP`, `u16` 2, then
/// each record's kind, sequence number, JSON state and partial line; see
/// `acp/checkpoint.rs`), as `(kind, seq)`: what a holder stores of a base
/// and the segments appended to it, read without decoding the states.
#[track_caller]
pub fn checkpoint_records(blob: &[u8]) -> Vec<(char, u32)> {
    let (header, mut rest) = blob.split_at(6);
    assert_eq!(header, b"BACP\x00\x02", "not a format 2 checkpoint");
    let take = |len: usize, rest: &mut &[u8]| -> Vec<u8> {
        assert!(rest.len() >= len, "a record runs past the end");
        let (taken, left) = rest.split_at(len);
        *rest = left;
        taken.to_vec()
    };
    let mut records = Vec::new();
    while !rest.is_empty() {
        let kind = take(1, &mut rest)[0] as char;
        let seq = u32::from_be_bytes(take(4, &mut rest).try_into().unwrap());
        for _ in 0..2 {
            let len = u32::from_be_bytes(take(4, &mut rest).try_into().unwrap());
            take(len as usize, &mut rest);
        }
        records.push((kind, seq));
    }
    records
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

/// A `SPAWN_PIPES` of `command args` with [`base_env`].
pub fn pipe_spawn(session: SessionId, command: &str, args: &[&str]) -> SpawnPipes {
    SpawnPipes {
        session,
        command: command.into(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        cwd: None,
        env: base_env(),
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

/// A decision the core logs about a session (`server/core/sessions.rs`).
/// Its line is the positive marker a "nothing happens" check waits for: once
/// it is in the log, the core has taken that decision, and the test asserts
/// the state after it rather than waiting out a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// A `detach` session lost its client and runs on.
    Detached,
    /// A `kill` session lost a client that asked for this many seconds'
    /// linger, and waits for it.
    Lingers(u32),
    /// An adopted `kill` session's owner didn't come back within the grace
    /// period, so it is hung up.
    OwnerGone,
    /// A lingering session's client didn't come back in time, so it is hung
    /// up.
    LingerOver,
}

impl Decision {
    /// The core's message for `session`, after its `core[pid]: ` prefix.
    fn message(self, session: SessionId) -> String {
        match self {
            Decision::Detached => format!("session {session}: detached"),
            Decision::Lingers(secs) => {
                format!("session {session}: its client left; it waits {secs}s for it")
            }
            Decision::OwnerGone => {
                format!("session {session}: its owner didn't come back; hanging up")
            }
            Decision::LingerOver => {
                format!("session {session}: its client didn't come back in time; hanging up")
            }
        }
    }
}

/// The messages of every core that logged in `dir`, in the order they were
/// written: each `core.log` line's text after its `<time> core[<pid>]: `
/// prefix. Lines of other roles' (a check's, say) are left out.
pub fn core_messages(dir: &TempDir) -> Vec<String> {
    let log = dir
        .path
        .join(brindle_remote_protocol::server_dir::CORE_LOG);
    let text = fs::read_to_string(log).unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let (_, rest) = line.split_once(" core[")?;
            let (_, message) = rest.split_once("]: ")?;
            Some(message.to_string())
        })
        .collect()
}

/// Waits until the core has logged `decision` for `session`, and returns
/// where the line is among [`core_messages`], for checks of their order.
#[track_caller]
pub fn core_logged(dir: &TempDir, session: SessionId, decision: Decision) -> usize {
    let wanted = decision.message(session);
    let found = eventually(TIMEOUT, || {
        core_messages(dir).contains(&wanted)
    });
    let messages = core_messages(dir);
    match messages.iter().position(|message| *message == wanted) {
        Some(at) if found => at,
        _ => panic!("the core never logged {wanted:?}; its log:\n{}", messages.join("\n")),
    }
}

/// Waits until the core has logged at least `count` clients' departures
/// (`client N left (…) with K session(s)`): each line is written in the same
/// handler as the close policies of that client's sessions, so once it is
/// there they have been applied.
#[track_caller]
pub fn left(dir: &TempDir, count: usize) {
    let departures = || {
        core_messages(dir)
            .iter()
            .filter(|message| message.starts_with("client ") && message.contains(" left ("))
            .count()
    };
    assert!(
        eventually(TIMEOUT, || departures() >= count),
        "the core logged {} of {count} clients' departures; its log:\n{}",
        departures(),
        core_messages(dir).join("\n")
    );
}

/// What [`TestClient::next`] got.
#[derive(Debug)]
pub enum Recv {
    Message(ToClient),
    Closed,
    TimedOut,
}

/// A client for tests: protocol 1 over the core's endpoint (or a relay's
/// stdio), read on the calling thread (so a test can stop reading, or hand
/// the connection to another process).
pub struct TestClient {
    stream: Stream,
    input: Vec<u8>,
    backlog: VecDeque<ToClient>,
    closed: bool,
    next_req: RequestId,
    pub welcome: Option<client::Welcome>,
    /// How many `RELAY_HEARTBEAT`s arrived (from a relay; a core sends
    /// none), which [`TestClient::next`] skips.
    pub heartbeats: usize,
    /// Each agent session's log as this client has it: every `TRANSCRIPT`
    /// is folded in as it is read ([`TestClient::next`]), and a
    /// `REPLAY_START` starts the session's log again.
    transcripts: HashMap<SessionId, Transcript>,
    /// The events folded into each session's log, in order, since its
    /// last `REPLAY_START` (for checks of what came in which order).
    events: HashMap<SessionId, Vec<Event>>,
}

impl TestClient {
    pub fn connect(dir: &Path) -> Self {
        let endpoint = core_endpoint(dir);
        let stream = connect(&endpoint)
            .unwrap_or_else(|error| panic!("connect {}: {error}", endpoint.display()));
        Self::over(stream)
    }

    /// A client on `stream`, before its `HELLO`.
    pub fn over(stream: Stream) -> Self {
        Self {
            stream,
            input: Vec::new(),
            backlog: VecDeque::new(),
            closed: false,
            next_req: 1,
            welcome: None,
            heartbeats: 0,
            transcripts: HashMap::new(),
            events: HashMap::new(),
        }
    }

    /// Connects and says `HELLO` with `token`, expecting `WELCOME`.
    pub fn hello(dir: &Path, token: &str) -> Self {
        Self::hello_with(dir, token, |_| {})
    }

    /// Connects and says `HELLO` with `token`, as `edit` changes it (a
    /// linger, process info, another protocol range), expecting `WELCOME`.
    pub fn hello_with(dir: &Path, token: &str, edit: impl FnOnce(&mut client::Hello)) -> Self {
        Self::connect(dir).greet(token, edit)
    }

    /// Says `HELLO` with `token`, as `edit` changes it, expecting `WELCOME`.
    pub fn greet(mut self, token: &str, edit: impl FnOnce(&mut client::Hello)) -> Self {
        let mut hello = client::Hello {
            min: client::PROTOCOLS.min,
            max: client::PROTOCOLS.max,
            client_version: "test".into(),
            client_token: token.into(),
            ..Default::default()
        };
        edit(&mut hello);
        self.send(client::ToCore::Hello(hello));
        match self.recv() {
            ToClient::Welcome(welcome) => self.welcome = Some(welcome),
            other => panic!("expected WELCOME, got {other:?}"),
        }
        self
    }

    /// The core's pid, from `WELCOME`.
    pub fn core_pid(&self) -> u32 {
        self.welcome.as_ref().expect("WELCOME").pid
    }

    pub fn send(&mut self, message: client::ToCore) {
        self.send_frame(&message.encode().unwrap());
    }

    /// Sends any frame (a relay's heartbeat, say).
    pub fn send_frame(&mut self, frame: &Frame) {
        frame.write_to(&mut self.stream).unwrap();
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
                if frame.msg_type == MSG_RELAY_HEARTBEAT {
                    self.heartbeats += 1;
                    continue;
                }
                assert!(
                    !is_relay_type(frame.msg_type),
                    "a relay frame of type {:#x} amid the protocol",
                    frame.msg_type
                );
                let message = ToClient::decode(&frame).unwrap();
                self.fold(&message);
                return Recv::Message(message);
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

    /// Keeps the agent sessions' logs: a replay starts one again, and each
    /// `TRANSCRIPT` event is decoded ([`Event::from_value`]) and folded.
    fn fold(&mut self, message: &ToClient) {
        match message {
            ToClient::ReplayStart(start) => {
                self.transcripts.remove(&start.session);
                self.events.remove(&start.session);
            }
            ToClient::Transcript(transcript) => {
                let log = self.transcripts.entry(transcript.session).or_default();
                let events = self.events.entry(transcript.session).or_default();
                for value in &transcript.events {
                    let event = Event::from_value(value)
                        .unwrap_or_else(|error| panic!("TRANSCRIPT event {value}: {error}"));
                    // Protocol 4's forms only: nothing the decode dropped.
                    assert_eq!(event.to_value(), *value, "TRANSCRIPT event {value} isn't one of protocol 4's forms");
                    log.apply(&event).unwrap_or_else(|error| {
                        panic!("session {}: {event:?} doesn't fold: {error}", transcript.session)
                    });
                    events.push(event);
                }
            }
            _ => {}
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

    /// Waits for the `EXIT` of a session that was hung up (`CLOSE`, `KILL`
    /// or `STOP`), checking that the hang-up ended it ([`assert_hung_up`]).
    pub fn hung_up(&mut self, session: SessionId) {
        let code = self.exit_code(session);
        assert_hung_up(session, code);
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
            page: None,
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
        self.detach_with(session, None);
    }

    /// `SET_LAYOUT` (protocol 3) for a session this client has attached.
    pub fn set_layout(&mut self, session: SessionId, layout: Option<brindle_remote_protocol::serde_json::Value>) {
        self.send(client::ToCore::SetLayout(client::SetLayout { session, layout }));
    }

    /// `DETACH` with a layout for the core to keep.
    pub fn detach_with(&mut self, session: SessionId, layout: Option<brindle_remote_protocol::serde_json::Value>) {
        self.send(client::ToCore::Detach(client::Detach { session, layout }));
    }

    /// The next `PROCESS` for `session` within `timeout`, if one comes.
    /// Other messages are kept for later.
    pub fn process_within(
        &mut self,
        session: SessionId,
        timeout: Duration,
    ) -> Option<client::Process> {
        let deadline = Instant::now() + timeout;
        let mut kept: Vec<ToClient> = self.backlog.drain(..).collect();
        // One already received comes first.
        let found = match kept
            .iter()
            .position(|message| matches!(message, ToClient::Process(p) if p.session == session))
        {
            Some(at) => match kept.remove(at) {
                ToClient::Process(process) => Some(process),
                _ => unreachable!(),
            },
            None => loop {
                match self.next(deadline.saturating_duration_since(Instant::now())) {
                    Recv::Message(ToClient::Process(process)) if process.session == session => {
                        break Some(process);
                    }
                    Recv::Message(other) => kept.push(other),
                    Recv::TimedOut => break None,
                    Recv::Closed => panic!("the core closed the connection"),
                }
            },
        };
        kept.extend(self.backlog.drain(..));
        self.backlog = kept.into();
        found
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

    // Agent sessions (client protocol 4).

    /// Sends a `SPAWN_AGENT` and returns the reply (`SPAWNED` or `ERROR`).
    /// Other messages are kept for later.
    pub fn spawn_agent_reply(&mut self, mut spawn: client::SpawnAgent) -> ToClient {
        spawn.req = self.take_req();
        let req = spawn.req;
        self.send(client::ToCore::SpawnAgent(spawn));
        self.reply(req)
    }

    /// Starts an agent session and returns its id and the agent's pid.
    pub fn spawn_agent(&mut self, spawn: client::SpawnAgent) -> (SessionId, u32) {
        match self.spawn_agent_reply(spawn) {
            ToClient::Spawned(spawned) => (spawned.session, spawned.pid),
            other => panic!("expected SPAWNED, got {other:?}"),
        }
    }

    /// The reply to agent request `req`: `Ok` for `OK`, the `ERROR`
    /// otherwise.
    fn agent_reply(&mut self, req: RequestId) -> Result<(), client::ErrorMessage> {
        match self.reply(req) {
            ToClient::Ok(_) => Ok(()),
            ToClient::Error(error) => Err(error),
            other => panic!("expected OK or ERROR, got {other:?}"),
        }
    }

    /// `AGENT_PROMPT`: `text` for the agent, sent now or queued.
    pub fn prompt(&mut self, session: SessionId, text: &str) -> Result<(), client::ErrorMessage> {
        let req = self.take_req();
        self.send(client::ToCore::AgentPrompt(client::AgentPrompt {
            req,
            session,
            text: text.into(),
        }));
        self.agent_reply(req)
    }

    /// `AGENT_UNQUEUE`: the prompt in the slot is discarded.
    pub fn unqueue(&mut self, session: SessionId) -> Result<(), client::ErrorMessage> {
        let req = self.take_req();
        self.send(client::ToCore::AgentUnqueue(client::AgentRequest { req, session }));
        self.agent_reply(req)
    }

    /// `AGENT_RESEND`: a held prompt is sent again.
    pub fn resend(&mut self, session: SessionId) -> Result<(), client::ErrorMessage> {
        let req = self.take_req();
        self.send(client::ToCore::AgentResend(client::AgentRequest { req, session }));
        self.agent_reply(req)
    }

    /// `AGENT_RESTART` (change `agents-session-recovery`): the ended agent
    /// is started again as the fake ACP agent playing `script`.
    pub fn restart(&mut self, session: SessionId, script: &Path) -> Result<(), client::ErrorMessage> {
        self.restart_with(session, FAKE_AGENT, &[&script.display().to_string()])
    }

    /// `AGENT_RESTART` of `command args`, with a minimal environment.
    pub fn restart_with(
        &mut self,
        session: SessionId,
        command: &str,
        args: &[&str],
    ) -> Result<(), client::ErrorMessage> {
        self.restart_in(session, command, args, base_env())
    }

    /// `AGENT_RESTART` of `command args` with exactly `env`.
    pub fn restart_in(
        &mut self,
        session: SessionId,
        command: &str,
        args: &[&str],
        env: BTreeMap<String, String>,
    ) -> Result<(), client::ErrorMessage> {
        let req = self.take_req();
        self.send(client::ToCore::AgentRestart(client::AgentRestart {
            req,
            session,
            command: command.into(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            env,
        }));
        self.agent_reply(req)
    }

    /// `AGENT_LOGIN` (change `agents-login`): the agent logs in with its
    /// login method `method`.
    pub fn login(&mut self, session: SessionId, method: &str) -> Result<(), client::ErrorMessage> {
        let req = self.take_req();
        self.send(client::ToCore::AgentLogin(client::AgentLogin { req, session, method: method.into() }));
        self.agent_reply(req)
    }

    /// `AGENT_CANCEL`, which has no reply (an `ERROR` without a `req` if
    /// it is refused).
    pub fn cancel(&mut self, session: SessionId) {
        self.send(client::ToCore::AgentCancel(client::SessionRef { session }));
    }

    /// `AGENT_PERMISSION`: permission request `entry` answered with
    /// `option`, or dismissed with `None`.
    pub fn answer(
        &mut self,
        session: SessionId,
        entry: u64,
        option: Option<&str>,
    ) -> Result<(), client::ErrorMessage> {
        let req = self.take_req();
        self.send(client::ToCore::AgentPermission(client::AgentPermission {
            req,
            session,
            entry,
            option: option.map(str::to_owned),
        }));
        self.agent_reply(req)
    }

    /// `AGENT_FETCH` (OpenSpec change `agents-log-paging`): the entries
    /// before `before` within `max_bytes`, as the `AGENT_PAGE` frames
    /// answering it carry them, and whether more are earlier; or the
    /// refusal. Other messages are kept for later.
    pub fn fetch(
        &mut self,
        session: SessionId,
        before: u64,
        max_bytes: u32,
    ) -> Result<(Vec<Event>, bool), client::ErrorMessage> {
        let req = self.take_req();
        self.send(client::ToCore::AgentFetch(client::AgentFetch {
            req,
            session,
            before,
            max_bytes,
        }));
        let mut kept = Vec::new();
        let mut events = Vec::new();
        let result = loop {
            match self.recv() {
                ToClient::AgentPage(page) if page.req == req => {
                    assert_eq!(page.session, session);
                    for value in &page.events {
                        let event = Event::from_value(value)
                            .unwrap_or_else(|error| panic!("AGENT_PAGE event {value}: {error}"));
                        assert_eq!(event.to_value(), *value, "AGENT_PAGE event {value} isn't one of protocol 4's forms");
                        events.push(event);
                    }
                    if let Some(last) = page.last {
                        break Ok((std::mem::take(&mut events), last.more));
                    }
                }
                ToClient::Error(error) if error.req == Some(req) => break Err(error),
                other => kept.push(other),
            }
        };
        kept.extend(self.backlog.drain(..));
        self.backlog = kept.into();
        result
    }

    /// `ATTACH` (which gives the session this client's token as its owner,
    /// so its replay waits for the new meta to reach the holder's socket)
    /// and an `AGENT_FETCH` written in one write, so the core reads both
    /// before its next flush: gives the fetch's answer, read before the
    /// replay, and the replay's start once it has ended.
    pub fn attach_agent_and_fetch(
        &mut self,
        session: SessionId,
        before: u64,
        max_bytes: u32,
    ) -> (Result<(), client::ErrorMessage>, client::ReplayStart) {
        let attach = self.take_req();
        let fetch = self.take_req();
        let mut bytes = Vec::new();
        for message in [
            client::ToCore::Attach(client::Attach { req: attach, session, cols: 0, rows: 0, page: None }),
            client::ToCore::AgentFetch(client::AgentFetch { req: fetch, session, before, max_bytes }),
        ] {
            message.encode().unwrap().write_to(&mut bytes).unwrap();
        }
        io::Write::write_all(&mut self.stream, &bytes).unwrap();
        let mut kept = Vec::new();
        let mut answer = None;
        let mut start = None;
        loop {
            match self.recv() {
                ToClient::Error(error) if error.req == Some(fetch) => answer = Some(Err(error)),
                ToClient::AgentPage(page) if page.req == fetch => {
                    if page.last.is_some() {
                        answer = Some(Ok(()));
                    }
                }
                ToClient::ReplayStart(replay) if replay.req == Some(attach) => {
                    assert!(answer.is_some(), "the replay started before the fetch was answered");
                    start = Some(replay);
                }
                ToClient::Transcript(transcript) if transcript.session == session => {}
                ToClient::ReplayEnd(end) if end.session == session => break,
                other => kept.push(other),
            }
        }
        kept.extend(self.backlog.drain(..));
        self.backlog = kept.into();
        (answer.expect("the fetch's answer"), start.expect("the replay's start"))
    }

    /// Fetches the pages before the session's log's window, a whole page
    /// at a time, folding each in front ([`Transcript::prepend`]) until the
    /// core says none are earlier; gives the log, now held from its start
    /// (or its trim). Live events read meanwhile are folded as they come.
    pub fn fetch_to_start(&mut self, session: SessionId) -> Transcript {
        while let Some(window) = self.transcript(session).window() {
            let (events, more) = self
                .fetch(session, window.from.0, brindle_transcript::PAGE_BYTES as u32)
                .unwrap_or_else(|error| panic!("the fetch was refused: {}", error.message));
            self.transcripts
                .entry(session)
                .or_default()
                .prepend(&events, more)
                .unwrap_or_else(|error| panic!("a fetched page doesn't fold: {error}"));
        }
        self.transcript(session)
    }

    /// The agent session's log as this client has folded it so far.
    pub fn transcript(&self, session: SessionId) -> Transcript {
        self.transcripts.get(&session).cloned().unwrap_or_default()
    }

    /// The events this client folded into the session's log so far, in
    /// the order they came, since its last replay started.
    pub fn events(&self, session: SessionId) -> Vec<Event> {
        self.events.get(&session).cloned().unwrap_or_default()
    }

    /// Reads until the session's log satisfies `done` (`what` names it for
    /// the failure) and returns it. `TRANSCRIPT`s are folded and dropped;
    /// other messages are kept for later.
    pub fn transcript_until(
        &mut self,
        session: SessionId,
        what: &str,
        mut done: impl FnMut(&Transcript) -> bool,
    ) -> Transcript {
        let deadline = Instant::now() + limit(TIMEOUT);
        let mut kept = Vec::new();
        // What is already kept was folded when it was read.
        for message in std::mem::take(&mut self.backlog) {
            if !matches!(message, ToClient::Transcript(_)) {
                kept.push(message);
            }
        }
        while !self.transcripts.get(&session).is_some_and(&mut done) {
            match self.next(deadline.saturating_duration_since(Instant::now())) {
                Recv::Message(ToClient::Transcript(_)) => {}
                Recv::Message(other) => kept.push(other),
                other => panic!(
                    "{other:?} before session {session}'s log {what}; it is {:#?}",
                    self.transcript(session)
                ),
            }
        }
        kept.extend(self.backlog.drain(..));
        self.backlog = kept.into();
        self.transcript(session)
    }

    /// `ATTACH`es an agent session (its size, 0×0, is ignored) and reads
    /// its replay, which starts the session's log again: the replay's
    /// `REPLAY_START`, or the refusal. Messages for other sessions are kept
    /// for later.
    pub fn attach_agent(
        &mut self,
        session: SessionId,
    ) -> Result<client::ReplayStart, client::ErrorMessage> {
        self.attach_agent_paged(session, None)
    }

    /// [`TestClient::attach_agent`] asking for a replay page of `page`
    /// bytes (`ATTACH.page`, OpenSpec change `agents-log-paging`).
    pub fn attach_agent_paged(
        &mut self,
        session: SessionId,
        page: Option<u32>,
    ) -> Result<client::ReplayStart, client::ErrorMessage> {
        let req = self.take_req();
        self.send(client::ToCore::Attach(client::Attach {
            req,
            session,
            cols: 0,
            rows: 0,
            page,
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
                other => kept.push(other),
            }
        };
        loop {
            match self.recv() {
                ToClient::Transcript(transcript) if transcript.session == session => {}
                ToClient::ReplayEnd(end) if end.session == session => break,
                other => kept.push(other),
            }
        }
        kept.extend(self.backlog.drain(..));
        self.backlog = kept.into();
        Ok(start)
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
        // `until`'s poll, keeping the last screen for the failure.
        let mut matched = None;
        let mut last = String::new();
        let found = eventually(TIMEOUT, || {
            let (start, bytes) = self
                .attach(session, 80, 24)
                .unwrap_or_else(|error| panic!("attach refused: {}", error.message));
            let screen = replayed(&bytes, 80, 24, scrollback);
            if done(&screen) {
                matched = Some((start, screen));
                return true;
            }
            last = snapshot(&screen);
            self.detach(session);
            false
        });
        match matched {
            Some(matched) if found => matched,
            _ => panic!("the replay never matched; last:\n{last}"),
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

/// A client `SPAWN_AGENT` of the fake ACP agent playing `script`
/// ([`fake_agent::AgentScript::write`]), with a minimal environment of its
/// own, in the server's default directory.
pub fn client_agent(script: &Path) -> client::SpawnAgent {
    client::SpawnAgent {
        req: 0,
        command: FAKE_AGENT.into(),
        args: vec![script.display().to_string()],
        cwd: None,
        env: base_env(),
        policy: client::Policy::Kill,
        profile: None,
        conversation: None,
        start: None,
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

/// A fresh emulator of `cols` × `rows`, fed `bytes` (a replay, say). It
/// reads directory reports as POSIX paths; the tests compare its screen,
/// which the path style doesn't change.
pub fn replayed(bytes: &[u8], cols: u16, rows: u16, scrollback: usize) -> Emulator {
    let mut screen = Emulator::new(cols, rows, scrollback, PathStyle::Posix);
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
            "command prompt {} input {:?} output {:?} end {:?} exit {:?} line {:?}",
            marks.line(command.prompt),
            command.input.map(|pos| (marks.line(pos), pos.col)),
            command.output.map(|pos| (marks.line(pos), pos.col)),
            command.end.map(|pos| (marks.line(pos), pos.col)),
            command.exit,
            command.line
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

/// Writes `bytes` to a file in `dir` and returns a `detach` session spawn
/// whose PowerShell prints the file as it is (the escape sequences too),
/// then waits. The console host renders what it prints, so a test compares
/// a replay with what a client saw, not with `bytes`.
#[cfg(windows)]
pub fn fixture_spawn(dir: &TempDir, name: &str, bytes: &[u8], scrollback: u32) -> client::Spawn {
    let path = dir.path.join(name);
    fs::write(&path, bytes).unwrap();
    let script = format!(
        "[Console]::Write([IO.File]::ReadAllText('{}')); Start-Sleep -Seconds 1000",
        path.display()
    );
    let mut spawn = client_spawn("powershell.exe", &["-NoProfile", "-Command", &script]);
    spawn.env = inherited_env(&[]);
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
