//! The label a mouse copy or paste shows at the pointer, apart from GPUI:
//! what it says, where it is, and whether it shows, fades or is gone at a
//! given moment. A copy that finishing a selection made (`copy_on_select`)
//! shows "Copied", and a right-click paste "Pasted" or "Nothing to paste".
//! Keys and the palette set none.
//!
//! The view keeps one [`Feedback`] at most: a new one replaces the old, so
//! two never stack. It takes `now` as an argument and reads no clock.

use std::time::{Duration, Instant};

use gpui::{Pixels, Point};

/// How long a label shows at full strength.
pub const SHOWN_FOR: Duration = Duration::from_millis(800);

/// How long it then takes to fade out.
pub const FADE_FOR: Duration = Duration::from_millis(200);

/// How often the fade redraws: about 60 Hz. The view's timer waits this
/// long between frames, which leaves the UI thread's queue idle in between
/// (see `terminal/pace.rs` on Windows wakes).
pub const FADE_FRAME: Duration = Duration::from_millis(16);

/// What a mouse copy or paste did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackKind {
    /// Finishing a selection copied it to the clipboard.
    Copied,
    /// A right-click pasted the clipboard.
    Pasted,
    /// A right-click found no text on the clipboard.
    NothingToPaste,
}

impl FeedbackKind {
    /// The label's text.
    pub fn label(self) -> &'static str {
        match self {
            FeedbackKind::Copied => "Copied",
            FeedbackKind::Pasted => "Pasted",
            FeedbackKind::NothingToPaste => "Nothing to paste",
        }
    }

    /// The name the `--dump-screen-after` line gives it.
    pub fn name(self) -> &'static str {
        match self {
            FeedbackKind::Copied => "copied",
            FeedbackKind::Pasted => "pasted",
            FeedbackKind::NothingToPaste => "nothing-to-paste",
        }
    }
}

/// Where a label is in its life.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// At full strength.
    Shown,
    /// Fading: the opacity left, from 1 down towards 0.
    Fading(f32),
    /// No longer drawn.
    Gone,
}

/// A label shown by a mouse copy or paste.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feedback {
    pub kind: FeedbackKind,
    /// The pointer when it happened, in window coordinates.
    pub at: Point<Pixels>,
    /// When it was shown.
    pub shown: Instant,
}

impl Feedback {
    pub fn new(kind: FeedbackKind, at: Point<Pixels>, now: Instant) -> Self {
        Self { kind, at, shown: now }
    }

    /// The label's phase at `now`: [`SHOWN_FOR`] at full strength, then a
    /// linear fade over [`FADE_FOR`], then gone.
    pub fn phase(&self, now: Instant) -> Phase {
        let age = now.saturating_duration_since(self.shown);
        if age < SHOWN_FOR {
            return Phase::Shown;
        }
        let fading = age - SHOWN_FOR;
        if fading >= FADE_FOR {
            return Phase::Gone;
        }
        Phase::Fading(1. - fading.as_secs_f32() / FADE_FOR.as_secs_f32())
    }

    /// How long from `now` until the label must be drawn again: the
    /// fade's start while it is shown, the next [`FADE_FRAME`] (or the
    /// fade's end, if sooner) while it fades, and `None` once it is gone.
    pub fn next_frame(&self, now: Instant) -> Option<Duration> {
        match self.phase(now) {
            Phase::Shown => Some((self.shown + SHOWN_FOR).saturating_duration_since(now)),
            Phase::Fading(_) => {
                let end = (self.shown + SHOWN_FOR + FADE_FOR).saturating_duration_since(now);
                Some(end.min(FADE_FRAME))
            }
            Phase::Gone => None,
        }
    }
}

/// The one label a view shows, if any.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FeedbackSlot(Option<Feedback>);

impl FeedbackSlot {
    /// Shows `kind` at `at` from `now`, replacing any label already there.
    pub fn show(&mut self, kind: FeedbackKind, at: Point<Pixels>, now: Instant) {
        self.0 = Some(Feedback::new(kind, at, now));
    }

    /// The label and its phase at `now`, unless there is none or it is
    /// gone.
    pub fn visible(&self, now: Instant) -> Option<(Feedback, Phase)> {
        let feedback = self.0?;
        match feedback.phase(now) {
            Phase::Gone => None,
            phase @ (Phase::Shown | Phase::Fading(_)) => Some((feedback, phase)),
        }
    }

    /// How long from `now` until the label must be drawn again
    /// ([`Feedback::next_frame`]); `None` with no label or a gone one.
    pub fn next_frame(&self, now: Instant) -> Option<Duration> {
        self.0?.next_frame(now)
    }

    /// Forgets a label that is gone at `now`, and says whether one is left.
    pub fn sweep(&mut self, now: Instant) -> bool {
        if self.visible(now).is_none() {
            self.0 = None;
        }
        self.0.is_some()
    }

    /// The `--dump-screen-after` line, `--- feedback kind=NAME`, while a
    /// label isn't gone at `now`; empty otherwise.
    pub fn describe(&self, now: Instant) -> String {
        self.visible(now)
            .map(|(feedback, _)| format!("--- feedback kind={}\n", feedback.kind.name()))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};

    fn at() -> Point<Pixels> {
        point(px(40.), px(30.))
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_label_shows_then_fades_then_goes() {
        let t0 = Instant::now();
        let feedback = Feedback::new(FeedbackKind::Copied, at(), t0);
        assert_eq!(feedback.phase(t0), Phase::Shown);
        assert_eq!(feedback.phase(t0 + ms(799)), Phase::Shown);
        let Phase::Fading(opacity) = feedback.phase(t0 + ms(900)) else {
            panic!("fading at 900 ms: {:?}", feedback.phase(t0 + ms(900)));
        };
        assert!((opacity - 0.5).abs() < 1e-3, "half faded at 900 ms: {opacity}");
        assert_eq!(feedback.phase(t0 + ms(1000)), Phase::Gone);
        assert_eq!(feedback.phase(t0 + ms(5000)), Phase::Gone);
    }

    #[test]
    fn frames_wait_for_the_fade_then_come_at_60_hz_then_stop() {
        let t0 = Instant::now();
        let feedback = Feedback::new(FeedbackKind::Pasted, at(), t0);
        // Nothing to redraw while it shows: one wait until the fade.
        assert_eq!(feedback.next_frame(t0), Some(ms(800)));
        assert_eq!(feedback.next_frame(t0 + ms(300)), Some(ms(500)));
        assert_eq!(feedback.next_frame(t0 + ms(800)), Some(FADE_FRAME));
        assert_eq!(feedback.next_frame(t0 + ms(900)), Some(FADE_FRAME));
        // The last frame lands on the fade's end, not past it.
        assert_eq!(feedback.next_frame(t0 + ms(990)), Some(ms(10)));
        assert_eq!(feedback.next_frame(t0 + ms(1000)), None);
        let mut slot = FeedbackSlot::default();
        assert_eq!(slot.next_frame(t0), None);
        slot.show(FeedbackKind::Copied, at(), t0);
        assert_eq!(slot.next_frame(t0 + ms(100)), Some(ms(700)));
    }

    #[test]
    fn a_new_label_replaces_the_old() {
        let t0 = Instant::now();
        let mut slot = FeedbackSlot::default();
        assert_eq!(slot.visible(t0), None);
        slot.show(FeedbackKind::Copied, at(), t0);
        let later = point(px(90.), px(10.));
        slot.show(FeedbackKind::NothingToPaste, later, t0 + ms(900));
        // The new one shows from its own start, where the old was gone.
        let (feedback, phase) = slot.visible(t0 + ms(1000)).expect("the new label shows");
        assert_eq!((feedback.kind, feedback.at, phase), (FeedbackKind::NothingToPaste, later, Phase::Shown));
        assert_eq!(slot.visible(t0 + ms(1900)), None);
    }

    #[test]
    fn a_gone_label_is_swept() {
        let t0 = Instant::now();
        let mut slot = FeedbackSlot::default();
        assert!(!slot.sweep(t0));
        slot.show(FeedbackKind::Pasted, at(), t0);
        assert!(slot.sweep(t0 + ms(950)));
        assert_ne!(slot, FeedbackSlot::default());
        assert!(!slot.sweep(t0 + ms(1000)));
        assert_eq!(slot, FeedbackSlot::default());
    }

    #[test]
    fn the_dump_line_names_the_kind_while_it_shows() {
        let t0 = Instant::now();
        let mut slot = FeedbackSlot::default();
        assert_eq!(slot.describe(t0), "");
        for (kind, name) in [
            (FeedbackKind::Copied, "copied"),
            (FeedbackKind::Pasted, "pasted"),
            (FeedbackKind::NothingToPaste, "nothing-to-paste"),
        ] {
            slot.show(kind, at(), t0);
            assert_eq!(slot.describe(t0 + ms(950)), format!("--- feedback kind={name}\n"));
            assert_eq!(slot.describe(t0 + ms(1000)), "");
        }
    }

    #[test]
    fn labels_read_as_the_spec_says() {
        assert_eq!(FeedbackKind::Copied.label(), "Copied");
        assert_eq!(FeedbackKind::Pasted.label(), "Pasted");
        assert_eq!(FeedbackKind::NothingToPaste.label(), "Nothing to paste");
    }
}
