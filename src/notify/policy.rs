//! Whether a notification is shown, and what it says.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::osc::{MAX_TITLE_CHARS, Source, truncate};
use crate::config::{NotificationConfig, NotifyWhen};

/// Whether `source` should notify, given its settings and whether its tab
/// is in front of the user (the window's active tab, in a focused window).
pub fn wanted(config: &NotificationConfig, source: Source, visible: bool) -> bool {
    let enabled = match source {
        Source::Bell => config.bell,
        Source::Osc9 => config.osc9,
        Source::Osc777 => config.osc777,
    };
    enabled && (config.when == NotifyWhen::Always || !visible)
}

/// The notification's title and body: a program's own title comes first,
/// then the tab's, so the part that matters survives truncation.
pub fn layout(source: Source, title: &str, body: &str, tab_title: &str) -> (String, String) {
    let tab_title = tab_title.trim();
    let tab_title = if tab_title.is_empty() { "Brindle" } else { tab_title };
    let summary = if title.is_empty() { tab_title.to_string() } else { format!("{title} — {tab_title}") };
    let body = match source {
        Source::Bell => "Bell".to_string(),
        _ => body.to_string(),
    };
    (truncate(&summary, MAX_TITLE_CHARS), body)
}

/// A token bucket: `capacity` at once, refilled one token per `interval`.
#[derive(Clone, Copy, Debug)]
pub struct Limit {
    pub capacity: u32,
    pub interval: Duration,
}

pub const BELL_PER_TAB: Limit = Limit { capacity: 1, interval: Duration::from_secs(5) };
pub const OSC_PER_TAB: Limit = Limit { capacity: 3, interval: Duration::from_secs(2) };
pub const GLOBAL: Limit = Limit { capacity: 10, interval: Duration::from_secs(2) };

#[derive(Clone, Copy, Debug)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn full(limit: Limit, now: Instant) -> Self {
        Self { tokens: f64::from(limit.capacity), at: now }
    }

    fn refill(&mut self, limit: Limit, now: Instant) {
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        let gained = elapsed / limit.interval.as_secs_f64();
        self.tokens = (self.tokens + gained).min(f64::from(limit.capacity));
        self.at = now;
    }

    fn has_token(&self) -> bool {
        self.tokens >= 1.0 - 1e-9
    }
}

/// Per-tab limits for bells and for program notifications, under a global
/// cap. A notification is charged only when every bucket it needs has a
/// token.
#[derive(Debug)]
pub struct RateLimiter {
    tabs: HashMap<(u64, bool), Bucket>,
    global: Option<Bucket>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self { tabs: HashMap::new(), global: None }
    }

    pub fn allow(&mut self, tab: u64, source: Source, now: Instant) -> bool {
        let is_bell = source == Source::Bell;
        let limit = if is_bell { BELL_PER_TAB } else { OSC_PER_TAB };
        let tab_bucket = self.tabs.entry((tab, is_bell)).or_insert_with(|| Bucket::full(limit, now));
        tab_bucket.refill(limit, now);
        let global = self.global.get_or_insert_with(|| Bucket::full(GLOBAL, now));
        global.refill(GLOBAL, now);
        if !tab_bucket.has_token() || !global.has_token() {
            return false;
        }
        tab_bucket.tokens -= 1.0;
        global.tokens -= 1.0;
        self.prune(now);
        true
    }

    /// Forgets buckets that have refilled completely; they'd start full anyway.
    fn prune(&mut self, now: Instant) {
        if self.tabs.len() < 64 {
            return;
        }
        self.tabs.retain(|&(_, is_bell), bucket| {
            let limit = if is_bell { BELL_PER_TAB } else { OSC_PER_TAB };
            bucket.refill(limit, now);
            bucket.tokens < f64::from(limit.capacity)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(when: NotifyWhen) -> NotificationConfig {
        NotificationConfig { when, ..Default::default() }
    }

    #[test]
    fn focus_rule() {
        let unfocused = config(NotifyWhen::Unfocused);
        let always = config(NotifyWhen::Always);
        for source in [Source::Bell, Source::Osc9, Source::Osc777] {
            assert!(!wanted(&unfocused, source, true), "{source:?} visible");
            assert!(wanted(&unfocused, source, false), "{source:?} hidden");
            assert!(wanted(&always, source, true), "{source:?} always");
        }
        assert_eq!(NotificationConfig::default().when, NotifyWhen::Unfocused);
    }

    #[test]
    fn sources_can_be_disabled() {
        let mut c = config(NotifyWhen::Always);
        c.bell = false;
        assert!(!wanted(&c, Source::Bell, false));
        assert!(wanted(&c, Source::Osc9, false));
        c.osc9 = false;
        c.osc777 = false;
        assert!(!wanted(&c, Source::Osc9, false));
        assert!(!wanted(&c, Source::Osc777, false));
    }

    #[test]
    fn titles_name_the_tab() {
        assert_eq!(layout(Source::Osc9, "", "done", "make"), ("make".into(), "done".into()));
        assert_eq!(layout(Source::Osc777, "Build", "ok", "bash"), ("Build — bash".into(), "ok".into()));
        assert_eq!(layout(Source::Bell, "", "", "vim"), ("vim".into(), "Bell".into()));
        assert_eq!(layout(Source::Osc9, "", "x", " "), ("Brindle".into(), "x".into()));
    }

    #[test]
    fn bell_storm_gives_one_per_five_seconds() {
        let mut limiter = RateLimiter::new();
        let t0 = Instant::now();
        let allowed = (0..100).filter(|i| limiter.allow(1, Source::Bell, t0 + Duration::from_millis(*i))).count();
        assert_eq!(allowed, 1);
        assert!(!limiter.allow(1, Source::Bell, t0 + Duration::from_millis(4900)));
        assert!(limiter.allow(1, Source::Bell, t0 + Duration::from_secs(5)));
        // Another tab has its own bucket.
        assert!(limiter.allow(2, Source::Bell, t0 + Duration::from_secs(5)));
    }

    #[test]
    fn osc_burst_then_one_per_two_seconds() {
        let mut limiter = RateLimiter::new();
        let t0 = Instant::now();
        let allowed = (0..50).filter(|_| limiter.allow(1, Source::Osc9, t0)).count();
        assert_eq!(allowed, 3);
        assert!(!limiter.allow(1, Source::Osc777, t0 + Duration::from_millis(1900)));
        assert!(limiter.allow(1, Source::Osc777, t0 + Duration::from_secs(2)));
        assert!(!limiter.allow(1, Source::Osc9, t0 + Duration::from_secs(2)));
        // Bells and program notifications don't share a bucket.
        assert!(limiter.allow(1, Source::Bell, t0));
    }

    #[test]
    fn global_cap_across_tabs() {
        let mut limiter = RateLimiter::new();
        let t0 = Instant::now();
        let allowed = (0..20u64).filter(|tab| limiter.allow(*tab, Source::Osc9, t0)).count();
        assert_eq!(allowed, 10);
        // Two seconds later the global bucket has one token again; tab 15,
        // refused before, takes it and tab 16 is refused.
        let later = t0 + Duration::from_secs(2);
        assert!(limiter.allow(15, Source::Osc9, later));
        assert!(!limiter.allow(16, Source::Osc9, later));
    }

    #[test]
    fn denied_requests_are_not_charged() {
        let mut limiter = RateLimiter::new();
        let t0 = Instant::now();
        assert!(limiter.allow(1, Source::Bell, t0));
        // Bell denied for tab 1: the global bucket keeps its tokens.
        for _ in 0..20 {
            assert!(!limiter.allow(1, Source::Bell, t0));
        }
        let allowed = (2..20u64).filter(|tab| limiter.allow(*tab, Source::Osc9, t0)).count();
        assert_eq!(allowed, 9);
    }
}
