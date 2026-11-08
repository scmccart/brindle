//! Roving list navigation and typeahead, for selects and command lists.
//!
//! [`ListNav`] keeps a list's highlight: `next`, `prev`, `first` and `last`
//! move it over the enabled options only, and past an end it either comes
//! round or stops, as the component says. [`Typeahead`] collects typed
//! characters into a query and finds the option whose label starts with
//! it. Both are pure; the clock is passed in.
//!
//! The keys come as the `List*` actions, bound in the `UiList` context by
//! `default_bindings()`: up/down (also ctrl-p/ctrl-n), home/end and enter.
//! A `TextInput` inside that context (a command list's filter) takes home
//! and end itself, since its own bindings are nearer the focus.

use std::time::{Duration, Instant};

/// The highlight of a list of `count` options, moved over the enabled ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListNav {
    pub highlight: Option<usize>,
    pub count: usize,
    /// Whether moving past an end comes round to the other end, or stops.
    pub wrap: bool,
}

impl ListNav {
    pub fn new(count: usize, wrap: bool) -> Self {
        Self { highlight: None, count, wrap }
    }

    /// Highlights the next enabled option (the first one when nothing is
    /// highlighted). At the end, it comes round when wrapping and stays
    /// otherwise; it also stays when no option is enabled. Returns the
    /// highlight.
    pub fn next(&mut self, enabled: impl Fn(usize) -> bool) -> Option<usize> {
        let start = self.highlight.map_or(0, |ix| ix as isize + 1);
        self.pick(self.walk(start, 1), enabled)
    }

    /// Highlights the previous enabled option (the last one when nothing is
    /// highlighted), coming round or staying at the start as [`next`] does.
    ///
    /// [`next`]: ListNav::next
    pub fn prev(&mut self, enabled: impl Fn(usize) -> bool) -> Option<usize> {
        let start = self.highlight.map_or(self.count as isize - 1, |ix| ix as isize - 1);
        self.pick(self.walk(start, -1), enabled)
    }

    /// Highlights the first enabled option.
    pub fn first(&mut self, enabled: impl Fn(usize) -> bool) -> Option<usize> {
        self.pick(self.walk(0, 1), enabled)
    }

    /// Highlights the last enabled option.
    pub fn last(&mut self, enabled: impl Fn(usize) -> bool) -> Option<usize> {
        self.pick(self.walk(self.count as isize - 1, -1), enabled)
    }

    /// The options from `start` on, `delta` apart: all `count` of them round
    /// the ends when wrapping, else those up to the end. `start` may lie
    /// just past an end, which wraps or yields nothing.
    fn walk(&self, start: isize, delta: isize) -> impl Iterator<Item = usize> + use<> {
        let count = self.count as isize;
        let wrap = self.wrap;
        (0..count)
            .map(move |step| start + step * delta)
            .take_while(move |ix| wrap || (0..count).contains(ix))
            .map(move |ix| ix.rem_euclid(count) as usize)
    }

    /// Moves the highlight to the first enabled candidate, if any.
    fn pick(&mut self, mut candidates: impl Iterator<Item = usize>, enabled: impl Fn(usize) -> bool) -> Option<usize> {
        if let Some(ix) = candidates.find(|ix| enabled(*ix)) {
            self.highlight = Some(ix);
        }
        self.highlight
    }
}

/// How long after a character the next one still extends the query.
pub const TYPEAHEAD_TIMEOUT: Duration = Duration::from_secs(1);

/// The characters typed into a list, as the query that moves its highlight.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Typeahead {
    buffer: String,
    last: Option<Instant>,
}

impl Typeahead {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes `ch`, typed at `now`: it extends the query when it came within
    /// [`TYPEAHEAD_TIMEOUT`] of the last character, and starts a new one
    /// otherwise. Returns the query, as [`query`] gives it.
    ///
    /// [`query`]: Typeahead::query
    pub fn push(&mut self, ch: char, now: Instant) -> &str {
        if !self.is_searching(now) {
            self.buffer.clear();
        }
        self.buffer.push(ch);
        self.last = Some(now);
        self.query()
    }

    /// The query to match labels against: the typed characters, or just
    /// one of them when they are all the same (`ff` asks for the next
    /// option starting with `f`, so repeating a character cycles).
    pub fn query(&self) -> &str {
        let mut chars = self.buffer.chars();
        match chars.next() {
            Some(first) if chars.all(|c| c == first) => &self.buffer[..first.len_utf8()],
            _ => &self.buffer,
        }
    }

    /// Whether a character typed at `now` extends the query. A select
    /// uses it to let space continue a search instead of choosing.
    pub fn is_searching(&self, now: Instant) -> bool {
        self.last.is_some_and(|last| now.saturating_duration_since(last) <= TYPEAHEAD_TIMEOUT)
    }

    /// Forgets the query, as when the list closes.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.last = None;
    }

    /// The option the query moves the highlight to from `current`; see
    /// [`typeahead_find`].
    pub fn find<S: AsRef<str>>(
        &self,
        labels: &[S],
        current: Option<usize>,
        enabled: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        typeahead_find(self.query(), labels, current, enabled)
    }
}

/// The first enabled option whose label starts with `query`,
/// case-insensitively, going round the list from after `current` for a
/// one-character query (so repeating the character cycles through the
/// options starting with it) and from `current` itself for a longer one.
/// `None` when the query is empty or matches no label: the highlight stays.
pub fn typeahead_find<S: AsRef<str>>(
    query: &str,
    labels: &[S],
    current: Option<usize>,
    enabled: impl Fn(usize) -> bool,
) -> Option<usize> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if query.is_empty() || labels.is_empty() {
        return None;
    }
    let start = match current {
        Some(ix) if query.len() == 1 => ix + 1,
        Some(ix) => ix,
        None => 0,
    };
    let count = labels.len();
    (0..count)
        .map(|step| (start + step) % count)
        .find(|&ix| enabled(ix) && starts_with_ignoring_case(labels[ix].as_ref(), &query))
}

fn starts_with_ignoring_case(label: &str, query_lowercase: &[char]) -> bool {
    let mut label = label.chars().flat_map(char::to_lowercase);
    query_lowercase.iter().all(|q| label.next() == Some(*q))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: fn(usize) -> bool = |_| true;

    #[test]
    fn moves_over_enabled_options_only() {
        // The spec's scenario: option 2 is disabled.
        let enabled = |ix| ix != 2;
        let mut nav = ListNav::new(4, false);
        nav.highlight = Some(1);
        assert_eq!(nav.next(enabled), Some(3));
        assert_eq!(nav.prev(enabled), Some(1));
        // From nothing, next goes to the first enabled and prev to the last.
        let mut nav = ListNav::new(4, false);
        assert_eq!(nav.next(|ix| ix != 0), Some(1));
        let mut nav = ListNav::new(4, false);
        assert_eq!(nav.prev(|ix| ix != 3), Some(2));
        // First and last skip disabled ends.
        let mut nav = ListNav::new(4, true);
        assert_eq!(nav.first(|ix| ix != 0), Some(1));
        assert_eq!(nav.last(|ix| ix != 3), Some(2));
        // A disabled highlight (hovered before it was disabled) is left.
        let mut nav = ListNav::new(4, false);
        nav.highlight = Some(2);
        assert_eq!(nav.next(enabled), Some(3));
    }

    #[test]
    fn wraps_or_stops_at_the_ends() {
        let mut stops = ListNav::new(3, false);
        stops.highlight = Some(2);
        assert_eq!(stops.next(ALL), Some(2));
        stops.highlight = Some(0);
        assert_eq!(stops.prev(ALL), Some(0));
        // With a disabled last option, next stops short of it.
        stops.highlight = Some(1);
        assert_eq!(stops.next(|ix| ix != 2), Some(1));

        let mut wraps = ListNav::new(3, true);
        wraps.highlight = Some(2);
        assert_eq!(wraps.next(ALL), Some(0));
        wraps.highlight = Some(0);
        assert_eq!(wraps.prev(ALL), Some(2));
        // Round the end to the next enabled option.
        wraps.highlight = Some(1);
        assert_eq!(wraps.next(|ix| ix == 0), Some(0));
        // The only enabled option is the highlight: it stays.
        wraps.highlight = Some(1);
        assert_eq!(wraps.next(|ix| ix == 1), Some(1));
        assert_eq!(wraps.prev(|ix| ix == 1), Some(1));
    }

    #[test]
    fn nothing_to_highlight() {
        let mut nav = ListNav::new(0, true);
        assert_eq!(nav.next(ALL), None);
        assert_eq!(nav.prev(ALL), None);
        assert_eq!(nav.first(ALL), None);
        assert_eq!(nav.last(ALL), None);
        // All disabled: the highlight is left where it is.
        let mut nav = ListNav::new(3, true);
        assert_eq!(nav.next(|_| false), None);
        nav.highlight = Some(1);
        assert_eq!(nav.next(|_| false), Some(1));
        assert_eq!(nav.first(|_| false), Some(1));
    }

    #[test]
    fn typeahead_accumulates_within_a_second() {
        let t0 = Instant::now();
        let mut typeahead = Typeahead::new();
        assert_eq!(typeahead.push('f', t0), "f");
        assert_eq!(typeahead.push('o', t0 + Duration::from_millis(999)), "fo");
        assert_eq!(typeahead.push('o', t0 + Duration::from_millis(1999)), "foo");
        // The second runs from the last character, not the first.
        assert!(typeahead.is_searching(t0 + Duration::from_millis(2999)));
        assert!(!typeahead.is_searching(t0 + Duration::from_millis(3000)));
        // Upper case is kept; matching ignores case.
        assert_eq!(typeahead.push('T', t0 + Duration::from_millis(2500)), "fooT");
    }

    #[test]
    fn typeahead_starts_over_after_the_timeout() {
        let t0 = Instant::now();
        let mut typeahead = Typeahead::new();
        typeahead.push('f', t0);
        assert_eq!(typeahead.push('i', t0 + Duration::from_millis(1001)), "i");
        typeahead.clear();
        assert_eq!(typeahead.query(), "");
        assert!(!typeahead.is_searching(t0));
        assert_eq!(typeahead.push('s', t0), "s");
    }

    #[test]
    fn repeating_a_character_cycles() {
        // The spec's scenario: bash, fish, foot; highlight on bash; `f`,
        // then `f` two seconds later.
        let labels = ["bash", "fish", "foot"];
        let t0 = Instant::now();
        let mut typeahead = Typeahead::new();
        typeahead.push('f', t0);
        let first = typeahead.find(&labels, Some(0), ALL);
        assert_eq!(first, Some(1));
        typeahead.push('f', t0 + Duration::from_secs(2));
        assert_eq!(typeahead.find(&labels, first, ALL), Some(2));
        // And round again.
        typeahead.push('f', t0 + Duration::from_secs(4));
        assert_eq!(typeahead.find(&labels, Some(2), ALL), Some(1));
        // Within the second, the repeated character is the same query.
        let mut quick = Typeahead::new();
        quick.push('f', t0);
        assert_eq!(quick.push('f', t0 + Duration::from_millis(500)), "f");
        assert_eq!(quick.find(&labels, Some(1), ALL), Some(2));
        // Three in a row too.
        assert_eq!(quick.push('f', t0 + Duration::from_millis(900)), "f");
    }

    #[test]
    fn longer_queries_match_from_the_highlight() {
        let labels = ["bash", "fish", "foot", "Fortran"];
        // `fo` from bash: foot, the first starting with it.
        assert_eq!(typeahead_find("fo", &labels, Some(0), ALL), Some(2));
        // From foot itself, a longer query may keep the highlight.
        assert_eq!(typeahead_find("fo", &labels, Some(2), ALL), Some(2));
        // Case-insensitive, both ways, and round the end.
        assert_eq!(typeahead_find("FOR", &labels, Some(0), ALL), Some(3));
        assert_eq!(typeahead_find("ba", &labels, Some(3), ALL), Some(0));
        // A disabled match is skipped.
        assert_eq!(typeahead_find("f", &labels, Some(0), |ix| ix != 1), Some(2));
        // Nothing matches, or nothing typed: the highlight stays.
        assert_eq!(typeahead_find("z", &labels, Some(0), ALL), None);
        assert_eq!(typeahead_find("", &labels, Some(0), ALL), None);
        assert_eq!(typeahead_find("f", &[] as &[&str], None, ALL), None);
        // With no highlight, the search starts at the top.
        assert_eq!(typeahead_find("f", &labels, None, ALL), Some(1));
    }
}
