//! Shell integration: OSC 133 semantic prompt marks.
//!
//! Shells that load one of the scripts in `shell-integration/` emit
//! `ESC ]133;A` (prompt start), `B` (command start), `C` (output start) and
//! `D;<exit>` (command finished). vte drops unknown OSCs, so [`Parser`]
//! scans the byte stream itself, splits it at each mark, and records the
//! mark at the cursor position the output before it produced.
//!
//! Marks are stored as absolute line numbers (`base + Line`). `base` counts
//! every line that scrolled off the top of the live screen. It is kept up
//! to date by noting which row sat at `Line(0)` before each processor call
//! and finding it again afterwards (alacritty rotates rows, it never copies
//! their cells), which still works when the history is full and recycling
//! rows. Resizes remap marks through logical (unwrapped) lines; see
//! `openspec/changes/archive/*-add-shell-integration/design.md`.

use std::collections::VecDeque;
use std::time::Instant;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use parking_lot::Mutex;

/// One OSC 133 mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    PromptStart,
    CommandStart,
    OutputStart,
    /// Command finished, with its exit status when the shell gave one.
    CommandEnd(Option<i32>),
}

/// A point in the stream where the parser must stop and look at the term.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Split {
    /// After the OSC terminator.
    Mark(Mark),
    /// Before the final byte of `CSI ? 1049|1047|47 h/l`.
    AltScreen,
    /// Before the final byte of `CSI 3 J`.
    ClearHistory,
    /// After `ESC c`.
    Reset,
}

impl Split {
    /// Whether the stream is split before the byte that completed the
    /// sequence (which is then fed on its own) rather than after it.
    fn before(self) -> bool {
        matches!(self, Split::AltScreen | Split::ClearHistory)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Ground,
    Escape,
    EscapeIntermediate,
    Csi,
    Osc,
    /// DCS, SOS, PM or APC: ignored until ESC.
    String,
}

const OSC_KEEP: usize = 64;
const CSI_KEEP: usize = 32;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// A byte-level state machine that follows vte's closely enough to find
/// OSC 133 marks and the few other sequences that move marks around. Its
/// state carries across calls, so sequences may be split between reads.
#[derive(Debug, Default)]
pub struct Scanner {
    state: State,
    buf: Vec<u8>,
    overflow: bool,
    intermediate: bool,
}

impl Scanner {
    /// Scans `bytes` up to the first split point. Returns the index of the
    /// byte that completed it and what was found, or `None` when the whole
    /// slice was consumed without one.
    fn scan(&mut self, bytes: &[u8]) -> Option<(usize, Split)> {
        let mut i = 0;
        while i < bytes.len() {
            if self.state == State::Ground {
                // Fast path: nothing interesting happens until an ESC.
                match bytes[i..].iter().position(|&b| b == ESC) {
                    Some(p) => i += p,
                    None => return None,
                }
            }
            if let Some(split) = self.byte(bytes[i]) {
                return Some((i, split));
            }
            i += 1;
        }
        None
    }

    fn reset_buf(&mut self) {
        self.buf.clear();
        self.overflow = false;
        self.intermediate = false;
    }

    fn keep(&mut self, b: u8, limit: usize) {
        if self.buf.len() < limit {
            self.buf.push(b);
        } else {
            self.overflow = true;
        }
    }

    fn byte(&mut self, b: u8) -> Option<Split> {
        if b == CAN || b == SUB {
            self.state = State::Ground;
            return None;
        }
        match self.state {
            State::Ground => {
                if b == ESC {
                    self.state = State::Escape;
                }
                None
            }
            State::Escape => {
                self.state = match b {
                    ESC => State::Escape,
                    b'[' => {
                        self.reset_buf();
                        State::Csi
                    }
                    b']' => {
                        self.reset_buf();
                        State::Osc
                    }
                    b'P' | b'X' | b'^' | b'_' => State::String,
                    b'c' => {
                        self.state = State::Ground;
                        return Some(Split::Reset);
                    }
                    0x20..=0x2f => State::EscapeIntermediate,
                    // C0 controls are executed without leaving the sequence.
                    0x00..=0x1f => State::Escape,
                    _ => State::Ground,
                };
                None
            }
            State::EscapeIntermediate => {
                self.state = match b {
                    ESC => State::Escape,
                    0x20..=0x2f | 0x00..=0x1f => State::EscapeIntermediate,
                    _ => State::Ground,
                };
                None
            }
            State::Csi => match b {
                ESC => {
                    self.state = State::Escape;
                    None
                }
                0x30..=0x3f => {
                    self.keep(b, CSI_KEEP);
                    None
                }
                0x20..=0x2f => {
                    self.intermediate = true;
                    None
                }
                0x40..=0x7e => {
                    self.state = State::Ground;
                    self.csi_split(b)
                }
                _ => None,
            },
            State::Osc => match b {
                BEL => {
                    self.state = State::Ground;
                    self.osc_split()
                }
                // vte dispatches the OSC when ESC arrives; the `\` of ST is
                // then an ordinary escape sequence.
                ESC => {
                    self.state = State::Escape;
                    self.osc_split()
                }
                0x00..=0x1f => None,
                _ => {
                    self.keep(b, OSC_KEEP);
                    None
                }
            },
            State::String => {
                if b == ESC {
                    self.state = State::Escape;
                }
                None
            }
        }
    }

    fn csi_split(&self, final_byte: u8) -> Option<Split> {
        if self.intermediate || self.overflow {
            return None;
        }
        match final_byte {
            b'J' if self.buf == b"3" => Some(Split::ClearHistory),
            b'h' | b'l' => {
                let params = self.buf.strip_prefix(b"?")?;
                params
                    .split(|&b| b == b';')
                    .any(|p| matches!(p, b"1049" | b"1047" | b"47"))
                    .then_some(Split::AltScreen)
            }
            _ => None,
        }
    }

    fn osc_split(&self) -> Option<Split> {
        parse_osc133(&self.buf).map(Split::Mark)
    }
}

/// Parses the body of an OSC (`133;D;1`) into a mark.
fn parse_osc133(body: &[u8]) -> Option<Mark> {
    let mut fields = body.split(|&b| b == b';');
    if fields.next()? != b"133" {
        return None;
    }
    let mark = match fields.next()? {
        b"A" => Mark::PromptStart,
        b"B" => Mark::CommandStart,
        b"C" => Mark::OutputStart,
        b"D" => Mark::CommandEnd(
            fields.next().and_then(|f| std::str::from_utf8(f).ok()).and_then(|s| s.trim().parse().ok()),
        ),
        _ => return None,
    };
    Some(mark)
}

// ---- marks ------------------------------------------------------------------

/// A position as an absolute line (see the module docs) and a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub line: i64,
    pub col: usize,
}

/// One prompt and, once the user runs something, its command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub prompt: Pos,
    pub input: Option<Pos>,
    pub output: Option<Pos>,
    pub end: Option<Pos>,
    pub exit: Option<i32>,
}

impl Command {
    fn new(prompt: Pos) -> Self {
        Self { prompt, input: None, output: None, end: None, exit: None }
    }

    /// The last line any of its marks is on.
    fn last_line(&self) -> i64 {
        [Some(self.prompt), self.input, self.output, self.end].into_iter().flatten().map(|p| p.line).max().unwrap_or(0)
    }

    pub fn failed(&self) -> bool {
        self.exit.is_some_and(|e| e != 0)
    }

    fn positions_mut(&mut self) -> [Option<&mut Pos>; 4] {
        [Some(&mut self.prompt), self.input.as_mut(), self.output.as_mut(), self.end.as_mut()]
    }
}

/// Most commands kept per terminal.
const MAX_COMMANDS: usize = 10_000;

/// A row's identity: the address of its cells. Only compared, never read.
type RowId = *const Cell;

fn row_id<T>(term: &Term<T>, line: i32) -> RowId {
    &term.grid()[Line(line)][Column(0)] as *const Cell
}

/// Where things were before a processor call.
struct Probe {
    alt: bool,
    top: RowId,
    cursor_line: i32,
    cursor: RowId,
}

impl Probe {
    fn new<T>(term: &Term<T>) -> Self {
        let cursor_line = term.grid().cursor.point.line.0;
        Probe {
            alt: term.mode().contains(TermMode::ALT_SCREEN),
            top: row_id(term, 0),
            cursor_line,
            cursor: row_id(term, cursor_line),
        }
    }
}

/// A mark's place in the text, independent of line wrapping: how many
/// logical lines above the cursor's it is, and its cell offset within it.
#[derive(Debug, Clone, Copy)]
struct Logical {
    above: i64,
    offset: usize,
}

/// Logical line index and row-within-logical-line for every grid line,
/// from the top of the history down.
struct LogicalMap {
    top: i32,
    rows: Vec<(i64, usize)>,
    /// First grid line of each logical line.
    starts: Vec<i32>,
    cols: usize,
}

impl LogicalMap {
    fn new<T>(term: &Term<T>) -> Self {
        let grid = term.grid();
        let top = grid.topmost_line().0;
        let bottom = grid.bottommost_line().0;
        let cols = grid.columns();
        let mut rows = Vec::with_capacity((bottom - top + 1) as usize);
        let mut starts = Vec::new();
        let mut wrapped = false;
        let mut index = -1i64;
        let mut row_in = 0;
        for line in top..=bottom {
            if wrapped {
                row_in += 1;
            } else {
                index += 1;
                row_in = 0;
                starts.push(line);
            }
            rows.push((index, row_in));
            wrapped = grid[Line(line)][Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        }
        LogicalMap { top, rows, starts, cols }
    }

    fn get(&self, line: i32) -> Option<(i64, usize)> {
        let i = usize::try_from(line - self.top).ok()?;
        self.rows.get(i).copied()
    }
}

/// The marks recorded for one terminal.
#[derive(Debug, Default)]
pub struct Marks {
    commands: VecDeque<Command>,
    /// Lines scrolled off the top of the live screen so far.
    base: i64,
    /// Logical positions taken on entering the alternate screen, restored
    /// on leaving it if the window was resized meanwhile.
    alt_snapshot: Option<Vec<Option<Logical>>>,
    alt_resized: bool,
    /// Absolute line of the top of the history, as of the last prune.
    top: i64,
}

impl Marks {
    #[cfg(test)]
    pub fn commands(&self) -> &VecDeque<Command> {
        &self.commands
    }

    /// A position's grid line (`Line` value; negative is history).
    pub fn line(&self, pos: Pos) -> i64 {
        pos.line - self.base
    }

    fn pos<T>(&self, term: &Term<T>) -> Pos {
        let p = term.grid().cursor.point;
        Pos { line: self.base + p.line.0 as i64, col: p.column.0 }
    }

    pub fn clear(&mut self) {
        self.commands.clear();
        self.alt_snapshot = None;
    }

    /// Records `mark` at the cursor.
    pub fn record<T>(&mut self, term: &Term<T>, mark: Mark) {
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        let pos = self.pos(term);
        let last = self.commands.back_mut();
        match mark {
            Mark::PromptStart => match last {
                // The same prompt drawn again (a redraw, or a second
                // integration emitting its own marks).
                Some(c) if c.output.is_none() && c.prompt.line == pos.line => {
                    c.prompt = pos;
                    c.input = None;
                }
                _ => self.push(Command::new(pos)),
            },
            Mark::CommandStart => match last {
                Some(c) if c.output.is_none() => c.input = Some(pos),
                _ => self.push(Command { input: Some(pos), ..Command::new(pos) }),
            },
            Mark::OutputStart => {
                if let Some(c) = last
                    && c.output.is_none()
                    && c.end.is_none()
                {
                    c.output = Some(pos);
                }
            }
            Mark::CommandEnd(exit) => {
                if let Some(c) = last
                    && c.output.is_some()
                    && c.end.is_none()
                {
                    c.end = Some(pos);
                    c.exit = exit;
                }
            }
        }
    }

    fn push(&mut self, command: Command) {
        if self.commands.len() >= MAX_COMMANDS {
            self.commands.pop_front();
        }
        self.commands.push_back(command);
    }

    /// Accounts for whatever a processor call did to the primary grid.
    fn track<T>(&mut self, term: &Term<T>, probe: Probe) {
        let alt = term.mode().contains(TermMode::ALT_SCREEN);
        if probe.alt || alt {
            // The primary grid doesn't scroll while it's hidden.
            if probe.alt && !alt {
                let snapshot = self.alt_snapshot.take();
                if std::mem::take(&mut self.alt_resized)
                    && let Some(snapshot) = snapshot
                {
                    self.restore(term, &snapshot);
                }
            }
            return;
        }
        let grid = term.grid();
        let top = grid.topmost_line().0;
        let bottom = grid.bottommost_line().0;
        let find = |id: RowId, mut lines: Box<dyn Iterator<Item = i32>>| lines.find(|&l| row_id(term, l) == id);
        let moved_up = if let Some(line) = find(probe.top, Box::new((top..=0).rev())) {
            Some(-(line as i64))
        } else if let Some(line) = find(probe.cursor, Box::new((top..=probe.cursor_line.min(bottom)).rev())) {
            Some((probe.cursor_line - line) as i64)
        } else {
            find(probe.top, Box::new(1..=bottom)).map(|line| -(line as i64))
        };
        match moved_up {
            Some(lines) => {
                self.base += lines;
                self.prune(term);
            }
            // Everything that was on screen has gone (more than the whole
            // history scrolled, or it was cleared): so have all marks.
            None => self.clear(),
        }
    }

    /// Drops commands whose lines are no longer in the grid.
    pub fn prune<T>(&mut self, term: &Term<T>) {
        let grid = term.grid();
        let top = self.base + grid.topmost_line().0 as i64;
        let bottom = self.base + grid.bottommost_line().0 as i64;
        self.top = top;
        while self.commands.front().is_some_and(|c| c.last_line() < top) {
            self.commands.pop_front();
        }
        while self.commands.back().is_some_and(|c| c.prompt.line > bottom) {
            self.commands.pop_back();
        }
    }

    fn snapshot<T>(&self, term: &Term<T>) -> Vec<Option<Logical>> {
        if self.commands.is_empty() {
            return Vec::new();
        }
        let map = LogicalMap::new(term);
        let Some((cursor_index, _)) = map.get(term.grid().cursor.point.line.0) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(self.commands.len() * 4);
        for c in &self.commands {
            for pos in [Some(c.prompt), c.input, c.output, c.end] {
                out.push(pos.and_then(|pos| {
                    let (index, row_in) = map.get(self.line(pos) as i32)?;
                    Some(Logical { above: cursor_index - index, offset: row_in * map.cols + pos.col })
                }));
            }
        }
        out
    }

    fn restore<T>(&mut self, term: &Term<T>, snapshot: &[Option<Logical>]) {
        if snapshot.len() != self.commands.len() * 4 {
            self.clear();
            return;
        }
        let map = LogicalMap::new(term);
        let Some((cursor_index, _)) = map.get(term.grid().cursor.point.line.0) else {
            self.clear();
            return;
        };
        let base = self.base;
        let place = |logical: Logical| -> Option<Pos> {
            let index = usize::try_from(cursor_index - logical.above).ok()?;
            let start = *map.starts.get(index)?;
            let end = map.starts.get(index + 1).map(|s| s - 1).unwrap_or(map.top + map.rows.len() as i32 - 1);
            let line = (start + (logical.offset / map.cols) as i32).min(end);
            Some(Pos { line: base + line as i64, col: (logical.offset % map.cols).min(map.cols - 1) })
        };
        let mut kept = VecDeque::with_capacity(self.commands.len());
        for (mut c, chunk) in self.commands.drain(..).zip(snapshot.chunks(4)) {
            let Some(prompt) = chunk[0].and_then(place) else { continue };
            c.prompt = prompt;
            let mut lost = false;
            for (slot, logical) in c.positions_mut().into_iter().zip(chunk).skip(1) {
                if let Some(slot) = slot {
                    match logical.and_then(place) {
                        Some(pos) => *slot = pos,
                        None => lost = true,
                    }
                }
            }
            if !lost {
                kept.push_back(c);
            }
        }
        self.commands = kept;
        self.prune(term);
    }

    /// Resizes `term`, carrying the marks along with the reflowed text.
    pub fn resize<T, S: Dimensions>(&mut self, term: &mut Term<T>, size: S) {
        if term.mode().contains(TermMode::ALT_SCREEN) {
            term.resize(size);
            self.alt_resized = true;
            return;
        }
        let snapshot = self.snapshot(term);
        term.resize(size);
        if !self.commands.is_empty() {
            self.restore(term, &snapshot);
        }
    }

    // ---- queries ----------------------------------------------------------------

    /// Grid lines of prompts still in the grid, oldest first.
    pub fn prompt_lines(&self) -> impl DoubleEndedIterator<Item = i64> + '_ {
        self.commands.iter().filter(|c| c.prompt.line >= self.top).map(|c| self.line(c.prompt))
    }

    /// The prompt line to bring to the top of the view when jumping from a
    /// view whose top line is `top`.
    pub fn jump_target(&self, top: i64, previous: bool) -> Option<i64> {
        if previous {
            self.prompt_lines().rev().find(|&l| l < top)
        } else {
            self.prompt_lines().find(|&l| l > top)
        }
    }

    /// Grid points spanning the most recent command output, inclusive, or
    /// `None` when there is none or it is empty.
    pub fn last_output<T>(&self, term: &Term<T>) -> Option<(Point, Point)> {
        let index = self.commands.iter().rposition(|c| c.output.is_some())?;
        let command = &self.commands[index];
        let start = command.output?;
        let end = command
            .end
            .or_else(|| self.commands.get(index + 1).map(|c| c.prompt))
            .unwrap_or_else(|| self.pos(term));
        let grid = term.grid();
        let top = grid.topmost_line().0 as i64;
        let last_col = grid.columns() - 1;
        let (mut start_line, mut start_col) = (self.line(start), start.col);
        if start_line < top {
            (start_line, start_col) = (top, 0);
        }
        // The end mark is exclusive.
        let (end_line, end_col) = if end.col == 0 {
            (self.line(end) - 1, last_col)
        } else {
            (self.line(end), end.col - 1)
        };
        if end_line < start_line || (end_line == start_line && end_col < start_col) {
            return None;
        }
        Some((
            Point::new(Line(start_line as i32), Column(start_col)),
            Point::new(Line(end_line as i32), Column(end_col.min(last_col))),
        ))
    }

    /// Line ranges (inclusive, grid lines) to mark as failed, for commands
    /// overlapping lines `top..=bottom`.
    pub fn failed_spans(&self, top: i64, bottom: i64) -> Vec<(i64, i64)> {
        let mut spans = Vec::new();
        for c in self.commands.iter().rev() {
            let first = self.line(c.prompt);
            let last = c.input.map(|p| self.line(p)).unwrap_or(first).max(first);
            if last < top {
                break;
            }
            if c.failed() && first <= bottom {
                spans.push((first.max(self.line(Pos { line: self.top, col: 0 })), last));
            }
        }
        spans
    }
}

// ---- parser -----------------------------------------------------------------

/// The vte processor plus the OSC 133 scanner, fed through [`Parser::advance`].
#[derive(Default)]
pub struct Parser {
    processor: Processor<StdSyncHandler>,
    scanner: Scanner,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    /// When a synchronized update (mode 2026) that is buffering output
    /// must be flushed, if one is.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.processor.sync_timeout().sync_timeout()
    }

    /// Runs `bytes` through the processor into `term`, recording marks.
    pub fn advance<T: EventListener>(&mut self, term: &mut Term<T>, marks: &Mutex<Marks>, bytes: &[u8]) {
        let mut fed = 0;
        let mut scanned = 0;
        while scanned < bytes.len() {
            let Some((offset, split)) = self.scanner.scan(&bytes[scanned..]) else { break };
            let at = scanned + offset;
            scanned = at + 1;
            if split.before() {
                self.feed(term, marks, &bytes[fed..at]);
                self.end_sync(term, marks);
                if split == Split::AltScreen && !term.mode().contains(TermMode::ALT_SCREEN) {
                    let mut marks = marks.lock();
                    marks.alt_snapshot = Some(marks.snapshot(term));
                    marks.alt_resized = false;
                }
                self.feed(term, marks, &bytes[at..=at]);
            } else {
                self.feed(term, marks, &bytes[fed..=at]);
                match split {
                    Split::Mark(mark) => {
                        self.end_sync(term, marks);
                        marks.lock().record(term, mark);
                    }
                    Split::Reset => marks.lock().clear(),
                    Split::AltScreen | Split::ClearHistory => {}
                }
            }
            fed = scanned;
        }
        self.feed(term, marks, &bytes[fed..]);
    }

    /// Applies a buffered synchronized update now (its timeout passed).
    pub fn stop_sync<T: EventListener>(&mut self, term: &mut Term<T>, marks: &Mutex<Marks>) {
        let probe = Probe::new(term);
        self.processor.stop_sync(term);
        marks.lock().track(term, probe);
    }

    /// Ends a synchronized update early so the cursor is current.
    fn end_sync<T: EventListener>(&mut self, term: &mut Term<T>, marks: &Mutex<Marks>) {
        if self.sync_deadline().is_some() {
            self.stop_sync(term, marks);
        }
    }

    fn feed<T: EventListener>(&mut self, term: &mut Term<T>, marks: &Mutex<Marks>, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let probe = Probe::new(term);
        self.processor.advance(term, bytes);
        marks.lock().track(term, probe);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::GridSize;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term;

    fn scan_all(input: &[&[u8]]) -> Vec<Split> {
        let mut scanner = Scanner::default();
        let mut found = Vec::new();
        for chunk in input {
            let mut rest: &[u8] = chunk;
            while let Some((i, split)) = scanner.scan(rest) {
                found.push(split);
                rest = &rest[i + 1..];
            }
        }
        found
    }

    #[test]
    fn scanner_finds_each_kind() {
        let marks = scan_all(&[b"\x1b]133;A\x07$ \x1b]133;B\x1b\\ls\r\n\x1b]133;C\x07out\x1b]133;D;2\x07"]);
        assert_eq!(
            marks,
            [
                Split::Mark(Mark::PromptStart),
                Split::Mark(Mark::CommandStart),
                Split::Mark(Mark::OutputStart),
                Split::Mark(Mark::CommandEnd(Some(2)))
            ]
        );
    }

    #[test]
    fn exit_status_parsing() {
        assert_eq!(parse_osc133(b"133;D"), Some(Mark::CommandEnd(None)));
        assert_eq!(parse_osc133(b"133;D;x"), Some(Mark::CommandEnd(None)));
        assert_eq!(parse_osc133(b"133;D;-1"), Some(Mark::CommandEnd(Some(-1))));
        assert_eq!(parse_osc133(b"133;D;130;aid=7"), Some(Mark::CommandEnd(Some(130))));
        assert_eq!(parse_osc133(b"133;A;click_events=1"), Some(Mark::PromptStart));
        assert_eq!(parse_osc133(b"133;P;k=i"), None);
        assert_eq!(parse_osc133(b"1337;A"), None);
        assert_eq!(parse_osc133(b"2;133;A"), None);
    }

    #[test]
    fn scanner_ignores_lookalikes() {
        // Inside a DCS string, in a title, after CAN, and an over-long OSC
        // whose prefix still parses.
        assert!(scan_all(&[b"\x1bPq\x1b]133;A\x07"]).len() == 1, "ESC ends the DCS, then the OSC is real");
        assert!(scan_all(&[b"\x1b]2;133;A\x07"]).is_empty());
        assert!(scan_all(&[b"\x1b]133\x18;A\x07"]).is_empty());
        assert!(scan_all(&[b"\x1b_133;A\x07\x1b\\"]).is_empty(), "APC is not an OSC");
        let long = [b"\x1b]133;D;1;".as_slice(), &[b'x'; 200], b"\x07"].concat();
        assert_eq!(scan_all(&[&long]), [Split::Mark(Mark::CommandEnd(Some(1)))]);
    }

    #[test]
    fn scanner_finds_screen_sequences() {
        assert_eq!(scan_all(&[b"\x1b[?1049h\x1b[?25;1049l\x1b[?2026h"]), [Split::AltScreen, Split::AltScreen]);
        assert_eq!(scan_all(&[b"\x1b[2J\x1b[3J\x1b[J"]), [Split::ClearHistory]);
        assert_eq!(scan_all(&[b"\x1bc\x1b(c"]), [Split::Reset]);
    }

    #[test]
    fn scanner_split_anywhere() {
        let input = b"x\x1b]133;D;42\x1b\\y";
        for cut in 0..=input.len() {
            let (a, b) = input.split_at(cut);
            assert_eq!(scan_all(&[a, b]), [Split::Mark(Mark::CommandEnd(Some(42)))], "cut at {cut}");
        }
    }

    // ---- against a real Term ------------------------------------------------------

    struct Harness {
        term: Term<VoidListener>,
        parser: Parser,
        marks: Mutex<Marks>,
    }

    impl Harness {
        fn new(cols: u16, rows: u16, history: usize) -> Self {
            let size = GridSize { cols, rows, ..Default::default() };
            let config = term::Config { scrolling_history: history, ..Default::default() };
            Harness { term: Term::new(config, &size, VoidListener), parser: Parser::new(), marks: Mutex::default() }
        }

        fn feed(&mut self, bytes: &[u8]) -> &mut Self {
            self.parser.advance(&mut self.term, &self.marks, bytes);
            self
        }

        /// A full command: prompt, `cmd`, then `lines` lines of output.
        fn command(&mut self, cmd: &str, lines: usize, exit: i32) -> &mut Self {
            let mut out = format!("\x1b]133;A\x07$ \x1b]133;B\x07{cmd}\r\n\x1b]133;C\x07");
            for i in 0..lines {
                out.push_str(&format!("{cmd}-{i}\r\n"));
            }
            out.push_str(&format!("\x1b]133;D;{exit}\x07"));
            self.feed(out.as_bytes())
        }

        fn text(&self, line: i64) -> String {
            let grid = self.term.grid();
            let row = &grid[Line(line as i32)];
            let s: String = (0..grid.columns()).map(|c| row[Column(c)].c).collect();
            s.trim_end().to_string()
        }

        /// The text on each prompt's line.
        fn prompts(&self) -> Vec<String> {
            let marks = self.marks.lock();
            marks.prompt_lines().map(|l| self.text(l)).collect()
        }

        fn resize(&mut self, cols: u16, rows: u16) {
            let size = GridSize { cols, rows, ..Default::default() };
            self.marks.lock().resize(&mut self.term, size);
        }
    }

    #[test]
    fn marks_follow_scrolling() {
        let mut h = Harness::new(20, 5, 100);
        h.command("one", 3, 0).command("two", 30, 1).feed(b"\x1b]133;A\x07$ ");
        assert_eq!(h.prompts(), ["$ one", "$ two", "$"]);
        let marks = h.marks.lock();
        let two = &marks.commands()[1];
        assert!(two.failed());
        assert_eq!(h.text(marks.line(two.output.unwrap())), "two-0");
    }

    #[test]
    fn marks_survive_full_history() {
        for history in [0, 5, 50] {
            let mut h = Harness::new(20, 6, history);
            for i in 0..20 {
                h.command(&format!("c{i}"), 2, 0);
            }
            h.feed(b"\x1b]133;A\x07$ ");
            let prompts = h.prompts();
            assert!(prompts.len() >= 2, "history {history}: {prompts:?}");
            // Whatever is kept is exactly the latest prompts, in order.
            let expected: Vec<String> =
                (20 - (prompts.len() - 1)..20).map(|i| format!("$ c{i}")).chain(["$".to_string()]).collect();
            assert_eq!(prompts, expected, "history {history}");
        }
    }

    #[test]
    fn marks_split_across_reads() {
        let input = b"ab\r\ncd\r\n\x1b]133;A\x07$ x";
        for cut in 0..=input.len() {
            let mut h = Harness::new(10, 4, 10);
            let (a, b) = input.split_at(cut);
            h.feed(a).feed(b);
            assert_eq!(h.prompts(), ["$ x"], "cut at {cut}");
            assert_eq!(h.marks.lock().commands()[0].prompt.col, 0);
        }
    }

    #[test]
    fn clear_and_reset_forget_marks() {
        let mut h = Harness::new(20, 5, 100);
        h.command("one", 10, 0).command("two", 1, 0);
        h.feed(b"\x1b[H\x1b[2J\x1b[3J").command("three", 0, 0);
        assert_eq!(h.prompts(), ["$ three"]);
        // CSI 3 J on its own keeps what's on screen.
        h.feed(b"\x1b[3J");
        assert_eq!(h.prompts(), ["$ three"]);
        h.feed(b"\x1bc");
        assert!(h.prompts().is_empty());
    }

    #[test]
    fn clear_scrollback_prunes() {
        let mut h = Harness::new(20, 5, 100);
        h.command("one", 10, 0).command("two", 1, 0);
        h.term.grid_mut().clear_history();
        h.marks.lock().prune(&h.term);
        assert_eq!(h.prompts(), ["$ two"]);
    }

    #[test]
    fn grouping_rules() {
        let mut h = Harness::new(20, 5, 100);
        // Enter on an empty line: D without C is ignored.
        h.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07\r\n\x1b]133;D;1\x07");
        assert!(!h.marks.lock().commands()[0].failed());
        // A prompt redrawn on the same line replaces the mark.
        h.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07\r\x1b]133;A\x07$ \x1b]133;B\x07");
        assert_eq!(h.marks.lock().commands().len(), 2);
        h.feed(b"false\r\n\x1b]133;C\x07\x1b]133;D;1\x07\x1b]133;D;0\x07");
        let marks = h.marks.lock();
        assert_eq!(marks.commands()[1].exit, Some(1), "a second D is ignored");
    }

    #[test]
    fn mark_inside_synchronized_update() {
        let mut h = Harness::new(20, 5, 100);
        h.feed(b"\x1b[?2026hline\r\n\x1b]133;A\x07$ y\x1b[?2026l");
        assert_eq!(h.prompts(), ["$ y"]);
    }

    #[test]
    fn marks_ignored_on_alternate_screen() {
        let mut h = Harness::new(20, 5, 100);
        h.command("vim", 0, 0).feed(b"\x1b[?1049h\x1b]133;A\x07x\x1b[?1049l");
        assert_eq!(h.prompts(), ["$ vim"]);
    }

    #[test]
    fn resize_reflows_marks() {
        let mut h = Harness::new(10, 6, 100);
        h.command("aaaaaaaaaaaaaaa", 2, 1).command("b", 1, 0).feed(b"\x1b]133;A\x07$ ");
        let before = h.prompts();
        assert_eq!(before, ["$ aaaaaaaa", "$ b", "$"]);
        h.resize(6, 6);
        assert_eq!(h.prompts(), ["$ aaaa", "$ b", "$"]);
        h.resize(30, 6);
        assert_eq!(h.prompts(), ["$ aaaaaaaaaaaaaaa", "$ b", "$"]);
        let marks = h.marks.lock();
        let first = &marks.commands()[0];
        assert_eq!(h.text(marks.line(first.output.unwrap())), "aaaaaaaaaaaaaaa-0");
    }

    #[test]
    fn resize_on_alternate_screen() {
        let mut h = Harness::new(10, 6, 100);
        h.command("aaaaaaaaaaaaaaa", 2, 1).command("b", 1, 0).feed(b"\x1b]133;A\x07$ ");
        h.feed(b"\x1b[?1049hfullscreen");
        h.resize(6, 8);
        h.feed(b"\x1b[?1049l");
        assert_eq!(h.prompts(), ["$ aaaa", "$ b", "$"]);
    }

    #[test]
    fn reverse_index_at_top() {
        let mut h = Harness::new(20, 5, 100);
        h.command("one", 0, 0).feed(b"\x1b]133;A\x07$ two");
        // Scroll the screen down by two (reverse index at the top).
        h.feed(b"\x1b[H\x1bM\x1bM");
        assert_eq!(h.prompts(), ["$ one", "$ two"]);
    }

    #[test]
    fn queries() {
        let mut h = Harness::new(20, 5, 100);
        h.command("one", 10, 1).feed(b"\x1b]133;A\x07$ \x1b]133;B\x07printf\r\n\x1b]133;C\x07a\r\nb\r\n\x1b]133;D;0\x07");
        h.feed(b"\x1b]133;A\x07$ ");
        let marks = h.marks.lock();
        let lines: Vec<i64> = marks.prompt_lines().collect();
        // Five rows: the last two prompts are on screen, the first in history.
        assert!(lines[0] < 0 && lines[1] > 0, "{lines:?}");
        assert_eq!(marks.jump_target(0, true), Some(lines[0]));
        assert_eq!(marks.jump_target(lines[2] + 1, true), Some(lines[2]));
        assert_eq!(marks.jump_target(lines[2], true), Some(lines[1]));
        assert_eq!(marks.jump_target(lines[0], true), None);
        assert_eq!(marks.jump_target(lines[0], false), Some(lines[1]));
        assert_eq!(marks.jump_target(lines[2], false), None);
        let (start, end) = marks.last_output(&h.term).unwrap();
        assert_eq!(h.term.bounds_to_string(start, end), "a\nb");
        let spans = marks.failed_spans(lines[0], lines[0] + 4);
        assert_eq!(spans, [(lines[0], lines[0])]);
        assert!(marks.failed_spans(lines[1], lines[2]).is_empty());
    }

    #[test]
    fn empty_output_selects_nothing() {
        let mut h = Harness::new(20, 5, 100);
        h.command("true", 0, 0).feed(b"\x1b]133;A\x07$ ");
        assert!(h.marks.lock().last_output(&h.term).is_none());
    }
}
