//! When a relay's client last sent a frame, as measured by the relay's
//! thread that reads its frames.
//!
//! Copying the core's output to a slow client can block for a long time, so
//! only the reader, which keeps taking frames off the input, can tell
//! silence from a backlog. The reader stamps every frame it reads,
//! heartbeats included, which it then drops. A real frame is handed on to
//! the core, which can block; while the reader
//! waits for that, `busy` says the client has just sent something, and
//! finishing the hand-over stamps again. A watcher reads `busy` before the
//! stamp (both Acquire, paired with Release stores in the reader's order
//! stamp, busy, stamp, not-busy), so it never pairs an old stamp with a
//! finished hand-over. Nothing here is a lock, so a blocked reader holds
//! nothing the watcher needs.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct Liveness {
    start: Instant,
    /// Milliseconds from `start` to the latest stamp.
    last_frame_ms: AtomicU64,
    busy: AtomicBool,
}

impl Liveness {
    /// The clock starts at `start`, so a client that sends nothing at all is
    /// still bounded.
    pub fn new(start: Instant) -> Self {
        Self {
            start,
            last_frame_ms: AtomicU64::new(0),
            busy: AtomicBool::new(false),
        }
    }

    pub fn stamp(&self, now: Instant) {
        let ms = now.saturating_duration_since(self.start).as_millis() as u64;
        self.last_frame_ms.fetch_max(ms, Ordering::Release);
    }

    pub fn set_busy(&self, busy: bool) {
        self.busy.store(busy, Ordering::Release);
    }

    /// The reader has passed a real frame on.
    pub fn finish_handover(&self, now: Instant) {
        self.stamp(now);
        self.set_busy(false);
    }

    /// Whether the client has sent nothing for the whole `limit`.
    pub fn expired(&self, now: Instant, limit: Duration) -> bool {
        if self.busy.load(Ordering::Acquire) {
            return false;
        }
        let last = Duration::from_millis(self.last_frame_ms.load(Ordering::Acquire));
        now.saturating_duration_since(self.start)
            .saturating_sub(last)
            >= limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_is_silent_only_after_the_whole_limit() {
        let start = Instant::now();
        let limit = Duration::from_secs(60);
        let live = Liveness::new(start);
        assert!(!live.expired(start, limit));
        assert!(!live.expired(start + Duration::from_secs(59), limit));
        assert!(live.expired(start + limit, limit));
        // A clock reading before the stamp is not silence.
        live.stamp(start + limit);
        assert!(!live.expired(start, limit));
    }

    #[test]
    fn a_stamp_clears_the_silence() {
        let start = Instant::now();
        let limit = Duration::from_secs(60);
        let live = Liveness::new(start);
        assert!(live.expired(start + limit, limit));
        live.stamp(start + limit);
        assert!(!live.expired(start + limit + Duration::from_secs(59), limit));
        assert!(live.expired(start + limit + limit, limit));
    }

    #[test]
    fn a_frame_being_handed_over_is_never_silence() {
        let start = Instant::now();
        let limit = Duration::from_secs(60);
        let live = Liveness::new(start);
        live.set_busy(true);
        assert!(!live.expired(start + limit * 10, limit));
        // Finishing the hand-over counts as the client's latest sign of life.
        live.finish_handover(start + limit * 10);
        assert!(!live.expired(start + limit * 10 + Duration::from_secs(1), limit));
        assert!(live.expired(start + limit * 11, limit));
    }
}
