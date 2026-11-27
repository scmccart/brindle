//! An agent zone's sink as the connection sees it: the reader's calls,
//! each under `Connection::deliver`'s `catch_unwind`, handed to
//! [`crate::agent::sink::Sink`]. Output (a relay's lost-link line, an
//! attach's retry line) is a terminal's: an agent session has none, so it
//! is logged and dropped.

use super::SessionSink;
use crate::agent::sink::Sink;
use crate::remote::protocol::RequestId;

impl SessionSink for Sink {
    fn output(&mut self, bytes: &[u8]) {
        log::debug!("agent session: {} bytes of terminal output dropped", bytes.len());
    }

    fn note(&mut self, text: &str) {
        Sink::note(self, text);
    }

    fn endpoint_note(&mut self, text: &str) {
        Sink::endpoint_note(self, text);
    }

    fn replay(&mut self, active: bool) {
        Sink::replay(self, active);
    }

    fn exited(&mut self, code: Option<i32>) {
        Sink::exited(self, code);
    }

    fn lost(&mut self, reason: &str) {
        Sink::lost(self, reason);
    }

    fn failed(&mut self, error: &anyhow::Error) {
        Sink::failed(self, error);
    }

    fn broken(&mut self, reason: &str) {
        Sink::broken(self, reason);
    }

    fn transcript(&mut self, events: Vec<serde_json::Value>) -> Result<(), String> {
        Sink::transcript(self, events)
    }

    fn reply(&mut self, request: RequestId, outcome: Result<(), String>) {
        Sink::reply(self, request, outcome);
    }

    fn page(&mut self, request: RequestId, events: Vec<serde_json::Value>, last: Option<bool>) -> Result<(), String> {
        Sink::page(self, request, events, last)
    }

    fn conversations(&mut self, request: RequestId, listed: crate::remote::protocol::client::AgentConversations) {
        Sink::conversations(self, request, listed);
    }

    fn link_down(&mut self) {
        Sink::link_down(self);
    }
}
