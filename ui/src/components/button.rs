//! `Button`: shadcn's button, in its six variants and four sizes.
//!
//! A `RenderOnce` builder. The owner gives it a label and/or a leading
//! [`Icon`], a variant and a size ([`button_style`] turns them into colours
//! and lengths), an `on_click` handler and, to make it focusable, a
//! `FocusHandle`. A click comes from the primary mouse button, or from the
//! key-down of enter or space while the button has focus
//! ([`activates`]; gpui's own keyboard click, on the key-up, is ignored). A
//! disabled button is drawn at half opacity, has no click handler and
//! tracks no focus, so a press on it runs nothing and moves focus nowhere
//! ([`interaction`]). The ring shows when the button is focused from the
//! keyboard ([`shows_ring`]). A tooltip (`.tooltip_for(..)`, [`TooltipFor`])
//! shows only while the button is enabled.

use gpui::{
    App, ClickEvent, ElementId, FocusHandle, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Pixels, Refineable, Rems, RenderOnce, SharedString, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::components::tooltip::{Tooltip, TooltipFor, TooltipHandler, attach};
use crate::components::{ACTIVATION_KEYS, ClickHandler, DISABLED_OPACITY, FocusRing, activates, inert_press, interaction};
use crate::icon::Icon;
use crate::primitives::focus_visible::{focus_from_pointer, modality, shows_ring};
use crate::style::{ButtonSize, ButtonVariant, button_style};
use crate::theme::ActiveTheme;

/// A button. Build it with [`Button::new`] or [`Button::icon_only`], refine
/// its layout through `Styled` (width, margins, flex), and render it.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<Icon>,
    /// The icon's side and minimum stroke instead of the size's.
    icon_size: Option<Rems>,
    icon_min_stroke: Option<Pixels>,
    variant: ButtonVariant,
    size: ButtonSize,
    disabled: bool,
    focus: Option<FocusHandle>,
    on_click: Option<ClickHandler>,
    tooltip: Option<TooltipHandler>,
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
            icon_size: None,
            icon_min_stroke: None,
            variant: ButtonVariant::default(),
            size: ButtonSize::default(),
            disabled: false,
            focus: None,
            on_click: None,
            tooltip: None,
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

    /// The icon drawn `size` square instead of the button size's (a 16px
    /// icon in an `IconXs` button, say).
    pub fn icon_size(mut self, size: Rems) -> Self {
        self.icon_size = Some(size);
        self
    }

    /// The icon's strokes at least `width` pixels wide
    /// ([`IconElement::min_stroke`](crate::icon::IconElement::min_stroke)).
    pub fn icon_min_stroke(mut self, width: Pixels) -> Self {
        self.icon_min_stroke = Some(width);
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

    /// Runs on a primary click, and on the enter or space key-down while
    /// focused (as a `ClickEvent::Keyboard`).
    pub fn on_click(mut self, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(on_click));
        self
    }
}

/// The tooltip shows only while the button is enabled.
impl TooltipFor for Button {
    fn tooltip_with(mut self, build: impl Fn(&mut Window, &mut App) -> Tooltip + 'static) -> Self {
        self.tooltip = Some(Box::new(build));
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
        let style = button_style(self.variant, self.size, cx.ui_theme());
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
            // has; the ring goes outside it.
            .when(ring_shown, |d| d.when_some(style.focus_border, |d, border| d.border_color(border)))
            .when(self.disabled, |d| inert_press(d.opacity(DISABLED_OPACITY)))
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
            // A press focuses the button as a pointer does (no ring); enter
            // and space reach it through the tracked handle.
            .when_some(focus, |d, handle| {
                d.track_focus(&handle)
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| focus_from_pointer(&handle, window, cx))
            })
            .when_some(on_click, |d, on_click| activates(d, &ACTIVATION_KEYS, on_click))
            .children(self.icon.map(|icon| {
                let icon = icon.size(self.icon_size.unwrap_or(style.icon_size));
                match self.icon_min_stroke {
                    Some(width) => icon.min_stroke(width),
                    None => icon,
                }
            }))
            .children(self.label);
        button = attach(button, self.tooltip.filter(|_| !self.disabled));
        // The owner's layout refinements win over the component's. The ring
        // follows the corners they leave.
        button.style().refine(&self.style);
        button.focus_ring(ring_shown, style.ring, window)
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
        assert_eq!((button.icon_size, button.icon_min_stroke), (None, None));
        let button = button.icon_size(gpui::rems(1.)).icon_min_stroke(gpui::px(2.));
        assert_eq!((button.icon_size, button.icon_min_stroke), (Some(gpui::rems(1.)), Some(gpui::px(2.))));
        // A labelled button keeps the default size with an icon added.
        let button = Button::new("save", "Save").icon(Icon::Check);
        assert_eq!(button.size, ButtonSize::Default);
        assert_eq!(button.label.as_ref().map(|label| label.as_ref()), Some("Save"));
    }
}
