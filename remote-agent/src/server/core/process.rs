//! Process info for clients that ask: client protocol 2's `PROCESS`, the
//! name and working directory of each session's foreground program.
//!
//! - **Who is told:** a session's attached client, when it negotiated
//!   protocol 2 and its `HELLO` set `process_info`: once after it starts
//!   or attaches the session, then whenever the name or the directory
//!   changes. Nothing goes before the session's `SPAWNED` or its replay.
//! - **When the core looks:** at most once every [`PROCESS_LOOK`] per
//!   session, and only at sessions that had output or input since its last
//!   look, or were just started or attached. A session whose activity is
//!   younger than [`ACTIVITY_SETTLE`] at a look stays flagged for the next
//!   one: the echo of the key that starts a program comes before the shell
//!   has made that program the foreground, and the program may print
//!   nothing after it, so a look between the two would otherwise be the
//!   last.
//! - **What it reads:** the platform's `pty::foreground`. On Linux that is
//!   the session terminal's foreground group (`tpgid` in
//!   `/proc/<pid>/stat`), then its `comm` and `cwd`. A Windows core reports
//!   nothing.

use std::time::{Duration, Instant};

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::{Process, ToClient};

use super::Core;
use crate::server::event_loop::{EventLoop, Token};
use crate::server::platform::pty;

/// How often the core looks at the sessions that had activity.
pub(super) const PROCESS_LOOK: Duration = Duration::from_secs(1);
/// Activity this recent at a look keeps the session flagged for the next.
const ACTIVITY_SETTLE: Duration = Duration::from_millis(250);

impl<L: EventLoop> Core<L> {
    /// Looks at the flagged sessions' foreground programs, and tells their
    /// clients of the ones that changed.
    pub(super) fn look_at_processes(&mut self, now: Instant) {
        let mut due: Vec<(SessionId, Token, u32)> = Vec::new();
        let clients = &self.clients;
        for (&id, session) in &mut self.sessions {
            if !session.active {
                continue;
            }
            let Some(client) = session.attached else {
                // Nobody to tell; an attach flags it again.
                session.active = false;
                continue;
            };
            // It stays flagged until its client has its SPAWNED or replay.
            if !session.reportable() {
                continue;
            }
            session.active = now.saturating_duration_since(session.active_at) < ACTIVITY_SETTLE;
            if clients
                .get(&client)
                .is_some_and(|client| client.process_info)
            {
                due.push((id, client, session.pid));
            }
        }
        for (id, client, pid) in due {
            let Some(seen) = pty::foreground(pid) else {
                continue;
            };
            let Some(session) = self.sessions.get_mut(&id) else {
                continue;
            };
            if session.foreground.as_ref() == Some(&seen) {
                continue;
            }
            session.foreground = Some(seen.clone());
            self.send_client(
                client,
                ToClient::Process(Process {
                    session: id,
                    name: seen.name,
                    cwd: seen.cwd,
                }),
            );
        }
    }
}
