//! `DropdownMenu`: shadcn's DropdownMenu, a [`Menu`] opened below a trigger.
//!
//! The owner keeps the entity, renders it where the trigger goes and
//! subscribes to [`MenuEvent`]. It builds the trigger itself on each frame
//! from a closure the owner gives it, told whether the menu is open: a
//! `Button` with no `on_click` (the dropdown takes the press and the keys),
//! with a focus handle for the keys to reach it, or without one, so a
//! press moves focus nowhere.
//!
//! As Radix's DropdownMenuTrigger: a primary press on the trigger opens the
//! menu at once (on the press, not the click that follows), and enter,
//! space or down on the key-down while the trigger has focus open it
//! ([`trigger_key_opens`]). A second press on the trigger closes it: the
//! open menu blocks the pointer outside it, the trigger included, so that
//! press is an outside press, which only closes the menu.
//!
//! The menu goes below the trigger, start-aligned and 4px away, flipping
//! above it when there is more room there and shifted along to stay in the
//! window ([`MenuAnchor::below`]); its height is capped at the room it has,
//! and it scrolls. The primary press that opened it is the start of a
//! gesture: its release within 10px does nothing, and further away it
//! chooses the item it lands on, so press, drag and release is one choice.
//! On closing, focus goes back to what had it when the menu opened (the
//! trigger, when it has a focus handle and was pressed or keyed).

use gpui::{
    AnyElement, App, AppContext, Bounds, Context, Entity, EventEmitter, InteractiveElement, IntoElement, KeyDownEvent, Modifiers,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Render, Styled, Subscription, Window, div,
};

use crate::components::menu::{
    Menu, MenuAnchor, MenuEntry, MenuEvent, MenuPress, forward_selection, menu_overlay, render_menu,
};
use crate::components::popover::follows_trigger;
use crate::components::{ACTIVATION_KEYS, key_activation};
use crate::primitives::focus_visible::{Modality, set_modality};
use crate::primitives::overlay::Overlay;
use crate::primitives::placement::anchor_trigger;

/// Whether the key-down of `key` (with `modifiers`, `is_held` for a
/// repeat) on a focused, closed trigger opens its menu: enter, space or
/// down, bare and first pressed, as Radix's trigger (`Enter`/` ` toggle,
/// `ArrowDown` opens).
pub fn trigger_key_opens(key: &str, modifiers: Modifiers, is_held: bool) -> bool {
    key_activation(key, modifiers, is_held, &ACTIVATION_KEYS).is_some()
        || (key == "down" && !modifiers.modified() && !is_held)
}

type TriggerBuilder = Box<dyn Fn(bool, &mut Window, &mut App) -> AnyElement + 'static>;

/// A dropdown menu and its trigger.
pub struct DropdownMenu {
    trigger: TriggerBuilder,
    entries: Vec<MenuEntry>,
    overlay: Overlay<Menu>,
    /// The trigger's bounds, in window coordinates, from the last frame.
    bounds: Option<Bounds<Pixels>>,
    disabled: bool,
    _selection: Option<Subscription>,
}

impl EventEmitter<MenuEvent> for DropdownMenu {}

impl DropdownMenu {
    /// A dropdown of `entries` whose trigger `trigger` builds, given
    /// whether the menu is open.
    pub fn new<E: IntoElement>(
        trigger: impl Fn(bool, &mut Window, &mut App) -> E + 'static,
        entries: impl IntoIterator<Item = impl Into<MenuEntry>>,
    ) -> Self {
        Self {
            trigger: Box::new(move |open, window, cx| trigger(open, window, cx).into_any_element()),
            entries: entries.into_iter().map(Into::into).collect(),
            overlay: menu_overlay(),
            bounds: None,
            disabled: false,
            _selection: None,
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// A disabled dropdown opens nothing, from a press, a key or
    /// [`DropdownMenu::open`]; an open menu closes. The owner draws the
    /// trigger disabled too (a disabled `Button`).
    pub fn set_disabled(&mut self, disabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        if disabled {
            self.close(window, cx);
        }
        self.disabled = disabled;
        cx.notify();
    }

    pub fn entries(&self) -> &[MenuEntry] {
        &self.entries
    }

    /// Replaces the entries for the next opening; an open menu keeps its
    /// own.
    pub fn set_entries(&mut self, entries: impl IntoIterator<Item = impl Into<MenuEntry>>) {
        self.entries = entries.into_iter().map(Into::into).collect();
    }

    pub fn is_open(&self) -> bool {
        self.overlay.is_open()
    }

    /// The open menu, to read its entries and highlight.
    pub fn menu(&self) -> Option<&Entity<Menu>> {
        self.overlay.view()
    }

    /// Where the trigger was drawn in the last frame, in window
    /// coordinates.
    pub fn trigger_bounds(&self) -> Option<Bounds<Pixels>> {
        self.bounds
    }

    /// Opens the menu below the trigger, highlighting nothing, and focuses
    /// it. Nothing happens when it is open already or disabled. For a key
    /// or a call from code: no pointer gesture is under way.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_from(None, window, cx);
    }

    fn open_from(&mut self, press: Option<MenuPress>, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.overlay.is_open() {
            return;
        }
        let anchor = MenuAnchor::below(self.bounds.unwrap_or_default());
        let entries = self.entries.clone();
        let menu = cx.new(|cx| Menu::new(entries, anchor, press, cx));
        self._selection = Some(forward_selection(&menu, |this: &mut Self| &mut this.overlay, window, cx));
        self.overlay.open(menu, |this: &mut Self| &mut this.overlay, window, cx);
    }

    /// Closes the menu without a choice; focus goes back.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.is_open() {
            self.overlay.close(window, cx);
            cx.notify();
        }
    }

    /// A primary press on the trigger opens the menu. It goes no further,
    /// so an ancestor that tracks focus doesn't take focus from the menu
    /// (or, from a trigger with no focus handle, from what had it). While
    /// the menu is open the trigger is beneath its pointer block and sees
    /// no press. A disabled dropdown leaves the press to the (disabled)
    /// trigger.
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        set_modality(window, Modality::Pointer, cx);
        window.prevent_default();
        cx.stop_propagation();
        self.open_from(Some(MenuPress { button: event.button, position: event.position }), window, cx);
    }

    /// Enter, space or down from the focused trigger. The open menu has
    /// focus, and its keys don't come here (it isn't inside the trigger).
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if self.disabled
            || self.overlay.is_open()
            || !trigger_key_opens(&keystroke.key, keystroke.modifiers, event.is_held)
        {
            return;
        }
        // A stopped key isn't seen by the observer that records keyboard
        // input, so it is recorded here.
        set_modality(window, Modality::Keyboard, cx);
        cx.stop_propagation();
        self.open(window, cx);
    }
}

impl Render for DropdownMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let trigger = (self.trigger)(self.overlay.is_open(), window, cx);
        // The trigger's bounds each frame, for placing the menu. Recorded
        // without a notify, since this runs while the frame is drawn; a
        // trigger that moved under an open menu moves the menu and asks for
        // one more frame.
        let this = cx.entity();
        let record = move |bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App| {
            let follow = this.update(cx, |dropdown, cx| {
                let follow = follows_trigger(dropdown.bounds, bounds, dropdown.overlay.is_open());
                dropdown.bounds = Some(bounds);
                if follow && let Some(menu) = dropdown.overlay.view() {
                    menu.update(cx, |menu, _| menu.anchor = MenuAnchor::below(bounds));
                }
                follow
            });
            if follow {
                window.request_animation_frame();
            }
        };
        let trigger = div()
            .flex()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_key_down(cx.listener(Self::key_down))
            .child(trigger);
        // The menu is the trigger's sibling, so its keys and presses don't
        // pass through the trigger's handlers.
        anchor_trigger(trigger, record).flex().children(render_menu(&self.overlay, window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a key on a focused, closed trigger does, the way gpui gets
    /// there: the crate's bindings in `contexts` must not take it (a key
    /// bound there never reaches the trigger's `on_key_down`), and then
    /// [`trigger_key_opens`] decides.
    fn opens(contexts: &[&str], key: &str, modifiers: Modifiers) -> bool {
        use crate::actions::default_bindings;
        use gpui::{KeyContext, Keymap, Keystroke};
        let keymap = Keymap::new(default_bindings());
        let contexts: Vec<KeyContext> = contexts.iter().map(|c| KeyContext::parse(c).unwrap()).collect();
        let press = Keystroke { modifiers, key: key.into(), key_char: None };
        let (bindings, pending) = keymap.bindings_for_input(&[press], &contexts);
        assert!(!pending, "{key} starts a longer binding");
        if let Some(binding) = bindings.first() {
            panic!("{key} is bound to {} around the trigger", binding.action().name());
        }
        trigger_key_opens(key, modifiers, false)
    }

    /// Radix's trigger keys: enter, space and down open; up, the other
    /// arrows, home, end and letters don't. On its own, in a focus scope
    /// and inside a dialog's layer, whose bindings take none of them.
    #[test]
    fn trigger_keys_match_radix() {
        let none = Modifiers::default();
        let table = [
            ("enter", true),
            ("space", true),
            ("down", true),
            ("up", false),
            ("left", false),
            ("right", false),
            ("home", false),
            ("end", false),
            ("a", false),
        ];
        for contexts in [&[][..], &["UiFocusScope"], &["UiLayer", "UiFocusScope"]] {
            for (key, expected) in table {
                assert_eq!(opens(contexts, key, none), expected, "{key} in {contexts:?}");
            }
            // ctrl-n and ctrl-p are a list's, not a trigger's.
            assert!(!opens(contexts, "n", Modifiers::control()), "ctrl-n in {contexts:?}");
            assert!(!opens(contexts, "p", Modifiers::control()), "ctrl-p in {contexts:?}");
            assert!(!opens(contexts, "down", Modifiers::shift()), "shift-down in {contexts:?}");
        }
        // A held key opens nothing (the menu opened on its first press).
        assert!(!trigger_key_opens("down", none, true));
        assert!(!trigger_key_opens("enter", none, true));
    }
}
