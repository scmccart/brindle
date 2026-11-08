//! Focus-visible: whether the focused control got focus from the keyboard,
//! so it shows a ring, or from the pointer, so it doesn't.
//!
//! The [`Modality`] is kept per window in a crate global. [`init`] (the
//! crate's `init`) registers a keystroke observer that sets `Keyboard`;
//! every component's mouse-down handler goes through [`focus_from_pointer`],
//! which sets `Pointer` before focusing. [`shows_ring`] is the rule a
//! component applies when it paints.
//!
//! gpui runs keystroke observers after a key's actions and handlers, and
//! not at all when an `on_key_down` handler stopped the key. So a key that
//! moves focus through an action (Tab, an arrow, enter on a trigger) counts,
//! while a key a terminal swallows doesn't; the terminal shows no ring, so
//! nothing is lost. The heuristic can misread mixed input, say a key pressed
//! while hovering, which only decides whether a ring shows: a keystroke
//! always makes rings visible, the safe side.

use std::collections::HashMap;

use gpui::{App, FocusHandle, Global, KeystrokeEvent, Subscription, Window, WindowId};

/// How focus is being moved in a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Modality {
    /// With the keyboard: focused controls show the ring. Also the state
    /// of a window nothing has pressed in yet, the safe side.
    #[default]
    Keyboard,
    /// With the pointer: focused controls show no ring, text inputs apart.
    Pointer,
}

/// The crate global: one modality per window, and the keystroke observer
/// that keeps them. Entries are tiny and windows few, so a closed window's
/// entry is left behind.
#[derive(Default)]
struct Modalities {
    windows: HashMap<WindowId, Modality>,
    _observer: Option<Subscription>,
}

impl Global for Modalities {}

/// The modality of `window`. Read-only, so it can be called while rendering.
pub fn modality(window: &Window, cx: &App) -> Modality {
    cx.try_global::<Modalities>()
        .and_then(|modalities| modalities.windows.get(&window.window_handle().window_id()).copied())
        .unwrap_or_default()
}

/// Records how focus is being moved in `window`, and redraws it when that
/// changed, since its rings change with it.
pub fn set_modality(window: &mut Window, modality: Modality, cx: &mut App) {
    let id = window.window_handle().window_id();
    let previous = cx.default_global::<Modalities>().windows.insert(id, modality);
    if previous != Some(modality) {
        window.refresh();
    }
}

/// Focuses `handle` from a pointer press: the control gets focus and shows
/// no ring. Every component's mouse-down handler goes through this. It
/// does only that: an outside press on a non-modal overlay focuses what
/// it hit this way, and the overlay's deferred close then puts focus back
/// where the overlay found it.
///
/// It prevents the press's default, as gpui's own focus transfer does, so
/// an ancestor that tracks focus (a dialog's view, an app's root) doesn't
/// take focus back as the press bubbles up to it. A press in an input's
/// padding is outside the text input's own element, which is why that
/// matters.
pub fn focus_from_pointer(handle: &FocusHandle, window: &mut Window, cx: &mut App) {
    set_modality(window, Modality::Pointer, cx);
    window.focus(handle);
    window.prevent_default();
}

/// Whether a control shows the focus ring: when it is focused and focus
/// came from the keyboard, or, for a text input, whenever it is focused.
pub fn shows_ring(focused: bool, modality: Modality, text_input: bool) -> bool {
    focused && (text_input || modality == Modality::Keyboard)
}

/// What [`register`] registers through: observers, and nothing else. There
/// is no way to bind a key through it, so `init` can't, and a config
/// reload that clears and rebinds every key loses nothing of the crate's.
pub(crate) trait Observers {
    fn observe_keystrokes(
        &mut self,
        f: impl FnMut(&KeystrokeEvent, &mut Window, &mut App) + 'static,
    ) -> Subscription;
}

impl Observers for App {
    fn observe_keystrokes(
        &mut self,
        f: impl FnMut(&KeystrokeEvent, &mut Window, &mut App) + 'static,
    ) -> Subscription {
        App::observe_keystrokes(self, f)
    }
}

/// Registers the keystroke observer that sets `Keyboard`.
pub(crate) fn register(observers: &mut impl Observers) -> Subscription {
    observers.observe_keystrokes(|_, window, cx| set_modality(window, Modality::Keyboard, cx))
}

/// Registers the crate's observers in the app. Calling it again replaces
/// them, so it is safe at startup and after.
pub(crate) fn init(cx: &mut App) {
    let observer = register(cx);
    cx.default_global::<Modalities>()._observer = Some(observer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_shows_for_keyboard_focus_and_on_any_focused_input() {
        // A focused button shows the ring when focus came from the keyboard.
        assert!(shows_ring(true, Modality::Keyboard, false));
        // Clicked: focused, no ring.
        assert!(!shows_ring(true, Modality::Pointer, false));
        // A text input shows it whenever it is focused.
        assert!(shows_ring(true, Modality::Pointer, true));
        assert!(shows_ring(true, Modality::Keyboard, true));
        // Nothing unfocused shows a ring.
        assert!(!shows_ring(false, Modality::Keyboard, false));
        assert!(!shows_ring(false, Modality::Keyboard, true));
        assert!(!shows_ring(false, Modality::Pointer, true));
    }

    #[test]
    fn a_window_starts_on_the_keyboard() {
        assert_eq!(Modality::default(), Modality::Keyboard);
    }

    /// What `init` registers, recorded through the [`Observers`] seam.
    #[derive(Default)]
    struct Recorder {
        keystroke_observers: usize,
    }

    impl Observers for Recorder {
        fn observe_keystrokes(
            &mut self,
            _: impl FnMut(&KeystrokeEvent, &mut Window, &mut App) + 'static,
        ) -> Subscription {
            self.keystroke_observers += 1;
            Subscription::new(|| {})
        }
    }

    /// A config reload clears every keybinding and rebinds the app's
    /// defaults, so anything `init` bound would be lost. It registers one
    /// keystroke observer, and the seam it goes through offers no way to
    /// bind a key; the bindings live in `default_bindings()`.
    #[test]
    fn init_registers_an_observer_and_no_keys() {
        let mut recorder = Recorder::default();
        let _subscription = register(&mut recorder);
        assert_eq!(recorder.keystroke_observers, 1);
    }
}
