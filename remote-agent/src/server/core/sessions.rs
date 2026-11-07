//! The core's sessions: spawning through a holder, their I/O between the
//! client and the holder, the close policies, hang-ups and exits.
//!
//! - **`kill`** sessions end on `CLOSE`, `KILL` and `STOP`, and when their
//!   owner is gone: its connection ended, or, for sessions a new core
//!   adopted, it didn't come back within [`super::RECONNECT_GRACE`].
//! - **`detach`** sessions end only on `CLOSE`, `KILL`, `STOP` or their
//!   program's exit. Without a client they keep running.
//! - **Owners** are `client_token`s. A client whose `HELLO` carries a
//!   session's owner token gets every such session that has no client.
//!   A client that attaches becomes the owner, as far as this core knows:
//!   H1 can't change the meta its holder keeps.
//! - **Attach:** `ATTACH` gives a session with no client to the client,
//!   after a replay of its screen (`REPLAY_START`, `DATA`…, `REPLAY_END`),
//!   then applies the client's size. `DETACH` leaves it running with no
//!   client.
//! - **Checkpoints:** the session's replay goes to its holder past
//!   [`CHECKPOINT_AFTER`] of output since the last one, when it is
//!   detached, and before the core steps down. Each says how much output it
//!   covers, so the holder keeps what was still on its way. An adopted
//!   session's emulator is rebuilt from the holder's checkpoint and tail
//!   (the tail alone, after `ESC c`, when it was truncated).

use brindle_remote_protocol::client::{
    self, Data, Exit, Kill, KillTarget, Policy, Request, Resize, Spawned, ToClient,
};
use brindle_remote_protocol::holder::{self as h1, HeldSession, SessionMeta, ToHolder};
use brindle_remote_protocol::{RequestId, SessionId};

use super::clients::error;
use super::holders::WaitingSpawn;
use super::screen::Screen;
use super::{Core, log};
use crate::server::event_loop::{EventLoop, Token};

/// How much of a replay each `DATA` carries.
const REPLAY_CHUNK: usize = 64 * 1024;
/// Past this much output since the last checkpoint, the core sends the
/// session's holder a new one.
pub(super) const CHECKPOINT_AFTER: u64 = 512 * 1024;

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
    /// Adopted, and its owner hasn't come back yet.
    pub(super) awaiting_owner: bool,
    /// Its program exited while its owner was away (`None` for a signal);
    /// kept so the owner hears of it when it comes back.
    exited: Option<Option<i32>>,
    /// Adopted, and its emulator isn't rebuilt from the holder's checkpoint
    /// and tail yet: whether the tail was truncated.
    rebuilding: Option<bool>,
    /// A replay for its client, waiting for the rebuild.
    pending_replay: Option<PendingReplay>,
    /// Its emulator, fed with all of its output.
    pub(super) screen: Screen,
}

struct PendingReplay {
    req: Option<RequestId>,
    /// The size an `ATTACH` asked for.
    size: Option<(u16, u16)>,
}

impl Session {
    /// Its program runs: it isn't starting, and didn't exit while its owner
    /// was away.
    pub(super) fn is_running(&self) -> bool {
        self.spawning.is_none() && self.exited.is_none()
    }

    /// A session reported in a holder's `WELCOME`. It has no client until
    /// its owner comes back. One that exited while no core was connected
    /// is gone from the holder already.
    pub(super) fn adopted(holder: Token, held: HeldSession) -> Self {
        let meta = SessionMeta::from_value(&held.meta);
        Self {
            holder,
            pid: held.pid,
            screen: Screen::new(held.cols, held.rows, meta.scrollback),
            meta,
            cols: held.cols,
            rows: held.rows,
            attached: None,
            spawning: None,
            paused: false,
            hung_up: false,
            awaiting_owner: true,
            exited: held.exit.map(|exit| exit.code),
            rebuilding: Some(held.truncated),
            pending_replay: None,
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
        if self.stopping.is_some() {
            return self.send_client(
                client,
                error(Some(spawn.req), None, "the server is stopping"),
            );
        }
        if self.stepping_down.is_some() {
            return self.send_client(
                client,
                error(
                    Some(spawn.req),
                    None,
                    "the core is stepping down for an upgrade; try again",
                ),
            );
        }
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
        let screen = Screen::new(spawn.cols, spawn.rows, spawn.scrollback);
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
                awaiting_owner: false,
                exited: None,
                rebuilding: None,
                pending_replay: None,
                screen,
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
        // A client attached now is told the session was lost, not that its
        // program exited, so it can keep the tab open and say why.
        if let Some(client) = session.attached {
            log!("session {id}: lost with its holder");
            self.sessions.remove(&id);
            return self.send_client(
                client,
                ToClient::Exit(Exit {
                    session: id,
                    code: None,
                    lost: Some("its holder ended".into()),
                }),
            );
        }
        self.session_exited(holder, id, None);
    }

    pub(super) fn session_output(&mut self, holder: Token, id: SessionId, bytes: Vec<u8>) {
        let Some(session) = self.holder_session(holder, id) else {
            return;
        };
        // Attached or not, so a detached session's screen stays current.
        session.screen.feed(id, &bytes);
        let due = session.screen.since_checkpoint > CHECKPOINT_AFTER;
        if let Some(client) = session.attached {
            self.send_client(client, ToClient::Data(Data { session: id, bytes }));
        }
        if due {
            self.checkpoint(id);
        }
    }

    /// An adopted session's checkpoint from its holder: the rebuild starts
    /// from it, unless the tail was truncated (and so doesn't follow it).
    pub(super) fn session_checkpoint(&mut self, holder: Token, id: SessionId, bytes: &[u8]) {
        if let Some(session) = self.holder_session(holder, id)
            && session.rebuilding == Some(false)
        {
            session.screen.restore(id, bytes);
        }
    }

    /// An adopted session's tail, the last thing its holder sends for it:
    /// the rebuild ends with it, and a replay waiting for that is sent.
    pub(super) fn session_tail(&mut self, holder: Token, id: SessionId, bytes: &[u8]) {
        let Some(session) = self.holder_session(holder, id) else {
            return;
        };
        let Some(truncated) = session.rebuilding.take() else {
            return;
        };
        if truncated {
            log!(
                "session {id}: its tail was truncated; rebuilt from the last {} bytes only",
                bytes.len()
            );
            session.screen.restore(id, b"\x1bc");
        }
        session.screen.restore(id, bytes);
        session.screen.since_checkpoint = bytes.len() as u64;
        if let (Some(client), Some(pending)) = (session.attached, session.pending_replay.take()) {
            self.replay(client, id, pending.req, pending.size);
        }
    }

    /// Sends the session's holder a checkpoint of its screen, covering the
    /// output fed since the last one.
    pub(super) fn checkpoint(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.rebuilding.is_some()
            || session.spawning.is_some()
            || session.hung_up
            || session.exited.is_some()
        {
            return;
        }
        let bytes = session.screen.render(id);
        let covers = std::mem::take(&mut session.screen.since_checkpoint);
        let holder = session.holder;
        for message in ToHolder::checkpoint(id, &bytes, covers) {
            self.send_holder(holder, message);
        }
    }

    /// Checkpoints every session, before the core steps down.
    pub(super) fn checkpoint_all(&mut self) {
        let ids: Vec<SessionId> = self.sessions.keys().copied().collect();
        for id in ids {
            self.checkpoint(id);
        }
    }

    /// The program exited (`code` is `None` for a signal, or a lost holder):
    /// the client gets `EXIT` and the session is gone. An adopted session
    /// whose owner may still come back keeps its exit for the owner.
    pub(super) fn session_exited(&mut self, holder: Token, id: SessionId, code: Option<i32>) {
        let Some(session) = self.holder_session(holder, id) else {
            return;
        };
        log!("session {id}: exited ({code:?})");
        if session.awaiting_owner && session.attached.is_none() && !session.hung_up {
            session.exited = Some(code);
            return;
        }
        let session = self.sessions.remove(&id).expect("the session");
        if let Some(client) = session.attached {
            self.send_client(client, ToClient::Exit(Exit { session: id, code, lost: None }));
        }
    }

    /// A returning owner (`HELLO` with the session's `client_token`) gets
    /// back every session of its own that has no client, each after its
    /// replay (`REPLAY_START` without a `req`). One that exited meanwhile
    /// gets just its `EXIT`.
    pub(super) fn reattach(&mut self, client: Token) {
        let Some(token) = self.client_token(client).map(str::to_string) else {
            return;
        };
        let returning: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                session.attached.is_none() && !session.hung_up && session.meta.owner == token
            })
            .map(|(&id, _)| id)
            .collect();
        if !returning.is_empty() {
            log!(
                "client {client} is back for {} session(s): {returning:?}",
                returning.len()
            );
        }
        for id in returning {
            let session = self.sessions.get_mut(&id).expect("the session");
            session.awaiting_owner = false;
            if let Some(code) = session.exited {
                self.sessions.remove(&id);
                self.send_client(client, ToClient::Exit(Exit { session: id, code, lost: None }));
                continue;
            }
            self.replay(client, id, None, None);
        }
    }

    /// The reconnect grace period is over: adopted `kill` sessions whose
    /// owners didn't come back are hung up.
    pub(super) fn end_grace(&mut self) {
        let orphans: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.awaiting_owner)
            .map(|(&id, _)| id)
            .collect();
        for id in orphans {
            let session = self.sessions.get_mut(&id).expect("the session");
            session.awaiting_owner = false;
            if session.exited.is_some() {
                self.sessions.remove(&id);
            } else if session.meta.policy == Policy::Kill {
                log!("session {id}: its owner didn't come back; hanging up");
                self.hang_up(id);
            } else {
                log!("session {id}: detached; its owner didn't come back");
            }
        }
    }

    /// The client's connection ended: its `kill` sessions are hung up, and
    /// its `detach` sessions keep running without a client.
    pub(super) fn client_gone(&mut self, client: Token) {
        for id in self.sessions_of(client) {
            self.detach_session(id);
            match self.sessions[&id].meta.policy {
                Policy::Kill => self.hang_up(id),
                Policy::Detach => {
                    log!("session {id}: detached");
                    self.checkpoint(id);
                }
            }
        }
    }

    /// `KILL`: hangs up one session or all of them, whatever their policy.
    pub(super) fn client_kill(&mut self, client: Token, kill: Kill) {
        let targets: Vec<SessionId> = match kill.target {
            KillTarget::All => self.sessions.keys().copied().collect(),
            KillTarget::Session(id) if self.sessions.contains_key(&id) => vec![id],
            KillTarget::Session(id) => {
                return self.send_client(
                    client,
                    error(Some(kill.req), Some(id), format!("no session {id}")),
                );
            }
        };
        for id in targets {
            self.end_session(id);
        }
        self.send_client(client, ToClient::Ok(Request { req: kill.req }));
    }

    /// `STOP`: every session is hung up, and the core exits (answering
    /// `OK`) once they have all ended.
    pub(super) fn client_stop(&mut self, client: Token, req: RequestId) {
        log!("stopping: hanging up {} session(s)", self.sessions.len());
        self.stopping = Some(super::Answer { client, req });
        for waiting in std::mem::take(&mut self.waiting) {
            let req = waiting.spawn.req;
            self.send_client(
                waiting.client,
                error(Some(req), None, "the server is stopping"),
            );
        }
        let all: Vec<SessionId> = self.sessions.keys().copied().collect();
        for id in all {
            self.end_session(id);
        }
    }

    /// Hangs a session up, or forgets one that already exited.
    fn end_session(&mut self, id: SessionId) {
        if self
            .sessions
            .get(&id)
            .is_some_and(|session| session.exited.is_some())
        {
            self.sessions.remove(&id);
        } else {
            self.hang_up(id);
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
        session.screen.input_seen(data.session);
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
        if self.client_session(client, resize.session).is_none() {
            return;
        }
        self.resize_session(
            resize.session,
            resize.cols,
            resize.rows,
            resize.cell_w,
            resize.cell_h,
        );
    }

    /// `ATTACH`: a session with no client gets this one, after a replay of
    /// its screen; then the client's size applies. A session that has a
    /// client is refused.
    pub(super) fn client_attach(&mut self, client: Token, attach: client::Attach) {
        let id = attach.session;
        let refusal = match self.sessions.get(&id) {
            None => Some(format!("no session {id}")),
            Some(session) if session.attached == Some(client) => {
                Some(format!("session {id} is already attached to this client"))
            }
            Some(session) if session.attached.is_some() => {
                Some(format!("session {id} is attached to another client"))
            }
            Some(session) if session.hung_up => Some(format!("session {id} is ending")),
            Some(_) => None,
        };
        if let Some(message) = refusal {
            return self.send_client(client, error(Some(attach.req), Some(id), message));
        }
        log!("session {id}: attached to client {client}");
        if let Some(token) = self.client_token(client).map(str::to_string) {
            self.sessions.get_mut(&id).expect("the session").meta.owner = token;
        }
        self.replay(
            client,
            id,
            Some(attach.req),
            Some((attach.cols, attach.rows)),
        );
    }

    /// `DETACH`: the session keeps running with no client.
    pub(super) fn client_detach(&mut self, client: Token, id: SessionId) {
        if self.client_session(client, id).is_none() {
            return;
        }
        log!("session {id}: detached by client {client}");
        self.detach_session(id);
        self.checkpoint(id);
    }

    /// Gives a session with no client to `client`: `REPLAY_START`, the
    /// replay as `DATA`, `REPLAY_END`; then `size` applies, and its live
    /// output follows. A session whose program exited meanwhile gets its
    /// `EXIT` after the replay. One still being rebuilt from its holder
    /// gets its replay once the rebuild is done (no output comes before).
    pub(super) fn replay(
        &mut self,
        client: Token,
        id: SessionId,
        req: Option<RequestId>,
        size: Option<(u16, u16)>,
    ) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.awaiting_owner = false;
        session.attached = Some(client);
        if session.rebuilding.is_some() {
            session.pending_replay = Some(PendingReplay { req, size });
            return;
        }
        let pid = session.pid;
        let exited = session.exited;
        // Rendered whole, in this turn of the loop: output read after it
        // is fed and forwarded after it, so nothing is lost or reordered.
        let bytes = session.screen.render(id);
        self.send_client(
            client,
            ToClient::ReplayStart(client::ReplayStart {
                req,
                session: id,
                pid,
            }),
        );
        for chunk in bytes.chunks(REPLAY_CHUNK) {
            self.send_client(
                client,
                ToClient::Data(Data {
                    session: id,
                    bytes: chunk.to_vec(),
                }),
            );
        }
        self.send_client(
            client,
            ToClient::ReplayEnd(client::SessionRef { session: id }),
        );
        if let Some(code) = exited {
            self.sessions.remove(&id);
            return self.send_client(client, ToClient::Exit(Exit { session: id, code, lost: None }));
        }
        if let Some((cols, rows)) = size
            && cols > 0
            && rows > 0
        {
            self.resize_session(id, cols, rows, 0, 0);
        }
        if self.client_is_over(client) {
            self.set_paused(id, true);
        }
    }

    /// Resizes the session's emulator and its PTY.
    fn resize_session(&mut self, id: SessionId, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.cols = cols;
        session.rows = rows;
        session.screen.resize(id, cols, rows);
        let holder = session.holder;
        self.send_holder(
            holder,
            ToHolder::Resize(h1::Resize {
                session: id,
                cols,
                rows,
                cell_w,
                cell_h,
            }),
        );
    }

    /// `CLOSE`: the session ends, whatever its policy; its `EXIT` follows.
    pub(super) fn client_close(&mut self, client: Token, id: SessionId) {
        if self.client_session(client, id).is_some() {
            self.hang_up(id);
        }
    }
}
