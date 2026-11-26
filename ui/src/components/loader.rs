//! `Loader`: AI Elements' loader, lucide's `loader-circle` turned to the
//! step its owner gives.
//!
//! The owner's clock, not the component, decides when it moves: a loader
//! asks for no animation frames (gpui's repeating animation redraws the
//! whole window every display frame). A turn is [`LOADER_STEPS`] steps.
//! It takes no focus. By default it is a small spinner, the size of the
//! status marks it stands in for, in the text colour.

use gpui::{App, Hsla, IntoElement, ParentElement, Pixels, RenderOnce, Rems, Styled, Window, div, px, rems};

use crate::icon::Icon;

/// A loader's steps per turn.
pub const LOADER_STEPS: u32 = 12;

/// The stroke a loader keeps at its small size: lucide's own scales down to
/// well under a pixel.
const LOADER_STROKE: Pixels = px(1.5);

#[derive(IntoElement)]
pub struct Loader {
    step: u32,
    size: Rems,
    color: Option<Hsla>,
}

impl Default for Loader {
    fn default() -> Self {
        Self::new()
    }
}

impl Loader {
    pub fn new() -> Self {
        Self { step: 0, size: rems(0.5), color: None }
    }

    /// The step to draw, counted from 0; [`LOADER_STEPS`] make a turn.
    pub fn step(mut self, step: u32) -> Self {
        self.step = step;
        self
    }

    /// The box it is drawn in (half a rem by default).
    pub fn size(mut self, size: Rems) -> Self {
        self.size = size;
        self
    }

    /// Its colour (the text colour by default).
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// How far a loader at `step` is turned, in turns.
    pub fn turns(step: u32) -> f32 {
        (step % LOADER_STEPS) as f32 / LOADER_STEPS as f32
    }
}

impl RenderOnce for Loader {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut icon = Icon::LoaderCircle.size(self.size).min_stroke(LOADER_STROKE).turn(Self::turns(self.step));
        if let Some(color) = self.color {
            icon = icon.color(color);
        }
        div().flex_none().size(self.size).child(icon)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui::{Context, Render};

    use super::*;
    use crate::test_support;

    /// The spec's "Owner's step": step 3 is a quarter of a turn, step 4 a
    /// twelfth further; a whole turn comes back to the start.
    #[test]
    fn each_step_is_a_twelfth_of_a_turn() {
        assert_eq!(Loader::turns(0), 0.);
        assert_eq!(Loader::turns(3), 0.25);
        assert!((Loader::turns(4) - Loader::turns(3) - 1. / 12.).abs() < 1e-6);
        assert_eq!(Loader::turns(12), 0.);
        assert_eq!(Loader::turns(13), Loader::turns(1));
    }

    struct Host {
        renders: usize,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.renders += 1;
            div().child(Loader::new().step(3))
        }
    }

    /// The spec's "No frames of its own": drawn once, a loader whose owner
    /// gives no new step redraws nothing, however long it waits.
    #[test]
    fn a_loader_asks_for_no_frames() {
        let mut app = test_support::app();
        let (host, cx) = app.add_window_view(|_, _| Host { renders: 0 });
        cx.run_until_parked();
        let drawn = host.read_with(cx, |host, _| host.renders);
        assert!(drawn >= 1);
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        assert_eq!(host.read_with(cx, |host, _| host.renders), drawn, "it redrew on its own");
    }
}
