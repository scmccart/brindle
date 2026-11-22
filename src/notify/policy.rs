//! Whether a notification is shown, and what it says.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::kitty::Occasion;
use super::osc::{MAX_TITLE_CHARS, Source, truncate};
use crate::config::{NotificationConfig, NotifyWhen};

/// Whether `source` is turned on in the settings.
pub fn enabled(config: &NotificationConfig, source: Source) -> bool {
    match source {
        Source::Bell => config.bell,
        Source::Osc9 => config.osc9,
        Source::Osc777 => config.osc777,
        Source::Osc99 => config.osc99,
        Source::Osc7501 => config.osc7501,
    }
}

/// Whether `source` should notify, given its settings, when the program
/// asked to be shown (`occasion`, OSC 99's `o`; `Always` for the others),
/// whether its tab is in front of the user (`visible`: the window's active
/// tab, in a focused window) and whether it is its window's active tab
/// (`active`). The occasion can only make a notification quieter than the
/// user's `when`.
pub fn wanted(config: &NotificationConfig, source: Source, occasion: Occasion, visible: bool, active: bool) -> bool {
    let program_allows = match occasion {
        Occasion::Always => true,
        Occasion::Unfocused => !visible,
        Occasion::Invisible => !active,
    };
    enabled(config, source) && (config.when == NotifyWhen::Always || !visible) && program_allows
}

/// The notification's title and body: a program's own title comes first,
/// then the tab's, so the part that matters survives truncation.
pub fn layout(source: Source, title: &str, body: &str, tab_title: &str) -> (String, String) {
    let tab_title = tab_title.trim();
    let tab_title = if tab_title.is_empty() { "Brindle" } else { tab_title };
    let summary = if title.is_empty() { tab_title.to_string() } else { format!("{title} — {tab_title}") };
    let body = match source {
        Source::Bell => "Bell".to_string(),
        Source::Osc9 | Source::Osc777 | Source::Osc99 | Source::Osc7501 => body.to_string(),
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

/// How far the attachment to GPUI's Wayland display, which raises a window
/// with an activation token, has got (`activation.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(any(test, target_os = "linux")),
    expect(dead_code, reason = "only Wayland activation (`activation.rs`, Linux) waits for an attachment")
)]
pub enum AttachState {
    /// Not Wayland, or not prepared yet.
    Unused,
    /// Being made on the background executor.
    Pending,
    /// Made: a raise can carry a token.
    Ready,
    /// Making it failed (logged once).
    Failed,
}

/// How a window is raised ([`raise_plan`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(any(test, target_os = "linux")),
    expect(dead_code, reason = "only Wayland activation (`activation.rs`, Linux) waits for an attachment")
)]
pub enum RaisePlan {
    /// Activate the window's surface with the token at once.
    WithToken,
    /// Keep the window and its token until the attachment is made or has
    /// failed, then raise it.
    AfterAttach,
    /// GPUI's own `activate_window`.
    Plain,
}

/// How to raise a window, given the attachment's `state` and whether the
/// raise `has_token`: with the token once attached, after the attachment
/// while it is being made, else plainly.
#[cfg_attr(
    not(any(test, target_os = "linux")),
    expect(dead_code, reason = "only Wayland activation (`activation.rs`, Linux) waits for an attachment")
)]
pub fn raise_plan(state: AttachState, has_token: bool) -> RaisePlan {
    match (state, has_token) {
        (AttachState::Ready, true) => RaisePlan::WithToken,
        (AttachState::Pending, true) => RaisePlan::AfterAttach,
        (AttachState::Unused | AttachState::Failed, _) | (AttachState::Ready | AttachState::Pending, false) => {
            RaisePlan::Plain
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raise_waits_for_a_pending_attachment_only_with_a_token() {
        use AttachState::{Failed, Pending, Ready, Unused};
        let table = [
            (Unused, false, RaisePlan::Plain),
            (Unused, true, RaisePlan::Plain),
            (Pending, false, RaisePlan::Plain),
            (Pending, true, RaisePlan::AfterAttach),
            (Ready, false, RaisePlan::Plain),
            (Ready, true, RaisePlan::WithToken),
            (Failed, false, RaisePlan::Plain),
            (Failed, true, RaisePlan::Plain),
        ];
        for (state, has_token, plan) in table {
            assert_eq!(raise_plan(state, has_token), plan, "{state:?}, token: {has_token}");
        }
    }

    fn config(when: NotifyWhen) -> NotificationConfig {
        NotificationConfig { when, ..Default::default() }
    }

    /// `wanted` for a source that has no occasion of its own.
    fn plain(config: &NotificationConfig, source: Source, visible: bool) -> bool {
        wanted(config, source, Occasion::Always, visible, visible)
    }

    #[test]
    fn focus_rule() {
        let unfocused = config(NotifyWhen::Unfocused);
        let always = config(NotifyWhen::Always);
        for source in [Source::Bell, Source::Osc9, Source::Osc777, Source::Osc99] {
            assert!(!plain(&unfocused, source, true), "{source:?} visible");
            assert!(plain(&unfocused, source, false), "{source:?} hidden");
            assert!(plain(&always, source, true), "{source:?} always");
        }
        assert_eq!(NotificationConfig::default().when, NotifyWhen::Unfocused);
    }

    #[test]
    fn sources_can_be_disabled() {
        let mut c = config(NotifyWhen::Always);
        c.bell = false;
        assert!(!plain(&c, Source::Bell, false));
        assert!(plain(&c, Source::Osc9, false));
        c.osc9 = false;
        c.osc777 = false;
        assert!(!plain(&c, Source::Osc9, false));
        assert!(!plain(&c, Source::Osc777, false));
        assert!(plain(&c, Source::Osc99, false));
        c.osc99 = false;
        assert!(!enabled(&c, Source::Osc99));
        assert!(!wanted(&c, Source::Osc99, Occasion::Always, false, false));
    }

    /// Each occasion under both `when` values, for a tab in each place: in
    /// front of the user (active in a focused window), active in an
    /// unfocused window, and in the background.
    #[test]
    fn occasions_only_make_notifications_quieter() {
        // (visible, active)
        let front = (true, true);
        let unfocused_window = (false, true);
        let background = (false, false);
        let table = [
            (NotifyWhen::Always, Occasion::Always, [true, true, true]),
            (NotifyWhen::Always, Occasion::Unfocused, [false, true, true]),
            (NotifyWhen::Always, Occasion::Invisible, [false, false, true]),
            (NotifyWhen::Unfocused, Occasion::Always, [false, true, true]),
            (NotifyWhen::Unfocused, Occasion::Unfocused, [false, true, true]),
            (NotifyWhen::Unfocused, Occasion::Invisible, [false, false, true]),
        ];
        for (when, occasion, want) in table {
            let c = config(when);
            for ((visible, active), want) in [front, unfocused_window, background].into_iter().zip(want) {
                assert_eq!(
                    wanted(&c, Source::Osc99, occasion, visible, active),
                    want,
                    "{when:?} {occasion:?} visible={visible} active={active}"
                );
            }
        }
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
