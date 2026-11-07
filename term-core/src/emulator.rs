//! A session's terminal state as the terminal server keeps it: a `Term`
//! fed through Brindle's [`Parser`], its [`Marks`], its status [`Records`]
//! (kept by the same [`osc::Scanner`] the client uses, so both follow the
//! stream alike) and its title. It answers nothing: its listener drops
//! every event, `PtyWrite` included, except the title changes the replay
//! needs, since `Term` keeps its title private.
//!
//! [`Emulator::render`] is the replay stream ([`crate::replay`]) of the
//! current state, and feeding that stream to a fresh emulator of the same
//! size and scrollback rebuilds it.

use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{self, Term};
use parking_lot::{Mutex, MutexGuard};

use crate::osc::{self, Found};
use crate::replay;
use crate::shell_integration::{Marks, Parser};
use crate::status::Records;

/// The scrollback a session gets when its `SPAWN` names none: Brindle's
/// own default (`scrollback_lines`).
pub const DEFAULT_SCROLLBACK: usize = 10_000;

/// Keeps the title from `Title` and `ResetTitle` and drops every other
/// event, so the emulator never writes to the PTY.
#[derive(Clone, Default)]
pub struct Titles(Arc<Mutex<Option<String>>>);

impl EventListener for Titles {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(title) => *self.0.lock() = Some(title),
            Event::ResetTitle => *self.0.lock() = None,
            _ => {}
        }
    }
}

/// A grid size, for `Term::new` and resizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct Emulator {
    term: Term<Titles>,
    parser: Parser,
    marks: Mutex<Marks>,
    records: Records,
    scanner: osc::Scanner,
    titles: Titles,
}

impl Emulator {
    /// An emulator of `cols` × `rows` (each at least 1) keeping `scrollback`
    /// lines of history; 0 means [`DEFAULT_SCROLLBACK`].
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let titles = Titles::default();
        let scrolling_history = if scrollback == 0 { DEFAULT_SCROLLBACK } else { scrollback };
        let config = term::Config { scrolling_history, ..Default::default() };
        Self {
            term: Term::new(config, &size(cols, rows), titles.clone()),
            parser: Parser::posix(),
            marks: Mutex::new(Marks::default()),
            records: Records::default(),
            scanner: osc::Scanner::new(),
            titles,
        }
    }

    /// Feeds program output. The status lifetimes the client applies
    /// (`ESC c` resets, OSC 133 `A` ends activity) apply in stream order.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut fed = 0;
        for (at, found) in self.scanner.scan_positions(bytes) {
            match found {
                Found::Reset => {
                    self.records.reset();
                    // `ESC c` clears the term's title without an event.
                    self.parser.advance(&mut self.term, &self.marks, &bytes[fed..at]);
                    fed = at;
                    *self.titles.0.lock() = None;
                }
                Found::PromptStart => {
                    self.records.end_activity();
                }
                Found::Status(message) => {
                    self.records.apply(&message);
                }
                Found::Notice(_) => {}
            }
        }
        self.parser.advance(&mut self.term, &self.marks, &bytes[fed..]);
    }

    /// The user typed: as in the client, `done` and `error` records go.
    pub fn input_seen(&mut self) {
        self.records.seen();
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let size = size(cols, rows);
        if (size.cols, size.rows) != (self.term.columns(), self.term.screen_lines()) {
            self.marks.lock().resize(&mut self.term, size);
        }
    }

    /// The replay stream of the current state. A synchronized update in
    /// progress is applied first: its buffered output was already taken
    /// from the stream, so the replay must include it.
    pub fn render(&mut self) -> Vec<u8> {
        if self.parser.sync_deadline().is_some() {
            self.parser.stop_sync(&mut self.term, &self.marks);
        }
        let title = self.titles.0.lock().clone();
        replay::render(&mut self.term, &self.marks.lock(), &self.records, title.as_deref())
    }

    pub fn term(&self) -> &Term<Titles> {
        &self.term
    }

    pub fn marks(&self) -> MutexGuard<'_, Marks> {
        self.marks.lock()
    }

    pub fn records(&self) -> &Records {
        &self.records
    }

    pub fn title(&self) -> Option<String> {
        self.titles.0.lock().clone()
    }
}

fn size(cols: u16, rows: u16) -> Size {
    Size { cols: usize::from(cols.max(1)), rows: usize::from(rows.max(1)) }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::index::{Column, Line};

    use super::*;
    use crate::status::State;

    fn text(emulator: &Emulator, line: i32) -> String {
        let term = emulator.term();
        let row = &term.grid()[Line(line)];
        let text: String = (0..term.columns()).map(|c| row[Column(c)].c).collect();
        text.trim_end().to_string()
    }

    #[test]
    fn a_render_rebuilds_the_screen_records_marks_and_title() {
        let mut emulator = Emulator::new(40, 6, 100);
        emulator.feed(b"\x1b]2;my title\x07\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07");
        emulator.feed(b"a b c\r\n\x1b]133;D;0\x07\x1b]7501;state=working:app=x\x1b\\");
        let mut copy = Emulator::new(40, 6, 100);
        copy.feed(&emulator.render());
        assert_eq!(text(&copy, 0), "$ ls");
        assert_eq!(text(&copy, 1), "a b c");
        assert_eq!(copy.title().as_deref(), Some("my title"));
        assert_eq!(copy.records().get("").map(|r| r.state), Some(State::Working));
        assert_eq!(copy.marks().commands().len(), 1);
        assert_eq!(copy.term().grid().cursor.point, emulator.term().grid().cursor.point);
    }

    #[test]
    fn queries_get_no_reply_and_the_title_resets() {
        let mut emulator = Emulator::new(20, 4, 0);
        emulator.feed(b"\x1b]2;t\x07\x1b[6n\x1b[c\x1b]11;?\x07");
        assert_eq!(emulator.title().as_deref(), Some("t"));
        emulator.feed(b"\x1bc");
        assert_eq!(emulator.title(), None);
    }

    #[test]
    fn a_pending_synchronized_update_is_in_the_render() {
        let mut emulator = Emulator::new(20, 4, 0);
        emulator.feed(b"\x1b[?2026hinside");
        let mut copy = Emulator::new(20, 4, 0);
        copy.feed(&emulator.render());
        assert_eq!(text(&copy, 0), "inside");
    }

    #[test]
    fn input_drops_done_records_and_resize_follows() {
        let mut emulator = Emulator::new(20, 4, 0);
        emulator.feed(b"\x1b]7501;state=done\x07");
        assert!(emulator.records().get("").is_some());
        emulator.input_seen();
        assert!(emulator.records().get("").is_none());
        emulator.resize(30, 5);
        assert_eq!((emulator.term().columns(), emulator.term().screen_lines()), (30, 5));
    }
}
