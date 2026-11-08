//! The components: stateless `RenderOnce` builders over the tokens and the
//! primitives, after shadcn/ui.
//!
//! The owner holds every value (a checkbox's state, an input's `TextInput`),
//! passes it in on each render and gets changes back through the `on_click`
//! handlers. A focusable control takes its `FocusHandle` from the owner
//! (`.focus(&handle)`), since a `RenderOnce` value keeps nothing between
//! frames; without one it can't take keyboard focus, and pressing it moves
//! focus nowhere. Enter and space activate a focused control through gpui's
//! own keyboard click, so there is no activation action.
//!
//! What every control shares is here: [`interaction`], the rule that a
//! disabled control tracks no focus and gets no click handler, the half
//! opacity it is drawn at, and the focus ring, a 3px box shadow in the ring
//! colour (shadcn's `ring-[3px] ring-ring/50`) that takes no room, so
//! focusing moves no layout.

pub mod button;

use gpui::{BoxShadow, Hsla, Pixels, point, px};

pub use button::Button;

/// shadcn's `disabled:opacity-50`.
pub const DISABLED_OPACITY: f32 = 0.5;

/// The ring's width, shadcn's `ring-[3px]`. In pixels, as shadcn's is.
pub const RING_WIDTH: Pixels = px(3.);

/// The ring's opacity, shadcn's `ring-ring/50`.
pub const RING_OPACITY: f32 = 0.5;

/// A disabled control takes no part: it tracks no focus, so neither a press
/// nor Tab can focus it, and it gets no click handler, so nothing runs.
/// Enabled, it keeps what the owner gave it.
pub fn interaction<F, C>(disabled: bool, focus: Option<F>, on_click: Option<C>) -> (Option<F>, Option<C>) {
    if disabled { (None, None) } else { (focus, on_click) }
}

/// The focus ring as a box shadow: `color` (the ring token, or the
/// destructive colour on an invalid input) at `opacity`, [`RING_WIDTH`]
/// around the control, with no blur or offset.
pub(crate) fn ring_shadow(color: Hsla, opacity: f32) -> BoxShadow {
    BoxShadow { color: color.opacity(opacity), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: RING_WIDTH }
}
