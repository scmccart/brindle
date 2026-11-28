//! Pacing for the event loops that feed the UI thread (terminal output,
//! remote output).
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
//! next. A short run of quick updates, such as a key's echo followed by
//! the output it caused, is handled at once, and so is output after a
//! quiet spell.
//!
//! Each update then keeps taking ready batches for up to [`BUDGET`]
//! ([`drain`]), so a flood is handled in long updates with short gaps: the
//! gap is what Windows needs, and its share of the time is what a flood's
//! throughput loses (measured on Linux with vtebench).

use std::time::{Duration, Instant};

use futures::{FutureExt as _, Stream, StreamExt as _};
use gpui::AsyncApp;

/// How long one update keeps taking ready batches.
pub const BUDGET: Duration = Duration::from_millis(16);

/// Handles `first`, then each batch `rx` already has ready, until it has
/// none or [`BUDGET`] has passed. A closed stream stops it too; the loop's
/// next `rx.next()` sees the end.
pub fn drain<S: Stream + Unpin>(rx: &mut S, first: S::Item, handle: impl FnMut(S::Item)) {
    drain_with(rx, first, handle, Instant::now);
}

/// [`drain`] on the clock `now`, which it reads once before `first` and
/// once before taking each further batch.
pub fn drain_with<S: Stream + Unpin>(
    rx: &mut S,
    first: S::Item,
    mut handle: impl FnMut(S::Item),
    mut now: impl FnMut() -> Instant,
) {
    let start = now();
    handle(first);
    while now().saturating_duration_since(start) < BUDGET {
        match rx.next().now_or_never() {
            Some(Some(more)) => handle(more),
            _ => break,
        }
    }
}

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

/// The least time between two draws of a growing agent message (OpenSpec
/// change `agents-absorb-cost`, decision 5): a frame.
pub const FRAME: Duration = BUDGET;

/// How long an agent zone's draw at `now` waits for its growing messages
/// when the last draw that rendered them was at `last_draw`: `None` once a
/// [`FRAME`] has passed (or there was none), else the rest of it.
pub fn draw_wait(last_draw: Option<Instant>, now: Instant) -> Option<Duration> {
    last_draw.and_then(|last| FRAME.checked_sub(now.saturating_duration_since(last))).filter(|left| !left.is_zero())
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
    fn drain_takes_what_is_ready() {
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        for i in 1..=3 {
            tx.unbounded_send(i).unwrap();
        }
        let mut seen = Vec::new();
        drain(&mut rx, 0, |i| seen.push(i));
        assert_eq!(seen, [0, 1, 2, 3]);
    }

    #[test]
    fn drain_stops_at_the_budget() {
        // Each item takes over half the budget on the test's clock, so the
        // queue outlasts it. The clock moves only while an item is handled.
        const ITEMS: usize = 10;
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        for i in 1..=ITEMS {
            tx.unbounded_send(i).unwrap();
        }
        let clock = std::cell::Cell::new(Instant::now());
        let mut seen = Vec::new();
        let mut calls: Vec<(Instant, Instant)> = Vec::new();
        let before = clock.get();
        drain_with(
            &mut rx,
            0,
            |i| {
                let start = clock.get();
                seen.push(i);
                clock.set(start + BUDGET / 2 + MS);
                calls.push((start, clock.get()));
            },
            || clock.get(),
        );
        let after = clock.get();

        // `first`, then the queue in order.
        assert!(!seen.is_empty());
        assert!(seen.iter().copied().eq(0..seen.len()), "{seen:?}");

        // drain's clock starts between `before` and the first call, and each
        // check comes between one call's end and the next call (or `after`).
        // So the check that took item k came at least `end[k-1] - start[0]`
        // into the budget, which must not have run out.
        let first = calls[0].0;
        for k in 1..calls.len() {
            let least = calls[k - 1].1 - first;
            assert!(least < BUDGET, "item {k} taken {least:?} into the budget");
        }
        // The check that stopped it came at most `after - before` in: past the
        // budget, unless the queue was empty.
        let most = after - before;
        assert!(seen.len() == ITEMS + 1 || most >= BUDGET, "stopped {most:?} in, after {seen:?}");
        // On this clock two items fill the budget: `first`, then one more.
        assert_eq!(seen, [0, 1]);
    }

    #[test]
    fn a_draw_waits_out_the_rest_of_a_frame() {
        let now = Instant::now();
        assert_eq!(draw_wait(Some(now), now + 6 * MS), Some(FRAME - 6 * MS));
        assert_eq!(draw_wait(Some(now), now), Some(FRAME));
        assert_eq!(draw_wait(Some(now), now + FRAME - MS), Some(MS));
    }

    #[test]
    fn a_draw_after_a_quiet_frame_goes_at_once() {
        let now = Instant::now();
        assert_eq!(draw_wait(Some(now), now + FRAME), None);
        assert_eq!(draw_wait(Some(now), now + 3 * FRAME), None);
    }

    #[test]
    fn the_first_draw_goes_at_once() {
        assert_eq!(draw_wait(None, Instant::now()), None);
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
