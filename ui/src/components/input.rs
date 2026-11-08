//! `Input`: shadcn's input, a frame around a [`TextInput`].
//!
//! The owner keeps the `Entity<TextInput>` (its text, placeholder and
//! `Changed` events are the entity's) and renders `Input::new(&input)` each
//! frame. The frame draws the border in the input colour, the ring whenever
//! the text input is focused (a text input shows it however it got focus,
//! [`shows_ring`]), and the destructive colour when the owner marks it
//! invalid ([`input_style`]). A press anywhere in the frame focuses the text
//! input, as a pointer does.

use gpui::{
    App, CursorStyle, Entity, Focusable, InteractiveElement, IntoElement, MouseButton, ParentElement, Refineable,
    RenderOnce, StyleRefinement, Styled, Window, div, prelude::FluentBuilder, rems,
};

use crate::components::ring_shadow;
use crate::primitives::focus_visible::{focus_from_pointer, modality, shows_ring};
use crate::style::input_style;
use crate::text_input::TextInput;
use crate::theme::ActiveTheme;

/// A framed text input. Refine its layout through `Styled` (it takes its
/// parent's width by default).
#[derive(IntoElement)]
pub struct Input {
    input: Entity<TextInput>,
    invalid: bool,
    style: StyleRefinement,
}

impl Input {
    pub fn new(input: &Entity<TextInput>) -> Self {
        Self { input: input.clone(), invalid: false, style: StyleRefinement::default() }
    }

    /// Draws the border (and the ring, while focused) in the destructive
    /// colour, for a value the owner rejects.
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }
}

impl Styled for Input {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Input {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let handle = self.input.read(cx).focus_handle(cx);
        let ring_shown = shows_ring(handle.is_focused(window), modality(window, cx), true);
        let style = input_style(ring_shown, self.invalid, t);
        // h-9 w-full min-w-0 rounded-md border border-input bg-transparent
        // px-3 py-1 text-sm
        let mut frame = div()
            .flex()
            .items_center()
            .h_9()
            .w_full()
            .min_w_0()
            .px_3()
            .py_1()
            .rounded(t.radius_md())
            .border_1()
            .border_color(style.border)
            .text_size(rems(0.875))
            .text_color(t.foreground)
            .cursor(CursorStyle::IBeam)
            .when_some(style.ring, |d, ring| d.shadow(vec![ring_shadow(ring)]))
            // The padding is part of the field: a press there focuses the
            // text input too.
            .on_mouse_down(MouseButton::Left, move |_, window, cx| focus_from_pointer(&handle, window, cx))
            .child(self.input);
        // The owner's layout refinements win over the component's.
        frame.style().refine(&self.style);
        frame
    }
}
