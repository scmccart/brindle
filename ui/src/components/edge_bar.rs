//! `EdgeBar`: a thin bar along the bottom edge of a tab trigger
//! ([`TabTrigger::edge_bar`](crate::TabTrigger::edge_bar)) or a card's
//! header ([`CardHeader::edge_bar`](crate::CardHeader::edge_bar)), filling
//! a share of its width from the left: a progress bar, say. shadcn has no
//! such part; it is listed in the crate's README.
//!
//! The bar is laid over the element's bottom edge, inside its rounded
//! corners, and takes no room, so the element is the same size with or
//! without one. It is paint only: a press on it reaches the element.

use gpui::{Div, Hsla, Length, ParentElement, Pixels, Styled, div, px, relative};

/// The bar's height.
pub const EDGE_BAR_HEIGHT: Pixels = px(2.);

/// A bar's length and colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeBar {
    fraction: f32,
    color: Hsla,
}

impl EdgeBar {
    /// A bar filling `fraction` of the width from the left, in `color`.
    /// The fraction is clamped to 0.0–1.0; one that isn't a number is 0.
    pub fn new(fraction: f32, color: impl Into<Hsla>) -> Self {
        let fraction = if fraction.is_nan() { 0. } else { fraction.clamp(0., 1.) };
        Self { fraction, color: color.into() }
    }

    /// The share of the width it fills, 0.0–1.0.
    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    pub fn color(&self) -> Hsla {
        self.color
    }

    /// The bar, placed absolutely over the bottom edge of its parent (which
    /// must be `relative`), `inset` in from the left and right so it stays
    /// inside rounded corners of that radius.
    pub(crate) fn element(self, inset: impl Into<Length>) -> Div {
        let inset = inset.into();
        div()
            .absolute()
            .bottom_0()
            .left(inset)
            .right(inset)
            .h(EDGE_BAR_HEIGHT)
            .child(div().h_full().w(relative(self.fraction)).bg(self.color))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fraction_is_clamped_and_a_nan_is_empty() {
        let red = gpui::red();
        assert_eq!(EdgeBar::new(0.25, red).fraction(), 0.25);
        assert_eq!(EdgeBar::new(-1., red).fraction(), 0.);
        assert_eq!(EdgeBar::new(3., red).fraction(), 1.);
        assert_eq!(EdgeBar::new(f32::NAN, red).fraction(), 0.);
        assert_eq!(EdgeBar::new(1., red).color(), red);
    }
}
