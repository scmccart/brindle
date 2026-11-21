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
use crate::shell_integration::{Marks, Parser, PathStyle};
use crate::status::Records;

/// The scrollback a session gets when its `SPAWN` names none: Brindle's
/// own default (`scrollback_lines`).
pub const DEFAULT_SCROLLBACK: usize = 10_000;

/// Keeps the title from `Title` and `ResetTitle` and drops every other
/// event, so the emulator never writes to the PTY.
#[derive(Clone, Default)]
pub struct Titles(Arc<Mutex<Option<String>>>);

impl EventListener for Titles {
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "foreign enum: alacritty_terminal::event::Event, of which only the titles matter here"
    )]
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(title) => *self.0.lock() = Some(title),
            Event::ResetTitle => *self.0.lock() = None,
            _ => {}
        }
    }
}

/// The fewest columns any grid Brindle creates or resizes has, on the
/// client and in the server's emulators alike, so their grids and replays
/// agree. `alacritty_terminal` 0.26 panics (`grid/mod.rs:439`) when a grid
/// 1 column wide is fed a double-width character, and nothing useful fits
/// in less than this anyway. Rows are clamped to 1 only.
pub const MIN_COLS: u16 = 10;

/// The most columns any grid Brindle creates or resizes has, on the client
/// and in the server's emulators alike. A size comes from outside (a
/// `SPAWN`, a `RESIZE`, a listing), and a grid allocates every cell of its
/// screen: at [`MAX_COLS`] × [`MAX_ROWS`] that is 2 M cells a screen, where
/// 65535 × 65535 would be more memory than a machine has. 2048 columns is
/// wider than a 7680-pixel display at a 4-pixel cell.
pub const MAX_COLS: u16 = 2048;

/// The most rows any grid has; see [`MAX_COLS`].
pub const MAX_ROWS: u16 = 1024;

/// `cols` × `rows` within the limits every grid keeps: [`MIN_COLS`] to
/// [`MAX_COLS`] columns and 1 to [`MAX_ROWS`] rows. A size already within
/// them comes back as it is.
pub fn clamped(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(MIN_COLS, MAX_COLS), rows.clamp(1, MAX_ROWS))
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
    /// An emulator of `cols` × `rows`, within the limits every grid keeps
    /// ([`clamped`]), keeping `scrollback` lines of history; 0 means
    /// [`DEFAULT_SCROLLBACK`].
    ///
    /// It reads directory reports (OSC 7, 9;9) as `style` spells paths: the
    /// style of the platform the session's program runs on, which the
    /// caller knows and this crate doesn't. So a Windows core keeps a 9;9
    /// `C:\src`, and its replays carry it.
    pub fn new(cols: u16, rows: u16, scrollback: usize, style: PathStyle) -> Self {
        let titles = Titles::default();
        let scrolling_history = if scrollback == 0 { DEFAULT_SCROLLBACK } else { scrollback };
        let config = term::Config { scrolling_history, ..Default::default() };
        Self {
            term: Term::new(config, &size(cols, rows), titles.clone()),
            parser: Parser::new(style),
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

    /// Resizes the grid to `cols` × `rows`, within the same limits.
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

/// The grid size an emulator asked for `cols` × `rows` has ([`clamped`]).
fn size(cols: u16, rows: u16) -> Size {
    let (cols, rows) = clamped(cols, rows);
    Size { cols: usize::from(cols), rows: usize::from(rows) }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

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
        let mut emulator = Emulator::new(40, 6, 100, PathStyle::Posix);
        emulator.feed(b"\x1b]2;my title\x07\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07");
        emulator.feed(b"a b c\r\n\x1b]133;D;0\x07\x1b]7501;state=working:app=x\x1b\\");
        let mut copy = Emulator::new(40, 6, 100, PathStyle::Posix);
        copy.feed(&emulator.render());
        assert_eq!(text(&copy, 0), "$ ls");
        assert_eq!(text(&copy, 1), "a b c");
        assert_eq!(copy.title().as_deref(), Some("my title"));
        assert_eq!(copy.records().get("").map(|r| r.state), Some(State::Working));
        assert_eq!(copy.marks().commands().len(), 1);
        assert_eq!(copy.term().grid().cursor.point, emulator.term().grid().cursor.point);
    }

    #[test]
    fn a_one_column_emulator_fed_a_double_width_char_is_ten_columns_wide_and_does_not_panic() {
        let mut emulator = Emulator::new(1, 4, 0, PathStyle::Posix);
        emulator.feed("😀😀漢".as_bytes());
        assert_eq!(emulator.term().columns(), usize::from(MIN_COLS));
        emulator.resize(1, 4);
        emulator.feed("😀😀".as_bytes());
        assert_eq!(emulator.term().columns(), usize::from(MIN_COLS));
        assert_eq!(emulator.term().screen_lines(), 4);
    }

    #[test]
    fn a_zero_by_zero_emulator_is_min_cols_by_one_row() {
        let emulator = Emulator::new(0, 0, 0, PathStyle::Posix);
        assert_eq!((emulator.term().columns(), emulator.term().screen_lines()), (usize::from(MIN_COLS), 1));
    }

    #[test]
    fn a_size_within_the_limits_is_kept_and_any_other_is_clamped_to_them() {
        assert_eq!((MIN_COLS, MAX_COLS, MAX_ROWS), (10, 2048, 1024));
        for size in [(MIN_COLS, 1), (80, 24), (132, 43), (MAX_COLS, MAX_ROWS)] {
            assert_eq!(clamped(size.0, size.1), size, "{size:?}");
        }
        assert_eq!(clamped(0, 0), (MIN_COLS, 1));
        assert_eq!(clamped(MIN_COLS - 1, 5), (MIN_COLS, 5));
        assert_eq!(clamped(MAX_COLS + 1, 5), (MAX_COLS, 5));
        assert_eq!(clamped(80, MAX_ROWS + 1), (80, MAX_ROWS));
        assert_eq!(clamped(u16::MAX, u16::MAX), (MAX_COLS, MAX_ROWS));
    }

    /// The largest size a `SPAWN` or a listing can carry builds a grid at
    /// the limits, not one of 65535 × 65535 cells.
    #[test]
    fn an_emulator_asked_for_65535_by_65535_is_2048_by_1024() {
        let emulator = Emulator::new(65535, 65535, 0, PathStyle::Posix);
        assert_eq!((emulator.term().columns(), emulator.term().screen_lines()), (2048, 1024));
    }

    #[test]
    fn a_resize_past_the_limits_stops_at_them() {
        let mut emulator = Emulator::new(132, 43, 0, PathStyle::Posix);
        assert_eq!((emulator.term().columns(), emulator.term().screen_lines()), (132, 43), "within them: kept");
        emulator.resize(65535, 2);
        assert_eq!((emulator.term().columns(), emulator.term().screen_lines()), (2048, 2));
        emulator.resize(20, 65535);
        assert_eq!((emulator.term().columns(), emulator.term().screen_lines()), (20, 1024));
        emulator.resize(120, 40);
        assert_eq!((emulator.term().columns(), emulator.term().screen_lines()), (120, 40));
    }

    #[test]
    fn queries_get_no_reply_and_the_title_resets() {
        let mut emulator = Emulator::new(20, 4, 0, PathStyle::Posix);
        emulator.feed(b"\x1b]2;t\x07\x1b[6n\x1b[c\x1b]11;?\x07");
        assert_eq!(emulator.title().as_deref(), Some("t"));
        emulator.feed(b"\x1bc");
        assert_eq!(emulator.title(), None);
    }

    #[test]
    fn a_pending_synchronized_update_is_in_the_render() {
        let mut emulator = Emulator::new(20, 4, 0, PathStyle::Posix);
        emulator.feed(b"\x1b[?2026hinside");
        let mut copy = Emulator::new(20, 4, 0, PathStyle::Posix);
        copy.feed(&emulator.render());
        assert_eq!(text(&copy, 0), "inside");
    }

    #[test]
    fn input_drops_done_records_and_resize_follows() {
        let mut emulator = Emulator::new(20, 4, 0, PathStyle::Posix);
        emulator.feed(b"\x1b]7501;state=done\x07");
        assert!(emulator.records().get("").is_some());
        emulator.input_seen();
        assert!(emulator.records().get("").is_none());
        emulator.resize(30, 5);
        assert_eq!((emulator.term().columns(), emulator.term().screen_lines()), (30, 5));
    }

    /// Whether `render` holds `needle`.
    fn replays(render: &[u8], needle: &[u8]) -> bool {
        render.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn a_windows_style_emulator_replays_a_9_9_directory() {
        let mut emulator = Emulator::new(20, 4, 0, PathStyle::Windows);
        emulator.feed(b"\x1b]9;9;C:\\src\x1b\\");
        assert_eq!(emulator.marks().reported_dir(), Some(Path::new(r"C:\src")));
        let render = emulator.render();
        assert!(replays(&render, b"\x1b]7;file:///C:/src\x1b\\"), "{:?}", String::from_utf8_lossy(&render));
    }

    #[test]
    fn a_posix_style_emulator_keeps_osc_7_and_drops_a_windows_9_9_path() {
        let mut emulator = Emulator::new(20, 4, 0, PathStyle::Posix);
        emulator.feed(b"\x1b]7;file://host/home/me\x1b\\");
        emulator.feed(b"\x1b]9;9;C:\\src\x1b\\");
        assert_eq!(emulator.marks().reported_dir(), Some(Path::new("/home/me")));
        let render = emulator.render();
        assert!(replays(&render, b"\x1b]7;file:///home/me\x1b\\"), "{:?}", String::from_utf8_lossy(&render));
        assert!(!replays(&render, b"C:"), "{:?}", String::from_utf8_lossy(&render));
    }

    /// A reported command line survives a render into a new emulator and
    /// that one's render (a core restart from a checkpoint), escaped.
    #[test]
    fn a_reported_command_line_survives_two_emulators() {
        let mut first = Emulator::new(30, 4, 50, PathStyle::Posix);
        first.feed(b"\x1b]633;A\x07$ \x1b]633;B\x07echo 'a;b' \\\\\r\n");
        first.feed(b"\x1b]633;E;echo 'a\\x3bb' \\\\\\\\;f00d\x07\x1b]633;C\x07a;b \\\r\n\x1b]633;D;0\x07");
        let line = |e: &Emulator| e.marks().commands().front().and_then(|c| c.line.clone());
        assert_eq!(line(&first).as_deref(), Some("echo 'a;b' \\\\"));
        let mut second = Emulator::new(30, 4, 50, PathStyle::Posix);
        second.feed(&first.render());
        let mut third = Emulator::new(30, 4, 50, PathStyle::Posix);
        third.feed(&second.render());
        assert_eq!(line(&third), line(&first));
        assert_eq!(text(&third, 0), "$ echo 'a;b' \\\\");
    }

    #[test]
    fn a_replayed_directory_survives_a_second_emulator() {
        let mut first = Emulator::new(20, 4, 0, PathStyle::Windows);
        first.feed(b"\x1b]9;9;C:\\src\x1b\\");
        // As a new core rebuilds a session from its checkpoint.
        let mut second = Emulator::new(20, 4, 0, PathStyle::Windows);
        second.feed(&first.render());
        assert_eq!(second.marks().reported_dir(), Some(Path::new(r"C:\src")));
        let render = second.render();
        assert!(replays(&render, b"\x1b]7;file:///C:/src\x1b\\"), "{:?}", String::from_utf8_lossy(&render));
    }
}
