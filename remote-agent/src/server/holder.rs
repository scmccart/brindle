//! `brindle-remote hold`: the holder, which owns sessions' PTYs for the
//! terminal server's core.
//!
//! A holder is the programs' parent. It pumps their I/O, resizes, signals
//! and reaps them, and parses nothing. Per session it keeps the core's
//! latest checkpoint and the raw output since then (the tail), as bytes it
//! doesn't interpret, so a new core can rebuild the screens.
//!
//! - **Socket:** `holder-<build>-<n>.sock` in the server directory, beside
//!   `holder-<build>-<n>.lock`, which it flocks for its whole life (a
//!   socket path can't be opened to flock), and its log.
//! - **One core at a time:** a newer `HELLO` takes the holder over from the
//!   connected core. `WELCOME` lists the sessions, then each one's
//!   checkpoint and tail follow.
//! - **Backpressure:** past [`CORE_QUEUE_HIGH`] queued to the core, it stops
//!   reading PTYs, and it doesn't read a session the core has paused. With
//!   no core, it reads a session until its tail is full, so the program
//!   blocks instead of losing output.
//! - **Supervision:** with sessions but no core for [`CORE_GRACE`], it
//!   starts `serve` from `/proc/self/exe` (which survives the file being
//!   replaced), retrying with backoff up to [`CORE_BACKOFF_MAX`].
//! - **Exit:** once it has no sessions (after having one, after `DRAIN`, or
//!   when no core used it within [`UNUSED_LIMIT`] of starting).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixListener;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use brindle_remote_protocol::{SessionId, scope};
use brindle_remote_protocol::holder::{
    self as h1, Assembler, Data, ErrorMessage, ExitStatus, Exited, HeldSession, Hello, Previous,
    Spawn, SpawnFailed, Spawned, ToCore, ToHolder, Welcome,
};

use super::conn::Conn;
use super::event_loop::{Event, EventLoop, Interest, PollLoop, Token};
use super::{build_id, daemon, dir, peer};
use crate::pty::{
    self, GRACEFUL_SHUTDOWN, OUTPUT_CHUNK_SIZE, OUTPUT_DRAIN_IDLE, OUTPUT_DRAIN_LIMIT, PtyCommand,
};

/// Past this much queued to the core, the holder stops reading PTYs.
pub const CORE_QUEUE_HIGH: usize = 8 * 1024 * 1024;
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
/// A connection must send `HELLO` within this long.
const HELLO_LIMIT: Duration = Duration::from_secs(5);
const MAX_PENDING: usize = 16;
/// How long an exiting holder waits for the core to take its last messages
/// (a flood's output, then `EXITED`) and close the connection.
const EXIT_CLOSE_LIMIT: Duration = Duration::from_secs(10);
const READS_PER_EVENT: usize = 4;
const TICK_FAST: Duration = Duration::from_millis(50);
const TICK_SLOW: Duration = Duration::from_millis(500);

/// The core the holder starts is run from its own binary
/// (`/proc/<pid>/exe`, see [`scope::command`]). Tests replace the arguments
/// with `BRINDLE_TEST_HOLDER_CORE_ARGS` (whitespace-separated); `{ready_fd}`
/// stands for the ready pipe's fd number.
const DEFAULT_CORE_ARGS: &[&str] = &["serve", "--ready-fd", "{ready_fd}"];
const CORE_ARGS_ENV: &str = "BRINDLE_TEST_HOLDER_CORE_ARGS";
/// The fd the started core's ready pipe gets.
const CORE_READY_FD: libc::c_int = 3;

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

/// Runs `brindle-remote hold [--ready-fd N]`. Without `--ready-fd` it stays
/// in the foreground and logs to stderr.
pub fn run(args: &[String]) -> Result<(), String> {
    let ready_fd = daemon::parse_ready_fd("hold", args)?;
    let ready = ready_fd
        .map(daemon::Ready::from_fd)
        .transpose()
        .map_err(|error| error.to_string())?;
    let dir = dir::from_env().map_err(|error| error.to_string())?;
    let build = build_id::current().map_err(|error| format!("can't read the build id: {error}"))?;
    let slot = Slot::claim(&dir, &build).map_err(|error| error.to_string())?;
    if ready.is_some() {
        let log = daemon::open_log(&slot.log_path)
            .map_err(|error| format!("can't open {}: {error}", slot.log_path.display()))?;
        daemon::daemonize(&log).map_err(|error| error.to_string())?;
    }
    let core_args = std::env::var(CORE_ARGS_ENV)
        .ok()
        .map(|args| args.split_whitespace().map(String::from).collect())
        .unwrap_or_else(|| {
            DEFAULT_CORE_ARGS
                .iter()
                .map(|arg| arg.to_string())
                .collect()
        });
    let ev = PollLoop::new().map_err(|error| error.to_string())?;
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

/// The holder's place in the server directory: index `n`, its flocked lock
/// file, socket and log.
struct Slot {
    _lock: File,
    lock_path: PathBuf,
    socket_path: PathBuf,
    log_path: PathBuf,
}

impl Slot {
    /// Takes the first index whose lock file no live holder of this build
    /// holds.
    fn claim(dir: &Path, build: &str) -> io::Result<Self> {
        let mut index = 0u32;
        let mut attempts = 0;
        while index < 1024 && attempts < 4096 {
            attempts += 1;
            let name = format!("holder-{build}-{index}");
            let lock_path = dir.join(format!("{name}.lock"));
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(&lock_path)?;
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == -1 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::WouldBlock {
                    return Err(error);
                }
                index += 1;
                continue;
            }
            // An exiting holder unlinks its lock file; if that happened
            // between the open and the flock, this lock is on a dead inode.
            let locked = lock.metadata()?;
            match fs::metadata(&lock_path) {
                Ok(now) if now.ino() == locked.ino() && now.dev() == locked.dev() => {}
                _ => continue,
            }
            let socket_path = dir.join(format!("{name}.sock"));
            // sun_path holds 108 bytes with the terminating NUL.
            if socket_path.as_os_str().len() > 107 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "the socket path {} is too long for a Unix socket; set a shorter BRINDLE_SERVER_DIR",
                        socket_path.display()
                    ),
                ));
            }
            return Ok(Self {
                _lock: lock,
                lock_path,
                socket_path,
                log_path: dir.join(format!("{name}.log")),
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

struct Session {
    /// The entry `WELCOME` reports; `exit` is set once the program has
    /// exited and its output is drained.
    held: HeldSession,
    child: Child,
    /// Closed on hang-up and once the session is finished.
    master: Option<OwnedFd>,
    token: Token,
    interest: Interest,
    checkpoint: Vec<u8>,
    tail: Tail,
    input: VecDeque<u8>,
    /// The program was reaped, with its exit code (`None` for a signal).
    reaped: Option<(Option<i32>, Instant)>,
    last_output: Instant,
    output_closed: bool,
    /// The connected core sent `PAUSE`: its client isn't keeping up.
    paused: bool,
}

struct Core {
    conn: Conn,
    token: Token,
    interest: Interest,
    instance: u64,
    checkpoints: Assembler,
    /// It sent `GOODBYE { handover: true }`.
    handover: bool,
    /// It sent `GOODBYE`: it is going, so exits wait for the next core.
    goodbye: bool,
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
    Master(SessionId),
    CoreReady,
}

const LISTENER: Token = 0;

struct Holder<L: EventLoop> {
    ev: L,
    build: String,
    slot: Slot,
    listener: UnixListener,
    uid: u32,
    sources: HashMap<Token, Source>,
    next_token: Token,
    core: Option<Core>,
    pending: HashMap<Token, Pending>,
    sessions: BTreeMap<SessionId, Session>,
    previous: Previous,
    draining: bool,
    had_session: bool,
    started: Instant,
    next_core_start: Option<Instant>,
    backoff: Duration,
    core_args: Vec<String>,
    core_starts: Vec<Child>,
    core_ready: HashMap<Token, OwnedFd>,
    /// Process groups to `SIGKILL` when their hang-up grace ends, with the
    /// session whose program leads each one.
    kills: Vec<(SessionId, libc::pid_t, Instant)>,
}

impl<L: EventLoop> Holder<L> {
    fn new(mut ev: L, build: String, slot: Slot, core_args: Vec<String>) -> io::Result<Self> {
        // The lock is ours, so any socket at this path is stale.
        match fs::remove_file(&slot.socket_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&slot.socket_path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("can't listen on {}: {error}", slot.socket_path.display()),
            )
        })?;
        listener.set_nonblocking(true)?;
        ev.register(listener.as_raw_fd(), LISTENER, Interest::READABLE)?;
        ev.waker().wake_on_signal(libc::SIGCHLD)?;
        log!(
            "listening on {} (build {build})",
            slot.socket_path.display()
        );
        Ok(Self {
            ev,
            build,
            slot,
            listener,
            uid: dir::own_uid(),
            sources: HashMap::from([(LISTENER, Source::Listener)]),
            next_token: LISTENER + 1,
            core: None,
            pending: HashMap::new(),
            sessions: BTreeMap::new(),
            previous: Previous::None,
            draining: false,
            had_session: false,
            started: Instant::now(),
            next_core_start: None,
            backoff: CORE_BACKOFF_START,
            core_args,
            core_starts: Vec::new(),
            core_ready: HashMap::new(),
            kills: Vec::new(),
        })
    }

    fn run(mut self) -> io::Result<()> {
        let mut events: Vec<Event> = Vec::new();
        let result = loop {
            self.update_interests();
            let fast = !self.kills.is_empty()
                || self
                    .sessions
                    .values()
                    .any(|session| session.reaped.is_some() && session.held.exit.is_none());
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
            Some(Source::CoreReady) => self.read_core_ready(event.token),
            None => {}
        }
    }

    fn accept(&mut self) {
        loop {
            // The peer check comes before reading anything.
            let stream = match peer::accept(&self.listener, self.uid) {
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
            if let Err(error) = self.ev.register(conn.fd(), token, Interest::READABLE) {
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
                        let _ = conn.flush();
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
                let _ = conn.flush_for(Duration::from_secs(1));
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
            checkpoints: Assembler::new(h1::MAX_CHECKPOINT_LEN),
            handover: false,
            goodbye: false,
        };
        let sessions = self
            .sessions
            .values()
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
            for message in ToCore::checkpoint(id, &session.checkpoint) {
                queue(&mut core.conn, message);
            }
            for message in ToCore::tail(id, &session.tail.to_vec()) {
                queue(&mut core.conn, message);
            }
            session.tail.sent();
        }
        // The core now knows how these ended.
        let finished: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.held.exit.is_some())
            .map(|(&id, _)| id)
            .collect();
        for id in finished {
            self.remove_session(id);
        }
        self.core = Some(core);
        self.next_core_start = None;
        self.backoff = CORE_BACKOFF_START;
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
        self.previous = if core.handover {
            Previous::Handover
        } else {
            Previous::Crash
        };
        log!(
            "core instance {} left ({reason}); {} session(s) held",
            core.instance,
            self.sessions.len()
        );
        self.next_core_start = Some(Instant::now() + CORE_GRACE);
    }

    fn flush_core(&mut self) {
        if let Some(core) = self.core.as_mut()
            && let Err(error) = core.conn.flush()
        {
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
                Ok(message) => self.handle(message),
                Err(error) => self.send_error(None, error.to_string()),
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
                let Some(master) = &session.master else {
                    return;
                };
                let result = pty::resize(
                    master.as_raw_fd(),
                    resize.cols,
                    resize.rows,
                    resize.cell_w,
                    resize.cell_h,
                );
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
                if let Err(error) = core.checkpoints.begin(begin) {
                    self.send_error(Some(begin.session), error.to_string());
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
                match core.checkpoints.end(end.blob_end()) {
                    Ok(bytes) => {
                        if let Some(session) = self.sessions.get_mut(&end.session) {
                            session.checkpoint = bytes;
                            session.tail.cover(end.covers);
                        }
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
                    core.handover = goodbye.handover;
                    core.goodbye = true;
                }
            }
            ToHolder::Reparent(reparent) => self.send_error(Some(reparent.session), "unsupported"),
            ToHolder::Pause(target) => self.set_paused(target.session, true),
            ToHolder::Resume(target) => self.set_paused(target.session, false),
            // The next core's `WELCOME` carries it. An unknown session may
            // just have ended.
            ToHolder::SetMeta(update) => {
                if let Some(session) = self.sessions.get_mut(&update.session) {
                    session.held.meta = update.meta;
                }
            }
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
        let refusal = if self.draining {
            Some("the holder is draining and takes no new sessions".to_string())
        } else if self.sessions.contains_key(&id) {
            Some(format!("session {id} already exists"))
        } else if spawn.command.as_deref() == Some("") {
            Some("command must not be empty".into())
        } else if spawn.cols == 0 || spawn.rows == 0 {
            Some("PTY rows and columns must be non-zero".into())
        } else {
            None
        };
        if let Some(message) = refusal {
            return self.send(ToCore::SpawnFailed(SpawnFailed {
                session: id,
                message,
            }));
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
        let spawned = match pty::spawn(&request) {
            Ok(spawned) => spawned,
            Err(error) => {
                let message = format!("{}: {error}", request.program().to_string_lossy());
                log!("session {id}: {message}");
                return self.send(ToCore::SpawnFailed(SpawnFailed {
                    session: id,
                    message,
                }));
            }
        };
        let token = self.token();
        if let Err(error) = self
            .ev
            .register(spawned.master.as_raw_fd(), token, Interest::READABLE)
        {
            pty::signal_process_group(spawned.pid, libc::SIGKILL);
            let mut child = spawned.child;
            let _ = child.wait();
            return self.send(ToCore::SpawnFailed(SpawnFailed {
                session: id,
                message: format!("can't watch the PTY: {error}"),
            }));
        }
        self.sources.insert(token, Source::Master(id));
        let pid = spawned.pid as u32;
        log!(
            "session {id}: started {} as pid {pid}",
            request.program().to_string_lossy()
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
                },
                child: spawned.child,
                master: Some(spawned.master),
                token,
                interest: Interest::READABLE,
                checkpoint: Vec::new(),
                tail: Tail::default(),
                input: VecDeque::new(),
                reaped: None,
                last_output: Instant::now(),
                output_closed: false,
                paused: false,
            },
        );
        self.had_session = true;
        self.send(ToCore::Spawned(Spawned { session: id, pid }));
    }

    fn input(&mut self, data: Data) {
        let Some(session) = self.sessions.get_mut(&data.session) else {
            return self.send_error(Some(data.session), no_session(data.session));
        };
        if session.master.is_none() {
            // Hung up: the program is going, so its input is moot.
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

    /// Closes the session's PTY and sends `SIGHUP` to its process group,
    /// then `SIGKILL` after the grace period. Says whether it exists.
    fn hang_up(&mut self, id: SessionId) -> bool {
        let Some(session) = self.sessions.get_mut(&id) else {
            return false;
        };
        if session.master.take().is_some() {
            let _ = self.ev.deregister(session.token);
            self.sources.remove(&session.token);
        }
        session.input.clear();
        session.output_closed = true;
        if session.reaped.is_none() {
            let pid = session.held.pid as libc::pid_t;
            pty::signal_process_group(pid, libc::SIGHUP);
            self.kills
                .push((id, pid, Instant::now() + GRACEFUL_SHUTDOWN));
            log!("session {id}: hung up");
        }
        true
    }

    fn read_output(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let Some(master) = &session.master else {
            return;
        };
        let fd = master.as_raw_fd();
        let mut buffer = [0u8; OUTPUT_CHUNK_SIZE];
        for _ in 0..READS_PER_EVENT {
            if session.output_closed {
                break;
            }
            let limit = match &self.core {
                Some(core) if core.conn.queued() >= CORE_QUEUE_HIGH || session.paused => break,
                Some(_) => buffer.len(),
                // Nothing is lost while the core is away: the program
                // blocks once its tail is full.
                None => session.tail.room().min(buffer.len()),
            };
            if limit == 0 {
                break;
            }
            let read = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), limit) };
            if read > 0 {
                let bytes = &buffer[..read as usize];
                session.tail.push(bytes);
                session.last_output = Instant::now();
                if let Some(core) = self.core.as_mut() {
                    queue(
                        &mut core.conn,
                        ToCore::Output(Data {
                            session: id,
                            bytes: bytes.to_vec(),
                        }),
                    );
                }
                continue;
            }
            if read == 0 {
                session.output_closed = true;
                break;
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EAGAIN) => break,
                Some(libc::EINTR) => continue,
                // Every slave fd is closed.
                Some(libc::EIO) => {
                    session.output_closed = true;
                    break;
                }
                _ => {
                    log!("session {id}: reading output failed: {error}");
                    session.output_closed = true;
                    break;
                }
            }
        }
    }

    fn write_input(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let Some(master) = &session.master else {
            return;
        };
        let fd = master.as_raw_fd();
        while !session.input.is_empty() {
            let (front, _) = session.input.as_slices();
            let written = unsafe { libc::write(fd, front.as_ptr().cast(), front.len()) };
            if written > 0 {
                session.input.drain(..written as usize);
                continue;
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EAGAIN) => break,
                Some(libc::EINTR) => continue,
                _ => {
                    log!("session {id}: writing input failed: {error}");
                    session.input.clear();
                    break;
                }
            }
        }
    }

    fn read_core_ready(&mut self, token: Token) {
        let Some(fd) = self.core_ready.get(&token) else {
            return;
        };
        let mut buffer = [0u8; 16];
        let read = unsafe { libc::read(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
        if read < 0 && io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
            return;
        }
        if read > 0 {
            log!("the core it started is listening");
        } else {
            log!("the core it started exited before listening");
        }
        self.core_ready.remove(&token);
        self.drop_source(token);
    }

    fn remove_session(&mut self, id: SessionId) {
        if let Some(session) = self.sessions.remove(&id)
            && session.master.is_some()
        {
            self.drop_source(session.token);
        }
    }

    /// Reaps exited programs and started cores; run after every wake-up
    /// (`SIGCHLD` wakes the loop).
    fn reap(&mut self) {
        let now = Instant::now();
        for (id, session) in &mut self.sessions {
            if session.reaped.is_some() {
                continue;
            }
            match session.child.try_wait() {
                Ok(Some(status)) => {
                    match (status.code(), status.signal()) {
                        (Some(code), _) => log!("session {id}: exited with {code}"),
                        (None, signal) => log!("session {id}: killed by signal {signal:?}"),
                    }
                    session.reaped = Some((status.code(), now));
                }
                Ok(None) => {}
                Err(error) => {
                    log!("session {id}: can't wait for its program: {error}");
                    session.reaped = Some((None, now));
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
            let Some((code, reaped_at)) = session.reaped else {
                continue;
            };
            if session.held.exit.is_some() {
                continue;
            }
            let quiet_since = reaped_at.max(session.last_output);
            if session.master.is_none()
                || session.output_closed
                || now >= quiet_since + OUTPUT_DRAIN_IDLE
                || now >= reaped_at + OUTPUT_DRAIN_LIMIT
            {
                session.held.exit = Some(ExitStatus { code });
                if session.master.take().is_some() {
                    let _ = self.ev.deregister(session.token);
                    self.sources.remove(&session.token);
                }
                finished.push((id, code));
            }
        }
        for (id, code) in finished {
            // Without a core, or with one that said GOODBYE (it may no
            // longer read), the next one's WELCOME reports it.
            if self.core.as_ref().is_some_and(|core| !core.goodbye) {
                self.send(ToCore::Exited(Exited { session: id, code }));
                self.remove_session(id);
            }
        }

        self.kills.retain(|&(_, pid, at)| {
            if now < at {
                return true;
            }
            // A group id can't be reused while the group has members, the
            // reaped leader's children that ignored SIGHUP included. So the
            // group gets the SIGKILL as long as it still exists.
            if pid > 0 && unsafe { libc::kill(-pid, 0) } == 0 {
                pty::signal_process_group(pid, libc::SIGKILL);
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

        if self.core.is_none() && !self.sessions.is_empty() {
            let due = *self.next_core_start.get_or_insert(now + CORE_GRACE);
            if now >= due {
                self.start_core();
                self.next_core_start = Some(now + self.backoff);
                self.backoff = (self.backoff * 2).min(CORE_BACKOFF_MAX);
            }
        }
    }

    /// Starts a core from this holder's own binary, which `/proc/self/exe`
    /// names even after an install replaced the file.
    fn start_core(&mut self) {
        let args: Vec<OsString> = self
            .core_args
            .iter()
            .map(|arg| arg.replace("{ready_fd}", &CORE_READY_FD.to_string()).into())
            .collect();
        let program = OsString::from(format!("/proc/{}/exe", std::process::id()));
        let decision = scope::current();
        if let scope::Decision::Plain(reason) = &decision {
            log!("starting the core without a scope of its own: {reason}");
        }
        let mut command = scope::command(&decision, "serve", &program, &args);
        let ready_read = match daemon::spawn_with_ready_pipe(&mut command, CORE_READY_FD) {
            Ok((child, ready_read)) => {
                log!(
                    "no core for a while; started {} {} as pid {}",
                    program.to_string_lossy(),
                    self.core_args.join(" "),
                    child.id()
                );
                self.core_starts.push(child);
                ready_read
            }
            Err(error) => return log!("can't start a core: {error}"),
        };
        let token = self.token();
        if self
            .ev
            .register(ready_read.as_raw_fd(), token, Interest::READABLE)
            .is_ok()
        {
            self.sources.insert(token, Source::CoreReady);
            self.core_ready.insert(token, ready_read);
        }
    }

    fn update_interests(&mut self) {
        self.flush_core();
        let queued = self.core.as_ref().map(|core| core.conn.queued());
        for session in self.sessions.values_mut() {
            if session.master.is_none() {
                continue;
            }
            let readable = !session.output_closed
                && match queued {
                    Some(queued) => queued < CORE_QUEUE_HIGH && !session.paused,
                    None => session.tail.room() > 0,
                };
            let want = Interest {
                readable,
                writable: !session.input.is_empty(),
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
            && (self.had_session
                || self.draining
                || (self.core.is_none() && now >= self.started + UNUSED_LIMIT))
    }

    /// Exits. The core gets everything queued for it (the last session's
    /// output and `EXITED`) and the connection's end, and the holder waits
    /// for the core to close its side, reading what it still sends: closing
    /// with unread input would reset the connection and lose the queued
    /// messages in the core's socket buffer.
    fn shutdown(mut self) {
        // Nothing new can find this holder.
        let _ = fs::remove_file(&self.slot.socket_path);
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

fn no_session(id: SessionId) -> String {
    format!("no session {id}")
}

fn queue(conn: &mut Conn, message: ToCore) {
    match message.encode() {
        Ok(frame) => conn.queue(&frame),
        Err(error) => log!("can't encode a message: {error}"),
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
    fn slots_take_the_first_free_index() {
        let temp = crate::server::dir::tests::TempDir::new("slots");
        let a = Slot::claim(&temp.0, "b1").unwrap();
        let b = Slot::claim(&temp.0, "b1").unwrap();
        let other = Slot::claim(&temp.0, "b2").unwrap();
        assert!(a.socket_path.ends_with("holder-b1-0.sock"));
        assert!(b.socket_path.ends_with("holder-b1-1.sock"));
        assert!(b.log_path.ends_with("holder-b1-1.log"));
        assert!(other.lock_path.ends_with("holder-b2-0.lock"));
        drop(a);
        let c = Slot::claim(&temp.0, "b1").unwrap();
        assert!(
            c.socket_path.ends_with("holder-b1-0.sock"),
            "a released index is reused"
        );
        // A lock file unlinked by an exiting holder doesn't count.
        fs::remove_file(&b.lock_path).unwrap();
        let d = Slot::claim(&temp.0, "b1").unwrap();
        assert!(d.socket_path.ends_with("holder-b1-1.sock"));
        drop(b);
    }
}
