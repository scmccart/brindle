//! Scrollback search on top of alacritty's regex search: building the
//! pattern from the find bar's query and options, stepping between matches,
//! the matches in view, counting, and scrolling a match into view.
//!
//! Matches are numbered and stepped from the newest output (the bottom) up,
//! because a terminal's history is searched backwards.

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

/// How far past the view a match may start and still reach into it, along
/// a wrapped line.
const MAX_EXTRA_LINES: i32 = 100;

/// How far up from the origin a search looks while the query is being
/// typed. A full-buffer scan for a query with no match takes tens of
/// milliseconds with 10,000 lines of history, too slow for every keystroke;
/// the debounced count does the full search afterwards (as alacritty does).
pub const MAX_LINES_WHILE_TYPING: usize = 1000;

/// Counting stops after this many matches.
pub const COUNT_CAP: usize = 1000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub regex: bool,
}

/// Escapes regex metacharacters so `text` matches literally.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The regex for a query, or `None` for an empty query (which matches
/// nothing). The inline flag overrides alacritty's smart case.
pub fn pattern(query: &str, options: SearchOptions) -> Option<String> {
    if query.is_empty() {
        return None;
    }
    let body = if options.regex { query.to_string() } else { escape(query) };
    let flag = if options.case_sensitive { "(?-i)" } else { "(?i)" };
    Some(format!("{flag}{body}"))
}

/// Compiles a query: `None` when empty, `Err` when it isn't a valid regex.
pub fn compile(query: &str, options: SearchOptions) -> Option<Result<RegexSearch, String>> {
    let pattern = pattern(query, options)?;
    Some(RegexSearch::new(&pattern).map_err(|err| err.to_string()))
}

/// Which way to step from the current match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Towards older output.
    Up,
    /// Towards newer output.
    Down,
}

/// The nearest match starting at or above `origin`, wrapping to the bottom.
/// With `max_lines`, only that many lines up from `origin` are searched,
/// with no wrapping.
pub fn find_from<T>(term: &Term<T>, regex: &mut RegexSearch, origin: Point, max_lines: Option<usize>) -> Option<Match> {
    term.search_next(regex, origin, Direction::Left, Side::Left, max_lines)
}

/// The next match after `current` in direction `step`, wrapping around.
pub fn step<T>(term: &Term<T>, regex: &mut RegexSearch, current: &Match, step: Step) -> Option<Match> {
    match step {
        Step::Up => {
            let origin = current.start().sub(term, Boundary::None, 1);
            term.search_next(regex, origin, Direction::Left, Side::Left, None)
        }
        Step::Down => {
            let origin = current.end().add(term, Boundary::None, 1);
            term.search_next(regex, origin, Direction::Right, Side::Left, None)
        }
    }
}

/// The bottom-right cell of the view.
pub fn view_bottom<T>(term: &Term<T>) -> Point {
    let offset = term.grid().display_offset() as i32;
    Point::new(Line(term.screen_lines() as i32 - 1 - offset), term.last_column())
}

/// Every match that is at least partly in view, in order.
pub fn visible_matches<T>(term: &Term<T>, regex: &mut RegexSearch) -> Vec<Match> {
    let offset = term.grid().display_offset() as i32;
    let top = Line(-offset);
    let bottom = Line(term.screen_lines() as i32 - 1 - offset);
    let mut start = term.line_search_left(Point::new(top, Column(0)));
    let mut end = term.line_search_right(Point::new(bottom, Column(0)));
    start.line = start.line.max(Line(top.0 - MAX_EXTRA_LINES));
    end.line = end.line.min(Line(bottom.0 + MAX_EXTRA_LINES));
    RegexIter::new(start, end, Direction::Right, term, regex)
        .skip_while(|m| m.end().line < top)
        .take_while(|m| m.start().line <= bottom)
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Count {
    /// Matches found, at most [`COUNT_CAP`].
    pub total: usize,
    /// Counting stopped at the cap; there are more.
    pub capped: bool,
    /// The focused match's number, from the bottom (1 = newest), if it is
    /// among the counted matches.
    pub index: Option<usize>,
}

/// Counts matches from the bottom of the buffer up, stopping at `cap`.
pub fn count<T>(term: &Term<T>, regex: &mut RegexSearch, focused: Option<&Match>, cap: usize) -> Count {
    let start = Point::new(term.bottommost_line(), term.last_column());
    let end = Point::new(term.topmost_line(), Column(0));
    let mut out = Count::default();
    for m in RegexIter::new(start, end, Direction::Left, term, regex) {
        if out.total == cap {
            out.capped = true;
            break;
        }
        out.total += 1;
        if focused == Some(&m) {
            out.index = Some(out.total);
        }
    }
    out
}

/// Whether all of `m` is on screen at `display_offset`.
pub fn is_visible(m: &Match, display_offset: usize, screen_lines: usize) -> bool {
    let top = -(display_offset as i32);
    let bottom = top + screen_lines as i32 - 1;
    m.start().line.0 >= top && m.end().line.0 <= bottom
}

/// The display offset that brings `m` into view with its first line in the
/// middle row, or `None` if it is already fully visible.
pub fn scroll_target(m: &Match, display_offset: usize, screen_lines: usize, history: usize) -> Option<usize> {
    if is_visible(m, display_offset, screen_lines) {
        return None;
    }
    let target = (screen_lines / 2) as i32 - m.start().line.0;
    Some(target.clamp(0, history as i32) as usize)
}

/// Scrolls `term` so that `m` is in view (see [`scroll_target`]).
pub fn scroll_to<T: alacritty_terminal::event::EventListener>(term: &mut Term<T>, m: &Match) {
    let grid = term.grid();
    let offset = grid.display_offset();
    let history = grid.total_lines() - grid.screen_lines();
    if let Some(target) = scroll_target(m, offset, grid.screen_lines(), history) {
        term.scroll_display(Scroll::Delta(target as i32 - offset as i32));
    }
}

/// Where `point` is on screen at `display_offset`, as (row, column), if it is.
pub fn viewport_position(point: Point, display_offset: usize, screen_lines: usize) -> Option<(usize, usize)> {
    let row = point.line.0 + display_offset as i32;
    (row >= 0 && (row as usize) < screen_lines).then_some((row as usize, point.column.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::GridSize;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    fn term_with(cols: u16, rows: u16, text: &str) -> Term<VoidListener> {
        let size = GridSize { cols, rows, ..Default::default() };
        let mut term = Term::new(term::Config::default(), &size, VoidListener);
        let mut processor: Processor<StdSyncHandler> = Processor::new();
        processor.advance(&mut term, text.replace('\n', "\r\n").as_bytes());
        term
    }

    fn regex(query: &str, options: SearchOptions) -> RegexSearch {
        compile(query, options).unwrap().unwrap()
    }

    fn text_at<T>(term: &Term<T>, m: &Match) -> String {
        term.bounds_to_string(*m.start(), *m.end())
    }

    #[test]
    fn literal_queries_are_escaped() {
        assert_eq!(escape("a.c(d)"), "a\\.c\\(d\\)");
        let plain = SearchOptions::default();
        assert_eq!(pattern("a.c", plain).unwrap(), "(?i)a\\.c");
        assert_eq!(pattern("", plain), None);
        let t = term_with(20, 4, "abc\na.c\n");
        let mut r = regex("a.c", plain);
        let found: Vec<_> = visible_matches(&t, &mut r).iter().map(|m| text_at(&t, m)).collect();
        assert_eq!(found, ["a.c"]);
        let mut r = regex("a.c", SearchOptions { regex: true, ..plain });
        assert_eq!(visible_matches(&t, &mut r).len(), 2);
    }

    #[test]
    fn match_case_overrides_smart_case() {
        let t = term_with(20, 4, "Foo\nfoo\n");
        let insensitive = SearchOptions::default();
        let sensitive = SearchOptions { case_sensitive: true, ..insensitive };
        // alacritty would make "Foo" case-sensitive; with match case off it isn't.
        assert_eq!(visible_matches(&t, &mut regex("Foo", insensitive)).len(), 2);
        // alacritty would make "foo" insensitive; with match case on it isn't.
        let mut r = regex("foo", sensitive);
        let found: Vec<_> = visible_matches(&t, &mut r).iter().map(|m| text_at(&t, m)).collect();
        assert_eq!(found, ["foo"]);
    }

    #[test]
    fn invalid_regex_is_an_error() {
        let options = SearchOptions { regex: true, ..Default::default() };
        assert!(compile("foo(", options).unwrap().is_err());
        assert!(compile("foo(", SearchOptions::default()).unwrap().is_ok());
        assert!(compile("", options).is_none());
    }

    #[test]
    fn steps_up_and_down_with_wrap() {
        let t = term_with(20, 6, "x1\nneedle a\nx2\nneedle b\nx3\nneedle c");
        let mut r = regex("needle", SearchOptions::default());
        let line = |m: &Match| m.start().line.0;
        let first = find_from(&t, &mut r, view_bottom(&t), None).unwrap();
        assert_eq!(line(&first), 5, "nearest above the bottom");
        let up = step(&t, &mut r, &first, Step::Up).unwrap();
        assert_eq!(line(&up), 3);
        let up2 = step(&t, &mut r, &up, Step::Up).unwrap();
        assert_eq!(line(&up2), 1);
        let wrapped = step(&t, &mut r, &up2, Step::Up).unwrap();
        assert_eq!(line(&wrapped), 5, "wraps to the newest");
        let down = step(&t, &mut r, &wrapped, Step::Down).unwrap();
        assert_eq!(line(&down), 1, "wraps to the oldest");
        assert_eq!(line(&step(&t, &mut r, &down, Step::Down).unwrap()), 3);
        // The origin's own line counts as "at or above".
        let at = find_from(&t, &mut r, Point::new(Line(3), Column(10)), None).unwrap();
        assert_eq!(line(&at), 3);
        assert!(find_from(&t, &mut regex("zzz", SearchOptions::default()), view_bottom(&t), None).is_none());
    }

    #[test]
    fn limited_search_stops_short() {
        let text: String = (1..=60).map(|i| format!("row {i}\n")).collect();
        let t = term_with(20, 10, &format!("needle\n{text}"));
        let mut r = regex("needle", SearchOptions::default());
        assert!(find_from(&t, &mut r, view_bottom(&t), Some(20)).is_none());
        assert!(find_from(&t, &mut r, view_bottom(&t), Some(100)).is_some());
        assert!(find_from(&t, &mut r, view_bottom(&t), None).is_some());
    }

    #[test]
    fn counts_from_the_bottom() {
        let text: String = (1..=200).map(|i| format!("line-{i}\n")).collect();
        let t = term_with(20, 10, &text);
        let mut r = regex("line-17", SearchOptions::default());
        let newest = find_from(&t, &mut r, view_bottom(&t), None).unwrap();
        assert_eq!(text_at(&t, &newest), "line-17");
        // line-17 and line-170..=179: 11 matches; the newest is line-179's prefix.
        let c = count(&t, &mut r, Some(&newest), COUNT_CAP);
        assert_eq!(c, Count { total: 11, capped: false, index: Some(1) });
        let next = step(&t, &mut r, &newest, Step::Up).unwrap();
        assert_eq!(count(&t, &mut r, Some(&next), COUNT_CAP).index, Some(2));
        let capped = count(&t, &mut regex("line", SearchOptions::default()), None, 50);
        assert_eq!(capped, Count { total: 50, capped: true, index: None });
        assert_eq!(count(&t, &mut regex("nope", SearchOptions::default()), None, 50).total, 0);
    }

    #[test]
    fn matches_across_a_wrapped_line() {
        // 10 columns: "xxxxxxxnee" wraps to "dle".
        let t = term_with(10, 4, "xxxxxxxneedle\n");
        let mut r = regex("needle", SearchOptions::default());
        let found = visible_matches(&t, &mut r);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].start().line.0, found[0].end().line.0), (0, 1));
    }

    #[test]
    fn visible_matches_stay_in_view() {
        let text: String = (1..=50).map(|i| format!("row {i}\n")).collect();
        let mut t = term_with(20, 5, &text);
        let mut r = regex("row", SearchOptions::default());
        // At the bottom: 4 rows of text plus the empty prompt line.
        assert_eq!(visible_matches(&t, &mut r).len(), 4);
        t.scroll_display(Scroll::Delta(20));
        let found = visible_matches(&t, &mut r);
        assert_eq!(found.len(), 5);
        assert!(found.iter().all(|m| is_visible(m, 20, 5)));
    }

    #[test]
    fn scroll_targets_center_and_clamp() {
        let m = |line: i32| Point::new(Line(line), Column(0))..=Point::new(Line(line), Column(3));
        // Visible already.
        assert_eq!(scroll_target(&m(3), 0, 10, 100), None);
        assert_eq!(scroll_target(&m(-5), 10, 10, 100), None);
        // Up in history: line -40 lands on row 5.
        assert_eq!(scroll_target(&m(-40), 0, 10, 100), Some(45));
        // Near the top of history: clamped.
        assert_eq!(scroll_target(&m(-100), 0, 10, 100), Some(100));
        // Below the view while scrolled up: back towards the bottom, clamped at 0.
        assert_eq!(scroll_target(&m(8), 30, 10, 100), Some(0));
        assert_eq!(viewport_position(Point::new(Line(-40), Column(2)), 45, 10), Some((5, 2)));
        assert_eq!(viewport_position(Point::new(Line(-40), Column(2)), 0, 10), None);
    }

    #[test]
    fn scroll_to_moves_the_view() {
        let text: String = (1..=100).map(|i| format!("row {i}\n")).collect();
        let mut t = term_with(20, 10, &text);
        // Rows 92-100 are on screen; the nearest "row 7" is row 79, in history.
        let mut r = regex("row 7", SearchOptions::default());
        let m = find_from(&t, &mut r, view_bottom(&t), None).unwrap();
        assert_eq!(t.bounds_to_string(*m.start(), Point::new(m.start().line, Column(5))), "row 79");
        scroll_to(&mut t, &m);
        let offset = t.grid().display_offset();
        assert_eq!(viewport_position(*m.start(), offset, 10), Some((5, 0)));
    }
}
