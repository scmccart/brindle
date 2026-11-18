//! Each session's emulator in the core, behind `catch_unwind`.
//!
//! The core feeds every session's output into a
//! [`brindle_term_core::emulator::Emulator`] whether or not a client is
//! attached, so a detached session's screen stays current. The emulator
//! never answers queries; the attached client does.
//!
//! **Paths:** the emulator reads the directories programs report as the
//! platform's programs spell them ([`pty::PATH_STYLE`]), so a Windows
//! session's OSC 9;9 `C:\src` is in its replays.
//!
//! **Failure isolation:** a panic while feeding, resizing or rendering
//! marks that session's emulator broken (logged once). The session keeps
//! forwarding output, and its replays and checkpoints are just `ESC c`.
//! Other sessions are untouched.
//!
//! **Fault injection, for tests:** in debug builds only, a feed whose bytes
//! contain the value of `BRINDLE_TEST_FEED_PANIC` panics, so a test can
//! break one session's emulator by making its program print that value
//! ([`fault::marker`]).

use std::panic::{self, AssertUnwindSafe};
use std::sync::OnceLock;

use brindle_remote_protocol::SessionId;
use brindle_term_core::emulator::Emulator;

use super::log;
use crate::server::fault;
use crate::server::platform::pty;

/// What a broken emulator replays: the receiver starts over.
const RESET: &[u8] = b"\x1bc";

pub(super) struct Screen {
    /// `None` once it panicked.
    emulator: Option<Emulator>,
    /// Output fed since the last checkpoint (or, once rebuilt from a
    /// holder, since the start of its tail): what the next one covers.
    pub(super) since_checkpoint: u64,
}

impl Screen {
    pub(super) fn new(cols: u16, rows: u16, scrollback: u32) -> Self {
        let emulator = Emulator::new(cols, rows, scrollback as usize, pty::PATH_STYLE);
        Self {
            emulator: Some(emulator),
            since_checkpoint: 0,
        }
    }

    /// The program's title; empty when it set none or the emulator broke.
    pub(super) fn title(&self) -> String {
        self.emulator.as_ref().and_then(Emulator::title).unwrap_or_default()
    }

    /// Feeds the session's output.
    pub(super) fn feed(&mut self, id: SessionId, bytes: &[u8]) {
        self.since_checkpoint += bytes.len() as u64;
        self.restore(id, bytes);
    }

    /// Feeds bytes that aren't new output: a checkpoint, or the tail the
    /// holder kept, when rebuilding.
    pub(super) fn restore(&mut self, id: SessionId, bytes: &[u8]) {
        self.guard(id, "feeding output", |emulator| {
            inject_fault(bytes);
            emulator.feed(bytes);
        });
    }

    pub(super) fn resize(&mut self, id: SessionId, cols: u16, rows: u16) {
        self.guard(id, "resizing", |emulator| emulator.resize(cols, rows));
    }

    /// The user typed into the session.
    pub(super) fn input_seen(&mut self, id: SessionId) {
        self.guard(id, "taking input", Emulator::input_seen);
    }

    /// The replay stream of the session's state; `ESC c` once broken.
    pub(super) fn render(&mut self, id: SessionId) -> Vec<u8> {
        self.guard(id, "rendering", Emulator::render)
            .unwrap_or_else(|| RESET.to_vec())
    }

    /// Runs `f` on the emulator under `catch_unwind`; a panic breaks it.
    fn guard<R>(
        &mut self,
        id: SessionId,
        doing: &str,
        f: impl FnOnce(&mut Emulator) -> R,
    ) -> Option<R> {
        let emulator = self.emulator.as_mut()?;
        match panic::catch_unwind(AssertUnwindSafe(|| f(emulator))) {
            Ok(result) => Some(result),
            Err(payload) => {
                let message = fault::describe(&*payload);
                log!(
                    "session {id}: the emulator panicked while {doing} ({message}); \
                     the session keeps running, but its replays are now only a reset"
                );
                self.emulator = None;
                None
            }
        }
    }
}

/// Panics when `bytes` hold `BRINDLE_TEST_FEED_PANIC`'s marker (debug
/// builds only).
fn inject_fault(bytes: &[u8]) {
    static MARKER: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    if let Some(marker) = fault::marker("BRINDLE_TEST_FEED_PANIC", &MARKER)
        && fault::holds(bytes, marker)
    {
        panic!("BRINDLE_TEST_FEED_PANIC matched");
    }
}
