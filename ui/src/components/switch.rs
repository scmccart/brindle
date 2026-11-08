//! `Switch`: shadcn's switch, a toggle drawn as a thumb sliding along a
//! track, controlled by its owner like a [`Checkbox`].
//!
//! The owner passes the state as `checked`; a primary click, or space while
//! the switch has focus, asks the owner to flip it through `on_click(&new,
//! …)` ([`toggle_request`]), and the thumb moves once the owner re-renders
//! with the new state ([`switch_style`] gives its position). A disabled
//! switch is drawn at half opacity, asks nothing and tracks no focus. The
//! ring shows when it is focused from the keyboard.
//!
//! [`Checkbox`]: crate::components::Checkbox

use gpui::{
    App, ElementId, FocusHandle, InteractiveElement, IntoElement, MouseButton, ParentElement, Refineable, RenderOnce,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, div, prelude::FluentBuilder, transparent_black,
};

use crate::components::checkbox::{ToggleHandler, click_key, toggle_request, toggles};
use crate::components::{DISABLED_OPACITY, interaction, ring_shadow};
use crate::primitives::focus_visible::{focus_from_pointer, modality, shows_ring};
use crate::style::{
    SWITCH_THUMB_SIZE, SWITCH_TRACK_BORDER, SWITCH_TRACK_HEIGHT, SWITCH_TRACK_WIDTH, focus_ring, switch_style,
};
use crate::theme::ActiveTheme;

/// A switch. Refine its layout through `Styled` (margins, say); its size
/// is fixed otherwise.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    checked: bool,
    disabled: bool,
    focus: Option<FocusHandle>,
    on_click: Option<ToggleHandler>,
    style: StyleRefinement,
}

impl Switch {
    /// A switch showing `checked`. `id` is the element id gpui tracks
    /// clicks by, unique among the controls of one parent.
    pub fn new(id: impl Into<ElementId>, checked: bool) -> Self {
        Self {
            id: id.into(),
            checked,
            disabled: false,
            focus: None,
            on_click: None,
            style: StyleRefinement::default(),
        }
    }

    /// The state the owner holds, when it isn't given to [`Switch::new`].
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// A disabled switch is drawn muted, asks nothing and takes no focus.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The handle the switch is focused through. Without one it can't take
    /// focus, so space can't reach it.
    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    /// Called with the state the user asks for: the opposite of `checked`.
    pub fn on_click(mut self, on_click: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(on_click));
        self
    }
}

impl Styled for Switch {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let (ring, ring_color) = (t.ring, focus_ring(t));
        let style = switch_style(self.checked, t);
        let (checked, disabled) = (self.checked, self.disabled);
        let (focus, on_click) = interaction(disabled, self.focus, self.on_click);
        let focused = focus.as_ref().is_some_and(|handle| handle.is_focused(window));
        let ring_shown = shows_ring(focused, modality(window, cx), false);
        // inline-flex h-[1.15rem] w-8 shrink-0 items-center rounded-full
        // border border-transparent; the thumb is size-4 rounded-full.
        let mut switch = div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .w(SWITCH_TRACK_WIDTH)
            .h(SWITCH_TRACK_HEIGHT)
            .rounded_full()
            .border(SWITCH_TRACK_BORDER)
            .border_color(transparent_black())
            .bg(style.track)
            .when(ring_shown, |d| d.border_color(ring).shadow(vec![ring_shadow(ring_color)]))
            .when(disabled, |d| d.opacity(DISABLED_OPACITY).cursor_not_allowed())
            .when(!disabled, |d| d.cursor_pointer())
            .when_some(focus, |d, handle| {
                d.track_focus(&handle)
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| focus_from_pointer(&handle, window, cx))
            })
            .when_some(on_click, |d, on_click| {
                d.on_click(move |event, window, cx| {
                    if toggles(click_key(event))
                        && let Some(value) = toggle_request(checked, disabled)
                    {
                        on_click(&value, window, cx);
                    }
                })
            })
            .child(
                div()
                    .flex_none()
                    .size(SWITCH_THUMB_SIZE)
                    .rounded_full()
                    .bg(style.thumb)
                    .ml(style.thumb_offset),
            );
        // The owner's layout refinements win over the component's.
        switch.style().refine(&self.style);
        switch
    }
}
