//! The components: stateless `RenderOnce` builders over the tokens and the
//! primitives, after shadcn/ui.
//!
//! The owner holds every value (a checkbox's state, an input's `TextInput`),
//! passes it in on each render and gets changes back through the `on_click`
//! handlers. A focusable control takes its `FocusHandle` from the owner
//! (`.focus(&handle)`), since a `RenderOnce` value keeps nothing between
//! frames; without one it can't take keyboard focus, and pressing it moves
//! focus nowhere.
//!
//! Enter and space activate a focused control on **key-down**, through
//! [`activates`], which hands the control's `on_click` a
//! `ClickEvent::Keyboard`; gpui's own keyboard click is ignored. gpui fires
//! that click on the key-*up*, for whatever has focus then, unpaired with
//! the key-down: enter in a command list or a select's menu (key-down)
//! confirms, the overlay closes and focus goes back to the button that
//! opened it, and the key-up would then click that button and open the
//! overlay again. Acting on the key-down each control sees itself, and on
//! that alone, keeps one key to one effect ([`key_activation`],
//! [`accepts_click`]). A key an ancestor binds to an action goes to that
//! binding and never reaches the control (gpui matches bindings before key
//! listeners); one the control takes stops there, so a typed space or
//! newline reaches no input handler, and a held key activates once. The
//! activation counts as keyboard input for the focus ring, since the
//! keystroke observer behind that doesn't run for a key that was stopped.
//!
//! What every control shares is here: [`interaction`], the rule that a
//! disabled control tracks no focus and gets no click handler, the half
//! opacity it is drawn at, and the focus ring (`FocusRing`), a 3px band in
//! the ring colour just outside the control (shadcn's `ring-[3px]
//! ring-ring/50`) that takes no room, so focusing moves no layout.

pub mod button;
pub mod checkbox;
pub mod command;
pub mod dialog;
pub mod input;
pub mod kbd;
pub mod popover;
pub mod select;
pub mod separator;
pub mod switch;
pub mod toggle;
pub mod tooltip;

use std::rc::Rc;

use gpui::{
    App, Bounds, ClickEvent, Hsla, InteractiveElement, KeyboardButton, KeyboardClickEvent, Modifiers, MouseButton,
    ParentElement, Pixels, Refineable, StatefulInteractiveElement, Style, Styled, Window, div, px,
};

use crate::primitives::focus_visible::{Modality, set_modality};
use crate::style::ring_geometry;

pub use button::Button;
pub use checkbox::{Checkbox, toggle_request, toggles};
pub use command::{Command, CommandEvent, CommandGroup, CommandItem, CommandRow, CommandRows};
pub use dialog::{Dialog, dialog_width};
pub use input::Input;
pub use kbd::{Kbd, KbdStyle, kbd_style, keycaps};
pub use popover::{Popover, PopoverContent, PopoverTrigger, TriggerAction, follows_trigger, trigger_action};
pub use select::{
    Activation, DRAG_THRESHOLD, ListKey, MenuRelease, ReleaseOver, Select, SelectEvent, SelectOption, Selection,
    TypedKey, closed_trigger_key, initial_highlight, menu_release, moved_off, trigger_activation, typed_key,
};
pub use separator::{Separator, SeparatorStyle, separator_style};
pub use switch::Switch;
pub use toggle::Toggle;
pub use tooltip::{Tooltip, TooltipFor, TooltipView};

/// shadcn's `disabled:opacity-50`.
pub const DISABLED_OPACITY: f32 = 0.5;

/// The ring's width, shadcn's `ring-[3px]`. In pixels, as shadcn's is.
pub const RING_WIDTH: Pixels = px(3.);

/// A disabled control takes no part: it tracks no focus, so neither a press
/// nor Tab can focus it, and it gets no click handler, so nothing runs.
/// Enabled, it keeps what the owner gave it.
pub fn interaction<F, C>(disabled: bool, focus: Option<F>, on_click: Option<C>) -> (Option<F>, Option<C>) {
    if disabled { (None, None) } else { (focus, on_click) }
}

/// Makes a press on a disabled control move focus nowhere. Without a
/// listener it would bubble to an ancestor that tracks focus (a dialog's
/// view, an app's root), which gpui focuses on a press nothing inside
/// prevented.
pub(crate) fn inert_press<E: InteractiveElement>(element: E) -> E {
    element.on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
}

/// Draws the focus ring around a control: shadcn's `ring-[3px]`.
///
/// The ring is an absolutely positioned child with a [`RING_WIDTH`] border
/// in `color` (`style::focus_ring`, or an invalid input's destructive ring),
/// placed by [`ring_geometry`] from the border widths and corner radii the
/// control has set so far, so call it after those. It takes no room, has no
/// hitbox, and only draws outside the control's border box.
///
/// It isn't a box shadow, which is how CSS draws it: gpui 0.2.2's DirectX
/// (Windows) and Blade (Linux) shadow shaders divide by the blur radius,
/// so a shadow with none comes out with a NaN alpha and isn't seen; only
/// the Metal one draws a sharp shadow. And a gpui shadow fills the whole
/// dilated box, under the control too, where CSS clips it to the outside,
/// so a transparent control (ghost, link, an input) would be tinted.
///
/// Public so an app can ring a focusable element of its own (a list, say)
/// the way the components ring theirs, deciding `shown` with
/// [`shows_ring`](crate::shows_ring).
pub trait FocusRing: ParentElement + Styled + Sized {
    fn focus_ring(mut self, shown: bool, color: Hsla, window: &Window) -> Self {
        if !shown {
            return self;
        }
        let rem_size = window.rem_size();
        let mut style = Style::default();
        style.refine(self.style());
        let ring = ring_geometry(style.border_widths.to_pixels(rem_size), style.corner_radii.to_pixels(rem_size), RING_WIDTH);
        let mut frame = div()
            .absolute()
            .top(-ring.outset.top)
            .right(-ring.outset.right)
            .bottom(-ring.outset.bottom)
            .left(-ring.outset.left)
            .border(ring.width)
            .border_color(color);
        let corners = &mut frame.style().corner_radii;
        corners.top_left = Some(ring.radii.top_left.into());
        corners.top_right = Some(ring.radii.top_right.into());
        corners.bottom_right = Some(ring.radii.bottom_right.into());
        corners.bottom_left = Some(ring.radii.bottom_left.into());
        self.child(frame)
    }
}

impl<E: ParentElement + Styled + Sized> FocusRing for E {}

/// The keys that activate a button: enter and space, as in a browser.
pub const ACTIVATION_KEYS: [KeyboardButton; 2] = [KeyboardButton::Enter, KeyboardButton::Space];

/// The key that toggles a checkbox: space alone. Enter passes to what is
/// around it, as in Radix, whose checkbox blocks enter (WAI-ARIA's checkbox
/// pattern has no enter).
pub const CHECKBOX_KEYS: [KeyboardButton; 1] = [KeyboardButton::Space];

/// The keys that toggle a switch: enter and space, as for a button, since
/// Radix's switch is a plain `<button>` and takes both.
pub const SWITCH_KEYS: [KeyboardButton; 2] = ACTIVATION_KEYS;

/// The key-down that activates a control, as the button it stands for:
/// `key` is the keystroke's key, pressed with `modifiers`, `is_held` for a
/// key repeat. Only a first press of a bare key in `keys` activates; a
/// repeat, a modified key or any other key gives `None`, and the key goes
/// on to whatever else handles it.
pub fn key_activation(key: &str, modifiers: Modifiers, is_held: bool, keys: &[KeyboardButton]) -> Option<KeyboardButton> {
    if is_held || modifiers.modified() {
        return None;
    }
    let button = match key {
        "enter" => KeyboardButton::Enter,
        "space" => KeyboardButton::Space,
        _ => return None,
    };
    keys.contains(&button).then_some(button)
}

/// Whether a control acts on a click gpui delivered: a mouse click, yes; a
/// keyboard click, no. gpui raises that one on the key-up for whatever has
/// focus then, and the control acted on the key-down it saw itself
/// ([`key_activation`]), or saw nothing, because the key went to the
/// control that had focus before.
pub fn accepts_click(event: &ClickEvent) -> bool {
    matches!(event, ClickEvent::Mouse(_))
}

/// A click handler, as the controls box theirs.
pub(crate) type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Makes `element` run `on_click` on a mouse click, and on the key-down of
/// one of `keys` while it has focus, as a `ClickEvent::Keyboard` for that
/// key (its bounds are empty: the key, not the pointer, placed it). The
/// key-down counts as keyboard input for the ring and stops there. gpui's
/// own keyboard click, on the key-up, is ignored (see the module doc).
pub(crate) fn activates<E: StatefulInteractiveElement>(
    element: E,
    keys: &'static [KeyboardButton],
    on_click: ClickHandler,
) -> E {
    let on_click = Rc::new(on_click);
    let on_key = on_click.clone();
    element
        .on_click(move |event, window, cx| {
            if accepts_click(event) {
                on_click(event, window, cx);
            }
        })
        .on_key_down(move |event, window, cx| {
            let keystroke = &event.keystroke;
            if let Some(button) = key_activation(&keystroke.key, keystroke.modifiers, event.is_held, keys) {
                // The observer that would record this doesn't run for a
                // stopped key, and what the activation focuses (an opened
                // overlay's first control) shows its ring for a key.
                set_modality(window, Modality::Keyboard, cx);
                let click = ClickEvent::Keyboard(KeyboardClickEvent { button, bounds: Bounds::default() });
                on_key(&click, window, cx);
                cx.stop_propagation();
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::MouseClickEvent;

    /// A first press of bare enter or space activates a button and a
    /// switch; a checkbox takes space alone. Repeats, modified keys and other keys pass.
    #[test]
    fn enter_and_space_key_downs_activate_once() {
        let none = Modifiers::default();
        assert_eq!(key_activation("enter", none, false, &ACTIVATION_KEYS), Some(KeyboardButton::Enter));
        assert_eq!(key_activation("space", none, false, &ACTIVATION_KEYS), Some(KeyboardButton::Space));
        // A held key repeats the key-down; it activated on the first.
        assert_eq!(key_activation("enter", none, true, &ACTIVATION_KEYS), None);
        // Modified keys are shortcuts.
        for modifiers in [
            Modifiers { shift: true, ..none },
            Modifiers { control: true, ..none },
            Modifiers { alt: true, ..none },
            Modifiers { platform: true, ..none },
        ] {
            assert_eq!(key_activation("enter", modifiers, false, &ACTIVATION_KEYS), None);
            assert_eq!(key_activation("space", modifiers, false, &ACTIVATION_KEYS), None);
        }
        // Other keys, and a key the control doesn't take.
        assert_eq!(key_activation("a", none, false, &ACTIVATION_KEYS), None);
        assert_eq!(key_activation("tab", none, false, &ACTIVATION_KEYS), None);
        // A checkbox takes space alone; a switch takes enter too.
        assert_eq!(key_activation("enter", none, false, &CHECKBOX_KEYS), None);
        assert_eq!(key_activation("space", none, false, &CHECKBOX_KEYS), Some(KeyboardButton::Space));
        assert_eq!(key_activation("enter", none, false, &SWITCH_KEYS), Some(KeyboardButton::Enter));
        assert_eq!(key_activation("space", none, false, &SWITCH_KEYS), Some(KeyboardButton::Space));
        assert_eq!(key_activation("enter", none, true, &SWITCH_KEYS), None);
    }

    /// The scenario: enter (key-down) in a command popover confirms, the
    /// host closes and focus returns to the opener; gpui's keyboard click on
    /// the key-up reaches the opener, which ignores it. Mouse clicks count.
    #[test]
    fn gpui_keyboard_clicks_are_ignored_and_mouse_clicks_taken() {
        let key_up = ClickEvent::Keyboard(KeyboardClickEvent { button: KeyboardButton::Enter, bounds: Bounds::default() });
        assert!(!accepts_click(&key_up));
        let space_up = ClickEvent::Keyboard(KeyboardClickEvent { button: KeyboardButton::Space, bounds: Bounds::default() });
        assert!(!accepts_click(&space_up));
        assert!(accepts_click(&ClickEvent::Mouse(MouseClickEvent::default())));
    }
}
