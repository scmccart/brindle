//! Each session's emulator in the core, behind `catch_unwind`.
//!
//! The core feeds every session's output into a
//! [`brindle_term_core::emulator::Emulator`] whether or not a client is
//! attached, so a detached session's screen stays current. The emulator
//! never answers queries; the attached client does.
//!
//! **Failure isolation:** a panic while feeding, resizing or rendering
//! marks that session's emulator broken (logged once). The session keeps
//! forwarding output, and its replays and checkpoints are just `ESC c`.
//! Other sessions are untouched.
//!
//! **Fault injection, for tests:** in debug builds only, a feed whose bytes
//! contain the value of `BRINDLE_TEST_FEED_PANIC` panics, so a test can
//! break one session's emulator by making its program print that value.
//! It is an environment variable rather than a `cfg(test)` hook because
//! the server tests run the built binary; `cfg(debug_assertions)` keeps it
//! out of release builds.

use std::panic::{self, AssertUnwindSafe};

use brindle_remote_protocol::SessionId;
use brindle_term_core::emulator::Emulator;

use super::log;

/// What a broken emulator replays: the receiver starts over.
const RESET: &[u8] = b"\x1bc";

pub(super) struct Screen {
    /// `None` once it panicked.
    emulator: Option<Emulator>,
    /// Output fed since the last checkpoint.
    pub(super) since_checkpoint: usize,
}

#[allow(dead_code, reason = "tasks 5.3 and 5.4 replay and checkpoint it")]
impl Screen {
    pub(super) fn new(cols: u16, rows: u16, scrollback: u32) -> Self {
        Self {
            emulator: Some(Emulator::new(cols, rows, scrollback as usize)),
            since_checkpoint: 0,
        }
    }

    pub(super) fn is_broken(&self) -> bool {
        self.emulator.is_none()
    }

    pub(super) fn feed(&mut self, id: SessionId, bytes: &[u8]) {
        self.since_checkpoint += bytes.len();
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
                let message = payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "no message".into());
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

#[cfg(debug_assertions)]
fn inject_fault(bytes: &[u8]) {
    use std::sync::OnceLock;
    static MARKER: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    let marker = MARKER.get_or_init(|| {
        std::env::var_os("BRINDLE_TEST_FEED_PANIC")
            .filter(|value| !value.is_empty())
            .map(|value| value.into_encoded_bytes())
    });
    if let Some(marker) = marker
        && bytes.windows(marker.len()).any(|window| window == marker.as_slice())
    {
        panic!("BRINDLE_TEST_FEED_PANIC matched");
    }
}

#[cfg(not(debug_assertions))]
fn inject_fault(_: &[u8]) {}
