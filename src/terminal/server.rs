//! A tab's session in a terminal server, local or remote: the sink its
//! output goes to on the connection's reader thread.
//!
//! The sink scans the bytes for the OSCs Brindle handles beside the parser
//! and sends what it finds, parses them under the terminal's locks (parser,
//! then term, then marks), and sends `Wakeup`, which advances the
//! [`OutputMarker`] for latency probes. Everything it sends passes the
//! terminal's event gate ([`Listener::send`]).
//!
//! [`OutputMarker`]: super::OutputMarker

use std::sync::Arc;

use alacritty_terminal::event::{Event as AlacEvent, EventListener as _};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Term, TermMode};
use parking_lot::Mutex;

use super::pointer::Screen;
use super::shell_integration::{Marks, Parser};
use super::tab_note::{self, NoteGate};
#[cfg(debug_assertions)]
use super::SINK_PANIC;
use super::{ChannelEvent, GridSize, Listener};
use alacritty_terminal::grid::Dimensions as _;
use crate::notify::osc::{Found, Scanner};
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
    /// Which notice was shown last, until the session prints again.
    gate: NoteGate,
}

impl Sink {
    pub fn new(
        term: Arc<FairMutex<Term<Listener>>>,
        parser: Arc<Mutex<Parser>>,
        marks: Arc<Mutex<Marks>>,
        listener: Listener,
        doing: String,
    ) -> Self {
        Self { term, parser, marks, listener, scanner: Scanner::new(), doing, replay_size: None, replay: None, gate: NoteGate::default() }
    }

    /// The first replay was rendered at `size` (an attached session's), so
    /// it is fed at that size, then the term resizes back to its own.
    pub fn replayed_at(mut self, size: GridSize) -> Self {
        self.replay_size = Some(size);
        self
    }

    fn send(&self, event: ChannelEvent) {
        self.listener.send(event);
    }

    /// Feeds a whole replay under the locks, so a resize from the UI can't
    /// land in the middle of it. A replay must be fed at the size it was
    /// rendered at, or its cursor and rows land elsewhere.
    fn feed_replay(&mut self, bytes: &[u8]) {
        let mut parser = self.parser.lock();
        let mut term = self.term.lock_unfair();
        let own = GridSize { cols: term.columns() as u16, rows: term.screen_lines() as u16, ..GridSize::default() };
        let size = replay_resize(own, self.replay_size.take());
        if let Some(size) = size {
            self.marks.lock().resize(&mut *term, size);
        }
        inject_fault(bytes);
        parser.advance(&mut *term, &self.marks, bytes);
        if size.is_some() {
            self.marks.lock().resize(&mut *term, own);
        }
    }
}

/// The size a replay rendered at `recorded` is fed at, by a terminal whose
/// grid is `own`: the recorded size, within the limits every grid keeps
/// ([`GridSize::clamped`]), when its columns or rows differ from the
/// grid's; `None` when nothing was recorded or the grid already has it.
/// The recorded size is a session's decoded one (`ListedSession::size`, or
/// the default grid for a core that lists none), never zero.
fn replay_resize(own: GridSize, recorded: Option<GridSize>) -> Option<GridSize> {
    recorded.map(GridSize::clamped).filter(|size| (size.cols, size.rows) != (own.cols, own.rows))
}

impl Sink {
    /// Scans, holds or parses `bytes`, and wakes the UI. A notice writes
    /// through here too, so one that arrives during a replay lands after
    /// the replayed screen; only the program's own output
    /// ([`SessionSink::output`]) lets the next repeat of a notice through.
    ///
    /// A pointer shape op acts on the screen active where it sits, so a
    /// chunk holding one is parsed in pieces, each find sent where it sits
    /// in the stream and each op tagged with the screen the parse reached
    /// ([`ChannelEvent::Pointer`]). Other chunks send their finds first and
    /// parse in one go. A held replay's ops are dropped: the server keeps
    /// no pointer, so a replay holds none, and its screen isn't parsed yet.
    fn write(&mut self, bytes: &[u8]) {
        let found = self.scanner.scan_positions(bytes);
        let split = self.replay.is_none() && found.iter().any(|(_, found)| matches!(found, Found::Pointer(_)));
        let (first, in_stream) = if split { (Vec::new(), found) } else { (found, Vec::new()) };
        for (_, found) in first {
            if !matches!(found, Found::Pointer(_)) {
                self.send(ChannelEvent::Osc(found));
            }
        }
        if let Some(replay) = &mut self.replay {
            replay.extend_from_slice(bytes);
            return;
        }
        {
            // Lock order: parser, then term, then marks (taken by `advance`).
            let mut parser = self.parser.lock();
            let mut term = self.term.lock_unfair();
            inject_fault(bytes);
            let mut fed = 0;
            for (at, found) in in_stream {
                match found {
                    Found::Pointer(op) => {
                        parser.advance(&mut *term, &self.marks, &bytes[fed..at]);
                        fed = at;
                        let screen = Screen::of(term.mode().contains(TermMode::ALT_SCREEN));
                        self.listener.send(ChannelEvent::Pointer(screen, op));
                    }
                    Found::Notice(_)
                    | Found::Kitty(_)
                    | Found::Status(_)
                    | Found::Progress(_)
                    | Found::Context(_)
                    | Found::PromptStart
                    | Found::Reset => self.listener.send(ChannelEvent::Osc(found)),
                }
            }
            parser.advance(&mut *term, &self.marks, &bytes[fed..]);
        }
        // Always wake the UI, even when the bytes are held by a
        // synchronized update: it schedules the update's timeout flush.
        self.listener.send_event(AlacEvent::Wakeup);
    }
}

impl SessionSink for Sink {
    fn output(&mut self, bytes: &[u8]) {
        self.gate.output();
        self.write(bytes);
    }

    /// One dim line, once until the session prints again.
    fn note(&mut self, text: &str) {
        if self.gate.admit(text) {
            self.write(tab_note::line(text).as_bytes());
        }
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
        self.send(ChannelEvent::Failed);
    }

    /// A recorded session's listed size, which its replay is rendered at.
    fn replay_size(&mut self, cols: u16, rows: u16) {
        self.replay_size = Some(GridSize { cols, rows, ..GridSize::default() });
    }

    /// A remote host reported another foreground program: the title may
    /// show it.
    fn foreground(&mut self) {
        self.send(ChannelEvent::Foreground);
    }

    /// The emulator failed on the session's output, which the connection
    /// let go: the terminal replaces its state and says so. Nothing is
    /// parsed here, since the state may be half-changed.
    fn broken(&mut self, reason: &str) {
        self.replay = None;
        self.send(ChannelEvent::Broken(reason.to_string()));
    }
}

/// The debug-only fault switch `BRINDLE_TEST_SINK_PANIC` (`process R20`):
/// parsing panics, under the terminal's locks as a real emulator failure
/// would, when the bytes hold the marker it names. The e2e suite prints the
/// marker with one `printf`, so it arrives in one read. It is read once.
#[cfg(debug_assertions)]
fn inject_fault(bytes: &[u8]) {
    static MARKER: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    let marker = MARKER.get_or_init(|| {
        std::env::var_os(SINK_PANIC).filter(|value| !value.is_empty()).map(|value| value.into_encoded_bytes())
    });
    if let Some(marker) = marker
        && bytes.windows(marker.len()).any(|window| window == marker.as_slice())
    {
        panic!("{SINK_PANIC}: the output held its marker");
    }
}

/// A release build has no fault switch.
#[cfg(not(debug_assertions))]
fn inject_fault(_bytes: &[u8]) {}

#[cfg(test)]
mod tests {
    use alacritty_terminal::term;
    use futures::channel::mpsc::UnboundedReceiver;

    use super::*;
    use crate::notify::osc::Found;
    use crate::terminal::events::{self, Merge};
    use crate::terminal::shell_integration::PathStyle;
    use crate::terminal::{GridSize, OutputMarker};

    /// A sink on a terminal whose events arrive on the receiver, through a
    /// gate as a terminal's do.
    fn sink() -> (Sink, UnboundedReceiver<ChannelEvent>, Arc<OutputMarker>) {
        let (listener, rx) = Listener::channel();
        let marker = listener.output.clone();
        let term = Term::new(term::Config::default(), &GridSize::default(), listener.clone());
        let sink = Sink::new(
            Arc::new(FairMutex::new(term)),
            Arc::new(Mutex::new(Parser::new(PathStyle::Posix))),
            Arc::new(Mutex::new(Marks::default())),
            listener,
            "start \"sh\"".into(),
        );
        (sink, rx, marker)
    }

    /// What the terminal's event task takes, by kind, in order; the gate
    /// learns it was taken, as the task's chunks tell it.
    fn kinds(sink: &Sink, rx: &mut UnboundedReceiver<ChannelEvent>) -> Vec<&'static str> {
        let taken: Vec<ChannelEvent> = std::iter::from_fn(|| rx.try_next().ok().flatten()).collect();
        sink.listener.gate.lock().took(taken.iter().map(events::kind_of));
        taken
            .iter()
            .map(|event| match event {
                ChannelEvent::Replay(true) => "replay-start",
                ChannelEvent::Replay(false) => "replay-end",
                ChannelEvent::Osc(Found::Notice(_)) => "notice",
                ChannelEvent::Osc(_) => "osc",
                ChannelEvent::Pointer(..) => "pointer",
                ChannelEvent::Alac(AlacEvent::Bell) => "bell",
                ChannelEvent::Alac(AlacEvent::Exit) => "exit",
                ChannelEvent::Alac(_) => "alac",
                ChannelEvent::Merged(Merge::Wakeup) => "wakeup",
                ChannelEvent::Merged(_) => "merged",
                ChannelEvent::Lost => "lost",
                ChannelEvent::Failed => "failed",
                ChannelEvent::Foreground => "foreground",
                ChannelEvent::Broken(_) => "broken",
                ChannelEvent::InputRefused => "input-refused",
            })
            .collect()
    }

    #[test]
    fn output_is_parsed_and_wakes_the_ui_like_a_pty_read() {
        let (mut sink, mut rx, marker) = sink();
        sink.output(b"hello\x1b]9;done\x07");
        assert_eq!(kinds(&sink, &mut rx), ["notice", "wakeup"]);
        assert_eq!(marker.generation(), 1);
        assert!(crate::terminal::screen_text(&sink.term.lock()).starts_with("hello\n"));
        sink.exited(Some(0));
        assert_eq!(kinds(&sink, &mut rx), ["exit"]);
    }

    /// A flood's reads each advance the latency marker, but only one
    /// `Wakeup` waits for the UI thread at a time; once it is taken, the
    /// next read queues another.
    #[test]
    fn many_reads_queue_one_wakeup_until_it_is_taken() {
        let (mut sink, mut rx, marker) = sink();
        for _ in 0..1000 {
            sink.output(b"x");
        }
        assert_eq!(marker.generation(), 1000);
        assert_eq!(kinds(&sink, &mut rx), ["wakeup"]);
        sink.output(b"y");
        sink.output(b"z");
        assert_eq!(kinds(&sink, &mut rx), ["wakeup"]);
        assert_eq!(kinds(&sink, &mut rx), Vec::<&str>::new());
    }

    /// The pointer ops a chunk holds, each with the screen it was tagged.
    fn pointer_ops(rx: &mut UnboundedReceiver<ChannelEvent>) -> Vec<(Screen, String)> {
        std::iter::from_fn(|| rx.try_next().ok().flatten())
            .filter_map(|event| match event {
                ChannelEvent::Pointer(screen, op) => Some((screen, format!("{op:?}"))),
                _ => None,
            })
            .collect()
    }

    /// A push made on the alternate screen is tagged `Alternate`, though
    /// the same chunk leaves that screen again before anything reads it.
    #[test]
    fn a_pointer_op_is_tagged_with_the_screen_active_where_it_sits() {
        use crate::terminal::pointer::{Op, Shape};
        let (mut sink, mut rx, _) = sink();
        sink.output(b"\x1b]22;>pointer\x07\x1b[?1049h\x1b]22;>crosshair\x1b\\\x1b[?1049lb\x1b]22;<\x07");
        let push = |shape| format!("{:?}", Op::Push(vec![shape]));
        assert_eq!(pointer_ops(&mut rx), [
            (Screen::Primary, push(Shape::Pointer)),
            (Screen::Alternate, push(Shape::Crosshair)),
            (Screen::Primary, format!("{:?}", Op::Pop)),
        ]);
        // The whole chunk was parsed, once.
        assert!(crate::terminal::screen_text(&sink.term.lock()).starts_with("b\n"));
    }

    /// Finds keep their stream order around pointer ops (a push then a
    /// reset must leave the stacks empty), and a chunk without one sends
    /// its finds before parsing, as before.
    #[test]
    fn finds_keep_their_order_around_pointer_ops() {
        let (mut sink, mut rx, _) = sink();
        sink.output(b"\x1b]9;one\x07\x1b]22;>grab\x07\x1bc\x07\x1b]22;?__current__\x07");
        assert_eq!(kinds(&sink, &mut rx), ["notice", "pointer", "osc", "bell", "pointer", "wakeup"]);
        sink.output(b"\x1b]9;two\x07\x07");
        assert_eq!(kinds(&sink, &mut rx), ["notice", "bell", "wakeup"]);
    }

    /// A held replay has no pointer ops (the server keeps none); one in its
    /// bytes is dropped, and the replay's other finds still arrive.
    #[test]
    fn a_held_replays_pointer_ops_are_dropped() {
        let (mut sink, mut rx, _) = sink();
        sink.replay(true);
        sink.output(b"\x1b]22;>pointer\x07\x1b]9;tail\x07");
        sink.replay(false);
        assert_eq!(kinds(&sink, &mut rx), ["replay-start", "notice", "wakeup", "replay-end"]);
    }

    #[test]
    fn a_replays_bell_and_notice_arrive_between_its_markers() {
        let (mut sink, mut rx, _) = sink();
        sink.replay(true);
        sink.output(b"\x07\x1b]9;from the tail\x07");
        sink.replay(false);
        assert_eq!(kinds(&sink, &mut rx), ["replay-start", "notice", "bell", "wakeup", "replay-end"]);
        sink.output(b"\x07");
        assert_eq!(kinds(&sink, &mut rx), ["bell", "wakeup"]);
    }

    /// A broken sink tells its terminal, and parses nothing: a replay it
    /// was holding is dropped.
    #[test]
    fn broken_sends_broken_and_drops_a_held_replay() {
        let (mut sink, mut rx, _) = sink();
        sink.replay(true);
        sink.output(b"held");
        sink.broken("the emulator failed");
        assert_eq!(kinds(&sink, &mut rx), ["replay-start", "broken"]);
        assert!(sink.replay.is_none());
        assert!(!crate::terminal::screen_text(&sink.term.lock()).contains("held"));
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

    /// A listed size of one column is fed as `MIN_COLS`, so a double-width
    /// character in the replay can't reach alacritty's 1-column panic.
    #[test]
    fn a_replay_listed_one_column_wide_is_fed_at_min_cols() {
        let (sink, _rx, _) = sink();
        let mut sink = sink.replayed_at(GridSize { cols: 1, rows: 5, ..GridSize::default() });
        sink.replay(true);
        sink.output("\x1bc😀😀漢".as_bytes());
        sink.replay(false);
        let term = sink.term.lock();
        assert_eq!((term.columns(), term.screen_lines()), (80, 24));
    }

    /// A replay is fed at its recorded size only when that differs from
    /// the grid's, in columns or rows, after the clamp; the cell size never
    /// counts.
    #[test]
    fn a_replay_is_resized_for_only_when_its_recorded_size_differs() {
        let size = |cols, rows| GridSize { cols, rows, ..GridSize::default() };
        let own = size(80, 24);
        let other_cells = GridSize { cell_width: own.cell_width * 2, cell_height: own.cell_height * 2, ..own };
        let min = crate::terminal::MIN_COLS;
        let cases = [
            ("nothing recorded", own, None, None),
            ("the grid's own size", own, Some(own), None),
            ("only the cells differ", own, Some(other_cells), None),
            ("other columns", own, Some(size(40, 24)), Some(size(40, 24))),
            ("other rows", own, Some(size(80, 5)), Some(size(80, 5))),
            ("both", own, Some(size(40, 5)), Some(size(40, 5))),
            ("one column, clamped", own, Some(size(1, 5)), Some(size(min, 5))),
            ("clamped to the grid's own", size(min, 5), Some(size(1, 5)), None),
            ("past the limits, clamped", own, Some(size(u16::MAX, u16::MAX)), Some(size(2048, 1024))),
            ("past the limits, clamped to the grid's own", size(2048, 1024), Some(size(u16::MAX, u16::MAX)), None),
        ];
        for (case, own, recorded, expected) in cases {
            assert_eq!(replay_resize(own, recorded), expected, "{case}");
        }
    }

    /// 7.1: a recorded session's replay is fed at the size its listing
    /// gave, which the connection tells the sink before the `ATTACH`.
    #[test]
    fn a_recorded_sessions_replay_is_fed_at_its_listed_size() {
        let (mut sink, _rx, _) = sink();
        sink.replay_size(40, 5);
        sink.replay(true);
        sink.output(b"\x1bc1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n$ \x1b[5;3H");
        sink.replay(false);
        let term = sink.term.lock();
        let text = crate::terminal::screen_text(&term);
        let prompt = text.lines().position(|line| line.starts_with('$')).expect("the prompt is on screen");
        assert_eq!(term.grid().cursor.point.line.0 as usize, prompt, "{text}");
    }

    /// A notice that arrives while a replay is held is written after the
    /// replayed screen, not before it, where its `ESC c` would wipe it; and
    /// a repeat is shown once until the session prints again.
    #[test]
    fn a_notice_during_a_held_replay_is_written_after_it() {
        let (mut sink, mut rx, _) = sink();
        sink.replay(true);
        sink.output(b"\x1bcreplayed screen");
        sink.note("the terminal server: no");
        sink.replay(false);
        let text = crate::terminal::screen_text(&sink.term.lock());
        let screen = text.find("replayed screen").expect("the replayed screen");
        let note = text.find("the terminal server: no").expect("the notice");
        assert!(screen < note, "{text}");
        kinds(&sink, &mut rx);
        sink.note("the terminal server: no");
        assert_eq!(kinds(&sink, &mut rx), Vec::<&str>::new(), "a repeat is refused");
        sink.output(b"x");
        sink.note("the terminal server: no");
        let text = crate::terminal::screen_text(&sink.term.lock());
        assert_eq!(text.matches("the terminal server: no").count(), 2, "{text}");
    }

    #[test]
    fn a_failure_says_why_and_marks_the_terminal() {
        let (mut sink, mut rx, _) = sink();
        sink.failed(&anyhow::anyhow!("session 3 on A8 has ended"));
        assert_eq!(kinds(&sink, &mut rx), ["wakeup", "failed"]);
        let text = crate::terminal::screen_text(&sink.term.lock());
        assert!(text.contains("session 3 on A8 has ended"), "{text}");
    }

    #[test]
    fn a_lost_session_says_why_on_screen() {
        let (mut sink, mut rx, _) = sink();
        sink.lost("its holder ended");
        assert_eq!(kinds(&sink, &mut rx), ["wakeup", "lost"]);
        let text = crate::terminal::screen_text(&sink.term.lock());
        assert!(text.contains("Session lost: its holder ended"), "{text}");
    }
}
