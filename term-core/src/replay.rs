//! The replay renderer: turns a session's terminal state into a byte stream
//! that, fed through [`Parser`](crate::shell_integration::Parser) into a
//! fresh `Term`, rebuilds the same screen. The terminal server sends it when
//! a client attaches, and uses it for checkpoints.
//!
//! The stream is, in order:
//! - `ESC c`, then the palette resets (OSC 104, 110, 111, 112) and every
//!   color the program set (OSC 4, 10, 11, 12);
//! - the primary screen: the scrollback, then the visible rows, with SGR
//!   runs and OSC 8 hyperlinks. A row joined to the next by `WRAPLINE` ends
//!   without a line break, so the receiver wraps it itself and can rewrap it
//!   on resize. OSC 133 marks are emitted where the receiver's cursor is at
//!   each mark's position, so its parser records equal [`Marks`];
//! - if the alternate screen is active: the primary cursor, `CSI ? 1049 h`
//!   and the alternate grid;
//! - the active screen's saved cursor (`ESC 7`), custom tab stops, the
//!   scroll region (DECSTBM) and origin mode;
//! - the cursor (with its pending wrap), its character sets, its SGR
//!   template, the modes (DECCKM, keypad, mouse 1000/1002/1003, 1005/1006,
//!   focus 1004, bracketed paste, 1007, cursor visibility, autowrap, 1042,
//!   insert, LNM, kitty keyboard flags), the cursor shape and blink
//!   (DECSCUSR), and the title (OSC 2);
//! - the working directory the shell last reported (OSC 7), then the latest
//!   OSC 7501 report of each status record, least recently updated first.
//!
//! # The receiver
//! The stream is rendered at the session's size. The receiver must be a
//! `Term` with the same columns and screen lines and a scrollback limit at
//! least the session's (use the session's limit), and it resizes to its own
//! size only after the whole replay. Its parser must be Brindle's `Parser`
//! (for the marks), and its OSC 7501 scanner must see the stream in order:
//! the reports come after the marks, because OSC 133 `A` ends `working`
//! records. A `blocked`, `done` or `error` record arrives as a fresh report,
//! so a client that notifies on those should not notify during a replay.
//!
//! # Fidelity limits
//! alacritty keeps some state private, so [`render`] probes it (see
//! below) or takes it from the caller. What can't be recovered:
//! - **The title stack** (`CSI 22 t` / `CSI 23 t`): only the current title
//!   is replayed, and the caller must track it from `Event::Title` and
//!   `Event::ResetTitle`, because `Term` doesn't expose it.
//! - **The keyboard mode stack** (kitty): only the active flags are pushed.
//! - **The active character set** is replayed only as G0 or G1 (SI/SO); a
//!   program that shifted to G2 or G3 gets whichever of G0/G1 matches.
//! - **A cursor style equal to the default** isn't replayed, so a program
//!   that set it explicitly to the default looks like one that never set it.
//! - **`WRAPLINE` on the last row** (only after scrolling the screen down,
//!   `CSI T` or a reverse index) is lost, since nothing follows to wrap
//!   into.
//! - **A prompt mark above the top of the scrollback**: the command's later
//!   marks survive, but the receiver records the command as starting at its
//!   first surviving mark.
//! - **Marks recorded out of position order** (the cursor moved back between
//!   them) are replayed in position order, which can group them into
//!   commands differently.
//! - **The primary screen's saved cursor while the alternate screen is
//!   active** becomes the primary cursor, as it is on entering it.
//! - **A cursor outside the scroll region in origin mode** is clamped into it.
//! - **Hyperlink ids containing `:` or `;`** are cut at that character.
//! - **Mode 2026 (synchronized update)** in progress: the renderer sees the
//!   term without the buffered output, which arrives in the live tail.
//!
//! # Purity
//! `Term` doesn't expose the inactive grid, the scroll region, the tab
//! stops or the active charset, so [`render`] takes `&mut Term`: it swaps
//! to the primary screen and back, and probes the others by running
//! handler calls on the visible rows. Everything it touches (rows, cursor,
//! selection, vi cursor, grids) is restored before it returns, so the term
//! is left as it was, except that the primary screen's saved cursor is the
//! primary cursor while the alternate screen is active (as alacritty itself
//! sets it on entry). It does no I/O and takes no locks.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::ops::Range;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Charsets, Cursor, Dimensions, Grid};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{
    CharsetIndex, Color, CursorShape, CursorStyle, Handler, NamedColor, StandardCharset,
};

use crate::shell_integration::{Mark, Marks};
use crate::status::Records;

/// Renders `term`, its `marks` and `records`, and the window `title` (which
/// `Term` doesn't expose) as a replay stream. See the module docs for what
/// the receiver must be and what doesn't round-trip.
pub fn render<T: EventListener>(
    term: &mut Term<T>,
    marks: &Marks,
    records: &Records,
    title: Option<&str>,
) -> Vec<u8> {
    let hidden = Hidden::probe(term);
    let mut out = Out::default();
    out.raw("\x1bc");
    out.raw("\x1b]104\x1b\\\x1b]110\x1b\\\x1b]111\x1b\\\x1b]112\x1b\\");
    let colors = term.colors();
    for i in 0..256 {
        if let Some(rgb) = colors[i] {
            out.raw(&format!("\x1b]4;{i};rgb:{:02x}/{:02x}/{:02x}\x1b\\", rgb.r, rgb.g, rgb.b));
        }
    }
    for (named, code) in [(NamedColor::Foreground, 10), (NamedColor::Background, 11), (NamedColor::Cursor, 12)] {
        if let Some(rgb) = colors[named] {
            out.raw(&format!("\x1b]{code};rgb:{:02x}/{:02x}/{:02x}\x1b\\", rgb.r, rgb.g, rgb.b));
        }
    }

    let alt = term.mode().contains(TermMode::ALT_SCREEN);
    let mark_map = mark_positions(marks);
    with_primary(term, |term| {
        let grid = term.grid();
        write_grid(&mut out, grid, &mark_map);
        if alt {
            // Entering the alternate screen copies the primary cursor (and
            // saves it as the primary's saved cursor), so it is set first.
            place_cursor(&mut out, grid, &grid.cursor, 0);
            out.charsets(&grid.cursor.charsets);
            out.pen(&Pen::of(&grid.cursor.template));
        }
    });
    let grid = term.grid();
    let lines = grid.screen_lines();
    if alt {
        out.raw("\x1b[?1049h");
        out.pen(&Pen::default());
        out.charsets(&Charsets::default());
        // The swap clears the alternate grid with the primary template.
        out.raw("\x1b[H\x1b[2J");
        write_grid(&mut out, grid, &BTreeMap::new());
    }

    let saved = &grid.saved_cursor;
    if *saved != Cursor::default() {
        out.raw(&format!("\x1b[{};{}H", saved.point.line.0 + 1, saved.point.column.0 + 1));
        out.pen(&Pen::of(&saved.template));
        out.charsets(&saved.charsets);
        out.raw("\x1b7");
        out.charsets(&Charsets::default());
    }

    let cols = grid.columns();
    let default_tabs: Vec<usize> = (1..cols.saturating_sub(1)).filter(|c| c % 8 == 0).collect();
    if hidden.tabs != default_tabs {
        out.raw("\x1b[3g");
        for stop in &hidden.tabs {
            out.raw(&format!("\x1b[{}G\x1bH", stop + 1));
        }
    }
    if hidden.scroll_region != (0..lines) {
        out.raw(&format!("\x1b[{};{}r", hidden.scroll_region.start + 1, hidden.scroll_region.end));
    }
    let mode = *term.mode();
    let origin = mode.contains(TermMode::ORIGIN);
    if origin {
        out.raw("\x1b[?6h");
    }
    let row_base = if origin { hidden.scroll_region.start } else { 0 };
    place_cursor(&mut out, grid, &grid.cursor, row_base);
    out.charsets(&grid.cursor.charsets);
    let charsets = &grid.cursor.charsets;
    let wanted = if hidden.line_drawing { StandardCharset::SpecialCharacterAndLineDrawing } else { StandardCharset::Ascii };
    if charsets[CharsetIndex::G0] != wanted && charsets[CharsetIndex::G1] == wanted {
        out.raw("\x0e");
    }
    out.pen(&Pen::of(&grid.cursor.template));

    for (flag, code) in [
        (TermMode::APP_CURSOR, "?1"),
        (TermMode::MOUSE_REPORT_CLICK, "?1000"),
        (TermMode::MOUSE_DRAG, "?1002"),
        (TermMode::MOUSE_MOTION, "?1003"),
        (TermMode::FOCUS_IN_OUT, "?1004"),
        (TermMode::UTF8_MOUSE, "?1005"),
        (TermMode::SGR_MOUSE, "?1006"),
        (TermMode::BRACKETED_PASTE, "?2004"),
        (TermMode::INSERT, "4"),
        (TermMode::LINE_FEED_NEW_LINE, "20"),
    ] {
        if mode.contains(flag) {
            out.raw(&format!("\x1b[{code}h"));
        }
    }
    // On by default after `ESC c`.
    for (flag, code) in [
        (TermMode::SHOW_CURSOR, "?25"),
        (TermMode::LINE_WRAP, "?7"),
        (TermMode::ALTERNATE_SCROLL, "?1007"),
        (TermMode::URGENCY_HINTS, "?1042"),
    ] {
        if !mode.contains(flag) {
            out.raw(&format!("\x1b[{code}l"));
        }
    }
    if mode.contains(TermMode::APP_KEYPAD) {
        out.raw("\x1b=");
    }
    let kitty = [
        (TermMode::DISAMBIGUATE_ESC_CODES, 1),
        (TermMode::REPORT_EVENT_TYPES, 2),
        (TermMode::REPORT_ALTERNATE_KEYS, 4),
        (TermMode::REPORT_ALL_KEYS_AS_ESC, 8),
        (TermMode::REPORT_ASSOCIATED_TEXT, 16),
    ]
    .into_iter()
    .filter(|(flag, _)| mode.contains(*flag))
    .map(|(_, bit)| bit)
    .sum::<u8>();
    if kitty != 0 {
        out.raw(&format!("\x1b[>{kitty}u"));
    }

    let style = term.cursor_style();
    if style != CursorStyle::default() {
        let code = match style.shape {
            CursorShape::Block => Some(2),
            CursorShape::Underline => Some(4),
            CursorShape::Beam => Some(6),
            CursorShape::HollowBlock | CursorShape::Hidden => None,
        };
        if let Some(code) = code {
            out.raw(&format!("\x1b[{} q", code - u8::from(style.blinking)));
        }
    }
    if let Some(title) = title {
        let title: String = title.chars().filter(|c| !c.is_control()).collect();
        out.raw(&format!("\x1b]2;{title}\x1b\\"));
    }
    if let Some(dir) = marks.reported_dir() {
        out.raw(&format!("\x1b]7;file://{}\x1b\\", dir_uri(&dir.to_string_lossy())));
    }
    for (id, record) in records.iter_by_update() {
        out.raw(&format!("\x1b]7501;{}\x1b\\", record.encode(id)));
    }
    out.buf
}

// ---- hidden state -------------------------------------------------------------

/// Term state alacritty keeps private, found by probing.
#[derive(Debug, PartialEq, Eq)]
struct Hidden {
    /// Screen lines, end exclusive.
    scroll_region: Range<usize>,
    /// Tab stops in `1..columns - 1` (the first and last columns never
    /// change where a tab goes).
    tabs: Vec<usize>,
    /// Whether the active charset maps to line drawing.
    line_drawing: bool,
}

impl Hidden {
    fn probe<T: EventListener>(term: &mut Term<T>) -> Self {
        let scroll_region = scroll_region(term);
        let tabs = tab_stops(term);
        let line_drawing = line_drawing(term);
        Hidden { scroll_region, tabs, line_drawing }
    }
}

/// Runs `f` on `term`, then restores the visible rows, the cursor, the
/// selection and the vi cursor.
fn probe<T, R>(term: &mut Term<T>, f: impl FnOnce(&mut Term<T>) -> R) -> R {
    let lines = term.screen_lines();
    let rows: Vec<_> = (0..lines).map(|l| term.grid()[Line(l as i32)].clone()).collect();
    let cursor = term.grid().cursor.clone();
    let selection = term.selection.clone();
    let vi = term.vi_mode_cursor;
    let result = f(term);
    for (l, row) in rows.into_iter().enumerate() {
        term.grid_mut()[Line(l as i32)] = row;
    }
    term.grid_mut().cursor = cursor;
    term.selection = selection;
    term.vi_mode_cursor = vi;
    result
}

fn set_cursor<T>(term: &mut Term<T>, line: usize, col: usize) {
    let cursor = &mut term.grid_mut().cursor;
    cursor.point = Point::new(Line(line as i32), Column(col));
    cursor.input_needs_wrap = false;
}

/// The scroll region: a reverse index scrolls (and so leaves the cursor in
/// place) only at its top, and inserting a line at the top pushes rows down
/// only as far as its bottom.
fn scroll_region<T: EventListener>(term: &mut Term<T>) -> Range<usize> {
    let lines = term.screen_lines();
    let start = probe(term, |t| {
        (1..lines)
            .find(|&i| {
                set_cursor(t, i, 0);
                t.reverse_index();
                t.grid().cursor.point.line.0 == i as i32
            })
            .unwrap_or(0)
    });
    let marker = |l: usize| char::from_u32(0xf0000 + l as u32).unwrap_or(' ');
    let end = probe(term, |t| {
        for l in 0..lines {
            t.grid_mut()[Line(l as i32)][Column(0)].c = marker(l);
        }
        set_cursor(t, start, 0);
        t.insert_blank_lines(1);
        (start + 1..lines).find(|&k| t.grid()[Line(k as i32)][Column(0)].c == marker(k)).unwrap_or(lines)
    });
    start..end
}

fn tab_stops<T: EventListener>(term: &mut Term<T>) -> Vec<usize> {
    let cols = term.columns();
    probe(term, |t| {
        set_cursor(t, 0, 0);
        let mut stops = Vec::new();
        loop {
            let before = t.grid().cursor.point.column.0;
            t.put_tab(1);
            let after = t.grid().cursor.point.column.0;
            if after == before || after + 1 >= cols {
                break stops;
            }
            stops.push(after);
        }
    })
}

fn line_drawing<T: EventListener>(term: &mut Term<T>) -> bool {
    let charsets = &term.grid().cursor.charsets;
    let indexes = [CharsetIndex::G0, CharsetIndex::G1, CharsetIndex::G2, CharsetIndex::G3];
    if indexes.iter().all(|&i| charsets[i] == StandardCharset::Ascii) {
        return false;
    }
    probe(term, |t| {
        set_cursor(t, 0, 0);
        t.input('q');
        t.grid()[Line(0)][Column(0)].c != 'q'
    })
}

/// Runs `f` with the primary screen active, swapping to it and back when
/// the alternate screen is active. Swapping back resets the alternate grid,
/// so it is cloned (it has no scrollback) and put back.
fn with_primary<T, R>(term: &mut Term<T>, f: impl FnOnce(&Term<T>) -> R) -> R {
    if !term.mode().contains(TermMode::ALT_SCREEN) {
        return f(term);
    }
    let alt = term.grid().clone();
    let selection = term.selection.clone();
    let vi = term.vi_mode_cursor;
    term.swap_alt();
    let result = f(term);
    term.swap_alt();
    *term.grid_mut() = alt;
    term.selection = selection;
    term.vi_mode_cursor = vi;
    result
}

// ---- output -------------------------------------------------------------------

/// The attributes a written cell takes from the receiver's cursor template.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pen {
    fg: Color,
    bg: Color,
    flags: Flags,
    underline: Option<Color>,
    link: Option<Hyperlink>,
}

const STYLE: Flags = Flags::BOLD
    .union(Flags::DIM)
    .union(Flags::ITALIC)
    .union(Flags::ALL_UNDERLINES)
    .union(Flags::INVERSE)
    .union(Flags::HIDDEN)
    .union(Flags::STRIKEOUT);

/// Cells that [`write_grid`] never writes through its fast path.
const SPECIAL: Flags = Flags::WIDE_CHAR_SPACER.union(Flags::LEADING_WIDE_CHAR_SPACER);

impl Default for Pen {
    fn default() -> Self {
        Pen::of(&Cell::default())
    }
}

impl Pen {
    fn of(cell: &Cell) -> Self {
        Pen {
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags & STYLE,
            underline: cell.underline_color(),
            link: cell.hyperlink(),
        }
    }

    fn sgr(&self) -> String {
        let mut s = String::from("\x1b[0");
        for (flag, code) in [
            (Flags::BOLD, "1"),
            (Flags::DIM, "2"),
            (Flags::ITALIC, "3"),
            (Flags::UNDERLINE, "4"),
            (Flags::DOUBLE_UNDERLINE, "4:2"),
            (Flags::UNDERCURL, "4:3"),
            (Flags::DOTTED_UNDERLINE, "4:4"),
            (Flags::DASHED_UNDERLINE, "4:5"),
            (Flags::INVERSE, "7"),
            (Flags::HIDDEN, "8"),
            (Flags::STRIKEOUT, "9"),
        ] {
            if self.flags.contains(flag) {
                s.push(';');
                s.push_str(code);
            }
        }
        sgr_color(&mut s, self.fg, 30, 38);
        sgr_color(&mut s, self.bg, 40, 48);
        if let Some(color) = self.underline {
            sgr_color(&mut s, color, 0, 58);
        }
        s.push('m');
        s
    }
}

/// Appends `;<code>` for `color`: `base` + 0..8 for the eight named colors
/// (`base` + 60 for bright ones), `ext;5;n` or `ext;2;r;g;b` otherwise. The
/// default colors need nothing after `0`, and the other named colors (dim,
/// cursor) never come from SGR.
fn sgr_color(s: &mut String, color: Color, base: u8, ext: u8) {
    match color {
        Color::Named(named) => {
            let i = named as usize;
            if base != 0 && i < 8 {
                let _ = write!(s, ";{}", base as usize + i);
            } else if base != 0 && i < 16 {
                let _ = write!(s, ";{}", base as usize + 60 + i - 8);
            } else if base == 0 && i < 16 {
                let _ = write!(s, ";{ext};5;{i}");
            }
        }
        Color::Indexed(i) => {
            let _ = write!(s, ";{ext};5;{i}");
        }
        Color::Spec(rgb) => {
            let _ = write!(s, ";{ext};2;{};{};{}", rgb.r, rgb.g, rgb.b);
        }
    }
}

#[derive(Default)]
struct Out {
    buf: Vec<u8>,
    /// The receiver's cursor template, as far as the stream set it.
    pen: Pen,
    /// The receiver cursor's character sets.
    charsets: Charsets,
}

impl Out {
    fn raw(&mut self, s: &str) {
        self.buf.extend_from_slice(s.as_bytes());
    }

    fn pen(&mut self, pen: &Pen) {
        if (pen.fg, pen.bg, pen.flags, pen.underline) != (self.pen.fg, self.pen.bg, self.pen.flags, self.pen.underline)
        {
            let sgr = pen.sgr();
            self.raw(&sgr);
        }
        if pen.link != self.pen.link {
            match &pen.link {
                Some(link) => {
                    let s = format!("\x1b]8;id={};{}\x1b\\", link.id(), link.uri());
                    self.raw(&s);
                }
                None => self.raw("\x1b]8;;\x1b\\"),
            }
        }
        self.pen = pen.clone();
    }

    /// Takes the default colors without touching the link, before a line
    /// feed that may scroll: new rows take the template's background.
    fn plain_background(&mut self) {
        if self.pen.bg != Color::Named(NamedColor::Background) {
            let pen = Pen { bg: Color::Named(NamedColor::Background), ..self.pen.clone() };
            self.pen(&pen);
        }
    }

    fn charsets(&mut self, target: &Charsets) {
        for (index, intro) in [
            (CharsetIndex::G0, '('),
            (CharsetIndex::G1, ')'),
            (CharsetIndex::G2, '*'),
            (CharsetIndex::G3, '+'),
        ] {
            if self.charsets[index] != target[index] {
                let set = match target[index] {
                    StandardCharset::Ascii => 'B',
                    StandardCharset::SpecialCharacterAndLineDrawing => '0',
                };
                self.raw(&format!("\x1b{intro}{set}"));
                self.charsets[index] = target[index];
            }
        }
    }

    fn mark(&mut self, mark: Mark) {
        match mark {
            Mark::PromptStart => self.raw("\x1b]133;A\x1b\\"),
            Mark::CommandStart => self.raw("\x1b]133;B\x1b\\"),
            Mark::OutputStart => self.raw("\x1b]133;C\x1b\\"),
            Mark::CommandEnd(Some(exit)) => self.raw(&format!("\x1b]133;D;{exit}\x1b\\")),
            Mark::CommandEnd(None) => self.raw("\x1b]133;D\x1b\\"),
        }
    }

    /// Whether `cell` needs no pen change: it has no extras (underline
    /// color, hyperlink or zero-width characters), and its colors and style
    /// are the pen's, which has none either. The same as
    /// `Pen::of(cell) == self.pen`, without building a `Pen`.
    fn pen_fits(&self, cell: &Cell) -> bool {
        cell.extra.is_none()
            && self.pen.underline.is_none()
            && self.pen.link.is_none()
            && cell.fg == self.pen.fg
            && cell.bg == self.pen.bg
            && cell.flags & STYLE == self.pen.flags
    }

    fn char(&mut self, c: char) {
        if c.is_ascii() {
            self.buf.push(c as u8);
        } else {
            let mut utf8 = [0; 4];
            self.buf.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
        }
    }

    /// Writes a cell's text (its character and zero-width ones).
    fn text(&mut self, cell: &Cell) {
        let c = if cell.c == '\t' || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) { ' ' } else { cell.c };
        let mut utf8 = [0; 4];
        self.buf.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
        for &z in cell.zerowidth().unwrap_or_default() {
            self.buf.extend_from_slice(z.encode_utf8(&mut utf8).as_bytes());
        }
    }
}

/// Marks by grid line, each line's in column order (stable, so marks at
/// one position keep the order they were recorded in).
fn mark_positions(marks: &Marks) -> BTreeMap<i64, Vec<(usize, Mark)>> {
    let mut map: BTreeMap<i64, Vec<(usize, Mark)>> = BTreeMap::new();
    for c in marks.commands() {
        let points = [
            (Some(c.prompt), Mark::PromptStart),
            (c.input, Mark::CommandStart),
            (c.output, Mark::OutputStart),
            (c.end, Mark::CommandEnd(c.exit)),
        ];
        for (pos, mark) in points {
            if let Some(pos) = pos {
                map.entry(marks.line(pos)).or_default().push((pos.col, mark));
            }
        }
    }
    for line in map.values_mut() {
        line.sort_by_key(|&(col, _)| col);
    }
    map
}

fn is_blank(cell: &Cell) -> bool {
    cell.c == ' '
        && cell.fg == Color::Named(NamedColor::Foreground)
        && cell.bg == Color::Named(NamedColor::Background)
        && (cell.flags - Flags::WRAPLINE).is_empty()
        && cell.extra.is_none()
}

/// Writes every row of `grid`, top of the scrollback first, starting with
/// the receiver's cursor at the top left of a clear screen. The receiver
/// ends on the last row; `marks` are keyed by grid line.
fn write_grid(out: &mut Out, grid: &Grid<Cell>, marks: &BTreeMap<i64, Vec<(usize, Mark)>>) {
    let cols = grid.columns();
    let top = grid.topmost_line().0;
    let bottom = grid.bottommost_line().0;
    let mut prev_wrapped = false;
    for line in top..=bottom {
        let row = &grid[Line(line)];
        let wrapped = line != bottom && row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        let mut extent = (0..cols).rev().find(|&c| !is_blank(&row[Column(c)])).map_or(0, |c| c + 1);
        if wrapped {
            extent = cols;
        }
        if prev_wrapped {
            // Something must wrap into this row, or the flag above is lost.
            extent = extent.max(1);
        }
        let row_marks: &[(usize, Mark)] = marks.get(&(line as i64)).map_or(&[], Vec::as_slice);
        let mut next_mark = 0;
        // The receiver's cursor column; `cols` is a pending wrap.
        let mut at = 0;
        let mut colored_wrap = false;
        // Whether the receiver has already wrapped onto the next row, and
        // is at its start.
        let mut joined = false;
        for col in 0..extent {
            // Marks at the start of a wrapped-into row wait until its first
            // cell has wrapped (the next column, or after the row): before
            // that, the cursor is still on the row above.
            let deferred = col == 0 && prev_wrapped;
            if !deferred {
                while let Some(&(mark_col, mark)) = row_marks.get(next_mark).filter(|m| m.0 <= col) {
                    if at.min(cols - 1) != mark_col {
                        out.raw(&format!("\x1b[{}G", mark_col + 1));
                        at = mark_col;
                    }
                    out.mark(mark);
                    next_mark += 1;
                }
            }
            let cell = &row[Column(col)];
            // The common case, a cell in the current pen right where the
            // receiver's cursor is: its text alone, as the general path
            // below would write it.
            if at == col && !deferred && !cell.flags.intersects(SPECIAL) && cell.c != '\t' && out.pen_fits(cell) {
                out.char(cell.c);
                at = col + if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
                continue;
            }
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            if cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) && wrapped {
                if grid[Line(line + 1)][Column(0)].flags.contains(Flags::WIDE_CHAR) {
                    // Writing that wide char makes the spacer.
                    continue;
                }
                // Its wide char was erased (`CSI K` leaves the spacer):
                // write one to wrap, then erase the row it went to.
                if at != col {
                    out.raw(&format!("\x1b[{}G", col + 1));
                }
                out.pen(&Pen::of(cell));
                out.raw("中\r");
                out.plain_background();
                out.raw("\x1b[K");
                joined = true;
                continue;
            }
            if at != col && !deferred {
                out.raw(&format!("\x1b[{}G", col + 1));
            }
            let pen = Pen::of(cell);
            if deferred {
                colored_wrap = pen.bg != Color::Named(NamedColor::Background);
            }
            out.pen(&pen);
            if cell.c == '\t' && col + 1 < cols {
                // A tab only marks a blank cell it passes over.
                out.raw(" \x08\t");
                out.raw(&format!("\x1b[{}G", col + 2));
                at = col + 1;
            } else {
                out.text(cell);
                at = col + if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
            }
        }
        if colored_wrap && extent < cols {
            // The scroll that made this row filled it with the pen's
            // background.
            if at != extent {
                out.raw(&format!("\x1b[{}G", extent + 1));
                at = extent;
            }
            out.plain_background();
            out.raw("\x1b[K");
        }
        for &(mark_col, mark) in &row_marks[next_mark..] {
            if at.min(cols - 1) != mark_col {
                out.raw(&format!("\x1b[{}G", mark_col + 1));
                at = mark_col;
            }
            out.mark(mark);
        }
        if line != bottom && !wrapped {
            out.plain_background();
            out.raw("\r\n");
        }
        prev_wrapped = wrapped && !joined;
    }
}

/// Moves the receiver's cursor to `cursor`, with its pending wrap: the last
/// cell is written again, which leaves the cursor on it waiting to wrap.
/// With origin mode on, rows count from `row_base`, the region's top.
fn place_cursor(out: &mut Out, grid: &Grid<Cell>, cursor: &Cursor<Cell>, row_base: usize) {
    let line = (cursor.point.line.0 as usize).saturating_sub(row_base);
    let mut col = cursor.point.column.0;
    if cursor.input_needs_wrap {
        let row = &grid[cursor.point.line];
        if col > 0 && row[Column(col)].flags.contains(Flags::WIDE_CHAR_SPACER) {
            col -= 1;
        }
        out.raw(&format!("\x1b[{};{}H", line + 1, col + 1));
        let cell = &row[Column(col)];
        out.pen(&Pen::of(cell));
        out.text(cell);
    } else {
        out.raw(&format!("\x1b[{};{}H", line + 1, col + 1));
    }
}

/// A `file://` URI's host-less part for a reported directory: Windows paths
/// (`C:\src`) become `/C:/src`; bytes outside the unreserved set and `/`
/// are percent-encoded.
fn dir_uri(dir: &str) -> String {
    let path = match dir.as_bytes() {
        [drive, b':', ..] if drive.is_ascii_alphabetic() => format!("/{}", dir.replace('\\', "/")),
        _ => dir.to_string(),
    };
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests;
