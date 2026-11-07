//! Pacing for the event loops that feed the UI thread (terminal output,
//! remote output, tmux control events).
//!
//! Each of those loops would otherwise update as fast as its producer
//! sends, and a flooding program keeps it busy forever. On Windows that
//! freezes the window: GPUI wakes the main thread with one posted message
//! per task wake, and Windows hands out `WM_PAINT` and `WM_QUIT` only when
//! no posted message is waiting. Yielding isn't enough, since every wake is
//! a post; waiting on a timer leaves the queue idle until it fires.
//!
//! So once [`BURST`] updates in a row have each come less than [`PACE`]
//! after the previous one, a loop waits out the rest of [`PACE`] before the
//! next. A short run of quick updates, such as a tmux command's response
//! followed by the echo it caused, is handled at once, and so is output
//! after a quiet spell.

use std::time::{Duration, Instant};

use gpui::AsyncApp;

/// The least time between two updates of one loop while events keep coming.
pub const PACE: Duration = Duration::from_millis(4);

/// How many updates in a row may come sooner than [`PACE`] before pacing.
pub const BURST: u32 = 4;

/// The pacing decision for an update at `now`, when the previous one ended
/// at `last_update` and `quick` updates in a row before it came sooner than
/// [`PACE`]: the new run length, and how long to wait first.
pub fn step(last_update: Option<Instant>, quick: u32, now: Instant) -> (u32, Option<Duration>) {
    let left = last_update.and_then(|last| PACE.checked_sub(now.saturating_duration_since(last))).filter(|left| !left.is_zero());
    match left {
        None => (0, None),
        Some(left) => {
            let quick = quick.saturating_add(1);
            (quick, (quick > BURST).then_some(left))
        }
    }
}

/// One loop's pacing state.
#[derive(Default)]
pub struct Pace {
    last_update: Option<Instant>,
    quick: u32,
}

impl Pace {
    /// Waits, if this update would make more than [`BURST`] in a row less
    /// than [`PACE`] apart.
    pub async fn wait(&mut self, cx: &AsyncApp) {
        let (quick, wait) = step(self.last_update, self.quick, Instant::now());
        self.quick = quick;
        if let Some(left) = wait {
            cx.background_executor().timer(left).await;
        }
    }

    /// Notes that an update just ended.
    pub fn updated(&mut self) {
        self.last_update = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn no_previous_update_means_no_wait() {
        assert_eq!(step(None, 0, Instant::now()), (0, None));
        assert_eq!(step(None, BURST + 5, Instant::now()), (0, None));
    }

    #[test]
    fn a_short_burst_doesnt_wait() {
        let last = Instant::now();
        assert_eq!(step(Some(last), 0, last + MS), (1, None));
        assert_eq!(step(Some(last), BURST - 1, last), (BURST, None));
    }

    #[test]
    fn past_the_burst_waits_for_the_rest() {
        let last = Instant::now();
        assert_eq!(step(Some(last), BURST, last + MS), (BURST + 1, Some(PACE - MS)));
        assert_eq!(step(Some(last), BURST + 1, last), (BURST + 2, Some(PACE)));
    }

    #[test]
    fn at_or_after_pace_ends_the_run() {
        let last = Instant::now();
        assert_eq!(step(Some(last), BURST + 1, last + PACE), (0, None));
        assert_eq!(step(Some(last), BURST + 1, last + PACE * 10), (0, None));
    }

    #[test]
    fn a_flood_stays_paced() {
        // Each batch is ready right after the previous update ended.
        let mut quick = 0;
        let mut waits = 0;
        let mut now = Instant::now();
        for _ in 0..20 {
            let (next, wait) = step(Some(now), quick, now);
            quick = next;
            waits += usize::from(wait.is_some());
            now += wait.unwrap_or_default() + MS;
        }
        assert_eq!(waits, 20 - BURST as usize);
    }
}
