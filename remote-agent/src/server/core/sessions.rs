//! The core's sessions: spawning through a holder, their I/O between the
//! client and the holder, hang-ups and exits.

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::{self, Data, Exit, Resize, Spawned, ToClient};
use brindle_remote_protocol::holder::{self as h1, HeldSession, SessionMeta, ToHolder};

use super::clients::error;
use super::holders::WaitingSpawn;
use super::{Core, log};
use crate::server::event_loop::{EventLoop, Token};

pub(super) struct Session {
    /// The holder it runs under.
    pub(super) holder: Token,
    /// The program's pid; 0 until the holder reports it.
    pub(super) pid: u32,
    pub(super) meta: SessionMeta,
    pub(super) cols: u16,
    pub(super) rows: u16,
    /// The client its I/O goes to.
    pub(super) attached: Option<Token>,
    /// The client's `SPAWN`, until the holder reports the program. It is
    /// sent again if the holder goes away first (an empty holder exits, and
    /// may not have read it).
    spawning: Option<client::Spawn>,
    /// Its holder was told to stop reading it, because its client is behind.
    paused: bool,
    /// Its holder was told to hang it up.
    hung_up: bool,
    /// What its holder kept, for a session this core adopted.
    adopted: Option<Adopted>,
}

/// A held session's last checkpoint and the raw output since then, kept as
/// opaque bytes until the session's emulator is rebuilt from them.
#[derive(Default)]
#[allow(
    dead_code,
    reason = "task 5.4 rebuilds the session's emulator from them"
)]
pub(super) struct Adopted {
    pub(super) checkpoint: Vec<u8>,
    pub(super) tail: Vec<u8>,
    pub(super) truncated: bool,
}

impl Session {
    /// A session reported in a holder's `WELCOME`. It has no client until
    /// its owner comes back.
    pub(super) fn adopted(holder: Token, held: HeldSession) -> Self {
        Self {
            holder,
            pid: held.pid,
            meta: SessionMeta::from_value(&held.meta),
            cols: held.cols,
            rows: held.rows,
            attached: None,
            spawning: None,
            paused: false,
            hung_up: false,
            adopted: Some(Adopted {
                truncated: held.truncated,
                ..Adopted::default()
            }),
        }
    }

    pub(super) fn store_adopted(&mut self, checkpoint: bool, bytes: Vec<u8>) {
        let adopted = self.adopted.get_or_insert_with(Adopted::default);
        if checkpoint {
            adopted.checkpoint = bytes;
        } else {
            adopted.tail = bytes;
        }
    }
}

/// The command line `list` shows; empty for the login shell.
fn command_line(spawn: &client::Spawn) -> String {
    match &spawn.command {
        Some(command) => std::iter::once(command.as_str())
            .chain(spawn.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        None => String::new(),
    }
}

impl<L: EventLoop> Core<L> {
    /// A client's `SPAWN`: it gets an id now and goes to a holder of the
    /// core's build once one is ready.
    pub(super) fn client_spawn(&mut self, client: Token, spawn: client::Spawn) {
        let id = self.allocate_id();
        self.waiting.push(WaitingSpawn { client, id, spawn });
        self.place_waiting();
    }

    fn allocate_id(&mut self) -> SessionId {
        loop {
            let id = self.next_session;
            self.next_session = self.next_session.checked_add(1).unwrap_or(1);
            let taken = self.sessions.contains_key(&id)
                || self.waiting.iter().any(|waiting| waiting.id == id);
            if id != 0 && !taken {
                return id;
            }
        }
    }

    pub(super) fn send_spawn(&mut self, holder: Token, waiting: WaitingSpawn) {
        let WaitingSpawn { client, id, spawn } = waiting;
        let meta = SessionMeta {
            command: command_line(&spawn),
            cwd: spawn.cwd.clone(),
            owner: self.client_token(client).unwrap_or_default().to_string(),
            policy: spawn.policy,
            scrollback: spawn.scrollback,
        };
        self.send_holder(
            holder,
            ToHolder::Spawn(h1::Spawn {
                session: id,
                command: spawn.command.clone(),
                args: spawn.args.clone(),
                cwd: spawn.cwd.clone(),
                env: spawn.env.clone(),
                cols: spawn.cols,
                rows: spawn.rows,
                cell_w: spawn.cell_w,
                cell_h: spawn.cell_h,
                meta: meta.to_value(),
            }),
        );
        self.sessions.insert(
            id,
            Session {
                holder,
                pid: 0,
                meta,
                cols: spawn.cols,
                rows: spawn.rows,
                attached: Some(client),
                spawning: Some(spawn),
                paused: false,
                hung_up: false,
                adopted: None,
            },
        );
    }

    /// The session if it runs under `holder`; anything else is a stale or
    /// confused report.
    fn holder_session(&mut self, holder: Token, id: SessionId) -> Option<&mut Session> {
        let session = self.sessions.get_mut(&id);
        if session
            .as_ref()
            .is_some_and(|session| session.holder != holder)
        {
            log!("holder {holder} reported session {id}, which isn't its");
            return None;
        }
        session
    }

    pub(super) fn session_spawned(&mut self, holder: Token, id: SessionId, pid: u32) {
        let Some(session) = self.holder_session(holder, id) else {
            return;
        };
        session.pid = pid;
        let req = session.spawning.take().map(|spawn| spawn.req);
        let client = session.attached;
        log!("session {id}: pid {pid} ({:?})", session.meta.command);
        if let (Some(client), Some(req)) = (client, req) {
            self.send_client(
                client,
                ToClient::Spawned(Spawned {
                    req,
                    session: id,
                    pid,
                }),
            );
            if self.client_is_over(client) {
                self.set_paused(id, true);
            }
        }
    }

    pub(super) fn session_spawn_failed(&mut self, holder: Token, id: SessionId, message: String) {
        if self.holder_session(holder, id).is_none() {
            return;
        }
        let session = self.sessions.remove(&id).expect("the session");
        log!("session {id}: {message}");
        if let (Some(client), Some(spawn)) = (session.attached, session.spawning) {
            self.send_client(client, error(Some(spawn.req), None, message));
        }
    }

    /// A session whose holder went away: one still starting goes to another
    /// holder; the rest have ended.
    pub(super) fn session_lost(&mut self, holder: Token, id: SessionId) {
        if self.holder_session(holder, id).is_none() {
            return;
        }
        let session = self.sessions.get_mut(&id).expect("the session");
        if let Some(spawn) = session.spawning.take() {
            let client = session.attached;
            self.sessions.remove(&id);
            if let Some(client) = client {
                self.waiting.push(WaitingSpawn { client, id, spawn });
            }
            return;
        }
        self.session_exited(holder, id, None);
    }

    pub(super) fn session_output(&mut self, holder: Token, id: SessionId, bytes: Vec<u8>) {
        let Some(session) = self.holder_session(holder, id) else {
            return;
        };
        if let Some(client) = session.attached {
            self.send_client(client, ToClient::Data(Data { session: id, bytes }));
        }
    }

    /// The program exited (`code` is `None` for a signal, or a lost holder):
    /// the client gets `EXIT` and the session is gone.
    pub(super) fn session_exited(&mut self, holder: Token, id: SessionId, code: Option<i32>) {
        if self.holder_session(holder, id).is_none() {
            return;
        }
        let session = self.sessions.remove(&id).expect("the session");
        log!("session {id}: exited ({code:?})");
        if let Some(client) = session.attached {
            self.send_client(client, ToClient::Exit(Exit { session: id, code }));
        }
    }

    /// Asks the holder to hang the session up; its `EXITED` follows.
    pub(super) fn hang_up(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.hung_up {
            return;
        }
        session.hung_up = true;
        let holder = session.holder;
        self.send_holder(holder, ToHolder::Hangup(h1::SessionRef { session: id }));
    }

    /// The session loses its client, and isn't paused for it any more.
    pub(super) fn detach_session(&mut self, id: SessionId) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.attached = None;
        }
        self.set_paused(id, false);
    }

    pub(super) fn sessions_of(&self, client: Token) -> Vec<SessionId> {
        self.sessions
            .iter()
            .filter(|(_, session)| session.attached == Some(client))
            .map(|(&id, _)| id)
            .collect()
    }

    fn set_paused(&mut self, id: SessionId, paused: bool) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.paused == paused {
            return;
        }
        session.paused = paused;
        let holder = session.holder;
        let target = h1::SessionRef { session: id };
        self.send_holder(
            holder,
            if paused {
                ToHolder::Pause(target)
            } else {
                ToHolder::Resume(target)
            },
        );
    }

    /// Pauses (or resumes) every session whose output goes to `client`.
    pub(super) fn pause_client_sessions(&mut self, client: Token, paused: bool) {
        for id in self.sessions_of(client) {
            self.set_paused(id, paused);
        }
    }

    /// The session, if it is attached to `client`; otherwise the `ERROR` is
    /// sent and `None` returned.
    fn client_session(&mut self, client: Token, id: SessionId) -> Option<&mut Session> {
        if self
            .sessions
            .get(&id)
            .is_some_and(|session| session.attached == Some(client))
        {
            return self.sessions.get_mut(&id);
        }
        let message = if self.sessions.contains_key(&id) {
            format!("session {id} isn't attached to this client")
        } else {
            format!("no session {id}")
        };
        self.send_client(client, error(None, Some(id), message));
        None
    }

    pub(super) fn client_input(&mut self, client: Token, data: Data) {
        let Some(session) = self.client_session(client, data.session) else {
            return;
        };
        let holder = session.holder;
        self.send_holder(
            holder,
            ToHolder::Input(h1::Data {
                session: data.session,
                bytes: data.bytes,
            }),
        );
    }

    pub(super) fn client_resize(&mut self, client: Token, resize: Resize) {
        let Some(session) = self.client_session(client, resize.session) else {
            return;
        };
        session.cols = resize.cols;
        session.rows = resize.rows;
        let holder = session.holder;
        self.send_holder(
            holder,
            ToHolder::Resize(h1::Resize {
                session: resize.session,
                cols: resize.cols,
                rows: resize.rows,
                cell_w: resize.cell_w,
                cell_h: resize.cell_h,
            }),
        );
    }

    /// `CLOSE`: the session ends whatever its policy; its `EXIT` follows.
    pub(super) fn client_close(&mut self, client: Token, id: SessionId) {
        if self.client_session(client, id).is_some() {
            self.hang_up(id);
        }
    }
}
