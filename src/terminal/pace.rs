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
//! So a loop waits out the rest of [`PACE`] after its previous update when
//! the next batch comes sooner. Output after a quiet spell is handled at
//! once.

use std::time::{Duration, Instant};

use gpui::AsyncApp;

/// The least time between two updates of one loop while events keep coming.
pub const PACE: Duration = Duration::from_millis(4);

/// How long to wait before an update at `now`, when the previous one ended
/// at `last_update`: the rest of [`PACE`], or `None` when it has passed.
pub fn delay(last_update: Option<Instant>, now: Instant) -> Option<Duration> {
    let left = PACE.checked_sub(now.saturating_duration_since(last_update?))?;
    (!left.is_zero()).then_some(left)
}

/// One loop's pacing state.
#[derive(Default)]
pub struct Pace {
    last_update: Option<Instant>,
}

impl Pace {
    /// Waits, if the previous update was less than [`PACE`] ago.
    pub async fn wait(&mut self, cx: &AsyncApp) {
        if let Some(left) = delay(self.last_update, Instant::now()) {
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

    #[test]
    fn no_previous_update_means_no_wait() {
        assert_eq!(delay(None, Instant::now()), None);
    }

    #[test]
    fn within_pace_waits_for_the_rest() {
        let last = Instant::now();
        assert_eq!(delay(Some(last), last + Duration::from_millis(1)), Some(PACE - Duration::from_millis(1)));
        assert_eq!(delay(Some(last), last), Some(PACE));
    }

    #[test]
    fn at_or_after_pace_doesnt_wait() {
        let last = Instant::now();
        assert_eq!(delay(Some(last), last + PACE), None);
        assert_eq!(delay(Some(last), last + PACE * 10), None);
    }
}
