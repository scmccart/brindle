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
use brindle_remote_protocol::ready::ReadyRead;

use super::clients::error;
use super::policy::{self, Adoption, HolderView, Placement, Stage};
use super::sessions::{Life, RestartStage, Session, SpawnRequest};
use super::{ADOPT_LIMIT, Core, Source, log};
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
    /// Where it listens, its lock file, and the build their names give.
    place: Place,
    /// Found at startup: clients wait until it is adopted.
    initial: bool,
    /// From its `WELCOME`.
    welcome: Option<Welcomed>,
    checkpoints: Assembler,
    tails: Assembler,
}

/// What a holder's `WELCOME` told the core, and how far its adoption got.
struct Welcomed {
    protocol: u32,
    build: String,
    pid: u32,
    /// Sessions whose checkpoint and tail are still to come.
    adopting: HashSet<SessionId>,
    /// Whether it takes new sessions, or drains ([`policy::adoption`]).
    adoption: Adoption,
}

/// A holder is adopted once it has welcomed the core and every session it
/// listed has its checkpoint and tail.
fn is_adopted(welcome: Option<&Welcomed>) -> bool {
    welcome.is_some_and(|welcome| welcome.adopting.is_empty())
}

/// A holder's entry in `SERVER_STATUS`, once it has welcomed the core.
fn status(welcome: Option<&Welcomed>, sessions: u32) -> Option<client::HolderStatus> {
    let welcome = welcome?;
    Some(client::HolderStatus {
        pid: welcome.pid,
        build: welcome.build.clone(),
        protocol: welcome.protocol,
        sessions,
        draining: match welcome.adoption {
            Adoption::Keep => false,
            Adoption::Drain => true,
        },
    })
}

impl Holder {
    /// The H1 version it is driven in, once it has welcomed the core.
    pub(super) fn protocol(&self) -> Option<u32> {
        self.welcome.as_ref().map(|welcome| welcome.protocol)
    }

    fn is_adopted(&self) -> bool {
        is_adopted(self.welcome.as_ref())
    }

    /// Its entry in `SERVER_STATUS`, once it has welcomed the core.
    pub(super) fn status(&self, sessions: u32) -> Option<client::HolderStatus> {
        status(self.welcome.as_ref(), sessions)
    }

    /// Whether it is of the core's build ([`policy::adoption`]); one that
    /// hasn't welcomed the core yet counts as another build's, so an agent
    /// checkpoint for it is written whole.
    pub(super) fn adoption(&self) -> Adoption {
        self.welcome
            .as_ref()
            .map_or(Adoption::Drain, |welcome| welcome.adoption)
    }

    fn draining(&self) -> bool {
        self.welcome
            .as_ref()
            .is_some_and(|welcome| welcome.adoption == Adoption::Drain)
    }

    /// What [`policy::placement`] reads of it.
    fn view(&self) -> HolderView<'_> {
        let stage = match &self.welcome {
            None => Stage::Connected,
            Some(welcome) if welcome.adopting.is_empty() => Stage::Adopted {
                build: &welcome.build,
            },
            Some(welcome) => Stage::Adopting {
                build: &welcome.build,
            },
        };
        HolderView {
            named: self.place.build.as_deref(),
            stage,
            protocol: self.protocol(),
        }
    }
}

/// A holder this core started, until it says it is listening.
pub(super) struct HolderStart {
    token: Token,
    ready: ReadyPipe,
    deadline: Instant,
}

/// A client's `SPAWN` or `SPAWN_AGENT`, waiting for a holder of the core's
/// build.
pub(super) struct WaitingSpawn {
    pub(super) client: Token,
    pub(super) id: SessionId,
    pub(super) spawn: SpawnRequest,
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
            &ToHolder::Hello(Hello {
                min: h1::PROTOCOLS.min,
                max: h1::PROTOCOLS.max,
                core_build: self.build.clone(),
                instance: self.instance,
            }),
        )?;
        let token = self.token();
        self.ev.register(&conn.source(), token, Interest::READABLE)?;
        self.sources.insert(token, Source::Holder);
        self.holders.insert(
            token,
            Holder {
                conn,
                interest: Interest::READABLE,
                place,
                initial,
                welcome: None,
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
            // An agent's stderr tail comes before its `EXITED`, for it.
            if let Ok(message @ (ToCore::Output(_) | ToCore::StderrTail(_) | ToCore::Exited(_))) =
                ToCore::decode(&frame)
            {
                self.handle_holder(token, message);
            }
        }
        self.checkpoint_due(token, false);
    }

    /// Whether more of the holder's output is waiting to be read. A
    /// connection that can't say counts as behind: the next read meets the
    /// error, and [`Core::drop_holder`] logs its cause.
    pub(super) fn holder_behind(&self, token: Token) -> bool {
        self.holders
            .get(&token)
            .is_some_and(|holder| holder.conn.input_waiting().unwrap_or(true))
    }

    pub(super) fn flush_holder(&mut self, token: Token) {
        if let Some(holder) = self.holders.get_mut(&token)
            && let Err(error) = holder.conn.flush()
        {
            self.drop_holder(token, &format!("write failed: {error}"));
        }
    }

    /// Queues a message for the holder; whether it was queued. One that
    /// can't be encoded is handled as [`unsent`] says: a spawn fails to its
    /// client, a session's message is reported to that session's client,
    /// and the rest are logged.
    pub(super) fn send_holder(&mut self, token: Token, message: ToHolder) -> bool {
        let Some(holder) = self.holders.get_mut(&token) else {
            return false;
        };
        let Err(failure) = queue(&mut holder.conn, &message) else {
            return true;
        };
        match unsent(&message, &failure) {
            Unsent::Spawn { session, why } => self.session_spawn_failed(token, session, why),
            Unsent::Meta { session, why } => log!("session {session}: {why}"),
            Unsent::Session { session, why } => {
                log!("session {session}: {why}");
                let client = self
                    .sessions
                    .get(&session)
                    .filter(|held| held.holder == token)
                    .and_then(|held| held.ownership.client());
                if let Some(client) = client {
                    self.send_client(client, error(None, Some(session), why));
                }
            }
            Unsent::Holder { why } => log!("holder {token}: {why}"),
        }
        false
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
                // What it stores of the session's checkpoint is no longer
                // known: the next is whole (agents-log-paging decision 9).
                if let Some(agent) = holder_error
                    .session
                    .and_then(|id| self.sessions.get_mut(&id))
                    .filter(|session| session.holder == token)
                    .and_then(|session| session.agent())
                {
                    agent.forget_stored();
                }
                let client = holder_error
                    .session
                    .and_then(|id| self.sessions.get(&id))
                    .filter(|session| session.holder == token)
                    .and_then(|session| session.ownership.client());
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
            // Only a pipe session has one: its agent's, kept for its exit.
            ToCore::StderrTail(tail) => self.agent_stderr(token, tail.session, tail.bytes),
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
        if welcome.previous == h1::Previous::Handover {
            log!(
                "upgrade: took holder {} over from a core that stepped down",
                holder.place.endpoint.display()
            );
        }
        let adoption = policy::adoption(&welcome.build, &self.build);
        match adoption {
            Adoption::Keep => {}
            Adoption::Drain => log!(
                "drain: holder {} is of build {}, not {}; it takes no new sessions and exits after its last one",
                holder.place.endpoint.display(),
                welcome.build,
                self.build
            ),
        }
        holder.welcome = Some(Welcomed {
            protocol: welcome.protocol,
            build: welcome.build,
            pid: welcome.pid,
            // Every listed session's checkpoint and tail follow.
            adopting: welcome.sessions.iter().map(|held| held.session).collect(),
            adoption,
        });
        match adoption {
            Adoption::Keep => {}
            Adoption::Drain => {
                self.send_holder(token, ToHolder::Drain(h1::Drain {}));
            }
        }
        let mut adopted_any = false;
        let now = Instant::now();
        for held in welcome.sessions {
            let id = held.session;
            if self.sessions.contains_key(&id) {
                log!("holder {token} reports session {id}, which another holder has; ignored");
                continue;
            }
            self.next_session = self.next_session.max(id.saturating_add(1));
            let (session, repaired) = match Session::adopted(token, held, now) {
                Ok(adopted) => adopted,
                Err(line) => {
                    log!("{line}");
                    continue;
                }
            };
            self.sessions.insert(id, session);
            // The next core reads the repaired meta.
            if let Some(line) = repaired {
                log!("{line}");
                self.store_meta(id);
            }
            adopted_any = true;
        }
        // A holder found after startup gives its sessions' owners the same
        // grace period (startup's starts once clients are accepted).
        if adopted_any {
            self.lifecycle.holder_adopted(now);
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
                if let Some(welcome) = &mut holder.welcome {
                    welcome.adopting.remove(&id);
                }
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
        if holder.draining() && lost.is_empty() {
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

    /// The holders in token order, and what [`policy::placement`] reads of
    /// each.
    fn holder_views(&self) -> (Vec<Token>, Vec<HolderView<'_>>) {
        let mut holders: Vec<(Token, &Holder)> = self
            .holders
            .iter()
            .map(|(&token, holder)| (token, holder))
            .collect();
        holders.sort_by_key(|&(token, _)| token);
        holders
            .into_iter()
            .map(|(token, holder)| (token, holder.view()))
            .unzip()
    }

    /// Sends waiting spawns to a holder of the core's build, starting one if
    /// there is none ([`policy::placement`]).
    pub(super) fn place_waiting(&mut self) {
        let restarts = self.waiting_restarts();
        if self.waiting.is_empty() && restarts.is_empty() {
            return;
        }
        let (tokens, views) = self.holder_views();
        // They go together, so one agent's among them (a restart's is one)
        // needs a holder that runs pipe sessions (which runs PTY sessions
        // too).
        let mode = if !restarts.is_empty()
            || self
                .waiting
                .iter()
                .any(|waiting| waiting.spawn.mode() == h1::SpawnMode::Pipes)
        {
            h1::SpawnMode::Pipes
        } else {
            h1::SpawnMode::Pty
        };
        let placement = policy::placement(&views, &self.build, self.holder_start.is_some(), mode);
        match placement {
            Placement::Spawn(index) => {
                let Some(&holder) = tokens.get(index) else {
                    return;
                };
                for waiting in std::mem::take(&mut self.waiting) {
                    self.send_spawn(holder, waiting);
                }
                for id in restarts {
                    self.send_restart(holder, id);
                }
            }
            Placement::Start => self.start_holder(),
            Placement::Wait => {}
        }
    }

    /// The sessions whose restart waits for a holder, in id order
    /// ([`RestartStage::Waiting`]).
    pub(super) fn waiting_restarts(&self) -> Vec<SessionId> {
        self.sessions
            .iter()
            .filter(|(_, session)| {
                matches!(
                    session.life,
                    Life::Restarting {
                        stage: RestartStage::Waiting,
                        ..
                    }
                )
            })
            .map(|(&id, _)| id)
            .collect()
    }

    /// The waiting spawns fail with `message`: to their clients, and a
    /// waiting restart as its agent's end ([`Core::restart_failed`]).
    pub(super) fn fail_waiting(&mut self, message: &str) {
        for waiting in std::mem::take(&mut self.waiting) {
            let req = waiting.spawn.answers();
            self.send_client(waiting.client, error(Some(req), None, message));
        }
        for id in self.waiting_restarts() {
            self.restart_failed(id, message);
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
        let Some(start) = &mut self.holder_start else {
            return;
        };
        let failure = match start.ready.read() {
            ReadyRead::Pending => return,
            ReadyRead::Listening => None,
            ReadyRead::Ended => Some("the holder exited before listening".to_string()),
            ReadyRead::Failed(error) => Some(format!(
                "the holder didn't say it was listening (reading the ready pipe failed: {error})"
            )),
        };
        let start = self.holder_start.take().expect("a holder start");
        self.drop_source(start.token);
        if let Some(why) = failure {
            log!("{why}; see its log");
            return self.fail_waiting(&format!("{why}; see its log in the server directory"));
        }
        self.discover(false);
        let (_, views) = self.holder_views();
        if !policy::holder_reached(&views, &self.build) {
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

/// Queues `message` on the holder's connection, or returns why it can't be
/// encoded (a payload over the frame limit, say).
fn queue(conn: &mut Conn, message: &ToHolder) -> io::Result<()> {
    conn.queue(&message.encode()?);
    Ok(())
}

/// What a message that couldn't be queued for a holder was for, and what
/// to say about it.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Unsent {
    /// A `SPAWN`: the waiting spawn fails, with `why`, to its client.
    Spawn { session: SessionId, why: String },
    /// A `SET_META`: the session's `storing` stays unset, since nothing
    /// went out for a replay to wait on.
    Meta { session: SessionId, why: String },
    /// Another message for one session: its attached client gets an
    /// `ERROR` with the session.
    Session { session: SessionId, why: String },
    /// A message for the holder as a whole: logged.
    Holder { why: String },
}

/// What [`queue`]'s `error` for `message` means (`railway R3`: every
/// message is named).
pub(super) fn unsent(message: &ToHolder, error: &io::Error) -> Unsent {
    let session = |session: SessionId, what: &str| Unsent::Session {
        session,
        why: format!("the terminal server couldn't pass {what} to its holder: {error}"),
    };
    let holder = |what: &str| Unsent::Holder {
        why: format!("can't send its {what}: {error}"),
    };
    match message {
        ToHolder::Spawn(spawn) => Unsent::Spawn {
            session: spawn.session,
            why: format!("the terminal server couldn't pass the request to its holder: {error}"),
        },
        ToHolder::SpawnPipes(spawn) => Unsent::Spawn {
            session: spawn.session,
            why: format!("the terminal server couldn't pass the request to its holder: {error}"),
        },
        ToHolder::SetMeta(update) => Unsent::Meta {
            session: update.session,
            why: format!("can't store its meta in its holder: {error}"),
        },
        ToHolder::Input(data) => session(data.session, "input"),
        ToHolder::Resize(resize) => session(resize.session, "a resize"),
        ToHolder::Hangup(target) => session(target.session, "a hang-up"),
        ToHolder::CheckpointBegin(begin) | ToHolder::CheckpointAppend(begin) => {
            session(begin.session, "a checkpoint")
        }
        ToHolder::CheckpointChunk(chunk) => session(chunk.session, "a checkpoint"),
        ToHolder::CheckpointEnd(end) => session(end.session, "a checkpoint"),
        ToHolder::Reparent(reparent) => session(reparent.session, "the window that owns its console"),
        ToHolder::Pause(target) => session(target.session, "a pause"),
        ToHolder::Resume(target) => session(target.session, "a resume"),
        ToHolder::Hello(_) => holder("HELLO"),
        ToHolder::Drain(_) => holder("DRAIN"),
        ToHolder::Goodbye(_) => holder("GOODBYE"),
    }
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::{MAX_FRAME_PAYLOAD_SIZE, serde_json};

    use super::*;

    /// A spawn too large for a frame: its args alone are over the limit.
    fn huge_spawn(session: SessionId) -> ToHolder {
        ToHolder::Spawn(h1::Spawn {
            session,
            command: Some("sh".into()),
            args: vec!["x".repeat(MAX_FRAME_PAYLOAD_SIZE + 1)],
            cwd: None,
            env: Default::default(),
            cols: 80,
            rows: 24,
            cell_w: 0,
            cell_h: 0,
            meta: serde_json::Value::Null,
        })
    }

    #[test]
    fn an_unencodable_spawn_fails_its_request() {
        let message = huge_spawn(4);
        let error = message.encode().unwrap_err();
        match unsent(&message, &error) {
            Unsent::Spawn { session, why } => {
                assert_eq!(session, 4);
                assert_eq!(
                    why,
                    format!("the terminal server couldn't pass the request to its holder: {error}")
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_unencodable_session_message_is_reported_to_its_client() {
        let message = ToHolder::Input(h1::Data {
            session: 5,
            bytes: vec![b'x'; MAX_FRAME_PAYLOAD_SIZE + 1],
        });
        let error = message.encode().unwrap_err();
        assert_eq!(
            unsent(&message, &error),
            Unsent::Session {
                session: 5,
                why: format!("the terminal server couldn't pass input to its holder: {error}"),
            }
        );

        // A meta is the session's, but its client isn't told: the replays
        // that would wait for it don't.
        let meta = ToHolder::SetMeta(h1::SetMeta {
            session: 5,
            meta: serde_json::Value::String("x".repeat(MAX_FRAME_PAYLOAD_SIZE)),
        });
        let error = meta.encode().unwrap_err();
        assert!(matches!(unsent(&meta, &error), Unsent::Meta { session: 5, .. }));
    }

    fn welcomed(adopting: &[SessionId], adoption: Adoption) -> Welcomed {
        Welcomed {
            protocol: 2,
            build: "b0".into(),
            pid: 77,
            adopting: adopting.iter().copied().collect(),
            adoption,
        }
    }

    #[test]
    fn a_holder_is_adopted_once_every_listed_session_has_its_tail() {
        assert!(!is_adopted(None));
        assert!(!is_adopted(Some(&welcomed(&[3, 4], Adoption::Keep))));
        assert!(is_adopted(Some(&welcomed(&[], Adoption::Keep))));
        assert!(is_adopted(Some(&welcomed(&[], Adoption::Drain))));
    }

    #[test]
    fn a_holders_status_comes_from_its_welcome() {
        assert_eq!(status(None, 0), None);
        let keep = status(Some(&welcomed(&[3], Adoption::Keep)), 2);
        assert_eq!(
            keep,
            Some(client::HolderStatus {
                pid: 77,
                build: "b0".into(),
                protocol: 2,
                sessions: 2,
                draining: false,
            })
        );
        let drain = status(Some(&welcomed(&[], Adoption::Drain)), 0);
        assert_eq!(drain.map(|status| status.draining), Some(true));
    }
}
