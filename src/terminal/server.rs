//! A local tab's session in the terminal server: the sink its output
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
use super::{ChannelEvent, GridSize, Listener};
use alacritty_terminal::grid::Dimensions as _;
use crate::notify::osc::Scanner;
use crate::remote::local::SessionSink;

pub struct Sink {
    term: Arc<FairMutex<Term<Listener>>>,
    parser: Arc<Mutex<Parser>>,
    marks: Arc<Mutex<Marks>>,
    listener: Listener,
    scanner: Scanner,
    /// What starting the session does (`start "bash"`, `attach session 3`),
    /// for the error line if it never starts.
    doing: String,
    /// The session's size, which the core renders an attach's replay at.
    replay_size: Option<GridSize>,
    /// A replay's bytes, held until it ends.
    replay: Option<Vec<u8>>,
}

impl Sink {
    pub fn new(
        term: Arc<FairMutex<Term<Listener>>>,
        parser: Arc<Mutex<Parser>>,
        marks: Arc<Mutex<Marks>>,
        listener: Listener,
        doing: String,
    ) -> Self {
        Self { term, parser, marks, listener, scanner: Scanner::new(), doing, replay_size: None, replay: None }
    }

    /// The first replay was rendered at `size` (an attached session's), so
    /// it is fed at that size, then the term resizes back to its own.
    pub fn replayed_at(mut self, size: GridSize) -> Self {
        self.replay_size = Some(size);
        self
    }

    fn send(&self, event: ChannelEvent) {
        self.listener.0.unbounded_send(event).ok();
    }

    /// Feeds a whole replay under the locks, so a resize from the UI can't
    /// land in the middle of it. A replay must be fed at the size it was
    /// rendered at, or its cursor and rows land elsewhere.
    fn feed_replay(&mut self, bytes: &[u8]) {
        let mut parser = self.parser.lock();
        let mut term = self.term.lock_unfair();
        let own = GridSize { cols: term.columns() as u16, rows: term.screen_lines() as u16, ..GridSize::default() };
        let size = self.replay_size.take().filter(|s| s.cols > 0 && s.rows > 0 && (s.cols, s.rows) != (own.cols, own.rows));
        if let Some(size) = size {
            self.marks.lock().resize(&mut *term, size);
        }
        parser.advance(&mut *term, &self.marks, bytes);
        if size.is_some() {
            self.marks.lock().resize(&mut *term, own);
        }
    }
}

impl SessionSink for Sink {
    fn output(&mut self, bytes: &[u8]) {
        for found in self.scanner.scan(bytes) {
            self.send(ChannelEvent::Osc(found));
        }
        if let Some(replay) = &mut self.replay {
            replay.extend_from_slice(bytes);
            return;
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
        if active {
            self.send(ChannelEvent::Replay(true));
            self.replay = Some(Vec::new());
            return;
        }
        if let Some(bytes) = self.replay.take() {
            self.feed_replay(&bytes);
            self.listener.send_event(AlacEvent::Wakeup);
        }
        self.send(ChannelEvent::Replay(false));
    }

    fn exited(&mut self, _code: Option<i32>) {
        self.listener.send_event(AlacEvent::Exit);
    }

    fn lost(&mut self, reason: &str) {
        let line = format!("\r\n\x1b[0;31mSession lost:\x1b[0m {}\r\n", reason.replace('\n', "\r\n"));
        self.output(line.as_bytes());
        self.send(ChannelEvent::Lost);
    }

    /// The tab shows the error, as a tab whose program can't start always
    /// has, and stays open.
    fn failed(&mut self, error: &anyhow::Error) {
        log::error!("could not {}: {error:#}", self.doing);
        let text = format!(
            "\x1b[31mBrindle could not {}:\x1b[0m\r\n{}\r\n",
            self.doing,
            format!("{error:#}").replace('\n', "\r\n")
        );
        self.output(text.as_bytes());
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
            "start \"sh\"".into(),
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

    /// A replay rendered at the session's 40x5, whose cursor is on its last
    /// row, into an 80x24 term: fed at 40x5 and resized back, the cursor
    /// stays on the last line written, as a resize of the session would
    /// leave it, not five rows from the top.
    #[test]
    fn a_replay_is_fed_at_the_sessions_size() {
        let (sink, _rx, _) = sink();
        let mut sink = sink.replayed_at(GridSize { cols: 40, rows: 5, ..GridSize::default() });
        sink.replay(true);
        sink.output(b"\x1bc1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n$ \x1b[5;3H");
        sink.replay(false);
        let term = sink.term.lock();
        assert_eq!((term.columns(), term.screen_lines()), (80, 24));
        let text = crate::terminal::screen_text(&term);
        let prompt = text.lines().position(|line| line.starts_with('$')).expect("the prompt is on screen");
        assert_eq!(term.grid().cursor.point.line.0 as usize, prompt, "{text}");
        assert_eq!(term.grid().cursor.point.column.0, 2);
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
