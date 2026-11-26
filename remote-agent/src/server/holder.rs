//! `brindle-remote hold`: the holder, which owns sessions' PTYs for the
//! terminal server's core.
//!
//! A holder is the programs' parent. It pumps their I/O, resizes, hangs up
//! and reaps them (on Windows over ConPTY: see `platform::pty`), and parses
//! nothing. Per session it keeps the core's latest checkpoint and the raw
//! output since then (the tail), as bytes it doesn't interpret, so a new
//! core can rebuild the screens.
//!
//! - **Endpoint:** holder `<n>` of its build listens where
//!   [`Endpoints::holder`] says: on Linux at the socket
//!   `holder-<build>-<n>.sock` in the server directory, on Windows at a
//!   named pipe. Beside it in the directory are `holder-<build>-<n>.lock`,
//!   which it holds for its whole life, and its log.
//! - **One core at a time:** a newer `HELLO` takes the holder over from the
//!   connected core. `WELCOME` lists the sessions, then each one's
//!   checkpoint and tail follow. A core that is gone is dropped only once
//!   what it had sent is handled, even when a write to it is what failed
//!   (up to [`DEAD_CORE_READ_LIMIT`]).
//! - **Backpressure:** from [`CORE_QUEUE_HIGH`] queued to the core until
//!   less than [`CORE_QUEUE_LOW`] is, it doesn't read PTYs, and it doesn't
//!   read a session the core has paused (`read_limit`). With no core, it
//!   reads a session until its tail is full, so the program blocks instead
//!   of losing output. Input past `INPUT_QUEUE_LIMIT` (4 MiB) waiting for
//!   one session is dropped with an `ERROR`: lossy by design, since
//!   Brindle's own budget in front of it is the same size.
//! - **Faults:** each message from the core is handled under
//!   `catch_unwind` (`Holder::guarded`). A failure is answered for its unit
//!   (`unit_of`): `SPAWN_FAILED` for a `SPAWN`, else an `ERROR` for the
//!   message's session, or for the holder. The sessions and the core
//!   connection carry on. In debug builds `BRINDLE_TEST_HOLDER_PANIC` names
//!   a marker that makes `SPAWN` (in an argument) and `INPUT` (in its
//!   bytes) fail, for the tests.
//! - **Refusals:** a connection whose first message isn't `HELLO`, or whose
//!   `HELLO` shares no version, gets an `ERROR` and a graceful close within
//!   `REFUSAL_CLOSE_LIMIT` (1 s), so it reads the error and then the end.
//! - **Supervision:** with sessions but no core for [`CORE_GRACE`], it
//!   starts `serve` from its own binary (which survives the file being
//!   replaced), retrying with backoff up to [`CORE_BACKOFF_MAX`]
//!   ([`CoreStart`]).
//! - **Exit:** once it has no sessions (after having one, after `DRAIN`,
//!   with no core after failing to start a program, or
//!   when no core used it within [`UNUSED_LIMIT`] of starting).
//! - **Pipe sessions** (H1 version 3's `SPAWN_PIPES`): a program on plain
//!   pipes, no terminal. Its stdout is the session's output, as a PTY's is;
//!   its stdin takes `INPUT`; its stderr is read always into a ring
//!   ([`StderrRing`]) sent once as `STDERR_TAIL` before `EXITED` (after its
//!   `TAIL_END` for one that ended with no core). A hang-up takes
//!   `platform::piped::HangUp`'s steps. A core below version 3 isn't shown
//!   them ([`policy::shown`]).
//!
//! Its decisions (spawn refusals, when an exited session is drained, when
//! to start a core) are pure, in [`policy`].

mod policy;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::endpoint::{self, Endpoints};
use brindle_remote_protocol::holder::{
    self as h1, Assembler, Data, ErrorMessage, ExitStatus, Exited, HeldSession, Hello, Previous,
    Spawn, SpawnFailed, SpawnMode, SpawnPipes, Spawned, ToCore, ToHolder, Welcome,
};
use brindle_remote_protocol::ready::ReadyRead;

use self::policy::{CoreStart, Output};
use super::conn::Conn;
use super::daemon::ReadyPipe;
use super::event_loop::{Event, EventLoop, Interest, Loop, Token};
use super::platform::listen::{self, Identity, Listener};
use super::platform::lock::Lock;
use super::platform::piped::{HangUp, HangUpStep};
use super::platform::pty::{self, Group, Program, Pty};
use super::platform::{self, PipeCommand, Piped};
use super::{build_id, daemon, dir, fault};
use crate::pty::{GRACEFUL_SHUTDOWN, OUTPUT_CHUNK_SIZE, PtyCommand, Reparented};

/// From this much queued to the core, the holder stops reading PTYs...
pub const CORE_QUEUE_HIGH: usize = 8 * 1024 * 1024;
/// ...until less than this is: a quarter, so reading resumes while the core
/// still has plenty queued, and doesn't starve between turns.
pub const CORE_QUEUE_LOW: usize = 2 * 1024 * 1024;
/// With sessions but no core for this long, the holder starts one.
pub const CORE_GRACE: Duration = Duration::from_secs(3);
/// The first retry delay after starting a core that didn't connect...
pub const CORE_BACKOFF_START: Duration = Duration::from_secs(3);
/// ...doubling up to this.
pub const CORE_BACKOFF_MAX: Duration = Duration::from_secs(60);
/// A holder that never had a session exits this long after starting, unless
/// a core is connected.
pub const UNUSED_LIMIT: Duration = Duration::from_secs(10);
/// Input queued for one session past this is refused with an `ERROR`.
const INPUT_QUEUE_LIMIT: usize = 4 * 1024 * 1024;
/// When a write to its core fails, the holder still reads what that core
/// had sent, up to this much, before it drops the core.
pub const DEAD_CORE_READ_LIMIT: usize = 16 * 1024 * 1024;
/// A connection must send `HELLO` within this long.
const HELLO_LIMIT: Duration = Duration::from_secs(5);
/// How long a refused connection gets to read its `ERROR` and the end.
const REFUSAL_CLOSE_LIMIT: Duration = Duration::from_secs(1);
const MAX_PENDING: usize = 16;
/// How long an exiting holder waits for the core to take its last messages
/// (a flood's output, then `EXITED`) and close the connection.
const EXIT_CLOSE_LIMIT: Duration = Duration::from_secs(10);
const READS_PER_EVENT: usize = 4;
const TICK_FAST: Duration = Duration::from_millis(50);
const TICK_SLOW: Duration = Duration::from_millis(500);

/// The core the holder starts is run from its own binary, with
/// [`daemon::role_args`]. Tests replace the arguments with
/// `BRINDLE_TEST_HOLDER_CORE_ARGS` (whitespace-separated), where the
/// platform's placeholder (`{ready_fd}` on Linux, `{ready_handle}` on
/// Windows) stands for the ready pipe.
const CORE_ARGS_ENV: &str = "BRINDLE_TEST_HOLDER_CORE_ARGS";

macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!(
            "{} holder[{}]: {}",
            daemon::timestamp(),
            std::process::id(),
            format_args!($($arg)*)
        )
    };
}

/// Runs `brindle-remote hold [--ready-fd N]` (`--ready-handle N` on
/// Windows). Without it the holder stays in the foreground and logs to
/// stderr.
pub fn run(args: &[String]) -> Result<(), String> {
    let ready = daemon::ready_from_args("hold", args)?;
    // Held until the holder exits: an elevated holder's directory stays the
    // one it checked.
    let server_dir = dir::from_env().map_err(|error| error.to_string())?;
    let build = build_id::current().map_err(|error| format!("can't read the build id: {error}"))?;
    let slot = Slot::claim(server_dir.path(), &build).map_err(|error| error.to_string())?;
    if ready.is_some() {
        let log = daemon::open_log(&slot.log_path)
            .map_err(|error| format!("can't open {}: {error}", slot.log_path.display()))?;
        daemon::daemonize(&log).map_err(|error| error.to_string())?;
    }
    let core_args = std::env::var(CORE_ARGS_ENV)
        .ok()
        .map(|args| args.split_whitespace().map(String::from).collect())
        .unwrap_or_else(|| daemon::role_args("serve"));
    let ev = Loop::new().map_err(|error| error.to_string())?;
    let holder = Holder::new(ev, build, slot, core_args).map_err(|error| {
        log!("can't start: {error}");
        error.to_string()
    })?;
    if let Some(ready) = ready
        && let Err(error) = ready.signal()
    {
        log!("couldn't tell the starter it is listening: {error}");
    }
    holder.run().map_err(|error| {
        log!("stopped: {error}");
        error.to_string()
    })
}

/// The holder's place in the server directory: index `n`, its held lock
/// file, its log, and where it listens.
struct Slot {
    _lock: Lock,
    lock_path: PathBuf,
    endpoint: PathBuf,
    log_path: PathBuf,
}

impl Slot {
    /// Takes the first index whose lock file no live holder of this build
    /// holds.
    fn claim(dir: &Path, build: &str) -> io::Result<Self> {
        let endpoints = Endpoints::of(dir)?;
        let mut index = 0u32;
        let mut attempts = 0;
        while index < 1024 && attempts < 4096 {
            attempts += 1;
            let place = endpoints.holder(build, index);
            let Some(lock) = Lock::create(&place.lock)? else {
                index += 1;
                continue;
            };
            // An exiting holder removes its lock file; if that happened
            // between the open and the lock, this lock protects nothing.
            if !lock.is_at(&place.lock)? {
                continue;
            }
            listen::check_path(&place.endpoint)?;
            return Ok(Self {
                _lock: lock,
                lock_path: place.lock,
                endpoint: place.endpoint,
                log_path: place.log,
            });
        }
        Err(io::Error::other(
            "no free holder slot in the server directory",
        ))
    }
}

/// The output kept since the session's last checkpoint, at most
/// [`h1::TAIL_LIMIT`] bytes; past that the oldest bytes go.
#[derive(Default)]
struct Tail {
    bytes: VecDeque<u8>,
    truncated: bool,
    /// Output read that no checkpoint covers yet, counted as the core
    /// counts `covers`: since the last checkpoint, or since the start of
    /// the tail last sent to a core. Truncated bytes count too.
    uncovered: u64,
}

impl Tail {
    const LIMIT: usize = h1::TAIL_LIMIT as usize;

    fn push(&mut self, data: &[u8]) {
        self.bytes.extend(data);
        self.uncovered += data.len() as u64;
        let excess = self.bytes.len().saturating_sub(Self::LIMIT);
        if excess > 0 {
            self.bytes.drain(..excess);
            self.truncated = true;
        }
    }

    fn room(&self) -> usize {
        Self::LIMIT.saturating_sub(self.bytes.len())
    }

    /// A checkpoint covers the first `covers` uncovered bytes: they go, and
    /// the rest (read after the core rendered it) stay. It is truncated
    /// only if some of the rest were lost.
    fn cover(&mut self, covers: u64) {
        self.uncovered = self.uncovered.saturating_sub(covers);
        let keep = self.uncovered.min(self.bytes.len() as u64) as usize;
        self.bytes.drain(..self.bytes.len() - keep);
        self.truncated = self.uncovered > keep as u64;
    }

    /// The tail goes to a new core, which counts from its start.
    fn sent(&mut self) {
        self.uncovered = self.bytes.len() as u64;
    }

    fn to_vec(&self) -> Vec<u8> {
        let (front, back) = self.bytes.as_slices();
        [front, back].concat()
    }
}

/// A pipe session's stderr: its newest [`h1::MAX_STDERR_TAIL`] bytes, read
/// always and sent once, as `STDERR_TAIL`, when the session has ended.
#[derive(Default)]
struct StderrRing {
    bytes: VecDeque<u8>,
}

impl StderrRing {
    const LIMIT: usize = h1::MAX_STDERR_TAIL;

    fn push(&mut self, data: &[u8]) {
        let data = &data[data.len().saturating_sub(Self::LIMIT)..];
        self.bytes.extend(data);
        let excess = self.bytes.len().saturating_sub(Self::LIMIT);
        self.bytes.drain(..excess);
    }

    fn to_vec(&self) -> Vec<u8> {
        let (front, back) = self.bytes.as_slices();
        [front, back].concat()
    }
}

/// Where a session's program is in its life (`types R1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Life {
    Running,
    /// Reaped `at`, with its exit code (`None` for a signal); its output
    /// may still be draining ([`policy::drained`]).
    Reaped { code: Option<i32>, at: Instant },
    /// Its output is drained, and `held.exit` says how it ended: the
    /// connected core is sent `EXITED`, or the next core's `WELCOME`
    /// reports it.
    Finished,
}

struct Session {
    /// The entry `WELCOME` reports; its `exit` is written only when the
    /// session becomes [`Life::Finished`]. Its `mode` says which [`Io`]
    /// the session has.
    held: HeldSession,
    program: Program,
    /// Closed once the session is finished, and a PTY's on hang-up.
    io: Option<Io>,
    /// The PTY's token, or a pipe session's stdout's.
    token: Token,
    interest: Interest,
    checkpoint: Vec<u8>,
    tail: Tail,
    input: VecDeque<u8>,
    life: Life,
    /// When output last came (a pipe session's stderr counts).
    last_output: Instant,
    /// Closed once the PTY's output (a pipe session's stdout) ended or a
    /// PTY session was hung up.
    output: Output,
    /// The connected core sent `PAUSE`: its client isn't keeping up.
    paused: bool,
    /// A pipe session's stderr; empty for a PTY session.
    stderr: StderrRing,
    /// A pipe session's hang-up has started ([`Holder::hang_ups`]).
    hung_up: bool,
}

/// A session's streams (`types R1`): a PTY, or a pipe session's three
/// pipes.
enum Io {
    Pty(Pty),
    Pipes(Pipes),
}

/// A pipe session's pipes and their tokens. stdout's token is the
/// session's own ([`Session::token`]).
struct Pipes {
    piped: Piped,
    /// stdin's token and what it waits for, while stdin is open.
    input: Option<(Token, Interest)>,
    /// stderr's token, while stderr is open. It always waits to read.
    error: Option<Token>,
}

impl Io {
    /// Reads the program's output: the PTY's, or stdout.
    fn read_output(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Pty(pty) => pty.read(buffer),
            Self::Pipes(pipes) => pipes.piped.read_output(buffer),
        }
    }

    /// Writes the program's input: to the PTY, or stdin.
    fn write_input(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Pty(pty) => pty.write(bytes),
            Self::Pipes(pipes) => pipes.piped.write_input(bytes),
        }
    }

    /// Whether input written now can reach the program.
    fn takes_input(&self) -> bool {
        match self {
            Self::Pty(_) => true,
            Self::Pipes(pipes) => pipes.input.is_some(),
        }
    }

    /// Every token it is registered with.
    fn tokens(&self, output: Token) -> Vec<Token> {
        match self {
            Self::Pty(_) => vec![output],
            Self::Pipes(pipes) => [Some(output), pipes.input.map(|(token, _)| token), pipes.error]
                .into_iter()
                .flatten()
                .collect(),
        }
    }
}

/// Whether the session's output can still come: a PTY's, or a pipe
/// session's stdout or stderr.
fn streams(session: &Session) -> Output {
    let stderr_open = match &session.io {
        Some(Io::Pipes(pipes)) => Some(pipes.error.is_some()),
        Some(Io::Pty(_)) | None => None,
    };
    streams_of(session.output, stderr_open)
}

/// [`streams`], given the PTY's or stdout's `output` and, for a pipe
/// session, whether its stderr is open.
fn streams_of(output: Output, stderr_open: Option<bool>) -> Output {
    match (output, stderr_open) {
        (Output::Closed, Some(true)) => Output::Open,
        (output, Some(_) | None) => output,
    }
}

/// Whether the connected core is staying, or said `GOODBYE` (`types R1`):
/// a handover is always a goodbye.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Farewell {
    Staying,
    /// It sent `GOODBYE`: it is going, so exits wait for the next core.
    Goodbye,
    /// It sent `GOODBYE { handover: true }`: it stepped down for the next.
    Handover,
}

struct Core {
    conn: Conn,
    token: Token,
    interest: Interest,
    instance: u64,
    /// The H1 version it negotiated: below 3 it isn't shown pipe sessions
    /// ([`policy::shown`]).
    protocol: u32,
    /// Its queue went past [`CORE_QUEUE_HIGH`] and hasn't been under
    /// [`CORE_QUEUE_LOW`] since ([`core_backlog`]). A new core's connection
    /// starts empty, so under.
    over: bool,
    checkpoints: Assembler,
    /// The sessions whose open checkpoint blob is an append
    /// (`CHECKPOINT_APPEND`): its `CHECKPOINT_END` appends to the stored
    /// checkpoint instead of replacing it.
    appends: HashSet<SessionId>,
    farewell: Farewell,
}

struct Pending {
    conn: Conn,
    since: Instant,
}

#[derive(Debug, Clone, Copy)]
enum Source {
    Listener,
    Pending,
    Core,
    /// A PTY, or a pipe session's stdout.
    Master(SessionId),
    /// A pipe session's stdin.
    Stdin(SessionId),
    /// A pipe session's stderr.
    Stderr(SessionId),
    CoreReady,
}

/// The connected core, if it is shown sessions of `mode` ([`policy::shown`]).
fn core_for(core: &mut Option<Core>, mode: SpawnMode) -> Option<&mut Core> {
    core.as_mut()
        .filter(|core| policy::shown(core.protocol, mode))
}

const LISTENER: Token = 0;

struct Holder<L: EventLoop> {
    ev: L,
    build: String,
    slot: Slot,
    listener: Listener,
    sources: HashMap<Token, Source>,
    next_token: Token,
    core: Option<Core>,
    pending: HashMap<Token, Pending>,
    sessions: BTreeMap<SessionId, Session>,
    previous: Previous,
    draining: bool,
    had_session: bool,
    /// A core asked it to start a program it couldn't run: a core used it,
    /// so with no session and no core it exits without waiting out
    /// [`UNUSED_LIMIT`], while a core still there can use it again.
    spawn_failed: bool,
    started: Instant,
    /// When it starts a core of its own, with no core connected.
    core_start: CoreStart,
    core_args: Vec<String>,
    core_starts: Vec<Child>,
    core_ready: HashMap<Token, ReadyPipe>,
    /// Process groups to kill when their hang-up grace ends, with the
    /// session whose program leads each one.
    kills: Vec<(SessionId, Group, Instant)>,
    /// Pipe sessions' hang-ups in progress, with the group each ends
    /// ([`HangUp`]'s steps).
    hang_ups: Vec<(SessionId, Group, HangUp)>,
    /// The log said that `REPARENT` does nothing here, which it says once.
    reparent_unsupported: bool,
}

impl<L: EventLoop> Holder<L> {
    fn new(mut ev: L, build: String, slot: Slot, core_args: Vec<String>) -> io::Result<Self> {
        // The lock is ours, so any socket at this path is stale.
        let listener = Listener::bind(&slot.endpoint, Identity::own())?;
        ev.register(&listener.source(), LISTENER, Interest::READABLE)?;
        ev.waker().wake_on_child_exit()?;
        log!(
            "listening on {} (build {build})",
            slot.endpoint.display()
        );
        Ok(Self {
            ev,
            build,
            slot,
            listener,
            sources: HashMap::from([(LISTENER, Source::Listener)]),
            next_token: LISTENER + 1,
            core: None,
            pending: HashMap::new(),
            sessions: BTreeMap::new(),
            previous: Previous::None,
            draining: false,
            had_session: false,
            spawn_failed: false,
            started: Instant::now(),
            core_start: CoreStart::new(),
            core_args,
            core_starts: Vec::new(),
            core_ready: HashMap::new(),
            kills: Vec::new(),
            hang_ups: Vec::new(),
            reparent_unsupported: false,
        })
    }

    fn run(mut self) -> io::Result<()> {
        let mut events: Vec<Event> = Vec::new();
        let result = loop {
            self.update_interests();
            // Ticks come fast while kills wait or a reaped program's
            // output drains.
            let fast = !self.kills.is_empty()
                || !self.hang_ups.is_empty()
                || self.sessions.values().any(|session| match session.life {
                    Life::Reaped { .. } => true,
                    Life::Running | Life::Finished => false,
                });
            if let Err(error) = self
                .ev
                .wait(&mut events, Some(if fast { TICK_FAST } else { TICK_SLOW }))
            {
                break Err(error);
            }
            for event in &events {
                self.dispatch(*event);
            }
            self.reap();
            let now = Instant::now();
            self.tick(now);
            if self.should_exit(now) {
                break Ok(());
            }
        };
        // Even on a failure, the core gets what is queued for it.
        self.shutdown();
        result
    }

    fn token(&mut self) -> Token {
        let token = self.next_token;
        self.next_token += 1;
        token
    }

    fn dispatch(&mut self, event: Event) {
        match self.sources.get(&event.token).copied() {
            Some(Source::Listener) => self.accept(),
            Some(Source::Pending) => self.read_pending(event.token),
            Some(Source::Core) => {
                if event.writable {
                    self.flush_core();
                }
                if event.readable || event.closed {
                    self.read_core();
                }
            }
            Some(Source::Master(session)) => {
                if event.writable {
                    self.write_input(session);
                }
                if event.readable || event.closed {
                    self.read_output(session);
                }
            }
            // A failed stdin shows as closed: the write reports it.
            Some(Source::Stdin(session)) => {
                if event.writable || event.closed {
                    self.write_input(session);
                }
            }
            Some(Source::Stderr(session)) => {
                if event.readable || event.closed {
                    self.read_errors(session);
                }
            }
            Some(Source::CoreReady) => self.read_core_ready(event.token),
            None => {}
        }
    }

    fn accept(&mut self) {
        loop {
            // The peer check comes before reading anything.
            let stream = match self.listener.accept() {
                Ok(Some(stream)) => stream,
                Ok(None) => return,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    log!("{error}");
                    continue;
                }
                Err(error) => {
                    log!("accept failed: {error}");
                    return;
                }
            };
            if self.pending.len() >= MAX_PENDING {
                log!("too many connections waiting to say HELLO; refusing one");
                continue;
            }
            let conn = match Conn::new(stream) {
                Ok(conn) => conn,
                Err(error) => {
                    log!("can't use a connection: {error}");
                    continue;
                }
            };
            let token = self.token();
            if let Err(error) = self.ev.register(&conn.source(), token, Interest::READABLE) {
                log!("can't watch a connection: {error}");
                continue;
            }
            self.sources.insert(token, Source::Pending);
            self.pending.insert(
                token,
                Pending {
                    conn,
                    since: Instant::now(),
                },
            );
        }
    }

    fn drop_source(&mut self, token: Token) {
        let _ = self.ev.deregister(token);
        self.sources.remove(&token);
    }

    fn read_pending(&mut self, token: Token) {
        let Some(pending) = self.pending.get_mut(&token) else {
            return;
        };
        let eof = pending.conn.read_available().unwrap_or(true);
        match pending.conn.next_frame() {
            Ok(Some(frame)) => {
                let pending = self.pending.remove(&token).expect("pending connection");
                match ToHolder::decode(&frame) {
                    Ok(ToHolder::Hello(hello)) => self.take_over(token, pending.conn, hello),
                    _ => {
                        let mut conn = pending.conn;
                        queue(
                            &mut conn,
                            ToCore::Error(ErrorMessage {
                                session: None,
                                message: "the first message must be HELLO".into(),
                            }),
                        );
                        refuse(&mut conn);
                        self.drop_source(token);
                    }
                }
            }
            Ok(None) if !eof => {}
            Ok(None) | Err(_) => {
                self.pending.remove(&token);
                self.drop_source(token);
            }
        }
    }

    /// A `HELLO`: this connection becomes the holder's core, replacing any
    /// other, and is told about every session.
    fn take_over(&mut self, token: Token, mut conn: Conn, hello: Hello) {
        let protocol = match h1::answer_hello(&hello, h1::PROTOCOLS) {
            Ok(protocol) => protocol,
            Err(error) => {
                log!("refused a core: {}", error.message);
                queue(&mut conn, ToCore::Error(error));
                refuse(&mut conn);
                self.drop_source(token);
                return;
            }
        };
        if let Some(old) = self.core.as_ref() {
            log!(
                "core instance {} takes over from instance {}",
                hello.instance,
                old.instance
            );
            self.lose_core("taken over by a newer HELLO");
        }
        log!(
            "core instance {} (build {:?}) connected, protocol H{protocol}",
            hello.instance,
            hello.core_build
        );
        self.sources.insert(token, Source::Core);
        let mut core = Core {
            conn,
            token,
            interest: Interest::READABLE,
            instance: hello.instance,
            protocol,
            over: false,
            checkpoints: Assembler::new(h1::MAX_CHECKPOINT_LEN),
            appends: HashSet::new(),
            farewell: Farewell::Staying,
        };
        let shown = |session: &Session| policy::shown(protocol, session.held.mode);
        let sessions = self
            .sessions
            .values()
            .filter(|session| shown(session))
            .map(|session| HeldSession {
                truncated: session.tail.truncated,
                ..session.held.clone()
            })
            .collect();
        queue(
            &mut core.conn,
            ToCore::Welcome(Welcome {
                protocol,
                build: self.build.clone(),
                pid: std::process::id(),
                previous: self.previous,
                sessions,
            }),
        );
        for (&id, session) in &mut self.sessions {
            if !shown(session) {
                continue;
            }
            for message in ToCore::checkpoint(id, &session.checkpoint) {
                queue(&mut core.conn, message);
            }
            for message in ToCore::tail(id, &session.tail.to_vec()) {
                queue(&mut core.conn, message);
            }
            session.tail.sent();
            // A pipe session that ended with no core: its stderr explains
            // the exit `WELCOME` reported, after its tail.
            match (session.held.mode, session.life) {
                (SpawnMode::Pipes, Life::Finished) => queue(
                    &mut core.conn,
                    ToCore::StderrTail(Data {
                        session: id,
                        bytes: session.stderr.to_vec(),
                    }),
                ),
                (SpawnMode::Pipes, Life::Running | Life::Reaped { .. }) | (SpawnMode::Pty, _) => {}
            }
        }
        // The core now knows how these ended.
        let finished: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, session)| shown(session))
            .filter(|(_, session)| match session.life {
                Life::Finished => true,
                Life::Running | Life::Reaped { .. } => false,
            })
            .map(|(&id, _)| id)
            .collect();
        for id in finished {
            self.remove_session(id);
        }
        self.core = Some(core);
        self.core_start.connected();
        self.process_core_frames();
    }

    fn lose_core(&mut self, reason: &str) {
        let Some(core) = self.core.take() else {
            return;
        };
        self.drop_source(core.token);
        // Pauses were that core's; the next one starts with none.
        for session in self.sessions.values_mut() {
            session.paused = false;
        }
        self.previous = match core.farewell {
            Farewell::Handover => Previous::Handover,
            Farewell::Staying | Farewell::Goodbye => Previous::Crash,
        };
        log!(
            "core instance {} left ({reason}); {} session(s) held",
            core.instance,
            self.sessions.len()
        );
        self.core_start.lost(Instant::now());
    }

    fn flush_core(&mut self) {
        if let Some(core) = self.core.as_mut()
            && let Err(error) = core.conn.flush()
        {
            // What the core sent before it went still counts, as in
            // `read_core`: it may already have told a client about it (an
            // attach's `SET_META`). All of it, not one turn's budget: this
            // connection has no later turn, and a socket buffer may be set
            // to hold more than that budget.
            let _ = core.conn.read_up_to(DEAD_CORE_READ_LIMIT);
            self.process_core_frames();
            self.lose_core(&format!("write failed: {error}"));
        }
    }

    fn read_core(&mut self) {
        let Some(core) = self.core.as_mut() else {
            return;
        };
        // What was read before a failure still counts.
        let ended = match core.conn.read_available() {
            Ok(false) => None,
            Ok(true) => Some("the connection closed".to_string()),
            Err(error) => Some(format!("read failed: {error}")),
        };
        self.process_core_frames();
        if let Some(reason) = ended {
            self.lose_core(&reason);
        }
    }

    fn process_core_frames(&mut self) {
        loop {
            let Some(core) = self.core.as_mut() else {
                return;
            };
            let frame = match core.conn.next_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) => return,
                Err(error) => {
                    self.lose_core(&format!("bad frame: {error}"));
                    return;
                }
            };
            match ToHolder::decode(&frame) {
                Ok(message) => self.guarded(message),
                Err(error) => self.send_error(None, error.to_string()),
            }
        }
    }

    /// Handles a message from the core under `catch_unwind`: a failure ends
    /// that message only ([`Holder::fault`]), never the holder, its other
    /// sessions or the core connection (`process R3`).
    fn guarded(&mut self, message: ToHolder) {
        let unit = unit_of(&message);
        let kind = name_of(&message);
        // A `SPAWN` that fails after making its session hangs that one up,
        // never one that had its id already.
        let id_was_free = matches!(unit, Unit::Spawn(id) if !self.sessions.contains_key(&id));
        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| self.handle(message))) {
            self.fault(unit, kind, id_was_free, &fault::describe(&*payload));
        }
    }

    /// Answers a failure while handling a `kind` message for `unit`, after
    /// the unwind dropped what the handler had made. The session it was for
    /// keeps running: its state is byte buffers that a half-run handler
    /// can't leave worse than ending the user's program would.
    fn fault(&mut self, unit: Unit, kind: &str, id_was_free: bool, what: &str) {
        match unit {
            Unit::Spawn(id) => {
                log!("session {id}: the holder failed handling {kind}: {what}");
                if id_was_free && self.sessions.contains_key(&id) {
                    self.hang_up(id);
                }
                self.send(ToCore::SpawnFailed(SpawnFailed {
                    session: id,
                    message: format!("the holder failed while starting it: {what}"),
                }));
            }
            Unit::Session(id) => {
                let message = format!("the holder failed handling {kind}: {what}");
                log!("session {id}: {message}");
                self.send_error(Some(id), message);
            }
            Unit::Checkpoint(id) => {
                // The stored checkpoint and tail stay: the next core
                // rebuilds from the last whole one.
                let message = format!("the holder failed handling {kind}: {what}");
                log!("session {id}: {message}");
                if let Some(core) = self.core.as_mut() {
                    core.checkpoints.abandon(id);
                }
                self.send_error(Some(id), message);
            }
            Unit::Holder => {
                // The connection stays: dropping it would make the core
                // count every session lost.
                let message = format!("the holder failed handling {kind}: {what}");
                log!("{message}");
                self.send_error(None, message);
            }
        }
    }

    fn send(&mut self, message: ToCore) {
        if let Some(core) = self.core.as_mut() {
            queue(&mut core.conn, message);
        }
    }

    fn send_error(&mut self, session: Option<SessionId>, message: impl Into<String>) {
        self.send(ToCore::Error(ErrorMessage {
            session,
            message: message.into(),
        }));
    }

    fn handle(&mut self, message: ToHolder) {
        match message {
            ToHolder::Hello(_) => self.send_error(None, "this connection already sent HELLO"),
            ToHolder::Spawn(spawn) => self.spawn(spawn),
            ToHolder::Input(data) => self.input(data),
            ToHolder::Resize(resize) => {
                if resize.cols == 0 || resize.rows == 0 {
                    return self.send_error(
                        Some(resize.session),
                        "PTY rows and columns must be non-zero",
                    );
                }
                let Some(session) = self.sessions.get_mut(&resize.session) else {
                    return self.send_error(Some(resize.session), no_session(resize.session));
                };
                let pty = match &session.io {
                    Some(Io::Pty(pty)) => pty,
                    Some(Io::Pipes(_)) => {
                        return self.send_error(
                            Some(resize.session),
                            format!("session {} is on pipes and has no size", resize.session),
                        );
                    }
                    None => return,
                };
                let result = pty.resize(resize.cols, resize.rows, resize.cell_w, resize.cell_h);
                session.held.cols = resize.cols;
                session.held.rows = resize.rows;
                if let Err(error) = result {
                    self.send_error(Some(resize.session), format!("resize failed: {error}"));
                }
            }
            ToHolder::Hangup(target) => {
                if !self.hang_up(target.session) {
                    self.send_error(Some(target.session), no_session(target.session));
                }
            }
            ToHolder::CheckpointBegin(begin) => {
                let core = self.core.as_mut().expect("a core sent this");
                match core.checkpoints.begin(begin) {
                    Ok(()) => {
                        core.appends.remove(&begin.session);
                    }
                    Err(error) => self.send_error(Some(begin.session), error.to_string()),
                }
            }
            ToHolder::CheckpointAppend(begin) => {
                let core = self.core.as_mut().expect("a core sent this");
                if let Some(refusal) = policy::append_open_refusal(core.protocol) {
                    return self.send_error(Some(begin.session), refusal.to_string());
                }
                match core.checkpoints.begin(begin) {
                    Ok(()) => {
                        core.appends.insert(begin.session);
                    }
                    Err(error) => self.send_error(Some(begin.session), error.to_string()),
                }
            }
            ToHolder::CheckpointChunk(chunk) => {
                let core = self.core.as_mut().expect("a core sent this");
                if let Err(error) = core.checkpoints.chunk(&chunk) {
                    self.send_error(Some(chunk.session), error.to_string());
                }
            }
            ToHolder::CheckpointEnd(end) => {
                let core = self.core.as_mut().expect("a core sent this");
                let append = core.appends.remove(&end.session);
                match core.checkpoints.end(end.blob_end()) {
                    Ok(bytes) => {
                        let Some(session) = self.sessions.get_mut(&end.session) else {
                            return;
                        };
                        if !append {
                            session.checkpoint = bytes;
                        } else if let Some(refusal) = policy::append_refusal(
                            session.checkpoint.len() as u64,
                            bytes.len() as u64,
                        ) {
                            return self.send_error(Some(end.session), refusal.to_string());
                        } else {
                            session.checkpoint.extend_from_slice(&bytes);
                        }
                        session.tail.cover(end.covers);
                    }
                    Err(error) => self.send_error(Some(end.session), error.to_string()),
                }
            }
            ToHolder::Drain(_) => {
                if !self.draining {
                    log!("draining: no new sessions; exiting after the last one");
                }
                self.draining = true;
            }
            ToHolder::Goodbye(goodbye) => {
                if let Some(core) = self.core.as_mut() {
                    core.farewell = if goodbye.handover {
                        Farewell::Handover
                    } else {
                        Farewell::Goodbye
                    };
                }
            }
            ToHolder::Reparent(reparent) => {
                // Advisory: no reply, and a session that just ended is no
                // error. What it came to is the log's. A pipe session has
                // no console window to give an owner, which is an error.
                let session = self.sessions.get(&reparent.session);
                if let Some(session) = session {
                    match session.held.mode {
                        SpawnMode::Pipes => {
                            return self.send_error(
                                Some(reparent.session),
                                format!(
                                    "session {} is on pipes and has no console window",
                                    reparent.session
                                ),
                            );
                        }
                        SpawnMode::Pty => {}
                    }
                }
                let program = session.map(|session| &session.program);
                match pty::reparent(program, reparent.hwnd) {
                    Ok(Reparented::Done) => {}
                    Ok(Reparented::Unsupported) => {
                        if !self.reparent_unsupported {
                            self.reparent_unsupported = true;
                            log!(
                                "console windows get no owner here (REPARENT needs the packaged ConPTY on Windows); \
                                 not said again"
                            );
                        }
                    }
                    Err(error) => log!(
                        "session {}: can't give its console window an owner: {error}",
                        reparent.session
                    ),
                }
            }
            ToHolder::Pause(target) => self.set_paused(target.session, true),
            ToHolder::Resume(target) => self.set_paused(target.session, false),
            // The next core's `WELCOME` carries it. An unknown session may
            // just have ended.
            ToHolder::SetMeta(update) => {
                if let Some(session) = self.sessions.get_mut(&update.session) {
                    session.held.meta = update.meta;
                }
            }
            ToHolder::SpawnPipes(spawn) => self.spawn_pipes(spawn),
        }
    }

    /// `PAUSE` and `RESUME`. An unknown session may just have ended, so it
    /// is no error.
    fn set_paused(&mut self, id: SessionId, paused: bool) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.paused = paused;
        }
    }

    fn spawn(&mut self, spawn: Spawn) {
        let id = spawn.session;
        let exists = self.sessions.contains_key(&id);
        if let Some(refusal) = policy::spawn_refusal(self.draining, exists, &spawn) {
            return self.send(ToCore::SpawnFailed(SpawnFailed {
                session: id,
                message: refusal.to_string(),
            }));
        }
        for arg in &spawn.args {
            inject_fault(arg.as_bytes());
        }
        let request = PtyCommand {
            command: spawn.command.as_deref(),
            args: &spawn.args,
            cwd: spawn.cwd.as_deref(),
            env: &spawn.env,
            cols: spawn.cols,
            rows: spawn.rows,
            cell_width: spawn.cell_w,
            cell_height: spawn.cell_h,
            exact_env: true,
        };
        let (pty, mut program) = match pty::spawn(&request) {
            Ok(spawned) => spawned,
            Err(error) => {
                let message = format!("{}: {error}", pty::program_name(&request).to_string_lossy());
                log!("session {id}: {message}");
                self.spawn_failed = true;
                return self.send(ToCore::SpawnFailed(SpawnFailed {
                    session: id,
                    message,
                }));
            }
        };
        let token = self.token();
        if let Err(error) = self.ev.register(&pty.source(), token, Interest::READABLE) {
            if let Err(kill) = program.group().kill() {
                log!("session {id}: can't end its program: {kill}");
            }
            let _ = program.wait();
            return self.send(ToCore::SpawnFailed(SpawnFailed {
                session: id,
                message: format!("can't watch the PTY: {error}"),
            }));
        }
        self.sources.insert(token, Source::Master(id));
        if let Err(error) = program.watch(&self.ev.waker()) {
            // The exit is then seen on a later turn of the loop.
            log!("session {id}: can't watch its program for its exit: {error}");
        }
        let pid = program.pid();
        log!(
            "session {id}: started {} as pid {pid}",
            pty::program_name(&request).to_string_lossy()
        );
        self.sessions.insert(
            id,
            Session {
                held: HeldSession {
                    session: id,
                    pid,
                    cols: spawn.cols,
                    rows: spawn.rows,
                    exit: None,
                    meta: spawn.meta,
                    truncated: false,
                    mode: SpawnMode::Pty,
                },
                program,
                io: Some(Io::Pty(pty)),
                token,
                interest: Interest::READABLE,
                checkpoint: Vec::new(),
                tail: Tail::default(),
                input: VecDeque::new(),
                life: Life::Running,
                last_output: Instant::now(),
                output: Output::Open,
                paused: false,
                stderr: StderrRing::default(),
                hung_up: false,
            },
        );
        self.had_session = true;
        self.send(ToCore::Spawned(Spawned { session: id, pid }));
    }

    /// `SPAWN_PIPES`: a program with its standard streams on pipes and no
    /// terminal. Its stdout is the session's output, as a PTY's is; its
    /// stderr is read always, into [`Session::stderr`].
    fn spawn_pipes(&mut self, spawn: SpawnPipes) {
        let id = spawn.session;
        let exists = self.sessions.contains_key(&id);
        if let Some(refusal) = policy::pipe_spawn_refusal(self.draining, exists, &spawn) {
            return self.send(ToCore::SpawnFailed(SpawnFailed {
                session: id,
                message: refusal.to_string(),
            }));
        }
        for arg in &spawn.args {
            inject_fault(arg.as_bytes());
        }
        let request = PipeCommand {
            command: &spawn.command,
            args: &spawn.args,
            cwd: spawn.cwd.as_deref(),
            env: &spawn.env,
        };
        let (piped, mut program) = match platform::spawn_piped(&request) {
            Ok(spawned) => spawned,
            Err(error) => {
                let message = format!("cannot run {}: {error}", spawn.command);
                log!("session {id}: {message}");
                self.spawn_failed = true;
                return self.send(ToCore::SpawnFailed(SpawnFailed {
                    session: id,
                    message,
                }));
            }
        };
        let tokens = [self.token(), self.token(), self.token()];
        let [output, input, error] = tokens;
        let sources = [
            (piped.output_source(), output, Interest::READABLE),
            (piped.error_source(), error, Interest::READABLE),
        ]
        .into_iter()
        .chain(piped.input_source().map(|source| (source, input, Interest::NONE)));
        let mut registered = Vec::new();
        for (source, token, interest) in sources {
            if let Err(failed) = self.ev.register(&source, token, interest) {
                for token in registered {
                    let _ = self.ev.deregister(token);
                }
                if let Err(kill) = program.group().kill() {
                    log!("session {id}: can't end its program: {kill}");
                }
                let _ = program.wait();
                return self.send(ToCore::SpawnFailed(SpawnFailed {
                    session: id,
                    message: format!("can't watch the program's pipes: {failed}"),
                }));
            }
            registered.push(token);
        }
        self.sources.insert(output, Source::Master(id));
        self.sources.insert(error, Source::Stderr(id));
        self.sources.insert(input, Source::Stdin(id));
        if let Err(failed) = program.watch(&self.ev.waker()) {
            // The exit is then seen on a later turn of the loop.
            log!("session {id}: can't watch its program for its exit: {failed}");
        }
        let pid = program.pid();
        log!("session {id}: started {} on pipes as pid {pid}", spawn.command);
        self.sessions.insert(
            id,
            Session {
                held: HeldSession {
                    session: id,
                    pid,
                    cols: 0,
                    rows: 0,
                    exit: None,
                    meta: spawn.meta,
                    truncated: false,
                    mode: SpawnMode::Pipes,
                },
                program,
                io: Some(Io::Pipes(Pipes {
                    piped,
                    input: Some((input, Interest::NONE)),
                    error: Some(error),
                })),
                token: output,
                interest: Interest::READABLE,
                checkpoint: Vec::new(),
                tail: Tail::default(),
                input: VecDeque::new(),
                life: Life::Running,
                last_output: Instant::now(),
                output: Output::Open,
                paused: false,
                stderr: StderrRing::default(),
                hung_up: false,
            },
        );
        self.had_session = true;
        self.send(ToCore::Spawned(Spawned { session: id, pid }));
    }

    fn input(&mut self, data: Data) {
        inject_fault(&data.bytes);
        let Some(session) = self.sessions.get_mut(&data.session) else {
            return self.send_error(Some(data.session), no_session(data.session));
        };
        if !session.io.as_ref().is_some_and(Io::takes_input) {
            // Hung up (or its stdin closed): the program is going, so its
            // input is moot.
            return;
        }
        if session.input.len() + data.bytes.len() > INPUT_QUEUE_LIMIT {
            return self.send_error(
                Some(data.session),
                "the program isn't reading its input; dropped some",
            );
        }
        session.input.extend(data.bytes);
    }

    /// Hangs the session's process group up, then closes its PTY, and kills
    /// the group after the grace period. Says whether it exists.
    ///
    /// The signal goes first: closing the PTY first fails a pending read of
    /// the terminal (EIO) before the kernel's own `SIGHUP` arrives, and a
    /// program blocked reading it, such as `cat`, now and then exits 1 from
    /// that failed read instead of dying of the signal.
    ///
    /// A pipe session is hung up in [`HangUp`]'s steps instead: its stdin
    /// closes at once, while its stdout and stderr are still read, so the
    /// core gets what the program says as it ends.
    fn hang_up(&mut self, id: SessionId) -> bool {
        let Some(session) = self.sessions.get_mut(&id) else {
            return false;
        };
        match session.held.mode {
            SpawnMode::Pty => {}
            SpawnMode::Pipes => {
                session.input.clear();
                if !session.hung_up {
                    session.hung_up = true;
                    log!("session {id}: hung up");
                    let group = session.program.group();
                    self.hang_ups.push((id, group, HangUp::new(Instant::now())));
                    // Its first step, closing stdin, is due at once.
                    self.take_hang_up_steps(Instant::now());
                }
                return true;
            }
        }
        match session.life {
            Life::Running => {
                let group = session.program.group();
                group.hang_up();
                self.kills
                    .push((id, group, Instant::now() + GRACEFUL_SHUTDOWN));
                log!("session {id}: hung up");
            }
            Life::Reaped { .. } | Life::Finished => {}
        }
        close_io(&mut self.ev, &mut self.sources, session);
        session.input.clear();
        session.output = Output::Closed;
        true
    }

    /// Takes the pipe sessions' hang-up steps due at `now`.
    fn take_hang_up_steps(&mut self, now: Instant) {
        let Self {
            ev,
            sources,
            sessions,
            hang_ups,
            ..
        } = self;
        for (id, group, hang_up) in hang_ups.iter_mut() {
            for step in hang_up.due(now) {
                match step {
                    HangUpStep::CloseInput => {
                        if let Some(session) = sessions.get_mut(id) {
                            close_stdin(ev, sources, session);
                        }
                    }
                    HangUpStep::EndGroup => group.hang_up(),
                    HangUpStep::KillRemaining => {
                        if let Err(error) = group.kill_remaining() {
                            log!("session {id}: can't end what its hang-up left running: {error}");
                        }
                    }
                }
            }
        }
        hang_ups.retain(|(_, _, hang_up)| hang_up.next_at().is_some());
    }

    /// Closes a pipe session's stdin ([`close_stdin`]).
    fn close_input(&mut self, id: SessionId) {
        if let Some(session) = self.sessions.get_mut(&id) {
            close_stdin(&mut self.ev, &mut self.sources, session);
        }
    }

    fn read_output(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let mode = session.held.mode;
        let Some(io) = &mut session.io else {
            return;
        };
        let mut buffer = [0u8; OUTPUT_CHUNK_SIZE];
        for _ in 0..READS_PER_EVENT {
            match session.output {
                Output::Open => {}
                Output::Closed => break,
            }
            let dest = match core_for(&mut self.core, mode) {
                Some(core) => {
                    core.over = core_backlog(core.over, core.conn.queued());
                    Dest::Core { over: core.over }
                }
                // Nothing is lost while the core is away (or isn't shown
                // the session): the program blocks once its tail is full.
                None => Dest::Tail {
                    room: session.tail.room(),
                },
            };
            let limit = read_limit(dest, session.paused, buffer.len());
            if limit == 0 {
                break;
            }
            match io.read_output(&mut buffer[..limit]) {
                // The program's side is closed.
                Ok(0) => {
                    session.output = Output::Closed;
                    break;
                }
                Ok(read) => {
                    let bytes = &buffer[..read];
                    session.tail.push(bytes);
                    session.last_output = Instant::now();
                    if let Some(core) = core_for(&mut self.core, mode) {
                        queue(
                            &mut core.conn,
                            ToCore::Output(Data {
                                session: id,
                                bytes: bytes.to_vec(),
                            }),
                        );
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    log!("session {id}: reading output failed: {error}");
                    session.output = Output::Closed;
                    break;
                }
            }
        }
    }

    fn write_input(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let Some(io) = &mut session.io else {
            return;
        };
        let mut failed = false;
        while !session.input.is_empty() && io.takes_input() {
            let (front, _) = session.input.as_slices();
            match io.write_input(front) {
                Ok(written) => {
                    session.input.drain(..written);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    log!("session {id}: writing input failed: {error}");
                    session.input.clear();
                    failed = true;
                    break;
                }
            }
        }
        // A pipe session's stdin that failed (its program closed its end)
        // takes nothing more.
        if failed {
            match session.held.mode {
                SpawnMode::Pipes => self.close_input(id),
                SpawnMode::Pty => {}
            }
        }
    }

    /// Reads a pipe session's stderr into its ring, whatever its stdout
    /// and the core are doing, so the program never blocks on it.
    fn read_errors(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let Some(Io::Pipes(pipes)) = &mut session.io else {
            return;
        };
        let mut buffer = [0u8; OUTPUT_CHUNK_SIZE];
        let mut ended = false;
        for _ in 0..READS_PER_EVENT {
            match pipes.piped.read_error(&mut buffer) {
                Ok(0) => {
                    ended = true;
                    break;
                }
                Ok(read) => {
                    session.stderr.push(&buffer[..read]);
                    session.last_output = Instant::now();
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    log!("session {id}: reading stderr failed: {error}");
                    ended = true;
                    break;
                }
            }
        }
        if ended && let Some(token) = pipes.error.take() {
            let _ = self.ev.deregister(token);
            self.sources.remove(&token);
        }
    }

    fn read_core_ready(&mut self, token: Token) {
        let Some(ready) = self.core_ready.get_mut(&token) else {
            return;
        };
        match ready.read() {
            ReadyRead::Pending => return,
            ReadyRead::Listening => log!("the core it started is listening"),
            ReadyRead::Ended => log!("the core it started exited before listening"),
            ReadyRead::Failed(error) => log!(
                "the core it started didn't say it was listening (reading the ready pipe failed: {error})"
            ),
        }
        self.core_ready.remove(&token);
        self.drop_source(token);
    }

    fn remove_session(&mut self, id: SessionId) {
        if let Some(mut session) = self.sessions.remove(&id) {
            close_io(&mut self.ev, &mut self.sources, &mut session);
        }
    }

    /// Reaps exited programs and started cores; run after every wake-up
    /// (a child's exit wakes the loop).
    fn reap(&mut self) {
        let now = Instant::now();
        for (id, session) in &mut self.sessions {
            match session.life {
                Life::Running => {}
                Life::Reaped { .. } | Life::Finished => continue,
            }
            match session.program.try_wait() {
                Ok(Some(exit)) => {
                    match (exit.code, exit.signal) {
                        (Some(code), _) => log!("session {id}: exited with {code}"),
                        (None, signal) => log!("session {id}: killed by signal {signal:?}"),
                    }
                    session.life = Life::Reaped {
                        code: exit.code,
                        at: now,
                    };
                }
                Ok(None) => {}
                Err(error) => {
                    log!("session {id}: can't wait for its program: {error}");
                    session.life = Life::Reaped { code: None, at: now };
                }
            }
        }
        self.core_starts
            .retain_mut(|child| matches!(child.try_wait(), Ok(None)));
    }

    fn tick(&mut self, now: Instant) {
        // A reaped program's session ends once its output is drained.
        let mut finished = Vec::new();
        for (&id, session) in &mut self.sessions {
            let (code, reaped_at) = match session.life {
                Life::Reaped { code, at } => (code, at),
                Life::Running | Life::Finished => continue,
            };
            // A hang-up, which closes the PTY, closes its output too. A
            // pipe session's output is its stdout and its stderr.
            if policy::drained(now, reaped_at, session.last_output, streams(session)) {
                session.life = Life::Finished;
                session.held.exit = Some(ExitStatus { code });
                close_io(&mut self.ev, &mut self.sources, session);
                finished.push((id, code, session.held.mode));
            }
        }
        for (id, code, mode) in finished {
            // Without a core, with one that isn't shown the session, or
            // with one that said GOODBYE (it may no longer read), the next
            // one's WELCOME reports it.
            let staying = core_for(&mut self.core, mode).is_some_and(|core| match core.farewell {
                Farewell::Staying => true,
                Farewell::Goodbye | Farewell::Handover => false,
            });
            if !staying {
                log!("session {id}: ended with no core to tell; kept for the next one");
                continue;
            }
            match mode {
                // What the program said on stderr explains its exit.
                SpawnMode::Pipes => {
                    let bytes = self
                        .sessions
                        .get(&id)
                        .map(|session| session.stderr.to_vec())
                        .unwrap_or_default();
                    self.send(ToCore::StderrTail(Data { session: id, bytes }));
                }
                SpawnMode::Pty => {}
            }
            self.send(ToCore::Exited(Exited { session: id, code }));
            self.remove_session(id);
        }
        self.take_hang_up_steps(now);

        self.kills.retain(|(id, group, at)| {
            if now < *at {
                return true;
            }
            // The reaped leader's children that ignored the hang-up are
            // still in the group, so it is killed as long as it exists.
            if let Err(error) = group.kill_remaining() {
                log!("session {id}: can't end what its hang-up left running: {error}");
            }
            false
        });

        let expired: Vec<Token> = self
            .pending
            .iter()
            .filter(|(_, pending)| now.duration_since(pending.since) >= HELLO_LIMIT)
            .map(|(&token, _)| token)
            .collect();
        for token in expired {
            self.pending.remove(&token);
            self.drop_source(token);
        }

        let needed = self.core.is_none() && !self.sessions.is_empty();
        if self.core_start.step(now, needed) {
            self.start_core();
        }
    }

    /// Starts a core from this holder's own binary, detached
    /// ([`daemon::start_role`]).
    fn start_core(&mut self) {
        let started = daemon::start_role("serve", &self.core_args, &[], |reason| {
            log!("starting the core without a scope of its own: {reason}")
        });
        let ready_read = match started {
            Ok(started) => {
                log!(
                    "no core for a while; started {} {} as pid {}",
                    started.program.to_string_lossy(),
                    self.core_args.join(" "),
                    started.child.id()
                );
                self.core_starts.push(started.child);
                started.ready
            }
            Err(error) => return log!("can't start a core: {error}"),
        };
        let token = self.token();
        if self
            .ev
            .register(&ready_read.source(), token, Interest::READABLE)
            .is_ok()
        {
            self.sources.insert(token, Source::CoreReady);
            self.core_ready.insert(token, ready_read);
        }
    }

    fn update_interests(&mut self) {
        self.flush_core();
        let core_over = self.core.as_mut().map(|core| {
            core.over = core_backlog(core.over, core.conn.queued());
            (core.over, core.protocol)
        });
        for session in self.sessions.values_mut() {
            let Some(io) = &mut session.io else {
                continue;
            };
            let shown_over = core_over
                .filter(|&(_, protocol)| policy::shown(protocol, session.held.mode))
                .map(|(over, _)| over);
            let dest = match shown_over {
                Some(over) => Dest::Core { over },
                None => Dest::Tail {
                    room: session.tail.room(),
                },
            };
            // A pipe session's stdin waits to be written to on its own.
            let input_queued = !session.input.is_empty();
            let pty_writable = match io {
                Io::Pty(_) => input_queued,
                Io::Pipes(pipes) => {
                    if let Some((token, interest)) = &mut pipes.input {
                        let want = Interest {
                            readable: false,
                            writable: input_queued,
                        };
                        if want != *interest && self.ev.modify(*token, want).is_ok() {
                            *interest = want;
                        }
                    }
                    false
                }
            };
            let readable = match session.output {
                Output::Open => read_limit(dest, session.paused, 1) > 0,
                Output::Closed => false,
            };
            let want = Interest {
                readable,
                writable: pty_writable,
            };
            if want != session.interest && self.ev.modify(session.token, want).is_ok() {
                session.interest = want;
            }
        }
        if let Some(core) = self.core.as_mut() {
            let want = Interest {
                readable: true,
                writable: core.conn.queued() > 0,
            };
            if want != core.interest && self.ev.modify(core.token, want).is_ok() {
                core.interest = want;
            }
        }
    }

    fn should_exit(&self, now: Instant) -> bool {
        self.sessions.is_empty()
            && self.kills.is_empty()
            && self.hang_ups.is_empty()
            && (self.had_session
                || self.draining
                || (self.core.is_none() && (self.spawn_failed || now >= self.started + UNUSED_LIMIT)))
    }

    /// Exits. The core gets everything queued for it (the last session's
    /// output and `EXITED`) and the connection's end, and the holder waits
    /// for the core to close its side, reading what it still sends: closing
    /// with unread input would reset the connection and lose the queued
    /// messages in the core's socket buffer.
    fn shutdown(mut self) {
        // Nothing new can find this holder.
        let _ = endpoint::remove(&self.slot.endpoint);
        let _ = fs::remove_file(&self.slot.lock_path);
        if let Some(core) = self.core.as_mut() {
            let queued = core.conn.queued();
            let started = Instant::now();
            match core.conn.close_for(EXIT_CLOSE_LIMIT) {
                Ok(()) => log!(
                    "delivered the last {queued} bytes to core instance {} in {} ms",
                    core.instance,
                    started.elapsed().as_millis()
                ),
                Err(error) => log!(
                    "core instance {} didn't take the last messages within {}s: {error}",
                    core.instance,
                    EXIT_CLOSE_LIMIT.as_secs()
                ),
            }
        }
        if self.sessions.is_empty() {
            log!("no sessions left; exiting");
        } else {
            log!("exiting with {} session(s)", self.sessions.len());
        }
    }
}

/// Closes `session`'s streams, if they are open, and stops watching them.
fn close_io<L: EventLoop>(ev: &mut L, sources: &mut HashMap<Token, Source>, session: &mut Session) {
    if let Some(io) = session.io.take() {
        for token in io.tokens(session.token) {
            let _ = ev.deregister(token);
            sources.remove(&token);
        }
    }
}

/// Closes a pipe session's stdin, and drops the input queued for it.
fn close_stdin<L: EventLoop>(ev: &mut L, sources: &mut HashMap<Token, Source>, session: &mut Session) {
    session.input.clear();
    let Some(Io::Pipes(pipes)) = &mut session.io else {
        return;
    };
    if let Some((token, _)) = pipes.input.take() {
        let _ = ev.deregister(token);
        sources.remove(&token);
    }
    pipes.piped.close_input();
}

fn no_session(id: SessionId) -> String {
    format!("no session {id}")
}

fn queue(conn: &mut Conn, message: ToCore) {
    match message.encode() {
        Ok(frame) => conn.queue(&frame),
        Err(error) => log!("can't encode a message: {error}"),
    }
}

/// Closes a refused connection gracefully, within [`REFUSAL_CLOSE_LIMIT`]:
/// the peer reads its `ERROR`, then the end, even with bytes of its own
/// still unread, which closing at once would turn into a reset.
fn refuse(conn: &mut Conn) {
    if let Err(error) = conn.close_for(REFUSAL_CLOSE_LIMIT) {
        log!("a refused connection didn't close in time: {error}");
    }
}

/// Whether the core's queue is over its marks, given whether it was: it
/// goes over at [`CORE_QUEUE_HIGH`] and stays over until it is under
/// [`CORE_QUEUE_LOW`]. (Brindle's `QueueMarks` rule; the helper can't share
/// the app's code.)
fn core_backlog(over: bool, queued: usize) -> bool {
    match over {
        true => queued >= CORE_QUEUE_LOW,
        false => queued >= CORE_QUEUE_HIGH,
    }
}

/// Where a session's output goes when the holder reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dest {
    /// To the connected core, whose queue is or isn't over its marks.
    Core { over: bool },
    /// Into the tail only, which has `room` bytes left: there is no core.
    Tail { room: usize },
}

/// How much of a session's output to read now, in reads of at most `chunk`
/// bytes: nothing for a core whose queue is over its marks or that paused
/// the session, at most the tail's room without a core. Past those, the
/// program blocks on its output instead of losing it.
fn read_limit(dest: Dest, paused: bool, chunk: usize) -> usize {
    match dest {
        Dest::Core { over } if over || paused => 0,
        Dest::Core { .. } => chunk,
        // Only a core pauses, and the pause goes with it.
        Dest::Tail { room } => room.min(chunk),
    }
}

/// What a message from the core is about: the unit a fault while handling
/// it is confined to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    /// Starting this session.
    Spawn(SessionId),
    /// A running session.
    Session(SessionId),
    /// A checkpoint being assembled for this session.
    Checkpoint(SessionId),
    /// The holder or the connection.
    Holder,
}

fn unit_of(message: &ToHolder) -> Unit {
    match message {
        ToHolder::Spawn(spawn) => Unit::Spawn(spawn.session),
        ToHolder::Input(data) => Unit::Session(data.session),
        ToHolder::Resize(resize) => Unit::Session(resize.session),
        ToHolder::Hangup(target) | ToHolder::Pause(target) | ToHolder::Resume(target) => {
            Unit::Session(target.session)
        }
        ToHolder::Reparent(reparent) => Unit::Session(reparent.session),
        ToHolder::SetMeta(update) => Unit::Session(update.session),
        ToHolder::SpawnPipes(spawn) => Unit::Spawn(spawn.session),
        ToHolder::CheckpointBegin(begin) | ToHolder::CheckpointAppend(begin) => {
            Unit::Checkpoint(begin.session)
        }
        ToHolder::CheckpointChunk(chunk) => Unit::Checkpoint(chunk.session),
        ToHolder::CheckpointEnd(end) => Unit::Checkpoint(end.session),
        ToHolder::Hello(_) | ToHolder::Drain(_) | ToHolder::Goodbye(_) => Unit::Holder,
    }
}

/// A message's name, as the protocol's table gives it.
fn name_of(message: &ToHolder) -> &'static str {
    match message {
        ToHolder::Hello(_) => "HELLO",
        ToHolder::Spawn(_) => "SPAWN",
        ToHolder::Input(_) => "INPUT",
        ToHolder::Resize(_) => "RESIZE",
        ToHolder::Hangup(_) => "HANGUP",
        ToHolder::CheckpointBegin(_) => "CHECKPOINT_BEGIN",
        ToHolder::CheckpointChunk(_) => "CHECKPOINT_CHUNK",
        ToHolder::CheckpointEnd(_) => "CHECKPOINT_END",
        ToHolder::Drain(_) => "DRAIN",
        ToHolder::Goodbye(_) => "GOODBYE",
        ToHolder::Reparent(_) => "REPARENT",
        ToHolder::Pause(_) => "PAUSE",
        ToHolder::Resume(_) => "RESUME",
        ToHolder::SetMeta(_) => "SET_META",
        ToHolder::SpawnPipes(_) => "SPAWN_PIPES",
        ToHolder::CheckpointAppend(_) => "CHECKPOINT_APPEND",
    }
}

/// Panics, in a debug build, when `bytes` hold `BRINDLE_TEST_HOLDER_PANIC`'s
/// marker: a fault the tests inject.
fn inject_fault(bytes: &[u8]) {
    static MARKER: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    if let Some(marker) = fault::marker(fault::HOLDER_PANIC, &MARKER)
        && fault::holds(bytes, marker)
    {
        panic!("{} matched", fault::HOLDER_PANIC);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_keeps_the_newest_bytes() {
        let mut tail = Tail::default();
        tail.push(b"abc");
        assert_eq!(
            (tail.to_vec(), tail.truncated, tail.room()),
            (b"abc".to_vec(), false, Tail::LIMIT - 3)
        );
        tail.push(&vec![b'x'; Tail::LIMIT - 3]);
        assert!(!tail.truncated);
        assert_eq!(tail.room(), 0);
        tail.push(b"yz");
        assert!(tail.truncated);
        let bytes = tail.to_vec();
        assert_eq!(bytes.len(), Tail::LIMIT);
        assert!(bytes.starts_with(b"cx") && bytes.ends_with(b"xyz"));
        tail.push(&vec![b'q'; Tail::LIMIT + 5]);
        assert_eq!(tail.to_vec(), vec![b'q'; Tail::LIMIT]);
        tail.cover(u64::MAX);
        assert_eq!((tail.to_vec().len(), tail.truncated), (0, false));
    }

    #[test]
    fn a_checkpoint_drops_only_what_it_covers() {
        let mut tail = Tail::default();
        tail.push(b"seen by the core");
        tail.push(b", still on its way");
        tail.cover(16);
        assert_eq!(tail.to_vec(), b", still on its way");
        assert!(!tail.truncated);

        // After a new core got the tail, it counts from its start.
        tail.sent();
        tail.push(b"!");
        tail.cover(18);
        assert_eq!(tail.to_vec(), b"!");

        // Uncovered output that overflowed the tail is lost.
        tail.push(&vec![b'x'; Tail::LIMIT + 10]);
        tail.cover(1);
        assert_eq!(tail.to_vec().len(), Tail::LIMIT);
        assert!(tail.truncated);
        tail.cover(10);
        assert!(!tail.truncated, "what is left follows the checkpoint");
        // Once a new core got a truncated tail, the next checkpoint
        // clears the flag.
        tail.push(&vec![b'y'; Tail::LIMIT]);
        assert!(tail.truncated);
        tail.sent();
        assert!(tail.truncated, "until a checkpoint, the tail is still cut");
        tail.cover(5);
        assert!(!tail.truncated);
        assert_eq!(tail.to_vec().len(), Tail::LIMIT - 5);
    }

    #[test]
    fn the_stderr_ring_keeps_the_newest_bytes() {
        let limit = StderrRing::LIMIT;
        let mut ring = StderrRing::default();
        ring.push(b"abc");
        assert_eq!(ring.to_vec(), b"abc");
        ring.push(&vec![b'x'; limit - 3]);
        assert_eq!(ring.to_vec().len(), limit);
        assert!(ring.to_vec().starts_with(b"abc"));
        ring.push(b"yz");
        let bytes = ring.to_vec();
        assert_eq!(bytes.len(), limit);
        assert!(bytes.starts_with(b"cx") && bytes.ends_with(b"xyz"));
        // One read longer than the ring keeps its own end.
        let long: Vec<u8> = (0..limit + 100).map(|i| i as u8).collect();
        ring.push(&long);
        assert_eq!(ring.to_vec(), &long[100..]);
    }

    #[test]
    fn a_pipe_sessions_output_is_open_while_stdout_or_stderr_is() {
        assert!(matches!(
            streams_of(Output::Closed, Some(true)),
            Output::Open
        ));
        assert!(matches!(
            streams_of(Output::Open, Some(false)),
            Output::Open
        ));
        assert!(matches!(
            streams_of(Output::Closed, Some(false)),
            Output::Closed
        ));
        // A PTY's (`None`) is its own.
        assert!(matches!(streams_of(Output::Closed, None), Output::Closed));
        assert!(matches!(streams_of(Output::Open, None), Output::Open));
    }

    #[test]
    fn core_backlog_resumes_only_under_the_low_mark() {
        assert!(!core_backlog(false, 0));
        assert!(!core_backlog(false, CORE_QUEUE_HIGH - 1));
        // Over at the high mark.
        assert!(core_backlog(false, CORE_QUEUE_HIGH));
        // Over, it stays over down to the low mark itself...
        assert!(core_backlog(true, CORE_QUEUE_HIGH - 1));
        assert!(core_backlog(true, CORE_QUEUE_LOW));
        // ...and resumes under it.
        assert!(!core_backlog(true, CORE_QUEUE_LOW - 1));
        // Under, the low mark alone doesn't stop it.
        assert!(!core_backlog(false, CORE_QUEUE_LOW));
    }

    #[test]
    fn read_limit_is_zero_for_a_core_over_its_mark_or_a_paused_session() {
        let chunk = 4096;
        assert_eq!(read_limit(Dest::Core { over: false }, false, chunk), chunk);
        assert_eq!(read_limit(Dest::Core { over: true }, false, chunk), 0);
        assert_eq!(read_limit(Dest::Core { over: false }, true, chunk), 0);
        assert_eq!(read_limit(Dest::Core { over: true }, true, chunk), 0);
    }

    #[test]
    fn read_limit_without_a_core_is_the_tails_room() {
        let chunk = 4096;
        assert_eq!(read_limit(Dest::Tail { room: 100 }, false, chunk), 100);
        let roomy = Dest::Tail { room: 1 << 20 };
        assert_eq!(read_limit(roomy, false, chunk), chunk);
        assert_eq!(read_limit(Dest::Tail { room: 0 }, false, chunk), 0);
        // A pause is the core's, so it means nothing without one.
        assert_eq!(read_limit(Dest::Tail { room: 100 }, true, chunk), 100);
    }

    #[test]
    fn every_message_has_its_unit() {
        use brindle_remote_protocol::holder::{
            BlobBegin, CheckpointEnd, Drain, Goodbye, Reparent, Resize, SessionRef, SetMeta,
        };

        let data = |session| Data {
            session,
            bytes: Vec::new(),
        };
        let target = |session| SessionRef { session };
        let spawn = Spawn {
            session: 1,
            command: None,
            args: Vec::new(),
            cwd: None,
            env: Default::default(),
            cols: 80,
            rows: 24,
            cell_w: 0,
            cell_h: 0,
            meta: Default::default(),
        };
        let cases = [
            (ToHolder::Spawn(spawn), Unit::Spawn(1), "SPAWN"),
            (ToHolder::Input(data(2)), Unit::Session(2), "INPUT"),
            (
                ToHolder::Resize(Resize {
                    session: 3,
                    cols: 80,
                    rows: 24,
                    cell_w: 0,
                    cell_h: 0,
                }),
                Unit::Session(3),
                "RESIZE",
            ),
            (ToHolder::Hangup(target(4)), Unit::Session(4), "HANGUP"),
            (
                ToHolder::Reparent(Reparent {
                    session: 5,
                    hwnd: 1,
                }),
                Unit::Session(5),
                "REPARENT",
            ),
            (ToHolder::Pause(target(6)), Unit::Session(6), "PAUSE"),
            (ToHolder::Resume(target(7)), Unit::Session(7), "RESUME"),
            (
                ToHolder::SetMeta(SetMeta {
                    session: 8,
                    meta: Default::default(),
                }),
                Unit::Session(8),
                "SET_META",
            ),
            (
                ToHolder::CheckpointBegin(BlobBegin { session: 9, len: 0 }),
                Unit::Checkpoint(9),
                "CHECKPOINT_BEGIN",
            ),
            (
                ToHolder::CheckpointChunk(data(10)),
                Unit::Checkpoint(10),
                "CHECKPOINT_CHUNK",
            ),
            (
                ToHolder::CheckpointEnd(CheckpointEnd {
                    session: 11,
                    covers: 0,
                }),
                Unit::Checkpoint(11),
                "CHECKPOINT_END",
            ),
            (
                ToHolder::Hello(Hello {
                    min: 1,
                    max: 2,
                    core_build: String::new(),
                    instance: 1,
                }),
                Unit::Holder,
                "HELLO",
            ),
            (
                ToHolder::SpawnPipes(h1::SpawnPipes {
                    session: 12,
                    command: "agent".into(),
                    args: Vec::new(),
                    cwd: None,
                    env: Default::default(),
                    meta: Default::default(),
                }),
                Unit::Spawn(12),
                "SPAWN_PIPES",
            ),
            (ToHolder::Drain(Drain {}), Unit::Holder, "DRAIN"),
            (
                ToHolder::Goodbye(Goodbye { handover: true }),
                Unit::Holder,
                "GOODBYE",
            ),
        ];
        for (message, unit, name) in &cases {
            assert_eq!((unit_of(message), name_of(message)), (*unit, *name));
        }
    }

    #[test]
    fn slots_take_the_first_free_index() {
        let temp = crate::server::dir::tests::TempDir::new("slots");
        let endpoints = Endpoints::of(&temp.0).unwrap();
        let a = Slot::claim(&temp.0, "b1").unwrap();
        let b = Slot::claim(&temp.0, "b1").unwrap();
        let other = Slot::claim(&temp.0, "b2").unwrap();
        assert_eq!(a.endpoint, endpoints.holder("b1", 0).endpoint);
        assert_eq!(b.endpoint, endpoints.holder("b1", 1).endpoint);
        assert!(b.log_path.ends_with("holder-b1-1.log"));
        assert!(other.lock_path.ends_with("holder-b2-0.lock"));
        assert_eq!(other.endpoint, endpoints.holder("b2", 0).endpoint);
        drop(a);
        let c = Slot::claim(&temp.0, "b1").unwrap();
        assert_eq!(
            c.endpoint,
            endpoints.holder("b1", 0).endpoint,
            "a released index is reused"
        );
        // A lock file unlinked by an exiting holder doesn't count.
        fs::remove_file(&b.lock_path).unwrap();
        let d = Slot::claim(&temp.0, "b1").unwrap();
        assert_eq!(d.endpoint, endpoints.holder("b1", 1).endpoint);
        drop(b);
    }
}
