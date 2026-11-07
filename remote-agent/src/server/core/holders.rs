//! The core's holders: finding them by their sockets, adopting them and
//! their sessions, starting one of the core's own build, and H1.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client;
use brindle_remote_protocol::holder::{self as h1, Assembler, Hello, ToCore, ToHolder, Welcome};

use super::clients::error;
use super::sessions::Session;
use super::{ADOPT_LIMIT, Core, RECONNECT_GRACE, Source, log};
use crate::server::conn::Conn;
use crate::server::daemon;
use crate::server::event_loop::{EventLoop, Interest, Token};

/// How long a started holder has to say it is listening.
const HOLDER_START_LIMIT: Duration = Duration::from_secs(10);
/// How long a holder found at startup may refuse connections (it may be
/// between taking its lock and listening).
const CONNECT_RETRY_LIMIT: Duration = Duration::from_secs(1);
/// The fd a started holder's ready pipe gets.
const HOLDER_READY_FD: libc::c_int = 3;

pub(super) struct Holder {
    pub(super) conn: Conn,
    pub(super) interest: Interest,
    socket: PathBuf,
    /// Its socket names the core's own build.
    own_build: bool,
    /// Found at startup: clients wait until it is adopted.
    initial: bool,
    /// From its `WELCOME`.
    welcome: Option<Welcomed>,
    /// Sessions whose checkpoint and tail are still to come.
    adopting: HashSet<SessionId>,
    checkpoints: Assembler,
    tails: Assembler,
}

struct Welcomed {
    protocol: u32,
    build: String,
    pid: u32,
}

impl Holder {
    /// The H1 version it is driven in, once it has welcomed the core.
    pub(super) fn protocol(&self) -> Option<u32> {
        self.welcome.as_ref().map(|welcome| welcome.protocol)
    }

    fn is_adopted(&self) -> bool {
        self.welcome.is_some() && self.adopting.is_empty()
    }
}

/// A holder this core started, until it says it is listening.
pub(super) struct HolderStart {
    token: Token,
    ready: OwnedFd,
    deadline: Instant,
}

/// A client's `SPAWN`, waiting for a holder of the core's build.
pub(super) struct WaitingSpawn {
    pub(super) client: Token,
    pub(super) id: SessionId,
    pub(super) spawn: client::Spawn,
}

/// Whether a holder socket belongs to a running holder.
enum Liveness {
    Live,
    /// No holder holds its lock: the socket is a leftover (and is removed).
    Stale,
    /// It has no lock file; leave it alone.
    Unknown,
}

/// A running holder holds `holder-<build>-<n>.lock` for its whole life. Its
/// lock free means the socket beside it is stale; then, still holding the
/// lock (so no new holder can take that slot meanwhile), the socket and the
/// lock file are removed.
fn liveness(socket: &Path) -> io::Result<Liveness> {
    let lock_path = socket.with_extension("lock");
    let lock = match OpenOptions::new().read(true).write(true).open(&lock_path) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Liveness::Unknown),
        Err(error) => return Err(error),
    };
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == -1 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::WouldBlock {
            return Ok(Liveness::Live);
        }
        return Err(error);
    }
    let _ = fs::remove_file(socket);
    let _ = fs::remove_file(&lock_path);
    Ok(Liveness::Stale)
}

fn connect(socket: &Path, retry_for: Duration) -> io::Result<UnixStream> {
    let deadline = Instant::now() + retry_for;
    loop {
        match UnixStream::connect(socket) {
            Ok(stream) => return Ok(stream),
            Err(error)
                if Instant::now() < deadline
                    && matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                    ) =>
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error),
        }
    }
}

impl<L: EventLoop> Core<L> {
    /// Connects to every running holder in the server directory that the
    /// core isn't connected to yet.
    pub(super) fn discover(&mut self, initial: bool) {
        let mut sockets: Vec<PathBuf> = match fs::read_dir(&self.dir) {
            Ok(entries) => entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    name.starts_with("holder-") && name.ends_with(".sock")
                })
                .collect(),
            Err(error) => return log!("can't read {}: {error}", self.dir.display()),
        };
        sockets.sort();
        for socket in sockets {
            if self.holders.values().any(|holder| holder.socket == socket) {
                continue;
            }
            match liveness(&socket) {
                Ok(Liveness::Live) => {}
                Ok(Liveness::Stale) => {
                    log!("removed the stale holder socket {}", socket.display());
                    continue;
                }
                Ok(Liveness::Unknown) => continue,
                Err(error) => {
                    log!("can't check {}: {error}", socket.display());
                    continue;
                }
            }
            let retry_for = if initial {
                CONNECT_RETRY_LIMIT
            } else {
                Duration::ZERO
            };
            if let Err(error) = self.connect_holder(&socket, retry_for, initial) {
                log!("can't connect to {}: {error}", socket.display());
            }
        }
    }

    fn connect_holder(
        &mut self,
        socket: &Path,
        retry_for: Duration,
        initial: bool,
    ) -> io::Result<()> {
        let mut conn = Conn::new(connect(socket, retry_for)?)?;
        queue(
            &mut conn,
            ToHolder::Hello(Hello {
                min: h1::PROTOCOLS.min,
                max: h1::PROTOCOLS.max,
                core_build: self.build.clone(),
                instance: self.instance,
            }),
        );
        let token = self.token();
        self.ev.register(conn.fd(), token, Interest::READABLE)?;
        self.sources.insert(token, Source::Holder);
        let name = socket.file_name().unwrap_or_default().to_string_lossy();
        let own_build = name.starts_with(&format!("holder-{}-", self.build));
        self.holders.insert(
            token,
            Holder {
                conn,
                interest: Interest::READABLE,
                socket: socket.to_path_buf(),
                own_build,
                initial,
                welcome: None,
                adopting: HashSet::new(),
                checkpoints: Assembler::new(h1::MAX_CHECKPOINT_LEN),
                tails: Assembler::new(h1::TAIL_LIMIT),
            },
        );
        Ok(())
    }

    /// Whether the holders found at startup are adopted, or have had long
    /// enough.
    pub(super) fn startup_adopted(&self, now: Instant) -> bool {
        now >= self.started + ADOPT_LIMIT
            || self
                .holders
                .values()
                .all(|holder| !holder.initial || holder.is_adopted())
    }

    pub(super) fn read_holder(&mut self, token: Token) {
        let Some(holder) = self.holders.get_mut(&token) else {
            return;
        };
        let eof = match holder.conn.read_available() {
            Ok(eof) => eof,
            Err(error) => return self.drop_holder(token, &format!("read failed: {error}")),
        };
        loop {
            let Some(holder) = self.holders.get_mut(&token) else {
                return;
            };
            let frame = match holder.conn.next_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(error) => return self.drop_holder(token, &format!("bad frame: {error}")),
            };
            match ToCore::decode(&frame) {
                Ok(message) => self.handle_holder(token, message),
                Err(error) => log!("holder {token} sent a bad message: {error}"),
            }
        }
        if eof {
            self.drop_holder(token, "the connection closed");
        }
    }

    pub(super) fn flush_holder(&mut self, token: Token) {
        if let Some(holder) = self.holders.get_mut(&token)
            && let Err(error) = holder.conn.flush()
        {
            self.drop_holder(token, &format!("write failed: {error}"));
        }
    }

    pub(super) fn send_holder(&mut self, token: Token, message: ToHolder) {
        if let Some(holder) = self.holders.get_mut(&token) {
            queue(&mut holder.conn, message);
        }
    }

    fn handle_holder(&mut self, token: Token, message: ToCore) {
        match message {
            ToCore::Welcome(welcome) => self.adopt(token, welcome),
            ToCore::Output(data) => self.session_output(token, data.session, data.bytes),
            ToCore::Spawned(spawned) => self.session_spawned(token, spawned.session, spawned.pid),
            ToCore::SpawnFailed(failed) => {
                self.session_spawn_failed(token, failed.session, failed.message)
            }
            ToCore::Exited(exited) => self.session_exited(token, exited.session, exited.code),
            ToCore::Error(holder_error) => {
                let client = holder_error
                    .session
                    .and_then(|id| self.sessions.get(&id))
                    .filter(|session| session.holder == token)
                    .and_then(|session| session.attached);
                match client {
                    Some(client) => self.send_client(
                        client,
                        error(None, holder_error.session, holder_error.message),
                    ),
                    None => log!(
                        "holder {token}: {} (session {:?})",
                        holder_error.message,
                        holder_error.session
                    ),
                }
            }
            ToCore::CheckpointBegin(begin) => self.blob(token, Blob::Checkpoint, |assembler| {
                assembler.begin(begin).map(|()| None)
            }),
            ToCore::CheckpointChunk(chunk) => self.blob(token, Blob::Checkpoint, |assembler| {
                assembler.chunk(&chunk).map(|()| None)
            }),
            ToCore::CheckpointEnd(end) => self.blob(token, Blob::Checkpoint, |assembler| {
                assembler.end(end).map(|bytes| Some((end.session, bytes)))
            }),
            ToCore::TailBegin(begin) => self.blob(token, Blob::Tail, |assembler| {
                assembler.begin(begin).map(|()| None)
            }),
            ToCore::TailChunk(chunk) => self.blob(token, Blob::Tail, |assembler| {
                assembler.chunk(&chunk).map(|()| None)
            }),
            ToCore::TailEnd(end) => self.blob(token, Blob::Tail, |assembler| {
                assembler.end(end).map(|bytes| Some((end.session, bytes)))
            }),
        }
    }

    /// A holder's `WELCOME`: its sessions become the core's, with ids
    /// continuing after the largest.
    fn adopt(&mut self, token: Token, welcome: Welcome) {
        let Some(holder) = self.holders.get_mut(&token) else {
            return;
        };
        if holder.welcome.is_some() {
            return log!("holder {token} sent a second WELCOME; ignored");
        }
        log!(
            "adopted holder {} (pid {}, build {}, protocol H{}, previous core {:?}) with {} session(s)",
            holder.socket.display(),
            welcome.pid,
            welcome.build,
            welcome.protocol,
            welcome.previous,
            welcome.sessions.len()
        );
        holder.own_build = welcome.build == self.build;
        holder.welcome = Some(Welcomed {
            protocol: welcome.protocol,
            build: welcome.build,
            pid: welcome.pid,
        });
        // Every listed session's checkpoint and tail follow.
        holder.adopting = welcome.sessions.iter().map(|held| held.session).collect();
        let mut adopted_any = false;
        for held in welcome.sessions {
            let id = held.session;
            if self.sessions.contains_key(&id) {
                log!("holder {token} reports session {id}, which another holder has; ignored");
                continue;
            }
            self.next_session = self.next_session.max(id.saturating_add(1));
            self.sessions.insert(id, Session::adopted(token, held));
            adopted_any = true;
        }
        // A holder found after startup gives its sessions' owners the same
        // grace period (startup's starts once clients are accepted).
        if adopted_any && self.accepting {
            let until = Instant::now() + RECONNECT_GRACE;
            self.grace_until = Some(self.grace_until.map_or(until, |grace| grace.max(until)));
        }
        self.holder_adopted(token);
    }

    /// Stores a checkpoint or tail as it completes. A tail is the last thing
    /// sent for an adopted session.
    fn blob(
        &mut self,
        token: Token,
        kind: Blob,
        step: impl FnOnce(&mut Assembler) -> io::Result<Option<(SessionId, Vec<u8>)>>,
    ) {
        let Some(holder) = self.holders.get_mut(&token) else {
            return;
        };
        let assembler = match kind {
            Blob::Checkpoint => &mut holder.checkpoints,
            Blob::Tail => &mut holder.tails,
        };
        let (id, bytes) = match step(assembler) {
            Ok(Some(done)) => done,
            Ok(None) => return,
            Err(error) => return self.drop_holder(token, &format!("bad {kind:?}: {error}")),
        };
        if kind == Blob::Tail {
            holder.adopting.remove(&id);
        }
        if let Some(session) = self.sessions.get_mut(&id)
            && session.holder == token
        {
            session.store_adopted(kind == Blob::Checkpoint, bytes);
        }
        if kind == Blob::Tail {
            self.holder_adopted(token);
        }
    }

    /// Once a holder's sessions are all adopted, waiting spawns may use it.
    fn holder_adopted(&mut self, token: Token) {
        if self.holders.get(&token).is_some_and(Holder::is_adopted) {
            self.place_waiting();
        }
    }

    /// The holder is gone, and so are its sessions.
    pub(super) fn drop_holder(&mut self, token: Token, reason: &str) {
        let Some(holder) = self.holders.remove(&token) else {
            return;
        };
        self.drop_source(token);
        let lost: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.holder == token)
            .map(|(&id, _)| id)
            .collect();
        log!(
            "holder {} (pid {:?}) left ({reason}); {} session(s) lost",
            holder.socket.display(),
            holder.welcome.as_ref().map(|welcome| welcome.pid),
            lost.len()
        );
        for id in lost {
            self.session_lost(token, id);
        }
        // Spawns it didn't answer go to another holder.
        self.place_waiting();
    }

    /// An adopted holder of the core's build to start sessions on.
    fn spawn_holder(&self) -> Option<Token> {
        self.holders
            .iter()
            .filter(|(_, holder)| {
                holder.is_adopted()
                    && holder
                        .welcome
                        .as_ref()
                        .is_some_and(|welcome| welcome.build == self.build)
            })
            .map(|(&token, _)| token)
            .min()
    }

    /// Sends waiting spawns to a holder of the core's build, starting one if
    /// there is none.
    pub(super) fn place_waiting(&mut self) {
        if self.waiting.is_empty() {
            return;
        }
        if let Some(holder) = self.spawn_holder() {
            for waiting in std::mem::take(&mut self.waiting) {
                self.send_spawn(holder, waiting);
            }
            return;
        }
        let coming = self
            .holders
            .values()
            .any(|holder| holder.own_build && holder.welcome.is_none());
        if !coming && self.holder_start.is_none() {
            self.start_holder();
        }
    }

    fn fail_waiting(&mut self, message: &str) {
        for waiting in std::mem::take(&mut self.waiting) {
            let req = waiting.spawn.req;
            self.send_client(waiting.client, error(Some(req), None, message));
        }
    }

    /// Starts `brindle-remote hold` from the core's own binary, which
    /// `/proc/self/exe` names even after an install replaced the file.
    fn start_holder(&mut self) {
        let mut command = Command::new("/proc/self/exe");
        command
            .args(["hold", "--ready-fd", &HOLDER_READY_FD.to_string()])
            .env("BRINDLE_SERVER_DIR", &self.dir);
        let (child, ready) = match daemon::spawn_with_ready_pipe(&mut command, HOLDER_READY_FD) {
            Ok(started) => started,
            Err(error) => {
                log!("can't start a holder: {error}");
                return self.fail_waiting(&format!("can't start a holder: {error}"));
            }
        };
        log!("started a holder (starter pid {})", child.id());
        self.starters.push(child);
        let token = self.token();
        if let Err(error) = self
            .ev
            .register(ready.as_raw_fd(), token, Interest::READABLE)
        {
            log!("can't watch the holder's ready pipe: {error}");
            return self.fail_waiting("can't start a holder");
        }
        self.sources.insert(token, Source::HolderReady);
        self.holder_start = Some(HolderStart {
            token,
            ready,
            deadline: Instant::now() + HOLDER_START_LIMIT,
        });
    }

    pub(super) fn read_holder_ready(&mut self) {
        let Some(start) = &self.holder_start else {
            return;
        };
        let mut byte = [0u8; 1];
        let read = unsafe { libc::read(start.ready.as_raw_fd(), byte.as_mut_ptr().cast(), 1) };
        if read < 0 && io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
            return;
        }
        let start = self.holder_start.take().expect("a holder start");
        self.drop_source(start.token);
        if read != 1 {
            log!("the holder exited before listening; see its log");
            return self.fail_waiting(
                "the holder exited before listening; see its log in the server directory",
            );
        }
        self.discover(false);
        let coming = self
            .holders
            .values()
            .any(|holder| holder.own_build && !holder.is_adopted());
        if !coming && self.spawn_holder().is_none() {
            self.fail_waiting("the started holder couldn't be reached");
        }
    }

    pub(super) fn check_holder_start(&mut self, now: Instant) {
        if let Some(start) = &self.holder_start
            && now >= start.deadline
        {
            let token = start.token;
            self.holder_start = None;
            self.drop_source(token);
            log!(
                "the holder didn't start within {}s",
                HOLDER_START_LIMIT.as_secs()
            );
            self.fail_waiting(
                "the holder didn't start in time; see its log in the server directory",
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blob {
    Checkpoint,
    Tail,
}

fn queue(conn: &mut Conn, message: ToHolder) {
    match message.encode() {
        Ok(frame) => conn.queue(&frame),
        Err(error) => log!("can't encode a message: {error}"),
    }
}
