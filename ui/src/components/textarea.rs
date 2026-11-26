//! `Textarea`: shadcn's textarea, a frame around a [`TextArea`] as
//! [`Input`](crate::components::Input) is around a text input.
//!
//! The owner keeps the `Entity<TextArea>` (its text, placeholder, row limit
//! and `Changed` events are the entity's) and renders
//! `Textarea::new(&area)` each frame. The frame draws the border in the
//! input colour, the ring whenever the text area is focused, and the
//! destructive colour when the owner marks it invalid ([`input_style`]).
//! It is at least shadcn's `min-h-16` tall and grows with the area's rows.
//! A press in the frame outside the text (its padding, below a short text)
//! focuses the area with the cursor at the end of its text; a press on the
//! text is the area's own.

use gpui::{
    App, CursorStyle, Entity, Focusable, InteractiveElement, IntoElement, MouseButton, ParentElement, Refineable,
    RenderOnce, StyleRefinement, Styled, Window, div, rems,
};

use crate::components::FocusRing;
use crate::primitives::focus_visible::{focus_from_pointer, modality, shows_ring};
use crate::style::input_style;
use crate::text_area::TextArea;
use crate::theme::ActiveTheme;

/// A framed text area. Refine its layout through `Styled` (it takes its
/// parent's width by default).
#[derive(IntoElement)]
pub struct Textarea {
    area: Entity<TextArea>,
    invalid: bool,
    style: StyleRefinement,
}

impl Textarea {
    pub fn new(area: &Entity<TextArea>) -> Self {
        Self { area: area.clone(), invalid: false, style: StyleRefinement::default() }
    }

    /// Draws the border (and the ring, while focused) in the destructive
    /// colour, for a value the owner rejects.
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }
}

impl Styled for Textarea {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Textarea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = &cx.ui_theme();
        let handle = self.area.read(cx).focus_handle(cx);
        let ring_shown = shows_ring(handle.is_focused(window), modality(window, cx), true);
        let style = input_style(ring_shown, self.invalid, t);
        let area = self.area.clone();
        // min-h-16 w-full rounded-md border border-input bg-transparent
        // px-3 py-2 text-sm
        let mut frame = div()
            .flex()
            .items_start()
            .min_h(rems(4.))
            .w_full()
            .min_w_0()
            .px_3()
            .py_2()
            .rounded(t.radius_md())
            .border_1()
            .border_color(style.border)
            .text_size(rems(0.875))
            .text_color(t.foreground)
            .cursor(CursorStyle::IBeam)
            // A press on the text is the area's (it stops there); one in
            // the frame around it focuses the area at the text's end.
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                focus_from_pointer(&handle, window, cx);
                area.update(cx, |area, cx| area.move_to_end(cx));
            })
            .child(self.area);
        frame.style().refine(&self.style);
        match style.ring {
            Some(ring) => frame.focus_ring(true, ring, window),
            None => frame,
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext, Context, Modifiers, Render, Window, point, px};

    use super::*;
    use crate::test_support;

    struct Host {
        area: Entity<TextArea>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(300.)).p(px(10.)).child(Textarea::new(&self.area))
        }
    }

    /// The spec's "Press in the frame": a press in the padding below the
    /// text focuses the area with the cursor at the end.
    #[test]
    fn a_press_in_the_frame_focuses_the_area_at_the_end() {
        let mut app = test_support::app();
        let (host, cx) = app.add_window_view(|_, cx| Host { area: cx.new(|cx| TextArea::new("hello\nworld", cx)) });
        let area = host.read_with(cx, |host, _| host.area.clone());
        cx.update(|_, cx| area.update(cx, |area, cx| area.select(0..0, cx)));
        cx.run_until_parked();
        assert_eq!(area.read_with(cx, |area, _| area.cursor()), 0);
        // The frame starts at (10, 10) and is at least 4rem (64px) tall,
        // well below the text's two rows.
        cx.simulate_click(point(px(150.), px(70.)), Modifiers::none());
        let (focused, cursor) =
            cx.update(|window, cx| (area.read(cx).is_focused(window), area.read(cx).cursor()));
        assert!(focused, "the press focuses the area");
        assert_eq!(cursor, "hello\nworld".len());
        // A press on the text is the area's: the cursor goes where it is.
        let origin = area.read_with(cx, |area, _| area.text_bounds()).expect("painted").origin;
        cx.simulate_click(origin + point(px(1.), px(2.)), Modifiers::none());
        assert_eq!(area.read_with(cx, |area, _| area.cursor()), 0);
    }
}
