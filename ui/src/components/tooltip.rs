//! `Tooltip`: shadcn's tooltip, a short label and an optional shortcut
//! shown beside a control while the pointer rests on it.
//!
//! It builds on gpui's own `tooltip`, so showing and hiding are gpui's: the
//! tooltip appears after the pointer has rested on the element for 500 ms,
//! placed just below and right of the pointer (flipped to stay in the
//! window), and hides when the pointer leaves the element, on a press and
//! on a scroll. The crate adds Escape: the shown tooltip watches the
//! window's key-downs (`intercept_keystrokes`, which sees a key before any
//! binding or handler takes it, and lets it go on) and draws nothing once
//! Escape was pressed, until the pointer leaves and rests again.
//!
//! [`TooltipFor`] puts one on an element: `.tooltip_for(label, shortcut)`
//! with fixed strings, or `.tooltip_with(|window, cx| Tooltip::new(..))`
//! to work the text out when it is shown (a shortcut read from the keymap
//! then, so a rebound key shows correctly). Either way the closure runs
//! and the view is built only when the tooltip is shown; a render pays for
//! the closure alone. It works on any stateful element (`div().id(..)`)
//! and on [`Button`] and [`Toggle`], which skip it while disabled, so a
//! disabled control shows no tooltip.
//!
//! The tooltip is drawn inverted, as shadcn's: the foreground token behind
//! text in the background token, `text-xs`, the shortcut as [`Kbd`]
//! keycaps in the tooltip's colours ([`tooltip_style`]). It takes no focus
//! and has no listeners, so it gets no hitbox and the pointer goes through
//! it. It stays on one line.
//!
//! [`Button`]: crate::components::Button
//! [`Toggle`]: crate::components::Toggle

use gpui::{
    AnyView, App, AppContext, Context, FontWeight, IntoElement, ParentElement, Render, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder,
};

use crate::components::Kbd;
use crate::style::tooltip_style;
use crate::theme::ActiveTheme;

/// What a tooltip shows: a label, and optionally a shortcut as keycaps.
/// It also renders as an element, the tooltip's panel drawn in place (the
/// gallery shows it so), which is how the shown view draws it.
#[derive(Clone, Debug, PartialEq, Eq, IntoElement)]
pub struct Tooltip {
    label: SharedString,
    shortcut: Option<SharedString>,
}

impl Tooltip {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), shortcut: None }
    }

    /// The shortcut, as a keymap string (`ctrl-shift-t`), drawn after the
    /// label with [`Kbd`].
    pub fn shortcut(mut self, keys: impl Into<SharedString>) -> Self {
        self.shortcut = Some(keys.into());
        self
    }

    /// The shortcut when there is one: `shortcut` with `None` adds none.
    pub fn shortcut_opt(mut self, keys: Option<SharedString>) -> Self {
        self.shortcut = keys;
        self
    }

    pub fn label(&self) -> &SharedString {
        &self.label
    }

    pub fn keys(&self) -> Option<&SharedString> {
        self.shortcut.as_ref()
    }

    /// The view gpui shows, for an element's own `tooltip` closure.
    pub fn build(self, window: &mut Window, cx: &mut App) -> AnyView {
        let window_id = window.window_handle().window_id();
        cx.new(|cx: &mut Context<TooltipView>| {
            let view = cx.weak_entity();
            let escape = cx.intercept_keystrokes(move |event, window, cx| {
                if event.keystroke.key == "escape" && window.window_handle().window_id() == window_id {
                    let hid = view
                        .update(cx, |view, cx| {
                            let was_shown = !view.dismissed;
                            view.dismissed = true;
                            cx.notify();
                            was_shown
                        })
                        .unwrap_or(false);
                    if hid {
                        window.refresh();
                    }
                }
            });
            TooltipView { tooltip: self, dismissed: false, _escape: escape }
        })
        .into()
    }
}

/// The handler a control keeps for its tooltip until it renders.
pub(crate) type TooltipHandler = Box<dyn Fn(&mut Window, &mut App) -> Tooltip + 'static>;

/// Puts the tooltip `build` describes on `element` (a control's own
/// stateful root), when there is one.
pub(crate) fn attach<E: StatefulInteractiveElement>(element: E, build: Option<TooltipHandler>) -> E {
    match build {
        Some(build) => element.tooltip(move |window, cx| build(window, cx).build(window, cx)),
        None => element,
    }
}

/// A tooltip on an element. Call it once per element: gpui allows one.
pub trait TooltipFor: Sized {
    /// A tooltip whose content `build` works out when it is shown.
    fn tooltip_with(self, build: impl Fn(&mut Window, &mut App) -> Tooltip + 'static) -> Self;

    /// A tooltip showing `label`, and `shortcut` as keycaps when given.
    fn tooltip_for(self, label: impl Into<SharedString>, shortcut: Option<SharedString>) -> Self {
        let tooltip = Tooltip::new(label).shortcut_opt(shortcut);
        self.tooltip_with(move |_, _| tooltip.clone())
    }
}

impl<E: StatefulInteractiveElement> TooltipFor for E {
    fn tooltip_with(self, build: impl Fn(&mut Window, &mut App) -> Tooltip + 'static) -> Self {
        self.tooltip(move |window, cx| build(window, cx).build(window, cx))
    }
}

/// The shown tooltip.
pub struct TooltipView {
    tooltip: Tooltip,
    /// Escape was pressed while it showed: it draws nothing from then on.
    dismissed: bool,
    _escape: Subscription,
}

impl TooltipView {
    pub fn tooltip(&self) -> &Tooltip {
        &self.tooltip
    }

    pub fn dismissed(&self) -> bool {
        self.dismissed
    }
}

impl Render for TooltipView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().when(!self.dismissed, |d| d.child(self.tooltip.clone()))
    }
}

impl RenderOnce for Tooltip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let style = tooltip_style(cx.ui_theme());
        let shortcut = self.shortcut;
        // gpui lays a tooltip out at its smallest size, so text that may
        // wrap would break at every word.
        div()
            .flex()
            .items_center()
            .gap(style.gap)
            .px(style.padding_x)
            .py(style.padding_y)
            .when(shortcut.is_some(), |d| d.pr(style.padding_right_with_shortcut))
            .rounded(style.radius)
            .bg(style.background)
            .text_color(style.foreground)
            .text_size(style.text_size)
            .font_weight(FontWeight::NORMAL)
            .whitespace_nowrap()
            .child(self.label)
            .children(shortcut.map(|keys| Kbd::new(keys).colors(style.kbd_background, style.kbd_foreground)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tooltip_keeps_its_label_and_shortcut() {
        let tooltip = Tooltip::new("New tab").shortcut("ctrl-shift-t");
        assert_eq!(tooltip.label().as_ref(), "New tab");
        assert_eq!(tooltip.keys().map(|keys| keys.as_ref()), Some("ctrl-shift-t"));
        let plain = Tooltip::new("Close");
        assert_eq!(plain.keys(), None);
        assert_eq!(Tooltip::new("Close").shortcut_opt(None), plain);
        assert_eq!(Tooltip::new("New tab").shortcut_opt(Some("ctrl-shift-t".into())), tooltip);
    }
}
