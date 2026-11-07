//! `brindle-remote hold`: the holder, which owns sessions' PTYs for the
//! terminal server's core.
//!
//! A holder is the programs' parent. It pumps their I/O, resizes, signals
//! and reaps them, and parses nothing.
//!
//! - **Socket:** `holder-<build>-<n>.sock` in the server directory, beside
//!   `holder-<build>-<n>.lock`, which it flocks for its whole life (a
//!   socket path can't be opened to flock), and its log.
//! - **One core at a time:** a newer `HELLO` takes the holder over from the
//!   connected core. `WELCOME` lists the sessions.
//! - **Exit:** once it has no sessions (after having one, or when no core
//!   used it within [`UNUSED_LIMIT`] of starting).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixListener;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::holder::{
    self as h1, Data, ErrorMessage, ExitStatus, Exited, HeldSession, Hello, Previous, Spawn,
    SpawnFailed, Spawned, ToCore, ToHolder, Welcome,
};

use super::conn::Conn;
use super::event_loop::{Event, EventLoop, Interest, PollLoop, Token};
use super::{build_id, daemon, dir, peer};
use crate::pty::{
    self, GRACEFUL_SHUTDOWN, OUTPUT_CHUNK_SIZE, OUTPUT_DRAIN_IDLE, OUTPUT_DRAIN_LIMIT, PtyCommand,
};

/// A holder that never had a session exits this long after starting, unless
/// a core is connected.
pub const UNUSED_LIMIT: Duration = Duration::from_secs(10);
/// Input queued for one session past this is refused with an `ERROR`.
const INPUT_QUEUE_LIMIT: usize = 4 * 1024 * 1024;
/// A connection must send `HELLO` within this long.
const HELLO_LIMIT: Duration = Duration::from_secs(5);
const MAX_PENDING: usize = 16;
/// How long an exiting holder waits for the core to take its last messages.
const EXIT_FLUSH_LIMIT: Duration = Duration::from_secs(2);
const READS_PER_EVENT: usize = 4;
const TICK_FAST: Duration = Duration::from_millis(50);
const TICK_SLOW: Duration = Duration::from_millis(500);

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
    let ev = PollLoop::new().map_err(|error| error.to_string())?;
    let holder = Holder::new(ev, build, slot).map_err(|error| {
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

struct Session {
    /// The entry `WELCOME` reports; `exit` is set once the program has
    /// exited and its output is drained.
    held: HeldSession,
    child: Child,
    /// Closed on hang-up and once the session is finished.
    master: Option<OwnedFd>,
    token: Token,
    interest: Interest,
    input: VecDeque<u8>,
    /// The program was reaped, with its exit code (`None` for a signal).
    reaped: Option<(Option<i32>, Instant)>,
    last_output: Instant,
    output_closed: bool,
}

struct Core {
    conn: Conn,
    token: Token,
    interest: Interest,
    instance: u64,
    /// It sent `GOODBYE { handover: true }`.
    handover: bool,
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
    had_session: bool,
    started: Instant,
    /// Process groups to `SIGKILL` when their hang-up grace ends.
    kills: Vec<(libc::pid_t, Instant)>,
}

impl<L: EventLoop> Holder<L> {
    fn new(mut ev: L, build: String, slot: Slot) -> io::Result<Self> {
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
            had_session: false,
            started: Instant::now(),
            kills: Vec::new(),
        })
    }

    fn run(mut self) -> io::Result<()> {
        let mut events: Vec<Event> = Vec::new();
        loop {
            self.update_interests();
            let fast = !self.kills.is_empty()
                || self
                    .sessions
                    .values()
                    .any(|session| session.reaped.is_some() && session.held.exit.is_none());
            self.ev
                .wait(&mut events, Some(if fast { TICK_FAST } else { TICK_SLOW }))?;
            for event in &events {
                self.dispatch(*event);
            }
            self.reap();
            let now = Instant::now();
            self.tick(now);
            if self.should_exit(now) {
                break;
            }
        }
        self.shutdown();
        Ok(())
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
            None => {}
        }
    }

    fn accept(&mut self) {
        loop {
            let stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    log!("accept failed: {error}");
                    return;
                }
            };
            // Before reading anything.
            if let Err(error) = peer::check(stream.as_raw_fd(), self.uid) {
                log!("{error}");
                continue;
            }
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
            handover: false,
        };
        let sessions = self
            .sessions
            .values()
            .map(|session| session.held.clone())
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
        // No checkpoints or tails are kept yet, so each is empty.
        for &id in self.sessions.keys() {
            for message in ToCore::checkpoint(id, &[])
                .into_iter()
                .chain(ToCore::tail(id, &[]))
            {
                queue(&mut core.conn, message);
            }
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
        self.process_core_frames();
    }

    fn lose_core(&mut self, reason: &str) {
        let Some(core) = self.core.take() else {
            return;
        };
        self.drop_source(core.token);
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
        let eof = match core.conn.read_available() {
            Ok(eof) => eof,
            Err(error) => {
                self.lose_core(&format!("read failed: {error}"));
                return;
            }
        };
        self.process_core_frames();
        if eof {
            self.lose_core("the connection closed");
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
            // Checkpoints aren't kept yet.
            ToHolder::CheckpointBegin(_)
            | ToHolder::CheckpointChunk(_)
            | ToHolder::CheckpointEnd(_) => {}
            ToHolder::Drain(_) => self.send_error(None, "DRAIN isn't supported yet"),
            ToHolder::Goodbye(goodbye) => {
                if let Some(core) = self.core.as_mut() {
                    core.handover = goodbye.handover;
                }
            }
            ToHolder::Reparent(reparent) => self.send_error(Some(reparent.session), "unsupported"),
        }
    }

    fn spawn(&mut self, spawn: Spawn) {
        let id = spawn.session;
        let refusal = if self.sessions.contains_key(&id) {
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
                input: VecDeque::new(),
                reaped: None,
                last_output: Instant::now(),
                output_closed: false,
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
            self.kills.push((pid, Instant::now() + GRACEFUL_SHUTDOWN));
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
            let read = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if read > 0 {
                let bytes = &buffer[..read as usize];
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

    fn remove_session(&mut self, id: SessionId) {
        if let Some(session) = self.sessions.remove(&id)
            && session.master.is_some()
        {
            self.drop_source(session.token);
        }
    }

    /// Reaps exited programs; run after every wake-up
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
            // Without a core, the next one's WELCOME reports it.
            if self.core.is_some() {
                self.send(ToCore::Exited(Exited { session: id, code }));
                self.remove_session(id);
            }
        }

        self.kills.retain(|&(pid, at)| {
            if now < at {
                return true;
            }
            pty::signal_process_group(pid, libc::SIGKILL);
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
    }

    fn update_interests(&mut self) {
        self.flush_core();
        for session in self.sessions.values_mut() {
            if session.master.is_none() {
                continue;
            }
            let want = Interest {
                readable: !session.output_closed,
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
            && (self.had_session || (self.core.is_none() && now >= self.started + UNUSED_LIMIT))
    }

    fn shutdown(mut self) {
        if let Some(core) = self.core.as_mut() {
            let _ = core.conn.flush_for(EXIT_FLUSH_LIMIT);
        }
        let _ = fs::remove_file(&self.slot.socket_path);
        let _ = fs::remove_file(&self.slot.lock_path);
        log!("no sessions left; exiting");
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
