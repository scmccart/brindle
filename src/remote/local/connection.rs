//! The one connection a Brindle process keeps to the local core, shared by
//! all its local tabs.
//!
//! - **Reader thread:** reads frames and demultiplexes them by session. Each
//!   session's output goes to its [`SessionSink`], which parses it on this
//!   thread (as alacritty's I/O thread parsed a PTY's) and wakes the UI.
//!   `SPAWNED` moves a spawn's sink into the session map before the next
//!   frame is read, so no output is missed.
//! - **Writer thread:** sends queued messages in order, so the UI thread
//!   never blocks on the socket.
//!
//! The reader holds the state lock only to look a sink up; each sink has
//! its own lock, taken without the state lock, so parsing never waits on
//! the map and closing a tab never waits on parsing.

use std::collections::HashMap;
use std::io::{BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use parking_lot::Mutex;

use super::{Connected, Endpoint};
use crate::remote::protocol::client::{self, Data, Policy, Resize, SessionRef, ToClient, ToCore};
use crate::remote::protocol::{Frame, RequestId, SessionId};

/// How long a spawn waits for its `SPAWNED`.
const SPAWN_LIMIT: Duration = Duration::from_secs(30);
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
}

type Sink = Arc<Mutex<Box<dyn SessionSink>>>;

/// What a new session runs.
pub struct SpawnRequest {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// The program's whole environment.
    pub env: std::collections::BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    pub cell_w: u16,
    pub cell_h: u16,
    pub scrollback: u32,
}

struct PendingSpawn {
    sink: Sink,
    reply: mpsc::SyncSender<Result<(SessionId, u32), String>>,
}

#[derive(Default)]
struct State {
    sessions: HashMap<SessionId, Sink>,
    spawns: HashMap<RequestId, PendingSpawn>,
    /// The connection ended for good; the next tab makes a new one.
    dead: bool,
}

/// The connection to the core.
pub struct Connection {
    state: Mutex<State>,
    outgoing: mpsc::Sender<ToCore>,
    next_req: AtomicU64,
    #[allow(dead_code)]
    endpoint: Endpoint,
}

impl Connection {
    /// Starts the reader and writer threads on a negotiated connection.
    pub fn start(endpoint: Endpoint, connected: Connected) -> Result<Arc<Self>> {
        let (outgoing, queue) = mpsc::channel();
        let writer = connected.stream.try_clone()?;
        let reader = connected.stream;
        let this = Arc::new(Self {
            state: Mutex::new(State::default()),
            outgoing,
            next_req: AtomicU64::new(1),
            endpoint,
        });
        std::thread::Builder::new()
            .name("server-writer".into())
            .spawn(move || write_loop(writer, queue))?;
        let reading = this.clone();
        std::thread::Builder::new()
            .name("server-reader".into())
            .spawn(move || reading.read_loop(reader))?;
        Ok(this)
    }

    pub fn is_dead(&self) -> bool {
        self.state.lock().dead
    }

    fn send(&self, message: ToCore) {
        self.outgoing.send(message).ok();
    }

    /// Starts a session whose output goes to `sink`, and waits for the core
    /// to report its id and pid.
    pub fn spawn(self: &Arc<Self>, request: SpawnRequest, sink: Box<dyn SessionSink>) -> Result<Session> {
        let req = self.next_req.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = mpsc::sync_channel(1);
        {
            let mut state = self.state.lock();
            if state.dead {
                bail!("the connection to the terminal server ended");
            }
            state.spawns.insert(req, PendingSpawn { sink: Arc::new(Mutex::new(sink)), reply });
        }
        let command = Some(request.command).filter(|command| !command.is_empty());
        self.send(ToCore::Spawn(client::Spawn {
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
            policy: Policy::Kill,
        }));
        let answer = answer.recv_timeout(SPAWN_LIMIT);
        // Gone already when answered; a late `SPAWNED` is closed.
        self.state.lock().spawns.remove(&req);
        match answer {
            Ok(Ok((id, pid))) => Ok(Session { id, pid, conn: self.clone() }),
            Ok(Err(message)) => Err(anyhow!("{message}")),
            Err(_) => bail!(
                "the terminal server didn't start the session within {} s",
                SPAWN_LIMIT.as_secs()
            ),
        }
    }

    fn read_loop(self: Arc<Self>, stream: UnixStream) {
        let mut reader = BufReader::with_capacity(READ_BUFFER, stream);
        let reason = loop {
            let frame = match Frame::read_from_or_eof(&mut reader) {
                Ok(Some(frame)) => frame,
                Ok(None) => break "the terminal server closed the connection".to_string(),
                Err(error) => break format!("reading from the terminal server failed: {error}"),
            };
            match ToClient::decode(&frame) {
                Ok(message) => self.handle(message),
                Err(error) => log::warn!("terminal server: {error}"),
            }
        };
        log::warn!("terminal server connection ended: {reason}");
        self.ended(&reason);
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
                match state.spawns.remove(&spawned.req) {
                    Some(pending) => {
                        state.sessions.insert(spawned.session, pending.sink);
                        pending.reply.try_send(Ok((spawned.session, spawned.pid))).ok();
                    }
                    None => {
                        // Its spawner gave up: nobody shows this session.
                        drop(state);
                        self.send(ToCore::Close(SessionRef { session: spawned.session }));
                    }
                }
            }
            ToClient::Error(error) => {
                let pending = error.req.and_then(|req| self.state.lock().spawns.remove(&req));
                match pending {
                    Some(pending) => {
                        pending.reply.try_send(Err(error.message)).ok();
                    }
                    None => log::debug!("terminal server: {}", error.message),
                }
            }
            ToClient::Exit(exit) => {
                let sink = self.state.lock().sessions.remove(&exit.session);
                if let Some(sink) = sink {
                    sink.lock().exited(exit.code);
                }
            }
            ToClient::ReplayStart(start) => match self.sink(start.session) {
                Some(sink) => sink.lock().replay(true),
                None => self.send(ToCore::Close(SessionRef { session: start.session })),
            },
            ToClient::ReplayEnd(end) => {
                if let Some(sink) = self.sink(end.session) {
                    sink.lock().replay(false);
                }
            }
            ToClient::Welcome(_) | ToClient::Sessions(_) | ToClient::ServerStatus(_) | ToClient::Ok(_) => {}
        }
    }

    fn sink(&self, session: SessionId) -> Option<Sink> {
        self.state.lock().sessions.get(&session).cloned()
    }

    /// The connection is gone: every session is lost and every spawn fails.
    fn ended(&self, reason: &str) {
        let (sessions, spawns) = {
            let mut state = self.state.lock();
            state.dead = true;
            (std::mem::take(&mut state.sessions), std::mem::take(&mut state.spawns))
        };
        for pending in spawns.into_values() {
            pending.reply.try_send(Err(reason.to_string())).ok();
        }
        for sink in sessions.into_values() {
            sink.lock().lost(reason);
        }
    }
}

/// Sends queued messages until the queue closes or the socket fails.
fn write_loop(mut stream: UnixStream, queue: mpsc::Receiver<ToCore>) {
    for message in queue {
        let written = message.encode().and_then(|frame| frame.write_to(&mut stream));
        if let Err(error) = written {
            log::warn!("writing to the terminal server failed: {error}");
            stream.shutdown(std::net::Shutdown::Both).ok();
            return;
        }
    }
    stream.flush().ok();
}

/// One session, as a local tab's backend holds it.
pub struct Session {
    id: SessionId,
    pid: u32,
    conn: Arc<Connection>,
}

impl Session {
    /// The program's pid.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn input(&self, bytes: &[u8]) {
        for chunk in bytes.chunks(INPUT_CHUNK) {
            self.conn.send(ToCore::Data(Data { session: self.id, bytes: chunk.to_vec() }));
        }
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        self.conn.send(ToCore::Resize(Resize { session: self.id, cols, rows, cell_w, cell_h }));
    }

    /// Ends the session (its policy is `kill`). Once is enough; a session
    /// that already exited isn't closed again.
    pub fn close(&self) {
        let known = self.conn.state.lock().sessions.remove(&self.id).is_some();
        if known {
            self.conn.send(ToCore::Close(SessionRef { session: self.id }));
        }
    }
}
