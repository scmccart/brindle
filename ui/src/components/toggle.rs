//! `Toggle`: shadcn's toggle, a two-state button whose state the owner
//! holds, in its two variants and three sizes.
//!
//! It mirrors [`Button`]: a text label and/or a leading [`Icon`], a variant
//! and a size ([`toggle_style`] turns them and the pressed state into
//! colours and lengths), and, to make it focusable, a `FocusHandle`. The
//! owner passes the state as `pressed`; a primary click, or the enter or
//! space key-down while the toggle has focus ([`activates`] with
//! [`ACTIVATION_KEYS`]: Radix's toggle is a button), asks the owner to flip
//! it through `on_click(&new, …)` ([`toggle_request`]), and it shows the new
//! state once the owner re-renders with it. Pressed, it is the accent pair;
//! unpressed, a transparent control that hovers muted (the outline variant
//! adds the input border and hovers in accent).
//!
//! A disabled toggle is drawn at half opacity, asks nothing, tracks no
//! focus and shows no tooltip ([`interaction`]). A toggle without a focus
//! handle (or a disabled one) takes no focus on a press, and keeps an
//! ancestor that tracks focus from taking it: the press moves focus
//! nowhere, so a find bar's option toggles leave focus in its query. The
//! ring shows when it is focused from the keyboard ([`shows_ring`]).
//!
//! [`Button`]: crate::components::Button

use gpui::{
    App, ElementId, FocusHandle, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Refineable,
    RenderOnce, SharedString, StyleRefinement, Styled, Window, div, prelude::FluentBuilder,
};

use crate::components::checkbox::{ToggleHandler, toggle_request};
use crate::components::tooltip::{Tooltip, TooltipFor, TooltipHandler, attach};
use crate::components::{ACTIVATION_KEYS, DISABLED_OPACITY, FocusRing, activates, inert_press, interaction};
use crate::icon::Icon;
use crate::primitives::focus_visible::{focus_from_pointer, modality, shows_ring};
use crate::style::{ToggleSize, ToggleVariant, toggle_style};
use crate::theme::ActiveTheme;

/// A toggle. Build it with [`Toggle::new`] or [`Toggle::icon_only`], refine
/// its layout through `Styled`, and render it.
#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<Icon>,
    pressed: bool,
    variant: ToggleVariant,
    size: ToggleSize,
    disabled: bool,
    focus: Option<FocusHandle>,
    on_click: Option<ToggleHandler>,
    tooltip: Option<TooltipHandler>,
    style: StyleRefinement,
}

impl Toggle {
    /// A toggle with a text label (`Aa`, say). `id` is the element id gpui
    /// tracks clicks by, unique among the controls of one parent.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: Some(label.into()),
            icon: None,
            pressed: false,
            variant: ToggleVariant::default(),
            size: ToggleSize::default(),
            disabled: false,
            focus: None,
            on_click: None,
            tooltip: None,
            style: StyleRefinement::default(),
        }
    }

    /// A toggle showing only `icon`: a square, from the size's min width.
    pub fn icon_only(id: impl Into<ElementId>, icon: Icon) -> Self {
        Self { label: None, icon: Some(icon), ..Self::new(id, "") }
    }

    /// A leading icon before the label.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The state the owner holds.
    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed = pressed;
        self
    }

    pub fn variant(mut self, variant: ToggleVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: ToggleSize) -> Self {
        self.size = size;
        self
    }

    /// A disabled toggle is drawn muted, asks nothing and takes no focus.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The handle the toggle is focused through. Without one it can't take
    /// focus, so enter and space can't reach it, and a press moves focus
    /// nowhere.
    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    /// Called with the state the user asks for: the opposite of `pressed`.
    pub fn on_click(mut self, on_click: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(on_click));
        self
    }
}

impl TooltipFor for Toggle {
    fn tooltip_with(mut self, build: impl Fn(&mut Window, &mut App) -> Tooltip + 'static) -> Self {
        self.tooltip = Some(Box::new(build));
        self
    }
}

impl Styled for Toggle {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Toggle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let style = toggle_style(self.variant, self.size, self.pressed, cx.ui_theme());
        let (pressed, disabled) = (self.pressed, self.disabled);
        let (focus, on_click) = interaction(disabled, self.focus, self.on_click);
        let focused = focus.as_ref().is_some_and(|handle| handle.is_focused(window));
        let ring_shown = shows_ring(focused, modality(window, cx), false);
        let tooltip = self.tooltip.filter(|_| !disabled);
        let takes_focus = focus.is_some();
        let mut toggle = div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(style.gap)
            .h(style.height)
            .min_w(style.min_width)
            .px(style.padding_x)
            .rounded(style.radius)
            .text_size(style.text_size)
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .text_color(style.foreground)
            .when_some(style.background, |d, background| d.bg(background))
            .when_some(style.border, |d, border| d.border_1().border_color(border))
            .when(ring_shown, |d| d.when_some(style.focus_border, |d, border| d.border_color(border)))
            .when(disabled, |d| d.opacity(DISABLED_OPACITY))
            .when(!disabled, |d| {
                d.cursor_pointer()
                    .hover(move |s| s.bg(style.hover_background).text_color(style.hover_foreground))
            })
            // Without a handle the press moves focus nowhere, not even to
            // an ancestor that tracks focus.
            .when(!takes_focus, inert_press)
            // A press focuses the toggle as a pointer does (no ring); enter
            // and space reach it through the tracked handle.
            .when_some(focus, |d, handle| {
                d.track_focus(&handle)
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| focus_from_pointer(&handle, window, cx))
            })
            .when_some(on_click, |d, on_click| {
                activates(
                    d,
                    &ACTIVATION_KEYS,
                    Box::new(move |_, window, cx| {
                        if let Some(value) = toggle_request(pressed, disabled) {
                            on_click(&value, window, cx);
                        }
                    }),
                )
            })
            .children(self.icon.map(|icon| icon.size(style.icon_size)))
            .children(self.label);
        toggle = attach(toggle, tooltip);
        // The owner's layout refinements win over the component's. The ring
        // follows the corners they leave.
        toggle.style().refine(&self.style);
        toggle.focus_ring(ring_shown, style.ring, window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toggle_starts_unpressed_default_and_unfocusable() {
        let toggle = Toggle::new("case", "Aa");
        assert!(!toggle.pressed);
        assert_eq!((toggle.variant, toggle.size), (ToggleVariant::Default, ToggleSize::Default));
        // No handle: it can't take focus, and a press moves focus nowhere.
        assert!(toggle.focus.is_none());
        assert!(toggle.tooltip.is_none());
        let toggle = Toggle::icon_only("bold", Icon::Check).pressed(true).size(ToggleSize::Sm);
        assert!(toggle.pressed);
        assert_eq!((toggle.label, toggle.icon), (None, Some(Icon::Check)));
    }

    /// Pressing asks for the opposite state; a disabled toggle asks
    /// nothing, keeps no handler and tracks no focus.
    #[test]
    fn a_disabled_toggle_asks_nothing_and_takes_no_focus() {
        assert_eq!(toggle_request(false, false), Some(true));
        assert_eq!(toggle_request(true, false), Some(false));
        assert_eq!(toggle_request(false, true), None);
        assert_eq!(interaction(true, Some("handle"), Some("on_click")), (None, None));
        // Enabled without a handle: it still clicks, and takes no focus.
        assert_eq!(interaction(false, None::<&str>, Some("on_click")), (None, Some("on_click")));
    }

    #[test]
    fn the_tooltip_is_kept_until_render() {
        let toggle = Toggle::new("regex", ".*").tooltip_for("Regular expression", Some("alt-r".into()));
        assert!(toggle.tooltip.is_some());
    }
}
