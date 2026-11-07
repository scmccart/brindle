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

#[cfg(test)]
mod tests {
    use alacritty_terminal::term;
    use futures::channel::mpsc::unbounded;

    use super::*;
    use crate::notify::osc::Found;
    use crate::terminal::{GridSize, OutputMarker};

    fn sink() -> (Sink, futures::channel::mpsc::UnboundedReceiver<ChannelEvent>, Arc<OutputMarker>) {
        let (tx, rx) = unbounded();
        let marker = Arc::new(OutputMarker::default());
        let listener = Listener(tx, marker.clone());
        let term = Term::new(term::Config::default(), &GridSize::default(), listener.clone());
        let sink = Sink::new(
            Arc::new(FairMutex::new(term)),
            Arc::new(Mutex::new(Parser::new())),
            Arc::new(Mutex::new(Marks::default())),
            listener,
        );
        (sink, rx, marker)
    }

    /// What the terminal's event task gets, by kind, in order.
    fn kinds(rx: &mut futures::channel::mpsc::UnboundedReceiver<ChannelEvent>) -> Vec<&'static str> {
        std::iter::from_fn(|| rx.try_next().ok().flatten())
            .map(|event| match event {
                ChannelEvent::Replay(true) => "replay-start",
                ChannelEvent::Replay(false) => "replay-end",
                ChannelEvent::Osc(Found::Notice(_)) => "notice",
                ChannelEvent::Osc(_) => "osc",
                ChannelEvent::Alac(AlacEvent::Bell) => "bell",
                ChannelEvent::Alac(AlacEvent::Wakeup) => "wakeup",
                ChannelEvent::Alac(AlacEvent::Exit) => "exit",
                ChannelEvent::Alac(_) => "alac",
                ChannelEvent::Lost => "lost",
            })
            .collect()
    }

    #[test]
    fn output_is_parsed_and_wakes_the_ui_like_a_pty_read() {
        let (mut sink, mut rx, marker) = sink();
        sink.output(b"hello\x1b]9;done\x07");
        assert_eq!(kinds(&mut rx), ["notice", "wakeup"]);
        assert_eq!(marker.generation(), 1);
        assert!(crate::terminal::screen_text(&sink.term.lock()).starts_with("hello\n"));
        sink.exited(Some(0));
        assert_eq!(kinds(&mut rx), ["exit"]);
    }

    #[test]
    fn a_replays_bell_and_notice_arrive_between_its_markers() {
        let (mut sink, mut rx, _) = sink();
        sink.replay(true);
        sink.output(b"\x07\x1b]9;from the tail\x07");
        sink.replay(false);
        sink.output(b"\x07");
        assert_eq!(
            kinds(&mut rx),
            ["replay-start", "notice", "bell", "wakeup", "replay-end", "bell", "wakeup"]
        );
    }

    #[test]
    fn a_lost_session_says_why_on_screen() {
        let (mut sink, mut rx, _) = sink();
        sink.lost("its holder ended");
        assert_eq!(kinds(&mut rx), ["wakeup", "lost"]);
        let text = crate::terminal::screen_text(&sink.term.lock());
        assert!(text.contains("Session lost: its holder ended"), "{text}");
    }
}
