//! Round trips: a fixture goes into a session's `Term` through the
//! `Parser`, the replay goes into a fresh `Term` of the same size, and the
//! two must match.

use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Cursor, Dimensions};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::color::COUNT;
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorStyle, Rgb};
use parking_lot::Mutex;

use super::*;
use crate::shell_integration::{Marks, Parser};
use crate::status::{self, Record, Records, State};

struct Size {
    cols: usize,
    rows: usize,
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

/// Keeps the title, as the server's listener must.
#[derive(Clone, Default)]
struct Titles(Arc<Mutex<Option<String>>>);

impl EventListener for Titles {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(title) => *self.0.lock() = Some(title),
            Event::ResetTitle => *self.0.lock() = None,
            _ => {}
        }
    }
}

/// What a session (or a client) keeps for one terminal.
struct Session {
    term: Term<Titles>,
    parser: Parser,
    marks: Mutex<Marks>,
    records: Records,
    titles: Titles,
}

impl Session {
    fn new(cols: usize, rows: usize, history: usize) -> Self {
        let titles = Titles::default();
        let config = term::Config { scrolling_history: history, ..Default::default() };
        Session {
            term: Term::new(config, &Size { cols, rows }, titles.clone()),
            parser: Parser::posix(),
            marks: Mutex::new(Marks::default()),
            records: Records::default(),
            titles,
        }
    }

    /// Feeds `bytes` whole, applying the status lifetimes the client
    /// applies (`ESC c` resets, OSC 133 `A` ends activity) in stream order.
    fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, &self.marks, bytes);
        for found in status_events(bytes) {
            match found {
                Found::Reset => {
                    self.records.reset();
                }
                Found::Prompt => {
                    self.records.end_activity();
                }
                Found::Status(body) => {
                    if let Some(message) = status::parse(&body) {
                        self.records.apply(&message);
                    }
                }
            }
        }
    }

    fn render(&mut self) -> Vec<u8> {
        let title = self.titles.0.lock().clone();
        render(&mut self.term, &self.marks.lock(), &self.records, title.as_deref())
    }

    /// A fresh session of the same size and limit, fed this one's replay.
    fn replayed(&mut self, history: usize) -> Session {
        let bytes = self.render();
        let mut copy = Session::new(self.term.columns(), self.term.screen_lines(), history);
        copy.feed(&bytes);
        copy
    }

    fn text(&self, line: i32) -> String {
        let row = &self.term.grid()[Line(line)];
        let text: String = (0..self.term.columns()).map(|c| row[Column(c)].c).collect();
        text.trim_end().to_string()
    }
}

enum Found {
    Reset,
    Prompt,
    Status(Vec<u8>),
}

/// The status-relevant sequences in `bytes`, in order (fixtures and the
/// replay keep each sequence in one piece).
fn status_events(bytes: &[u8]) -> Vec<Found> {
    let mut found = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] != 0x1b {
            i += 1;
            continue;
        }
        match bytes[i + 1] {
            b'c' => {
                found.push(Found::Reset);
                i += 2;
            }
            b']' => {
                let start = i + 2;
                let mut end = start;
                while end < bytes.len() && bytes[end] != 0x07 && bytes[end] != 0x1b {
                    end += 1;
                }
                let body = &bytes[start..end];
                if let Some(status) = body.strip_prefix(b"7501;") {
                    found.push(Found::Status(status.to_vec()));
                } else if body == b"133;A" {
                    found.push(Found::Prompt);
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    found
}

/// Everything a replay must reproduce, for one terminal.
#[derive(Debug, PartialEq)]
struct Dump {
    cols: usize,
    lines: usize,
    history: usize,
    mode: TermMode,
    cursor_style: CursorStyle,
    cursor: Cursor<Cell>,
    saved_cursor: Cursor<Cell>,
    primary_cursor: Cursor<Cell>,
    hidden: Hidden,
    colors: Vec<Option<Rgb>>,
    title: Option<String>,
    marks: Vec<String>,
    reported_dir: Option<std::path::PathBuf>,
    records: Vec<(String, Record)>,
}

/// Grids line by line, then the rest, so a failure names the line.
fn assert_same(a: &mut Session, b: &mut Session) {
    let a_grids = grids(&mut a.term);
    let b_grids = grids(&mut b.term);
    assert_eq!(a_grids.len(), b_grids.len(), "screens");
    for (screen, (ga, gb)) in a_grids.iter().zip(&b_grids).enumerate() {
        assert_eq!(ga.len(), gb.len(), "screen {screen}: line count");
        for (i, (ra, rb)) in ga.iter().zip(gb).enumerate() {
            assert_eq!(ra, rb, "screen {screen}, row {i} from the top");
        }
    }
    assert_eq!(dump(a), dump(b));
}

/// Every row of the active grid and, with the alternate screen active, of
/// the primary one.
fn grids(term: &mut Term<Titles>) -> Vec<Vec<Vec<Cell>>> {
    let rows = |grid: &alacritty_terminal::Grid<Cell>| -> Vec<Vec<Cell>> {
        (grid.topmost_line().0..=grid.bottommost_line().0)
            .map(|l| (0..grid.columns()).map(|c| grid[Line(l)][Column(c)].clone()).collect())
            .collect()
    };
    let mut out = vec![rows(term.grid())];
    if term.mode().contains(TermMode::ALT_SCREEN) {
        out.push(with_primary(term, |t| rows(t.grid())));
    }
    out
}

fn dump(s: &mut Session) -> Dump {
    let hidden = Hidden::probe(&mut s.term);
    let primary_cursor = with_primary(&mut s.term, |t| t.grid().cursor.clone());
    let marks = s.marks.lock();
    let term = &s.term;
    Dump {
        cols: term.columns(),
        lines: term.screen_lines(),
        history: term.grid().history_size(),
        mode: *term.mode(),
        cursor_style: term.cursor_style(),
        cursor: term.grid().cursor.clone(),
        saved_cursor: term.grid().saved_cursor.clone(),
        primary_cursor,
        hidden,
        colors: (0..COUNT).map(|i| term.colors()[i]).collect(),
        title: s.titles.0.lock().clone(),
        marks: marks
            .commands()
            .iter()
            .map(|c| {
                let at = |p: Option<crate::shell_integration::Pos>| p.map(|p| (marks.line(p), p.col));
                format!("{:?} {:?} {:?} {:?} exit={:?}", at(Some(c.prompt)), at(c.input), at(c.output), at(c.end), c.exit)
            })
            .collect(),
        reported_dir: marks.reported_dir().map(Into::into),
        records: s.records.iter_by_update().map(|(id, r)| (id.to_string(), r.clone())).collect(),
    }
}

/// Feeds `fixture`, replays it, and checks the copy, and that rendering
/// left the session as it was.
fn round_trip(cols: usize, rows: usize, history: usize, fixture: &[u8]) -> (Session, Session) {
    let mut session = Session::new(cols, rows, history);
    session.feed(fixture);
    let before_grids = grids(&mut session.term);
    let before = dump(&mut session);
    let mut copy = session.replayed(history);
    assert_eq!(grids(&mut session.term), before_grids, "render changed the session's grids");
    assert_eq!(dump(&mut session), before, "render changed the session");
    assert_same(&mut session, &mut copy);
    // A replay of the replay is the same stream.
    assert_eq!(copy.render(), session.render(), "replays differ");
    (session, copy)
}

fn encode(text: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(text)
}

const PROMPT: &str = "\x1b]133;A\x07$ \x1b]133;B\x07";

fn command(cmd: &str, output: &str, exit: i32) -> String {
    format!("{PROMPT}{cmd}\r\n\x1b]133;C\x07{output}\x1b]133;D;{exit}\x07")
}

#[test]
fn shell_output_with_wrapped_lines() {
    let mut fixture = String::from("\x1b]7;file://host/home/me/src%20dir\x07\x1b]2;me@host: ~\x07");
    fixture += &command("echo a long line", &format!("{}\r\n", "0123456789".repeat(5)), 0);
    fixture += &command("ls", "\x1b[1;34mdir\x1b[0m  \x1b[32mrun.sh\x1b[0m\r\n", 0);
    fixture += PROMPT;
    fixture += "typed but not run, long enough to wrap";
    let (session, copy) = round_trip(20, 6, 100, fixture.as_bytes());
    assert!(session.term.grid().history_size() > 0);
    assert_eq!(copy.titles.0.lock().as_deref(), Some("me@host: ~"));
    assert_eq!(copy.marks.lock().reported_dir(), Some(std::path::Path::new("/home/me/src dir")));
}

/// Plain cells take the renderer's fast path, and each of these leaves it
/// and comes back within a row: a style, a wide char, a combining mark, a
/// hyperlink, an underline color, a tab, and a gap the cursor jumps.
#[test]
fn a_row_mixing_plain_cells_with_everything_else() {
    let fixture = concat!(
        "ab\x1b[1;31mcd\x1b[0mef中gh",
        "e\u{301}ij\x1b]8;id=1;https://example.com\x07kl\x1b]8;;\x07mn",
        "\x1b[4;58;5;3mop\x1b[0mqr\tst\x1b[5Cuv\r\n",
        "\x1b[44mwhole row in blue\x1b[0m then plain\r\n",
        "plain é ünïcode",
    );
    let (session, copy) = round_trip(80, 5, 10, fixture.as_bytes());
    assert_eq!(copy.text(0), session.text(0));
}

#[test]
fn wrapped_rows_rewrap_in_the_copy() {
    let fixture = format!("{}\r\n$ ", "abcdefghij".repeat(4));
    let (_, mut copy) = round_trip(15, 5, 10, fixture.as_bytes());
    copy.marks.lock().resize(&mut copy.term, Size { cols: 40, rows: 5 });
    let line = (copy.term.grid().topmost_line().0..=copy.term.grid().bottommost_line().0)
        .map(|l| copy.text(l))
        .find(|t| t.starts_with("abc"))
        .unwrap();
    assert_eq!(line, "abcdefghij".repeat(4));
}

#[test]
fn htop_like_alternate_screen_with_mouse_modes() {
    let mut fixture = String::from("$ htop\r\n");
    fixture += "\x1b]0;htop\x07\x1b[?1049h\x1b[?1h\x1b=\x1b[?25l\x1b[?1002h\x1b[?1006h\x1b[?2004h\x1b[?1004h";
    fixture += "\x1b[H\x1b[2J";
    fixture += "\x1b[1;1H\x1b[30;46m  PID USER      CPU%\x1b[K\x1b[0m";
    for (i, cpu) in [12.5, 3.0, 0.4].iter().enumerate() {
        fixture += &format!("\x1b[{};1H\x1b[36m{:5}\x1b[0m \x1b[1mroot\x1b[0m     \x1b[38;5;196m{cpu:4.1}\x1b[0m", i + 2, 100 + i);
    }
    fixture += "\x1b[10;1H\x1b[44m\x1b[KF1\x1b[30;46mHelp\x1b[0m\x1b[5;3H";
    let (session, copy) = round_trip(40, 10, 50, fixture.as_bytes());
    let mode = *copy.term.mode();
    assert!(mode.contains(TermMode::ALT_SCREEN | TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE));
    assert!(!mode.contains(TermMode::SHOW_CURSOR));
    assert_eq!(copy.text(0), session.text(0));
    assert_eq!(copy.titles.0.lock().as_deref(), Some("htop"));
}

#[test]
fn leaving_the_alternate_screen_after_a_replay() {
    let fixture = "\x1b[31mprimary\x1b[0m text\r\n$ vim\x1b[?1049h\x1b[Hin vim\x1b[2;5H";
    let (mut session, mut copy) = round_trip(30, 6, 20, fixture.as_bytes());
    for s in [&mut session, &mut copy] {
        s.feed(b"\x1b[?1049lback");
    }
    assert_same(&mut session, &mut copy);
    assert_eq!(copy.text(1), "$ vimback");
}

#[test]
fn five_hundred_lines_into_a_thousand_line_scrollback() {
    let fixture: String = (1..=500).map(|i| format!("line {i}\r\n")).collect();
    let (_, copy) = round_trip(80, 24, 1000, fixture.as_bytes());
    let grid = copy.term.grid();
    let text: Vec<String> = (grid.topmost_line().0..=grid.bottommost_line().0).map(|l| copy.text(l)).collect();
    let expected: Vec<String> = (1..=500).map(|i| format!("line {i}")).collect();
    assert_eq!(&text[..500], &expected[..]);
}

#[test]
fn a_full_scrollback_that_has_rotated() {
    let fixture: String = (1..=300).map(|i| format!("{i:>5} \x1b[3{}mcolored\x1b[0m\r\n", i % 8)).collect();
    let (_, copy) = round_trip(20, 10, 100, fixture.as_bytes());
    assert_eq!(copy.term.grid().history_size(), 100);
}

#[test]
fn marks_for_three_commands() {
    let mut fixture = command("true", "", 0);
    fixture += &command("false", "", 1);
    fixture += &command("ls", "a  b\r\n", 0);
    fixture += PROMPT;
    let (session, copy) = round_trip(40, 10, 100, fixture.as_bytes());
    let marks = copy.marks.lock();
    assert_eq!(marks.commands().len(), 4);
    assert_eq!(marks.commands().iter().filter(|c| c.failed()).count(), 1);
    let prompts: Vec<i64> = marks.prompt_lines().collect();
    assert_eq!(prompts, session.marks.lock().prompt_lines().collect::<Vec<_>>());
    assert_eq!(marks.jump_target(9, true), session.marks.lock().jump_target(9, true));
    assert_eq!(marks.failed_spans(0, 9), session.marks.lock().failed_spans(0, 9));
}

#[test]
fn marks_in_the_scrollback_and_on_wrapped_rows() {
    let mut fixture = String::new();
    for i in 0..30 {
        fixture += &command(&format!("echo {i}"), &format!("{}\r\n", "x".repeat(i * 3)), i as i32 % 3);
    }
    // A prompt that ends exactly at the margin, and one that wraps.
    fixture += &format!("\x1b]133;A\x07{}\x1b]133;B\x07", "p".repeat(16));
    fixture += "ls\r\n\x1b]133;C\x07\x1b]133;D;0\x07";
    fixture += &format!("\x1b]133;A\x07{}\x1b]133;B\x07", "q".repeat(20));
    // A mark at the start of a row that a wrap joined to the one above.
    fixture += &format!("\r\n{}x\x1b[1G\x1b]133;A\x07$ \x1b]133;B\x07", "r".repeat(16));
    round_trip(16, 8, 200, fixture.as_bytes());
}

/// The copy writes every scrollback row at the bottom, so a wrap there
/// scrolls in a row filled with the pen's background, which the session's
/// row (wrapped higher up, without scrolling) doesn't have.
#[test]
fn a_colored_wrap_that_scrolls_only_in_the_copy() {
    let mut fixture: String = (0..10).map(|i| format!("{i}\r\n")).collect();
    fixture += "\x1b[1;1H\x1b[41mwwwwwwwwwwwww\x1b[0m\x1b[4;1H";
    round_trip(10, 4, 20, fixture.as_bytes());
}

#[test]
fn a_working_status_record() {
    let fixture = format!(
        "{PROMPT}cargo build\r\n\x1b]133;C\x07\x1b]7501;state=working:app=cargo:progress=40:msg={}\x1b\\\
         \x1b]7501;state=blocked:id=agent/1:kind=permission:title={}\x07Compiling\r\n",
        encode("Compiling brindle"),
        encode("Sub task"),
    );
    let (_, copy) = round_trip(40, 6, 10, fixture.as_bytes());
    let root = copy.records.get("").expect("root record");
    assert_eq!(root.state, State::Working);
    assert_eq!(root.progress, Some(40));
    assert_eq!(root.msg.as_deref(), Some("Compiling brindle"));
    assert_eq!(copy.records.get("agent/1").map(|r| r.state), Some(State::Blocked));
}

#[test]
fn record_encoding_round_trips() {
    let record = Record {
        state: State::Blocked,
        kind: Some(status::Kind::Question),
        progress: None,
        app: Some("claude-code".into()),
        title: Some("Ask: «which?»".into()),
        msg: Some("Pick one; or two".into()),
    };
    match status::parse(record.encode("a/b").as_bytes()) {
        Some(status::Message::Report { id, record: parsed }) => {
            assert_eq!(id, "a/b");
            assert_eq!(parsed, record);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn colors_attributes_and_wide_cells() {
    let mut fixture = String::new();
    fixture += "\x1b[38;2;255;128;0;48;2;0;0;64mtrue\x1b[0m \x1b[38;5;123;48;5;236m256\x1b[0m ";
    fixture += "\x1b[91;104mbright\x1b[0m \x1b[1;2;3mbdi\x1b[0m \x1b[7;8;9mrhs\x1b[0m\r\n";
    fixture += "\x1b[4mu\x1b[4:2md\x1b[4:3mc\x1b[4:4mo\x1b[4:5ma\x1b[0m \x1b[4;58;2;1;2;3mcol\x1b[58;5;9mor\x1b[0m\r\n";
    fixture += "中文 😀 e\u{301} ❤\u{fe0f} \x1b[31m宽\x1b[0m\r\n";
    // A wide char that doesn't fit at the end of a row wraps whole.
    fixture += "123456789中x\r\n";
    fixture += "\x1b]8;id=doc;https://example.com/a;b\x1b\\link\x1b]8;;\x1b\\ \x1b]8;;file:///tmp\x07anon\x1b]8;;\x07\r\n";
    fixture += "\x1b[42mbg to the end\x1b[K\x1b[0m\r\n";
    round_trip(10, 12, 20, fixture.as_bytes());
}

#[test]
fn a_colored_row_wrapping_at_the_bottom() {
    let fixture: String = (0..8).map(|i| format!("\x1b[4{}m{}\x1b[0m", i % 8, "w".repeat(13))).collect();
    round_trip(10, 4, 20, fixture.as_bytes());
}

#[test]
fn a_scroll_region() {
    let fixture = "header\r\n\x1b[3;8r\x1b[8;1H".to_string()
        + &(0..12).map(|i| format!("\r\nregion {i}")).collect::<String>()
        + "\x1b[9;1Hfooter\x1b[6;4H";
    let (_, copy) = round_trip(20, 10, 10, fixture.as_bytes());
    assert_eq!(Hidden::probe(&mut Session::new(20, 10, 0).term).scroll_region, 0..10);
    let mut copy = copy;
    assert_eq!(Hidden::probe(&mut copy.term).scroll_region, 2..8);
}

#[test]
fn origin_mode_inside_a_scroll_region() {
    round_trip(20, 10, 10, b"\x1b[4;7r\x1b[?6h\x1b[2;5Hin region");
}

#[test]
fn modes_cursor_and_palette() {
    let mut fixture = String::from("\x1b[?1000h\x1b[?1005h\x1b[?1007l\x1b[?7l\x1b[?1042l\x1b[4h\x1b[20h");
    fixture += "\x1b[5 q\x1b]4;1;rgb:12/34/56\x07\x1b]4;200;#abcdef\x07\x1b]11;rgb:10/20/30\x07";
    fixture += "\x1b[3;3H\x1b[1;33msaved\x1b7\x1b[0m\x1b[10;1H\x1b[45;4mpen";
    round_trip(20, 12, 10, fixture.as_bytes());
}

#[test]
fn the_other_mouse_modes() {
    round_trip(20, 5, 0, b"\x1b[?1003h\x1b[?1006hx");
    round_trip(20, 5, 0, b"\x1b[?25l\x1b[?12h\x1b[3 qx");
}

#[test]
fn a_pending_wrap() {
    let (_, mut copy) = round_trip(10, 4, 10, b"\x1b[32m0123456789");
    assert!(copy.term.grid().cursor.input_needs_wrap);
    copy.feed(b"x");
    assert_eq!(copy.text(1), "x");
    round_trip(10, 4, 10, b"12345678\xe4\xb8\xad");
}

#[test]
fn tab_stops_and_tab_cells() {
    round_trip(30, 4, 0, b"a\tb\tc\r\n\x1b[3g\x1b[5G\x1bH\x1b[12G\x1bH\r\tx\ty\x1b[1;2H");
}

#[test]
fn line_drawing_charset() {
    round_trip(20, 4, 0, b"\x1b(0lqqk\r\nx  x\r\nmqqj");
    round_trip(20, 4, 0, b"\x1b)0\x0elqk\x0f ok\x0e");
}

#[test]
fn an_empty_terminal() {
    let (_, copy) = round_trip(80, 24, 100, b"");
    assert_eq!(copy.term.grid().history_size(), 0);
}

#[test]
fn rendering_replaces_what_the_receiver_had() {
    let mut session = Session::new(20, 5, 10);
    session.feed(b"fresh");
    let bytes = session.render();
    let mut copy = Session::new(20, 5, 10);
    copy.feed(b"\x1b]4;3;rgb:ff/00/00\x07\x1b[?1049h\x1b[?1000hstale\x1b]133;A\x07");
    copy.feed(&bytes);
    assert_same(&mut session, &mut copy);
}

#[test]
#[should_panic(expected = "row 0")]
fn the_comparison_sees_a_changed_cell() {
    let mut session = Session::new(10, 3, 0);
    session.feed(b"\x1b[31mred");
    let mut copy = session.replayed(0);
    copy.term.grid_mut()[Line(0)][Column(1)].fg = Color::Named(NamedColor::Blue);
    assert_same(&mut session, &mut copy);
}

#[test]
fn hyperlinks_survive() {
    let (_, copy) = round_trip(20, 3, 0, b"\x1b]8;id=x;https://a.example/\x07ab\x1b]8;;\x07c");
    let link = copy.term.grid()[Line(0)][Column(1)].hyperlink().expect("a link");
    assert_eq!((link.id(), link.uri()), ("x", "https://a.example/"));
    assert!(copy.term.grid()[Line(0)][Column(2)].hyperlink().is_none());
}

// ---- known limits -----------------------------------------------------------

/// Overwriting a wrapped wide char also clears its leading spacer, but
/// erasing it with `CSI K` leaves the spacer behind.
#[test]
fn an_overwritten_wrapped_wide_char() {
    round_trip(10, 3, 0, "123456789中\x1b[2;1Hxy".as_bytes());
    round_trip(10, 3, 0, "123456789中\x1b[2;1H\x1b[K".as_bytes());
}

/// The title stack isn't visible through `Term`, so popping a title the
/// program pushed before the replay restores nothing in the copy.
#[test]
fn limit_the_title_stack_is_lost() {
    let mut session = Session::new(20, 4, 0);
    session.feed(b"\x1b]2;outer\x07\x1b[22;0t\x1b]2;inner\x07");
    let mut copy = session.replayed(0);
    session.feed(b"\x1b[23;0t");
    copy.feed(b"\x1b[23;0t");
    assert_eq!(session.titles.0.lock().as_deref(), Some("outer"));
    assert_eq!(copy.titles.0.lock().as_deref(), Some("inner"));
}

/// A wrapped row scrolled down to the bottom has nothing to wrap into, so
/// the copy's row loses `WRAPLINE`.
#[test]
fn limit_wrapline_on_the_last_row() {
    let mut session = Session::new(10, 3, 0);
    session.feed(b"0123456789ab\x1b[2T");
    let last = Line(2);
    assert!(session.term.grid()[last][Column(9)].flags.contains(Flags::WRAPLINE));
    let copy = session.replayed(0);
    assert!(!copy.term.grid()[last][Column(9)].flags.contains(Flags::WRAPLINE));
    assert_eq!(copy.text(2), session.text(2));
}

/// A command whose prompt scrolled out of the history keeps its later
/// marks, but the copy starts the command at its first surviving mark.
#[test]
fn limit_a_prompt_above_the_scrollback() {
    let mut fixture = String::from("\x1b]133;A\x07$ \x1b]133;B\x07cat\r\n\x1b]133;C\x07");
    fixture += &(0..8).map(|i| format!("{i}\r\n")).collect::<String>();
    let mut session = Session::new(10, 4, 5);
    session.feed(fixture.as_bytes());
    let copy = session.replayed(5);
    let (s, c) = (session.marks.lock(), copy.marks.lock());
    assert_eq!(s.commands().len(), 1);
    assert_eq!(s.prompt_lines().count(), 0, "the prompt is above the history");
    assert_eq!(c.commands().len(), 0, "an output mark alone opens no command");
}
