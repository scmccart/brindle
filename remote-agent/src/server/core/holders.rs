//! The core's holders: finding them in the server directory (by their
//! sockets on Linux, by their lock files on Windows), adopting them and
//! their sessions, starting one of the core's own build, and H1.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use brindle_remote_protocol::endpoint::{self, Holder as Place};
use brindle_remote_protocol::{SessionId, client};
use brindle_remote_protocol::holder::{self as h1, Assembler, Hello, ToCore, ToHolder, Welcome};

use super::clients::error;
use super::sessions::Session;
use super::{ADOPT_LIMIT, Core, RECONNECT_GRACE, Source, log};
use crate::server::conn::Conn;
use crate::server::daemon::{self, ReadyPipe};
use crate::server::event_loop::{EventLoop, Interest, Token};
use crate::server::platform::conn::Stream;
use crate::server::platform::listen;
use crate::server::platform::lock::Lock;

/// How long a started holder has to say it is listening.
const HOLDER_START_LIMIT: Duration = Duration::from_secs(10);
/// How long a holder found at startup may refuse connections (it may be
/// between taking its lock and listening).
const CONNECT_RETRY_LIMIT: Duration = Duration::from_secs(1);

pub(super) struct Holder {
    pub(super) conn: Conn,
    pub(super) interest: Interest,
    /// Where it listens, and its lock file.
    place: Place,
    /// Its files name the core's own build.
    own_build: bool,
    /// Found at startup: clients wait until it is adopted.
    initial: bool,
    /// From its `WELCOME`.
    welcome: Option<Welcomed>,
    /// Sessions whose checkpoint and tail are still to come.
    adopting: HashSet<SessionId>,
    /// Of another build: told to `DRAIN`, so it takes no new sessions and
    /// exits after its last one.
    draining: bool,
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

    /// Its entry in `SERVER_STATUS`, once it has welcomed the core.
    pub(super) fn status(&self, sessions: u32) -> Option<client::HolderStatus> {
        let welcome = self.welcome.as_ref()?;
        Some(client::HolderStatus {
            pid: welcome.pid,
            build: welcome.build.clone(),
            protocol: welcome.protocol,
            sessions,
            draining: self.draining,
        })
    }
}

/// A holder this core started, until it says it is listening.
pub(super) struct HolderStart {
    token: Token,
    ready: ReadyPipe,
    deadline: Instant,
}

/// A client's `SPAWN`, waiting for a holder of the core's build.
pub(super) struct WaitingSpawn {
    pub(super) client: Token,
    pub(super) id: SessionId,
    pub(super) spawn: client::Spawn,
}

/// Whether what the directory shows of a holder belongs to a running one.
enum Liveness {
    Live,
    /// No holder holds its lock: its files are leftovers (and are removed).
    Stale,
    /// It has no lock file (any more); leave it alone.
    Unknown,
}

/// A running holder holds `holder-<build>-<n>.lock` for its whole life. Its
/// lock free means that its holder is gone (killed, since an exiting one
/// removes its files); then, still holding the lock (so no new holder can
/// take that slot meanwhile), the socket and the lock file are removed.
fn liveness(place: &Place) -> io::Result<Liveness> {
    let _lock = match Lock::existing(&place.lock) {
        Ok(Some(lock)) => lock,
        Ok(None) => return Ok(Liveness::Live),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Liveness::Unknown),
        Err(error) => return Err(error),
    };
    let _ = endpoint::remove(&place.endpoint);
    let _ = fs::remove_file(&place.lock);
    Ok(Liveness::Stale)
}

fn connect(endpoint: &Path, retry_for: Duration) -> io::Result<Stream> {
    let deadline = Instant::now() + retry_for;
    loop {
        match listen::connect(endpoint) {
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
        let places = match self.paths.endpoints.holders() {
            Ok(places) => places,
            Err(error) => return log!("can't read {}: {error}", self.dir.display()),
        };
        for place in places {
            if self.holders.values().any(|holder| holder.place == place) {
                continue;
            }
            match liveness(&place) {
                Ok(Liveness::Live) => {}
                Ok(Liveness::Stale) => {
                    log!(
                        "removed what a holder that is gone left behind ({})",
                        place.lock.display()
                    );
                    continue;
                }
                Ok(Liveness::Unknown) => continue,
                Err(error) => {
                    log!("can't check {}: {error}", place.lock.display());
                    continue;
                }
            }
            let retry_for = if initial {
                CONNECT_RETRY_LIMIT
            } else {
                Duration::ZERO
            };
            let endpoint = place.endpoint.clone();
            if let Err(error) = self.connect_holder(place, retry_for, initial) {
                log!("can't connect to {}: {error}", endpoint.display());
            }
        }
    }

    fn connect_holder(
        &mut self,
        place: Place,
        retry_for: Duration,
        initial: bool,
    ) -> io::Result<()> {
        let mut conn = Conn::new(connect(&place.endpoint, retry_for)?)?;
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
        self.ev.register(&conn.source(), token, Interest::READABLE)?;
        self.sources.insert(token, Source::Holder);
        let name = place.lock.file_name().unwrap_or_default().to_string_lossy();
        let own_build = name.starts_with(&format!("holder-{}-", self.build));
        self.holders.insert(
            token,
            Holder {
                conn,
                interest: Interest::READABLE,
                place,
                own_build,
                initial,
                welcome: None,
                adopting: HashSet::new(),
                draining: false,
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
        // What was read before a failure still counts: a holder's last
        // messages (`EXITED`) come right before its end.
        let ended = match holder.conn.read_available() {
            Ok(false) => None,
            Ok(true) => Some("the connection closed".to_string()),
            Err(error) => Some(format!("read failed: {error}")),
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
        match ended {
            Some(reason) => self.drop_holder(token, &reason),
            None => self.checkpoint_due(token, true),
        }
    }

    /// While the core exits, a holder's output and exits still reach the
    /// attached clients, whose connections close after the holders'.
    /// Everything else it says is moot now.
    pub(super) fn closing_holder_frames(&mut self, token: Token) {
        loop {
            let Some(holder) = self.holders.get_mut(&token) else {
                return;
            };
            let frame = match holder.conn.next_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(_) => {
                    holder.conn.discard_input();
                    break;
                }
            };
            if let Ok(message @ (ToCore::Output(_) | ToCore::Exited(_))) = ToCore::decode(&frame) {
                self.handle_holder(token, message);
            }
        }
        self.checkpoint_due(token, false);
    }

    /// Whether more of the holder's output is waiting to be read.
    pub(super) fn holder_behind(&self, token: Token) -> bool {
        self.holders
            .get(&token)
            .is_some_and(|holder| holder.conn.input_waiting())
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

    /// Where what is queued for the holder so far ends, as a count of the
    /// bytes written to its connection ([`Core::holder_has`]). `None`
    /// without the holder.
    pub(super) fn holder_queue_end(&self, token: Token) -> Option<u64> {
        self.holders
            .get(&token)
            .map(|holder| holder.conn.queued_end())
    }

    /// Whether the holder's socket has taken the first `end` bytes of its
    /// connection, so the holder reads them even if the core is killed
    /// now. Also true once the holder is gone: nothing can reach it.
    pub(super) fn holder_has(&self, token: Token, end: u64) -> bool {
        self.holders
            .get(&token)
            .is_none_or(|holder| holder.conn.written() >= end)
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
            holder.place.endpoint.display(),
            welcome.pid,
            welcome.build,
            welcome.protocol,
            welcome.previous,
            welcome.sessions.len()
        );
        holder.own_build = welcome.build == self.build;
        if welcome.previous == h1::Previous::Handover {
            log!(
                "upgrade: took holder {} over from a core that stepped down",
                holder.place.endpoint.display()
            );
        }
        let drain = !holder.own_build;
        if drain {
            log!(
                "drain: holder {} is of build {}, not {}; it takes no new sessions and exits after its last one",
                holder.place.endpoint.display(),
                welcome.build,
                self.build
            );
            holder.draining = true;
        }
        holder.welcome = Some(Welcomed {
            protocol: welcome.protocol,
            build: welcome.build,
            pid: welcome.pid,
        });
        if drain {
            self.send_holder(token, ToHolder::Drain(h1::Drain {}));
        }
        let Some(holder) = self.holders.get_mut(&token) else {
            return;
        };
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
        match kind {
            Blob::Checkpoint => self.session_checkpoint(token, id, &bytes),
            Blob::Tail => {
                holder.adopting.remove(&id);
                self.session_tail(token, id, &bytes);
                self.holder_adopted(token);
            }
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
        if holder.draining && lost.is_empty() {
            log!(
                "drain: holder {} (pid {:?}) ended its last session and exited",
                holder.place.endpoint.display(),
                holder.welcome.as_ref().map(|welcome| welcome.pid),
            );
        } else if lost.is_empty() {
            log!(
                "holder {} (pid {:?}) closed its connection with no sessions ({reason})",
                holder.place.endpoint.display(),
                holder.welcome.as_ref().map(|welcome| welcome.pid),
            );
        } else {
            log!(
                "holder {} (pid {:?}) left ({reason}); {} session(s) lost",
                holder.place.endpoint.display(),
                holder.welcome.as_ref().map(|welcome| welcome.pid),
                lost.len()
            );
        }
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

    pub(super) fn fail_waiting(&mut self, message: &str) {
        for waiting in std::mem::take(&mut self.waiting) {
            let req = waiting.spawn.req;
            self.send_client(waiting.client, error(Some(req), None, message));
        }
    }

    /// Starts `brindle-remote hold` from the core's own binary, detached
    /// ([`daemon::start_role`]).
    fn start_holder(&mut self) {
        let env: [(&str, &OsStr); 1] = [("BRINDLE_SERVER_DIR", self.dir.as_os_str())];
        let started = daemon::start_role("hold", &daemon::role_args("hold"), &env, |reason| {
            log!("starting a holder without a scope of its own: {reason}")
        });
        let (child, ready) = match started {
            Ok(started) => (started.child, started.ready),
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
            .register(&ready.source(), token, Interest::READABLE)
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
        let Some(listening) = start.ready.read() else {
            return;
        };
        let start = self.holder_start.take().expect("a holder start");
        self.drop_source(start.token);
        if !listening {
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
