//! Program progress: ConEmu's `OSC 9 ; 4 ; <state> [; <percent>]`, which
//! Windows Terminal, Ghostty and ConEmu show and winget, cargo and others
//! send.
//!
//! [`parse`] reads one report, [`apply`] folds it into a terminal's
//! progress, and [`most_pressing`] picks the one a tab shows from its
//! terminals'. A terminal has no progress (`None`) until a report sets one,
//! and loses it on a clear report, a new prompt (OSC 133 `A`), its
//! program's exit or a full reset (`ESC c`); those lifetimes are the
//! callers', the client's `Terminal` and the server's
//! [`Emulator`](crate::emulator::Emulator), which apply them in stream
//! order alike.

/// A whole percentage, 0 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Percent(u8);

impl Percent {
    /// `value`, or 100 when it is more.
    pub fn clamped(value: u64) -> Self {
        Self(value.min(100) as u8)
    }

    pub fn get(self) -> u8 {
        self.0
    }

    /// The share of a whole this is, 0.0 to 1.0.
    pub fn fraction(self) -> f32 {
        f32::from(self.0) / 100.0
    }
}

/// A terminal's progress, as the latest report left it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Progress {
    /// State 1: under way, this far.
    Normal(Percent),
    /// State 2: failed, at the percent given or the one it had.
    Error(Option<Percent>),
    /// State 3: under way, of unknown length.
    Indeterminate,
    /// State 4: paused, at the percent given or the one it had.
    Paused(Option<Percent>),
}

impl Progress {
    /// The percent it carries, if any.
    pub fn percent(self) -> Option<Percent> {
        match self {
            Self::Normal(percent) => Some(percent),
            Self::Error(percent) | Self::Paused(percent) => percent,
            Self::Indeterminate => None,
        }
    }

    /// The state's name in the dump (`--- progress … state=NAME`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal(_) => "normal",
            Self::Error(_) => "error",
            Self::Indeterminate => "indeterminate",
            Self::Paused(_) => "paused",
        }
    }

    /// The share of the width its bar fills from the left, 0.0 to 1.0: its
    /// percent, or the whole width for an indeterminate one and for an
    /// error or a pause that never had a percent, so neither is invisible.
    pub fn fraction(self) -> f32 {
        match self {
            Self::Normal(percent) => percent.fraction(),
            Self::Error(percent) | Self::Paused(percent) => percent.map_or(1.0, Percent::fraction),
            Self::Indeterminate => 1.0,
        }
    }

    /// `state=NAME[ percent=N]`, as the dump prints it.
    pub fn describe(self) -> String {
        match self.percent() {
            Some(percent) => format!("state={} percent={}", self.name(), percent.get()),
            None => format!("state={}", self.name()),
        }
    }

    /// The `OSC 9` payload that reports it (`4;1;30`): fed to [`parse`]
    /// and [`apply`] from no progress, it gives this progress back.
    pub fn encode(self) -> String {
        let state = match self {
            Self::Normal(_) => 1,
            Self::Error(_) => 2,
            Self::Indeterminate => 3,
            Self::Paused(_) => 4,
        };
        match self.percent() {
            Some(percent) => format!("4;{state};{}", percent.get()),
            None => format!("4;{state}"),
        }
    }

    /// How pressing it is, lowest first: error, paused, normal,
    /// indeterminate, and within each the shortest bar.
    fn urgency(self) -> (u8, u8) {
        // A missing percent draws the whole width, so it ranks as 100.
        let length = self.percent().map_or(100, Percent::get);
        match self {
            Self::Error(_) => (0, length),
            Self::Paused(_) => (1, length),
            Self::Normal(_) => (2, length),
            Self::Indeterminate => (3, 0),
        }
    }
}

/// What one report says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Report {
    /// State 0: the terminal has no progress.
    Clear,
    Set(Reported),
}

/// A report's state and the percent it gave, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reported {
    Normal(Option<Percent>),
    Error(Option<Percent>),
    Indeterminate,
    Paused(Option<Percent>),
}

/// The report in an `OSC 9` payload (the body after `9;`): `4;<state>`,
/// then optionally `;<percent>` and fields that are ignored. `None` for
/// anything else: another ConEmu command, a message, a missing or unknown
/// state.
pub fn parse(payload: &[u8]) -> Option<Report> {
    let mut fields = payload.split(|&b| b == b';');
    if fields.next() != Some(b"4") {
        return None;
    }
    let state = whole(fields.next()?)?;
    let percent = fields.next().and_then(whole).map(Percent::clamped);
    let reported = match state {
        0 => return Some(Report::Clear),
        1 => Reported::Normal(percent),
        2 => Reported::Error(percent),
        3 => Reported::Indeterminate,
        4 => Reported::Paused(percent),
        _ => return None,
    };
    Some(Report::Set(reported))
}

/// A field of ASCII digits as a number, saturating; `None` when it is
/// empty or holds anything else.
fn whole(field: &[u8]) -> Option<u64> {
    if field.is_empty() || !field.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(field.iter().fold(0u64, |n, &b| n.saturating_mul(10).saturating_add(u64::from(b - b'0'))))
}

/// The progress after `report`, from `current`. Normal without a percent
/// is 0%; error and paused without one keep the current percent.
pub fn apply(current: Option<Progress>, report: Report) -> Option<Progress> {
    let kept = |given: Option<Percent>| given.or_else(|| current.and_then(Progress::percent));
    match report {
        Report::Clear => None,
        Report::Set(Reported::Normal(percent)) => Some(Progress::Normal(percent.unwrap_or(Percent(0)))),
        Report::Set(Reported::Error(percent)) => Some(Progress::Error(kept(percent))),
        Report::Set(Reported::Indeterminate) => Some(Progress::Indeterminate),
        Report::Set(Reported::Paused(percent)) => Some(Progress::Paused(kept(percent))),
    }
}

/// The progress a tab shows from its terminals': an error, else a pause,
/// else a normal one, else an indeterminate one; the shortest bar among
/// equals (the lowest percent).
pub fn most_pressing(progresses: impl IntoIterator<Item = Progress>) -> Option<Progress> {
    progresses.into_iter().min_by_key(|progress| progress.urgency())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pct(n: u8) -> Percent {
        Percent(n)
    }

    /// The progress after each payload in turn, from none.
    fn after(payloads: &[&str]) -> Option<Progress> {
        payloads.iter().fold(None, |current, payload| match parse(payload.as_bytes()) {
            Some(report) => apply(current, report),
            None => current,
        })
    }

    #[test]
    fn each_state_parses() {
        assert_eq!(parse(b"4;0"), Some(Report::Clear));
        assert_eq!(parse(b"4;0;50"), Some(Report::Clear));
        assert_eq!(parse(b"4;1;30"), Some(Report::Set(Reported::Normal(Some(pct(30))))));
        assert_eq!(parse(b"4;2;10"), Some(Report::Set(Reported::Error(Some(pct(10))))));
        assert_eq!(parse(b"4;3"), Some(Report::Set(Reported::Indeterminate)));
        assert_eq!(parse(b"4;3;50"), Some(Report::Set(Reported::Indeterminate)));
        assert_eq!(parse(b"4;4;50"), Some(Report::Set(Reported::Paused(Some(pct(50))))));
        assert_eq!(parse(b"4;1;30;extra"), Some(Report::Set(Reported::Normal(Some(pct(30))))));
    }

    #[test]
    fn other_payloads_are_not_reports() {
        for payload in ["", "4", "4;", "4;x", "4;-1", "4;7;50", "4;5", "4;01x", "42;1;30", "04;1;30", "9;/home", "1;100", "hello"] {
            assert_eq!(parse(payload.as_bytes()), None, "{payload:?}");
        }
    }

    #[test]
    fn a_percent_over_100_is_100_and_a_bad_one_is_absent() {
        assert_eq!(after(&["4;1;250"]), Some(Progress::Normal(pct(100))));
        assert_eq!(after(&["4;1;999999999999999999999999"]), Some(Progress::Normal(pct(100))));
        assert_eq!(after(&["4;1;100"]), Some(Progress::Normal(pct(100))));
        assert_eq!(after(&["4;1;0"]), Some(Progress::Normal(pct(0))));
        for bad in ["4;1", "4;1;", "4;1;x", "4;1;-5", "4;1;12.5", "4;1; 5"] {
            assert_eq!(after(&[bad]), Some(Progress::Normal(pct(0))), "{bad:?}: normal without a percent is 0%");
        }
    }

    #[test]
    fn error_and_paused_keep_the_previous_percent() {
        assert_eq!(after(&["4;1;60", "4;2"]), Some(Progress::Error(Some(pct(60)))));
        assert_eq!(after(&["4;1;60", "4;4"]), Some(Progress::Paused(Some(pct(60)))));
        assert_eq!(after(&["4;1;60", "4;2;10"]), Some(Progress::Error(Some(pct(10)))));
        assert_eq!(after(&["4;4;20", "4;2"]), Some(Progress::Error(Some(pct(20)))));
        assert_eq!(after(&["4;2"]), Some(Progress::Error(None)));
        assert_eq!(after(&["4;3", "4;4"]), Some(Progress::Paused(None)), "indeterminate had no percent");
        // Normal never keeps one.
        assert_eq!(after(&["4;1;60", "4;1"]), Some(Progress::Normal(pct(0))));
    }

    #[test]
    fn indeterminate_carries_no_percent() {
        assert_eq!(after(&["4;1;60", "4;3;50"]), Some(Progress::Indeterminate));
        assert_eq!(Progress::Indeterminate.percent(), None);
    }

    #[test]
    fn an_unknown_state_changes_nothing() {
        assert_eq!(after(&["4;1;10", "4;7;50"]), Some(Progress::Normal(pct(10))));
    }

    #[test]
    fn a_clear_report_drops_it() {
        assert_eq!(after(&["4;1;50", "4;0"]), None);
        assert_eq!(after(&["4;0"]), None);
    }

    #[test]
    fn the_most_pressing_is_error_then_paused_then_the_lowest_normal_then_indeterminate() {
        use Progress::{Error, Indeterminate, Normal, Paused};
        assert_eq!(most_pressing([]), None);
        assert_eq!(most_pressing([Normal(pct(80)), Error(Some(pct(10)))]), Some(Error(Some(pct(10)))));
        assert_eq!(most_pressing([Normal(pct(30)), Paused(Some(pct(70)))]), Some(Paused(Some(pct(70)))));
        assert_eq!(most_pressing([Paused(Some(pct(5))), Error(Some(pct(90)))]), Some(Error(Some(pct(90)))));
        assert_eq!(most_pressing([Normal(pct(80)), Normal(pct(20)), Normal(pct(50))]), Some(Normal(pct(20))));
        assert_eq!(most_pressing([Indeterminate, Normal(pct(90))]), Some(Normal(pct(90))));
        assert_eq!(most_pressing([Indeterminate]), Some(Indeterminate));
        assert_eq!(most_pressing([Error(None), Error(Some(pct(40)))]), Some(Error(Some(pct(40)))));
    }

    #[test]
    fn the_bar_length_is_the_percent_or_the_whole_width() {
        assert_eq!(Progress::Normal(pct(25)).fraction(), 0.25);
        assert_eq!(Progress::Error(Some(pct(10))).fraction(), 0.1);
        assert_eq!(Progress::Paused(Some(pct(50))).fraction(), 0.5);
        assert_eq!(Progress::Error(None).fraction(), 1.0);
        assert_eq!(Progress::Paused(None).fraction(), 1.0);
        assert_eq!(Progress::Indeterminate.fraction(), 1.0);
    }

    #[test]
    fn describe_names_the_state_and_percent() {
        assert_eq!(Progress::Normal(pct(30)).describe(), "state=normal percent=30");
        assert_eq!(Progress::Error(None).describe(), "state=error");
        assert_eq!(Progress::Indeterminate.describe(), "state=indeterminate");
        assert_eq!(Progress::Paused(Some(pct(20))).describe(), "state=paused percent=20");
    }

    #[test]
    fn an_encoded_progress_parses_back_from_none() {
        use Progress::{Error, Indeterminate, Normal, Paused};
        for progress in [Normal(pct(0)), Normal(pct(100)), Error(None), Error(Some(pct(7))), Indeterminate, Paused(None), Paused(Some(pct(20)))] {
            let report = parse(progress.encode().as_bytes());
            assert_eq!(report.and_then(|report| apply(None, report)), Some(progress), "{progress:?}");
        }
    }
}
