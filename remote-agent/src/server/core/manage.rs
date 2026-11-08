//! The core's management requests and the step-down for upgrades.
//!
//! - **`LIST`** answers `SESSIONS`: every running session with its pid,
//!   whether a client has it, its command and directory (from the meta the
//!   core keeps), its policy and size.
//! - **`STATUS`** answers `SERVER_STATUS`: the core's version, build,
//!   protocol ranges, pid, uptime and client count, and each adopted
//!   holder's pid, build, H1 version, session count and draining flag.
//! - **`STEP_DOWN`** checkpoints every session, sends each holder
//!   `GOODBYE { handover: true }`, and exits once that has reached the
//!   holders. Only then does the asking client get its `OK`, so the core it
//!   starts next finds the checkpoints. The other clients' connections
//!   close, and they reconnect to the next core.

use brindle_remote_protocol::RequestId;
use brindle_remote_protocol::client::{
    self, HolderStatus, ServerStatus, SessionInfo, Sessions, ToClient,
};
use brindle_remote_protocol::holder::{self as h1, ToHolder};

use super::clients::error;
use super::{Answer, Core, log};
use crate::server::event_loop::{EventLoop, Token};

impl<L: EventLoop> Core<L> {
    /// `LIST`: the running sessions. Ones still starting, or whose program
    /// exited while their owner was away, aren't listed.
    pub(super) fn client_list(&mut self, client: Token, req: RequestId) {
        let sessions = self
            .sessions
            .iter()
            .filter(|(_, session)| session.is_running())
            .map(|(&id, session)| SessionInfo {
                session: id,
                pid: session.pid,
                attached: session.attached.is_some(),
                command: session.meta.command.clone(),
                cwd: session.meta.cwd.clone(),
                policy: session.meta.policy,
                cols: session.cols,
                rows: session.rows,
                key: session.meta.key.clone(),
                title: session.screen.title(),
                started: session.meta.started,
                profile: session.meta.profile.clone(),
            })
            .collect();
        self.send_client(client, ToClient::Sessions(Sessions { req, sessions }));
    }

    /// `STATUS`: the core and its adopted holders.
    pub(super) fn client_status(&mut self, client: Token, req: RequestId) {
        let mut holders: Vec<(Token, HolderStatus)> = self
            .holders
            .iter()
            .filter_map(|(&token, holder)| {
                let sessions = self
                    .sessions
                    .values()
                    .filter(|session| session.holder == token)
                    .count();
                holder.status(sessions as u32).map(|status| (token, status))
            })
            .collect();
        holders.sort_by_key(|(token, _)| *token);
        let status = ServerStatus {
            req,
            core_version: env!("CARGO_PKG_VERSION").into(),
            core_build: self.build.clone(),
            protocols: client::PROTOCOLS,
            holder_protocols: h1::PROTOCOLS,
            pid: std::process::id(),
            uptime_secs: self.started.elapsed().as_secs(),
            clients: self.clients.len() as u32,
            holders: holders.into_iter().map(|(_, status)| status).collect(),
        };
        self.send_client(client, ToClient::ServerStatus(status));
    }

    /// `STEP_DOWN`: checkpoint every session, hand the holders over, and
    /// exit (see the module docs). Refused while a `STOP` is ending the
    /// sessions.
    pub(super) fn client_step_down(&mut self, client: Token, req: RequestId) {
        let refusal = if self.stopping.is_some() {
            Some("the server is stopping")
        } else if self.stepping_down.is_some() {
            Some("it is already stepping down")
        } else {
            None
        };
        if let Some(reason) = refusal {
            log!("upgrade refused: client {client} asked to step down, but {reason}");
            return self.send_client(
                client,
                error(Some(req), None, format!("can't step down: {reason}")),
            );
        }
        self.checkpoint_all();
        self.fail_waiting("the core is stepping down for an upgrade; try again");
        let holders: Vec<Token> = self.holders.keys().copied().collect();
        for &holder in &holders {
            self.send_holder(holder, ToHolder::Goodbye(h1::Goodbye { handover: true }));
        }
        log!(
            "upgrade: client {client} asked to step down; checkpointed {} session(s) and handed {} holder(s) over",
            self.sessions.len(),
            holders.len()
        );
        self.stepping_down = Some(Answer { client, req });
    }
}
