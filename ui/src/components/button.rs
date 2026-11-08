//! `Button`: shadcn's button, in its six variants and four sizes.
//!
//! A `RenderOnce` builder. The owner gives it a label and/or a leading
//! [`Icon`], a variant and a size ([`button_style`] turns them into colours
//! and lengths), an `on_click` handler and, to make it focusable, a
//! `FocusHandle`. A click comes from the primary mouse button, or from enter
//! or space while the button has focus (gpui's keyboard click). A disabled
//! button is drawn at half opacity, has no click handler and tracks no
//! focus, so a press on it runs nothing and moves focus nowhere
//! ([`interaction`]). The ring shows when the button is focused from the
//! keyboard ([`shows_ring`]).

use gpui::{
    App, ClickEvent, ElementId, FocusHandle, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Refineable, RenderOnce, SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::components::{DISABLED_OPACITY, interaction, ring_shadow};
use crate::icon::Icon;
use crate::primitives::focus_visible::{focus_from_pointer, modality, shows_ring};
use crate::style::{ButtonSize, ButtonVariant, button_style, focus_ring};
use crate::theme::ActiveTheme;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A button. Build it with [`Button::new`] or [`Button::icon_only`], refine
/// its layout through `Styled` (width, margins, flex), and render it.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<Icon>,
    variant: ButtonVariant,
    size: ButtonSize,
    disabled: bool,
    focus: Option<FocusHandle>,
    on_click: Option<ClickHandler>,
    style: StyleRefinement,
}

impl Button {
    /// A button with a text label. `id` is the element id gpui tracks
    /// clicks by, unique among the buttons of one parent.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: Some(label.into()),
            icon: None,
            variant: ButtonVariant::default(),
            size: ButtonSize::default(),
            disabled: false,
            focus: None,
            on_click: None,
            style: StyleRefinement::default(),
        }
    }

    /// A button showing only `icon`, in the square `ButtonSize::Icon`.
    pub fn icon_only(id: impl Into<ElementId>, icon: Icon) -> Self {
        Self { label: None, icon: Some(icon), size: ButtonSize::Icon, ..Self::new(id, "") }
    }

    /// A leading icon before the label.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// A disabled button is drawn muted, runs nothing and takes no focus.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The handle the button is focused through. Without one it can't take
    /// focus, so enter and space can't reach it and a press moves focus
    /// nowhere.
    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    /// Runs on a primary click, and on enter or space while focused.
    pub fn on_click(mut self, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(on_click));
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let (ring, ring_color) = (t.ring, focus_ring(t));
        let style = button_style(self.variant, self.size, t);
        let (focus, on_click) = interaction(self.disabled, self.focus, self.on_click);
        let focused = focus.as_ref().is_some_and(|handle| handle.is_focused(window));
        let ring_shown = shows_ring(focused, modality(window, cx), false);
        // shadcn's `has-[>svg]:px-*`: tighter padding beside an icon.
        let padding_x = if self.icon.is_some() { style.padding_x_with_icon } else { style.padding_x };
        let mut button = div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(style.gap)
            .h(style.height)
            .when_some(style.width, |d, width| d.w(width))
            .px(padding_x)
            .rounded(style.radius)
            .text_size(style.text_size)
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .text_color(style.foreground)
            .when_some(style.background, |d, background| d.bg(background))
            .when_some(style.border, |d, border| d.border_1().border_color(border))
            // `focus-visible:border-ring` recolours the border a variant
            // has; the shadow is the ring itself.
            .when(ring_shown, |d| d.border_color(ring).shadow(vec![ring_shadow(ring_color)]))
            .when(self.disabled, |d| d.opacity(DISABLED_OPACITY))
            .when(!self.disabled, |d| {
                d.cursor_pointer().hover(move |mut s| {
                    s = s.text_color(style.hover_foreground);
                    if let Some(background) = style.hover_background {
                        s = s.bg(background);
                    }
                    if style.hover_underline {
                        s = s.text_decoration_1();
                    }
                    s
                })
            })
            // A press focuses the button as a pointer does (no ring); gpui
            // registers enter and space for a focused tracked handle.
            .when_some(focus, |d, handle| {
                d.track_focus(&handle)
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| focus_from_pointer(&handle, window, cx))
            })
            .when_some(on_click, |d, on_click| d.on_click(move |event, window, cx| on_click(event, window, cx)))
            .children(self.icon)
            .children(self.label);
        // The owner's layout refinements win over the component's.
        button.style().refine(&self.style);
        button
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disabled button resolves to no click handler and no tracked focus,
    /// so a press on it runs nothing and moves focus nowhere.
    #[test]
    fn a_disabled_button_has_no_click_handler_and_tracks_no_focus() {
        assert_eq!(interaction(true, Some("handle"), Some("on_click")), (None, None));
        // Enabled, it keeps both.
        assert_eq!(interaction(false, Some("handle"), Some("on_click")), (Some("handle"), Some("on_click")));
        // Without a handle it still clicks, with the pointer only.
        assert_eq!(interaction(false, None::<&str>, Some("on_click")), (None, Some("on_click")));
        // Without a handler it still takes focus.
        assert_eq!(interaction(false, Some("handle"), None::<&str>), (Some("handle"), None));
    }

    #[test]
    fn icon_only_buttons_are_the_icon_size() {
        let button = Button::icon_only("close", Icon::X);
        assert_eq!(button.size, ButtonSize::Icon);
        assert_eq!(button.icon, Some(Icon::X));
        assert_eq!(button.label, None);
        // A labelled button keeps the default size with an icon added.
        let button = Button::new("save", "Save").icon(Icon::Check);
        assert_eq!(button.size, ButtonSize::Default);
        assert_eq!(button.label.as_ref().map(|label| label.as_ref()), Some("Save"));
    }
}
