//! Waits for a condition within a limit scaled by `BRINDLE_TEST_SLOW`
//! (`testing R7`): a slow machine stretches only the limits, which passing
//! runs never reach, so nothing here waits longer than its condition needs.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The variable whose positive number scales every [`limit`] (default 1).
pub const SLOW_VAR: &str = "BRINDLE_TEST_SLOW";

/// How often [`eventually`] and [`until`] look again.
const POLL: Duration = Duration::from_millis(20);

/// The factor `value` (the variable's text, if set) gives, or why it gives
/// none.
pub fn parse_factor(value: Option<&str>) -> Result<f64, String> {
    let Some(text) = value else {
        return Ok(1.0);
    };
    match text.trim().parse::<f64>() {
        Ok(factor) if factor.is_finite() && factor > 0.0 => Ok(factor),
        _ => Err(format!("{SLOW_VAR} must be a positive number, not {text:?}")),
    }
}

/// The factor, read once; a bad value is a test-setup error.
fn factor() -> f64 {
    static FACTOR: OnceLock<f64> = OnceLock::new();
    *FACTOR.get_or_init(|| {
        let value = std::env::var_os(SLOW_VAR);
        let text = value.as_ref().map(|value| {
            value.to_str().unwrap_or_else(|| panic!("{SLOW_VAR} is not UTF-8: {value:?}"))
        });
        parse_factor(text).unwrap_or_else(|error| panic!("{error}"))
    })
}

/// `base` scaled by `factor`.
fn scaled(base: Duration, factor: f64) -> Duration {
    base.mul_f64(factor)
}

/// `base` scaled by [`SLOW_VAR`].
pub fn limit(base: Duration) -> Duration {
    scaled(base, factor())
}

/// Polls `condition` until it holds (true) or [`limit`]`(limit)` has passed
/// (false).
pub fn eventually(limit: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + self::limit(limit);
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// Polls `probe` until it gives a value, within [`limit`]`(limit)`; past
/// that it panics, naming `what`.
pub fn until<T>(what: &str, limit: Duration, mut probe: impl FnMut() -> Option<T>) -> T {
    let mut found = None;
    let scaled = self::limit(limit);
    if eventually(limit, || {
        found = probe();
        found.is_some()
    }) {
        if let Some(value) = found {
            return value;
        }
    }
    panic!("never: {what} (within {scaled:?})");
}

/// Watches a sampled value, without blocking: [`Quiet::observe`] reports
/// true once the value has stayed the same for the window. Poll it through
/// [`eventually`], so the window is measured and never slept.
pub struct Quiet {
    window: Duration,
    since: Option<(u64, Instant)>,
}

impl Quiet {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            since: None,
        }
    }

    /// Notes `value` now; `None` (no sample) starts the window again.
    pub fn observe(&mut self, value: Option<u64>) -> bool {
        self.observe_at(value, Instant::now())
    }

    /// [`Quiet::observe`] at `now`.
    pub fn observe_at(&mut self, value: Option<u64>, now: Instant) -> bool {
        let Some(value) = value else {
            self.since = None;
            return false;
        };
        match self.since {
            Some((seen, since)) if seen == value => now.saturating_duration_since(since) >= self.window,
            _ => {
                self.since = Some((value, now));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_millis(250);

    #[test]
    fn a_value_unchanged_for_the_window_is_quiet() {
        let start = Instant::now();
        let mut quiet = Quiet::new(WINDOW);
        assert!(!quiet.observe_at(Some(7), start));
        assert!(!quiet.observe_at(Some(7), start + WINDOW / 2));
        assert!(quiet.observe_at(Some(7), start + WINDOW));
    }

    #[test]
    fn a_change_starts_the_window_again() {
        let start = Instant::now();
        let mut quiet = Quiet::new(WINDOW);
        quiet.observe_at(Some(7), start);
        assert!(!quiet.observe_at(Some(8), start + WINDOW));
        assert!(!quiet.observe_at(Some(8), start + WINDOW * 3 / 2));
        assert!(quiet.observe_at(Some(8), start + WINDOW * 2));
    }

    #[test]
    fn no_sample_starts_the_window_again() {
        let start = Instant::now();
        let mut quiet = Quiet::new(WINDOW);
        quiet.observe_at(Some(7), start);
        assert!(!quiet.observe_at(None, start + WINDOW));
        assert!(!quiet.observe_at(Some(7), start + WINDOW));
        assert!(quiet.observe_at(Some(7), start + WINDOW * 2));
    }

    #[test]
    fn observe_reads_the_clock_itself() {
        let mut quiet = Quiet::new(Duration::ZERO);
        assert!(!quiet.observe(Some(1)));
        assert!(quiet.observe(Some(1)));
    }

    #[test]
    fn the_factor_is_a_positive_number_or_one() {
        assert_eq!(parse_factor(None), Ok(1.0));
        assert_eq!(parse_factor(Some("3")), Ok(3.0));
        assert_eq!(parse_factor(Some(" 1.5 ")), Ok(1.5));
        for bad in ["", "0", "-2", "slow", "inf", "NaN"] {
            let error = parse_factor(Some(bad)).unwrap_err();
            assert!(error.contains(SLOW_VAR), "{bad:?}: {error}");
        }
    }

    #[test]
    fn a_limit_is_scaled_by_the_factor() {
        assert_eq!(scaled(Duration::from_secs(10), 1.0), Duration::from_secs(10));
        assert_eq!(scaled(Duration::from_secs(10), 2.5), Duration::from_secs(25));
        assert!(limit(Duration::from_secs(10)) >= Duration::from_secs(10) / 1000);
    }

    #[test]
    fn until_gives_the_first_value_found() {
        let mut tries = 0;
        let found = until("the third try", Duration::from_secs(10), || {
            tries += 1;
            (tries == 3).then_some(tries)
        });
        assert_eq!(found, 3);
    }

    #[test]
    #[should_panic(expected = "never: nothing")]
    fn until_names_what_never_came() {
        until::<()>("nothing", Duration::ZERO, || None);
    }
}
