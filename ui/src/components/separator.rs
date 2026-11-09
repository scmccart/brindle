//! `Separator`: shadcn's one-pixel rule in the border colour.
//!
//! Horizontal it spans its container's width, vertical its height. It takes
//! no focus and handles no pointer events, so it is never a stop in a list
//! or menu's navigation; a menu skips it by not listing it as an item.

use gpui::{App, Axis, Hsla, IntoElement, Pixels, RenderOnce, StyleRefinement, Styled, Window, div, px};
use gpui::{Refineable, prelude::FluentBuilder};

use crate::theme::{ActiveTheme, Tokens};

/// A separator's resolved look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeparatorStyle {
    /// `bg-border`.
    pub color: Hsla,
    /// `h-px` or `w-px`.
    pub thickness: Pixels,
}

/// shadcn's separator (`bg-border shrink-0`, `h-px w-full` or `h-full w-px`).
pub fn separator_style(t: &Tokens) -> SeparatorStyle {
    SeparatorStyle { color: t.border, thickness: px(1.) }
}

/// A rule. Build it with [`Separator::horizontal`] or
/// [`Separator::vertical`]; refine its margins through `Styled`.
#[derive(IntoElement)]
pub struct Separator {
    axis: Axis,
    style: StyleRefinement,
}

impl Separator {
    /// A rule along the horizontal axis: a line one pixel tall.
    pub fn horizontal() -> Self {
        Self { axis: Axis::Horizontal, style: StyleRefinement::default() }
    }

    /// A rule along the vertical axis: a line one pixel wide, as tall as
    /// its container.
    pub fn vertical() -> Self {
        Self { axis: Axis::Vertical, style: StyleRefinement::default() }
    }
}

impl Styled for Separator {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Separator {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl gpui::IntoElement {
        let style = separator_style(cx.ui_theme());
        let mut rule = div()
            .flex_none()
            .bg(style.color)
            .when(self.axis == Axis::Horizontal, |d| d.h(style.thickness).w_full())
            .when(self.axis == Axis::Vertical, |d| d.w(style.thickness).h_full());
        rule.style().refine(&self.style);
        rule
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_is_one_pixel_in_the_border_colour() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            assert_eq!(separator_style(&t), SeparatorStyle { color: t.border, thickness: px(1.) });
        }
    }

    #[test]
    fn the_constructors_set_the_axis() {
        assert_eq!(Separator::horizontal().axis, Axis::Horizontal);
        assert_eq!(Separator::vertical().axis, Axis::Vertical);
    }
}
