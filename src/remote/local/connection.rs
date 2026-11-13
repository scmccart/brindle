//! The one connection a Brindle process keeps to the local core, shared by
//! all its local tabs.
//!
//! - **Reader thread:** reads frames and demultiplexes them by session. Each
//!   session's output goes to its [`SessionSink`], which parses it on this
//!   thread (as alacritty's I/O thread parsed a PTY's) and wakes the UI.
//!   `SPAWNED` moves a spawn's sink into the session map before the next
//!   frame is read, so no output is missed.
//! - **Writer thread:** sends queued messages in order, so the UI thread
//!   never blocks on the socket. While the connection is down it holds
//!   them, so input typed meanwhile reaches each session once reconnected.
//! - **Reconnect:** when the connection ends (a core restart or upgrade),
//!   the reader connects again with the same `client_token`, starting a
//!   core itself if none appears within [`START_AFTER`]. The core then
//!   replays every session it still has for this client. A `LIST` sent
//!   right after `WELCOME` tells which sessions it doesn't have: those, and
//!   every session when no core answers within [`RECONNECT_LIMIT`], are
//!   lost, and their tabs say so.
//!
//! The reader holds the state lock only to look a sink up; each sink has
//! its own lock, taken without the state lock, so parsing never waits on
//! the map and closing a tab never waits on parsing.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::BufReader;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use parking_lot::{Condvar, Mutex};

use super::{Connected, Endpoint, Refused, Stream};
use crate::remote::protocol::client::{
    self, Data, Kill, KillTarget, Policy, Reparent, ReplayStart, Request, Resize, SessionInfo,
    SessionRef, Spawned, ToClient, ToCore,
};
use crate::remote::protocol::{Frame, RequestId, SessionId};

/// How long a request waits for its answer (a spawn's `SPAWNED`, an
/// attach's `REPLAY_START`), a reconnect included.
const REQUEST_LIMIT: Duration = Duration::from_secs(30);
/// While reconnecting, how long to wait for a core to appear before
/// starting one, so an upgrading client's new core wins (design.md).
pub const START_AFTER: Duration = Duration::from_secs(5);
/// How long reconnecting may take before the sessions are lost.
pub const RECONNECT_LIMIT: Duration = Duration::from_secs(15);
const RECONNECT_POLL: Duration = Duration::from_millis(100);
/// The largest `DATA` frame sent; the frame limit is 1 MiB.
const INPUT_CHUNK: usize = 64 * 1024;
/// The reader's buffer.
const READ_BUFFER: usize = 64 * 1024;

/// Where a session's output and events go: its terminal.
pub trait SessionSink: Send {
    /// Output from the program, or from a replay.
    fn output(&mut self, bytes: &[u8]);
    /// A replay starts (`true`) or ends.
    fn replay(&mut self, active: bool);
    /// The program exited.
    fn exited(&mut self, code: Option<i32>);
    /// The session can't be got back; `reason` says why.
    fn lost(&mut self, reason: &str);
    /// The session never started: no server, or the core refused it.
    fn failed(&mut self, error: &anyhow::Error);
}

pub type Sink = Arc<Mutex<Box<dyn SessionSink>>>;

/// What a new session runs.
pub struct SpawnRequest {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// The program's whole environment.
    pub env: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    pub cell_w: u16,
    pub cell_h: u16,
    pub scrollback: u32,
    /// `detach` keeps the program running when its tab closes or Brindle
    /// goes away; `kill` ends it.
    pub policy: Policy,
    /// The profile that started it, for whoever attaches it later.
    pub profile: Option<String>,
}

/// Which detached session to attach, and the tab's size.
pub struct AttachRequest {
    pub session: SessionId,
    /// The key the session was listed with, kept for the tab's state.
    pub key: String,
    pub cols: u16,
    pub rows: u16,
}

type Reply<T> = mpsc::SyncSender<Result<T, String>>;

/// What a request waits for.
// Unused on Windows until sessions are listed and attached there (task 8.1
// of `terminal-server-windows`).
#[cfg_attr(windows, allow(dead_code))]
enum Waiting {
    /// `SPAWNED`, whose sink then joins the session map.
    Spawn { sink: Sink, reply: Reply<Spawned> },
    /// The `REPLAY_START` answering an `ATTACH` (its pid), whose sink joins
    /// the map before the replay's first `DATA` is read.
    Attach { sink: Sink, reply: Reply<u32> },
    /// `SESSIONS`.
    List(Reply<Vec<SessionInfo>>),
    /// `OK`.
    Done(Reply<()>),
}

impl Waiting {
    fn fail(self, message: String) {
        match self {
            Self::Spawn { reply, .. } => reply.try_send(Err(message)).ok(),
            Self::Attach { reply, .. } => reply.try_send(Err(message)).ok(),
            Self::List(reply) => reply.try_send(Err(message)).ok(),
            Self::Done(reply) => reply.try_send(Err(message)).ok(),
        };
    }
}

struct Pending {
    waiting: Waiting,
    /// Its request went out on the current connection: if that ends, the
    /// answer is lost with it.
    written: bool,
}

#[derive(Default)]
struct State {
    sessions: HashMap<SessionId, Sink>,
    /// The window each session's console is parented to (Windows), sent
    /// again after a reconnect's replay: a core doesn't keep it.
    parents: HashMap<SessionId, u64>,
    requests: HashMap<RequestId, Pending>,
    /// The `LIST` sent after a reconnect's `WELCOME`.
    list: Option<RequestId>,
    /// The connection ended for good; the next tab makes a new one.
    dead: bool,
}

/// The stream the writer writes to; `None` while reconnecting.
#[derive(Default)]
struct Slot {
    stream: Option<Arc<Stream>>,
    closed: bool,
}

/// The connection to the core.
pub struct Connection {
    state: Mutex<State>,
    outgoing: mpsc::Sender<ToCore>,
    slot: Mutex<Slot>,
    slot_ready: Condvar,
    next_req: AtomicU64,
    endpoint: Endpoint,
    token: String,
}

impl Connection {
    /// Starts the reader and writer threads on a negotiated connection.
    pub fn start(endpoint: Endpoint, token: &str, connected: Connected) -> Result<Arc<Self>> {
        let (outgoing, queue) = mpsc::channel();
        let writer = Arc::new(connected.stream.try_clone()?);
        let this = Arc::new(Self {
            state: Mutex::new(State::default()),
            outgoing,
            slot: Mutex::new(Slot { stream: Some(writer), closed: false }),
            slot_ready: Condvar::new(),
            next_req: AtomicU64::new(1),
            endpoint,
            token: token.to_string(),
        });
        let writing = this.clone();
        std::thread::Builder::new()
            .name("server-writer".into())
            .spawn(move || writing.write_loop(queue))?;
        let reading = this.clone();
        std::thread::Builder::new()
            .name("server-reader".into())
            .spawn(move || reading.run(connected.stream))?;
        Ok(this)
    }

    pub fn is_dead(&self) -> bool {
        self.state.lock().dead
    }

    fn send(&self, message: ToCore) {
        self.outgoing.send(message).ok();
    }

    fn req(&self) -> RequestId {
        self.next_req.fetch_add(1, Ordering::Relaxed)
    }

    /// Sends the request `make` builds and waits up to [`REQUEST_LIMIT`]
    /// for its answer. Not on the UI thread: this blocks.
    fn ask<T>(
        &self,
        what: &str,
        make: impl FnOnce(RequestId, Reply<T>) -> (Waiting, ToCore),
    ) -> Result<T> {
        let req = self.req();
        let (reply, answer) = mpsc::sync_channel(1);
        let (waiting, message) = make(req, reply);
        {
            let mut state = self.state.lock();
            if state.dead {
                bail!("the connection to the terminal server ended");
            }
            state.requests.insert(req, Pending { waiting, written: false });
        }
        self.send(message);
        let answer = answer.recv_timeout(REQUEST_LIMIT);
        // Gone already when answered; a late answer is dealt with by `handle`.
        self.state.lock().requests.remove(&req);
        match answer {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(message)) => Err(anyhow!("{message}")),
            Err(_) => bail!(
                "the terminal server didn't {what} within {} s",
                REQUEST_LIMIT.as_secs()
            ),
        }
    }

    /// Starts a session whose output goes to `sink`, and waits for the core
    /// to report its id and pid. Not on the UI thread: this blocks.
    pub fn spawn(self: &Arc<Self>, request: SpawnRequest, sink: Sink) -> Result<Running> {
        let command = Some(request.command).filter(|command| !command.is_empty());
        let spawned = self.ask("start the session", |req, reply| {
            let message = ToCore::Spawn(client::Spawn {
                req,
            command,
            args: request.args,
            cwd: request.cwd,
            env: request.env,
            cols: request.cols,
            rows: request.rows,
            cell_w: request.cell_w,
            cell_h: request.cell_h,
            scrollback: request.scrollback,
                policy: request.policy,
                profile: request.profile,
            });
            (Waiting::Spawn { sink, reply }, message)
        })?;
        Ok(Running { id: spawned.session, pid: spawned.pid, key: spawned.key, conn: self.clone() })
    }

    /// Attaches a detached session, whose replay and then live output go to
    /// `sink`, and waits for its `REPLAY_START`. Not on the UI thread.
    pub fn attach(self: &Arc<Self>, request: AttachRequest, sink: Sink) -> Result<Running> {
        let session = request.session;
        let pid = self.ask("attach the session", |req, reply| {
            let message = ToCore::Attach(client::Attach {
                req,
                session,
                cols: request.cols,
                rows: request.rows,
            });
            (Waiting::Attach { sink, reply }, message)
        })?;
        Ok(Running { id: session, pid, key: request.key, conn: self.clone() })
    }

    /// The core's sessions, this client's and detached ones. Blocks.
    // Unused on Windows until sessions are listed and attached there (task 8.1
    // of `terminal-server-windows`).
    #[cfg_attr(windows, allow(dead_code))]
    pub fn list(&self) -> Result<Vec<SessionInfo>> {
        self.ask("list its sessions", |req, reply| {
            (Waiting::List(reply), ToCore::List(Request { req }))
        })
    }

    /// Ends a session whatever its policy or client. Blocks.
    // Unused on Windows until sessions are listed and attached there (task 8.1
    // of `terminal-server-windows`).
    #[cfg_attr(windows, allow(dead_code))]
    pub fn kill(&self, session: SessionId) -> Result<()> {
        self.ask("end the session", |req, reply| {
            let target = KillTarget::Session(session);
            (Waiting::Done(reply), ToCore::Kill(Kill { req, target }))
        })
    }

    /// Reads each connection in turn, reconnecting when one ends.
    fn run(self: Arc<Self>, mut stream: Stream) {
        loop {
            let reason = self.read(stream);
            log::warn!("terminal server connection ended ({reason}); reconnecting");
            self.disconnected(&reason);
            match self.reconnect() {
                Ok(next) => stream = next,
                Err(reason) => {
                    log::warn!("terminal server lost: {reason}");
                    return self.ended(&reason);
                }
            }
        }
    }

    /// Reads one connection until it ends, saying why.
    fn read(&self, stream: Stream) -> String {
        let mut reader = BufReader::with_capacity(READ_BUFFER, stream);
        loop {
            let frame = match Frame::read_from_or_eof(&mut reader) {
                Ok(Some(frame)) => frame,
                Ok(None) => return "the terminal server closed the connection".to_string(),
                Err(error) => return format!("reading from the terminal server failed: {error}"),
            };
            match ToClient::decode(&frame) {
                Ok(message) => self.handle(message),
                Err(error) => log::warn!("terminal server: {error}"),
            }
        }
    }

    fn handle(&self, message: ToClient) {
        match message {
            ToClient::Data(Data { session, bytes }) => {
                if let Some(sink) = self.sink(session) {
                    sink.lock().output(&bytes);
                }
            }
            ToClient::Spawned(spawned) => {
                let mut state = self.state.lock();
                match state.requests.remove(&spawned.req).map(|pending| pending.waiting) {
                    Some(Waiting::Spawn { sink, reply }) => {
                        state.sessions.insert(spawned.session, sink);
                        reply.try_send(Ok(spawned)).ok();
                    }
                    other => {
                        // Its spawner gave up: nobody shows this session.
                        drop(state);
                        if let Some(other) = other {
                            other.fail("answered with SPAWNED".into());
                        }
                        self.send(ToCore::Close(SessionRef { session: spawned.session }));
                    }
                }
            }
            ToClient::Error(error) => {
                let pending = error.req.and_then(|req| self.state.lock().requests.remove(&req));
                match pending {
                    Some(pending) => pending.waiting.fail(error.message),
                    None => log::debug!("terminal server: {}", error.message),
                }
            }
            ToClient::Sessions(sessions) if self.state.lock().list != Some(sessions.req) => {
                let pending = self.state.lock().requests.remove(&sessions.req);
                match pending.map(|pending| pending.waiting) {
                    Some(Waiting::List(reply)) => {
                        reply.try_send(Ok(sessions.sessions)).ok();
                    }
                    Some(other) => other.fail("answered with SESSIONS".into()),
                    None => {}
                }
            }
            ToClient::Ok(ok) => {
                let pending = self.state.lock().requests.remove(&ok.req);
                match pending.map(|pending| pending.waiting) {
                    Some(Waiting::Done(reply)) => {
                        reply.try_send(Ok(())).ok();
                    }
                    Some(other) => other.fail("answered with OK".into()),
                    None => {}
                }
            }
            ToClient::ReplayStart(ReplayStart { req: Some(req), session, pid }) => {
                let mut state = self.state.lock();
                match state.requests.remove(&req).map(|pending| pending.waiting) {
                    Some(Waiting::Attach { sink, reply }) => {
                        state.sessions.insert(session, sink.clone());
                        drop(state);
                        sink.lock().replay(true);
                        reply.try_send(Ok(pid)).ok();
                    }
                    other => {
                        // Its attacher gave up: leave the session detached,
                        // as it was.
                        drop(state);
                        if let Some(other) = other {
                            other.fail("answered with REPLAY_START".into());
                        }
                        self.send(ToCore::Detach(SessionRef { session }));
                    }
                }
            }
            ToClient::Exit(exit) => {
                let sink = {
                    let mut state = self.state.lock();
                    state.parents.remove(&exit.session);
                    state.sessions.remove(&exit.session)
                };
                if let Some(sink) = sink {
                    match exit.lost {
                        Some(reason) => sink.lock().lost(&format!("the terminal server lost it: {reason}")),
                        None => sink.lock().exited(exit.code),
                    }
                }
            }
            ToClient::ReplayStart(start) => match self.sink(start.session) {
                Some(sink) => sink.lock().replay(true),
                // Not one of this process's tabs (any more): end it.
                None => self.send(ToCore::Close(SessionRef { session: start.session })),
            },
            ToClient::ReplayEnd(end) => {
                if let Some(sink) = self.sink(end.session) {
                    sink.lock().replay(false);
                }
                // The core that replayed may be a new one, which doesn't
                // know the session's window.
                let parent = self.state.lock().parents.get(&end.session).copied();
                if let Some(hwnd) = parent {
                    self.send(ToCore::Reparent(Reparent { session: end.session, hwnd }));
                }
            }
            ToClient::Sessions(sessions) => {
                let lost = {
                    let mut state = self.state.lock();
                    if state.list != Some(sessions.req) {
                        return;
                    }
                    state.list = None;
                    let kept: HashSet<SessionId> =
                        sessions.sessions.iter().filter(|s| s.attached).map(|s| s.session).collect();
                    state.parents.retain(|session, _| kept.contains(session));
                    missing(&mut state.sessions, &kept)
                };
                for sink in lost {
                    sink.lock().lost("the terminal server no longer has this session");
                }
            }
            ToClient::Welcome(_) | ToClient::ServerStatus(_) => {}
        }
    }

    fn sink(&self, session: SessionId) -> Option<Sink> {
        self.state.lock().sessions.get(&session).cloned()
    }

    /// The connection ended: the writer holds its messages, and requests
    /// that went out on it fail, since their answers are gone.
    fn disconnected(&self, reason: &str) {
        if let Some(stream) = self.slot.lock().stream.take() {
            stream.shutdown(std::net::Shutdown::Both).ok();
        }
        let mut state = self.state.lock();
        state.list = None;
        let lost: Vec<RequestId> =
            state.requests.iter().filter(|(_, pending)| pending.written).map(|(&req, _)| req).collect();
        for req in lost {
            if let Some(pending) = state.requests.remove(&req) {
                pending.waiting.fail(format!("the terminal server went away ({reason})"));
            }
        }
    }

    /// Connects again with the same token, starting a core if none appears
    /// within [`START_AFTER`], for at most [`RECONNECT_LIMIT`]. Then asks
    /// for the session list and hands the writer the new stream.
    fn reconnect(&self) -> std::result::Result<Stream, String> {
        let started = Instant::now();
        let mut next_start = START_AFTER;
        loop {
            let elapsed = started.elapsed();
            if elapsed >= RECONNECT_LIMIT {
                return Err(format!("no terminal server came back within {} s", RECONNECT_LIMIT.as_secs()));
            }
            match super::try_connect(&self.endpoint) {
                Ok(Some(stream)) => match super::hello(stream, &self.token) {
                    Ok(connected) => return self.resume(connected).map_err(|error| format!("{error:#}")),
                    Err(error) if error.downcast_ref::<Refused>().is_some() => return Err(format!("{error:#}")),
                    Err(error) => log::debug!("reconnecting: {error:#}"),
                },
                Ok(None) if elapsed >= next_start => {
                    next_start += START_AFTER;
                    if let Err(error) = super::start_core(&self.endpoint) {
                        log::warn!("reconnecting: {error:#}");
                    }
                    continue;
                }
                Ok(None) => {}
                Err(error) => log::debug!("reconnecting: {error:#}"),
            }
            std::thread::sleep(RECONNECT_POLL);
        }
    }

    fn resume(&self, connected: Connected) -> Result<Stream> {
        let Connected { mut stream, welcome } = connected;
        log::info!("reconnected to the terminal server (core pid {})", welcome.pid);
        let req = self.req();
        self.state.lock().list = Some(req);
        ToCore::List(Request { req }).encode()?.write_to(&mut stream)?;
        let writer = Arc::new(stream.try_clone()?);
        {
            let mut slot = self.slot.lock();
            slot.stream = Some(writer);
        }
        self.slot_ready.notify_all();
        Ok(stream)
    }

    /// The connection is gone for good: every session is lost and every
    /// spawn fails.
    fn ended(&self, reason: &str) {
        let (sessions, requests) = {
            let mut state = self.state.lock();
            state.dead = true;
            state.parents.clear();
            (std::mem::take(&mut state.sessions), std::mem::take(&mut state.requests))
        };
        self.slot.lock().closed = true;
        self.slot_ready.notify_all();
        for pending in requests.into_values() {
            pending.waiting.fail(reason.to_string());
        }
        for sink in sessions.into_values() {
            sink.lock().lost(reason);
        }
    }

    /// Sends queued messages in order. While there is no stream it waits
    /// for one; a message whose write failed goes out on the next stream.
    fn write_loop(&self, queue: mpsc::Receiver<ToCore>) {
        for message in queue {
            let frame = match message.encode() {
                Ok(frame) => frame,
                Err(error) => {
                    log::warn!("terminal server message not sent: {error}");
                    continue;
                }
            };
            let request = match &message {
                ToCore::Spawn(client::Spawn { req, .. })
                | ToCore::Attach(client::Attach { req, .. })
                | ToCore::List(Request { req })
                | ToCore::Kill(Kill { req, .. }) => Some(*req),
                _ => None,
            };
            loop {
                let stream = {
                    let mut slot = self.slot.lock();
                    loop {
                        if slot.closed {
                            return;
                        }
                        if let Some(stream) = &slot.stream {
                            break stream.clone();
                        }
                        self.slot_ready.wait(&mut slot);
                    }
                };
                if let Some(req) = request {
                    self.mark_written(req, true);
                }
                match frame.write_to(&mut &*stream) {
                    Ok(()) => break,
                    Err(error) => {
                        log::debug!("writing to the terminal server failed: {error}");
                        if let Some(req) = request {
                            self.mark_written(req, false);
                        }
                        let mut slot = self.slot.lock();
                        if slot.stream.as_ref().is_some_and(|current| Arc::ptr_eq(current, &stream)) {
                            slot.stream = None;
                        }
                    }
                }
            }
        }
    }

    fn mark_written(&self, req: RequestId, written: bool) {
        if let Some(pending) = self.state.lock().requests.get_mut(&req) {
            pending.written = written;
        }
    }
}

/// Takes the sessions out of `sessions` that the core no longer holds for
/// this client (not in `kept`).
fn missing<T>(sessions: &mut HashMap<SessionId, T>, kept: &HashSet<SessionId>) -> Vec<T> {
    let gone: Vec<SessionId> = sessions.keys().filter(|id| !kept.contains(id)).copied().collect();
    gone.into_iter().filter_map(|id| sessions.remove(&id)).collect()
}

/// A session the core started, once [`super::Session`] has it.
pub struct Running {
    id: SessionId,
    pid: u32,
    key: String,
    conn: Arc<Connection>,
}

impl Running {
    /// The program's pid.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn id(&self) -> SessionId {
        self.id
    }

    /// The session's key; empty from a core that predates keys.
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn input(&self, bytes: &[u8]) {
        for chunk in bytes.chunks(INPUT_CHUNK) {
            self.conn.send(ToCore::Data(Data { session: self.id, bytes: chunk.to_vec() }));
        }
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        self.conn.send(ToCore::Resize(Resize { session: self.id, cols, rows, cell_w, cell_h }));
    }

    /// Makes the window `hwnd` the owner of the session's console window
    /// (`REPARENT`, which only a Windows core takes), and keeps it to send
    /// again after each reconnect's replay. A session that exited or was
    /// lost sends nothing.
    pub fn reparent(&self, hwnd: u64) {
        {
            let mut state = self.conn.state.lock();
            if !state.sessions.contains_key(&self.id) {
                return;
            }
            state.parents.insert(self.id, hwnd);
        }
        self.conn.send(ToCore::Reparent(Reparent { session: self.id, hwnd }));
    }

    /// Ends the session (`CLOSE`). Once is enough; a session that already
    /// exited or was lost isn't closed again.
    pub fn close(&self) {
        if self.forget() {
            self.conn.send(ToCore::Close(SessionRef { session: self.id }));
        }
    }

    /// Leaves the session running with no client (`DETACH`), unless it
    /// already exited or was lost.
    pub fn detach(&self) {
        if self.forget() {
            self.conn.send(ToCore::Detach(SessionRef { session: self.id }));
        }
    }

    /// Stops showing the session; says whether it was still shown.
    fn forget(&self) -> bool {
        let mut state = self.conn.state.lock();
        state.parents.remove(&self.id);
        state.sessions.remove(&self.id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_the_core_no_longer_lists_are_missing() {
        let mut sessions: HashMap<SessionId, &str> = [(1, "a"), (2, "b"), (3, "c")].into();
        let mut lost = missing(&mut sessions, &[2].into());
        lost.sort();
        assert_eq!(lost, ["a", "c"]);
        assert_eq!(sessions.keys().copied().collect::<Vec<_>>(), [2]);
        assert!(missing(&mut sessions, &[2, 9].into()).is_empty());
    }
}
