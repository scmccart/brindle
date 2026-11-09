//! `Checkbox`: shadcn's checkbox, controlled by its owner.
//!
//! The owner holds the value and passes it as `checked`; a primary click,
//! or the space key-down while the checkbox has focus ([`activates`] with
//! [`CHECKBOX_KEYS`]), asks the owner to flip it through `on_click(&new, …)`
//! ([`toggle_request`]), and the box shows the new state once the owner
//! re-renders with it. Enter doesn't toggle it, as in Radix, whose checkbox
//! blocks enter ([`toggles`]), and passes to what is around the checkbox
//! (a [`Switch`] takes enter). A disabled checkbox is drawn at half
//! opacity, asks nothing and tracks no focus ([`interaction`]). The tick is
//! [`Icon::Check`] in the primary foreground; the ring shows when it is
//! focused from the keyboard.
//!
//! [`Switch`]: crate::components::Switch

use gpui::{
    App, ClickEvent, ElementId, FocusHandle, InteractiveElement, IntoElement, KeyboardButton, MouseButton,
    ParentElement, Refineable, RenderOnce, StyleRefinement, Styled, Window, div, prelude::FluentBuilder,
};
// (`on_click` and the key-down come through `activates`, so the stateful
// trait isn't named here.)

use crate::components::{CHECKBOX_KEYS, DISABLED_OPACITY, FocusRing, activates, inert_press, interaction};
use crate::icon::Icon;
use crate::primitives::focus_visible::{focus_from_pointer, modality, shows_ring};
use crate::style::{CHECKBOX_RADIUS, CHECKBOX_SIZE, CHECKBOX_TICK_SIZE, checkbox_style, focus_ring};
use crate::theme::ActiveTheme;

/// The handler a toggle calls with the value it asks for.
pub(crate) type ToggleHandler = Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>;

/// A checkbox. Refine its layout through `Styled` (margins, say); it is a
/// fixed square otherwise.
#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    checked: bool,
    disabled: bool,
    focus: Option<FocusHandle>,
    on_click: Option<ToggleHandler>,
    style: StyleRefinement,
}

impl Checkbox {
    /// A checkbox showing `checked`. `id` is the element id gpui tracks
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

    /// The state the owner holds, when it isn't given to [`Checkbox::new`].
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// A disabled checkbox is drawn muted, asks nothing and takes no focus.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The handle the checkbox is focused through. Without one it can't
    /// take focus, so space can't reach it.
    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    /// Called with the value the user asks for: the opposite of `checked`.
    pub fn on_click(mut self, on_click: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(on_click));
        self
    }
}

impl Styled for Checkbox {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// What a toggle asks its owner for on a click: the opposite of `checked`,
/// or nothing when it is disabled.
pub fn toggle_request(checked: bool, disabled: bool) -> Option<bool> {
    (!disabled).then_some(!checked)
}

/// Whether a click toggles a control that takes `keys`: a mouse click
/// always does, a keyboard click only for one of `keys` ([`CHECKBOX_KEYS`]:
/// space; [`SWITCH_KEYS`]: enter and space). `key` is the key of a keyboard
/// click, `None` for a mouse click. The controls take only their keys on
/// key-down, so this is the same rule stated over the click that arrives.
///
/// [`SWITCH_KEYS`]: crate::components::SWITCH_KEYS
pub fn toggles(keys: &[KeyboardButton], key: Option<KeyboardButton>) -> bool {
    key.is_none_or(|key| keys.contains(&key))
}

/// The key behind a click, `None` for a mouse click.
pub(crate) fn click_key(event: &ClickEvent) -> Option<KeyboardButton> {
    match event {
        ClickEvent::Keyboard(click) => Some(click.button),
        ClickEvent::Mouse(_) => None,
    }
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let (ring, ring_color) = (t.ring, focus_ring(t));
        let style = checkbox_style(self.checked, t);
        let (checked, disabled) = (self.checked, self.disabled);
        let (focus, on_click) = interaction(disabled, self.focus, self.on_click);
        let focused = focus.as_ref().is_some_and(|handle| handle.is_focused(window));
        let ring_shown = shows_ring(focused, modality(window, cx), false);
        // size-4 shrink-0 rounded-[4px] border border-input
        let mut checkbox = div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(CHECKBOX_SIZE)
            .rounded(CHECKBOX_RADIUS)
            .border_1()
            .border_color(style.border)
            .when_some(style.background, |d, background| d.bg(background))
            .text_color(style.tick)
            .when(ring_shown, |d| d.border_color(ring))
            .focus_ring(ring_shown, ring_color, window)
            .when(disabled, |d| inert_press(d.opacity(DISABLED_OPACITY).cursor_not_allowed()))
            .when(!disabled, |d| d.cursor_pointer())
            .when_some(focus, |d, handle| {
                d.track_focus(&handle)
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| focus_from_pointer(&handle, window, cx))
            })
            .when_some(on_click, |d, on_click| {
                activates(
                    d,
                    &CHECKBOX_KEYS,
                    Box::new(move |event, window, cx| {
                        if toggles(&CHECKBOX_KEYS, click_key(event))
                            && let Some(value) = toggle_request(checked, disabled)
                        {
                            on_click(&value, window, cx);
                        }
                    }),
                )
            })
            .when(checked, |d| d.child(Icon::Check.size(CHECKBOX_TICK_SIZE)));
        // The owner's layout refinements win over the component's.
        checkbox.style().refine(&self.style);
        checkbox
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A click asks for the opposite value, and a disabled toggle asks for
    /// nothing.
    #[test]
    fn a_toggle_asks_for_the_opposite_value_unless_disabled() {
        assert_eq!(toggle_request(false, false), Some(true));
        assert_eq!(toggle_request(true, false), Some(false));
        assert_eq!(toggle_request(false, true), None);
        assert_eq!(toggle_request(true, true), None);
    }

    /// A checkbox: mouse clicks and space toggle, enter doesn't. A switch:
    /// mouse clicks, space and enter all toggle, as Radix's button does.
    #[test]
    fn space_and_the_mouse_toggle_and_enter_only_a_switch() {
        use crate::components::SWITCH_KEYS;
        assert!(toggles(&CHECKBOX_KEYS, None));
        assert!(toggles(&CHECKBOX_KEYS, Some(KeyboardButton::Space)));
        assert!(!toggles(&CHECKBOX_KEYS, Some(KeyboardButton::Enter)));
        assert!(toggles(&SWITCH_KEYS, None));
        assert!(toggles(&SWITCH_KEYS, Some(KeyboardButton::Space)));
        assert!(toggles(&SWITCH_KEYS, Some(KeyboardButton::Enter)));
    }
}
