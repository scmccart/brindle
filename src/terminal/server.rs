//! A local tab's session in the terminal server (Linux): the sink its output
//! goes to on the connection's reader thread, and what its `SPAWN` carries.
//!
//! The sink does what `pty.rs`'s `ParsingReader` does for a PTY: it scans
//! the bytes for the OSCs Brindle handles beside the parser and sends what
//! it finds, parses them under the terminal's locks (parser, then term,
//! then marks), and sends `Wakeup`, which advances the [`OutputMarker`]
//! for latency probes.
//!
//! [`OutputMarker`]: super::OutputMarker

use std::sync::Arc;

use alacritty_terminal::event::{Event as AlacEvent, EventListener as _};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use parking_lot::Mutex;

use super::shell_integration::{Marks, Parser};
use super::{ChannelEvent, Listener};
use crate::notify::osc::Scanner;
use crate::remote::local::SessionSink;

pub struct Sink {
    term: Arc<FairMutex<Term<Listener>>>,
    parser: Arc<Mutex<Parser>>,
    marks: Arc<Mutex<Marks>>,
    listener: Listener,
    scanner: Scanner,
}

impl Sink {
    pub fn new(
        term: Arc<FairMutex<Term<Listener>>>,
        parser: Arc<Mutex<Parser>>,
        marks: Arc<Mutex<Marks>>,
        listener: Listener,
    ) -> Self {
        Self { term, parser, marks, listener, scanner: Scanner::new() }
    }

    fn send(&self, event: ChannelEvent) {
        self.listener.0.unbounded_send(event).ok();
    }
}

impl SessionSink for Sink {
    fn output(&mut self, bytes: &[u8]) {
        for found in self.scanner.scan(bytes) {
            self.send(ChannelEvent::Osc(found));
        }
        {
            // Lock order: parser, then term, then marks (taken by `advance`).
            let mut parser = self.parser.lock();
            let mut term = self.term.lock_unfair();
            parser.advance(&mut *term, &self.marks, bytes);
        }
        // Always wake the UI, even when the bytes are held by a
        // synchronized update: it schedules the update's timeout flush.
        self.listener.send_event(AlacEvent::Wakeup);
    }

    fn replay(&mut self, active: bool) {
        self.send(ChannelEvent::Replay(active));
    }

    fn exited(&mut self, _code: Option<i32>) {
        self.listener.send_event(AlacEvent::Exit);
    }

    fn lost(&mut self, reason: &str) {
        let line = format!("\r\n\x1b[0;31mSession lost:\x1b[0m {}\r\n", reason.replace('\n', "\r\n"));
        self.output(line.as_bytes());
        self.send(ChannelEvent::Lost);
    }
}
