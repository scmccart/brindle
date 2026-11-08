//! Focus containment: Tab and shift-Tab cycle among the controls a view
//! lists, and never leave it.
//!
//! GPUI's own tab order is private and window-wide (`focus_next` walks the
//! whole window and wraps), so a view lists its stops itself through
//! [`FocusStops`], in order. The [`FocusScope`] element around its content
//! binds `FocusNext`/`FocusPrev` (Tab and shift-Tab in the `UiFocusScope`
//! context) and moves focus with the pure [`cycle`]. [`Overlay`] wraps every
//! view it shows in one, and uses the stops for the initial focus too.
//!
//! [`Overlay`]: crate::primitives::overlay::Overlay

use gpui::{
    App, Div, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, StyleRefinement, Styled,
    Window, div,
};

use crate::actions::{FocusNext, FocusPrev};

/// The focusable controls of a view, in Tab order.
pub trait FocusStops {
    /// The handles of the enabled controls, first to last. A control that
    /// isn't listed can't be reached with Tab.
    fn stops(&self, cx: &App) -> Vec<FocusHandle>;

    /// The control to focus when the view opens as an overlay, when it
    /// isn't the first stop.
    fn initial_focus(&self, _cx: &App) -> Option<FocusHandle> {
        None
    }
}

/// Which way Tab moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabDirection {
    /// Tab: to the next stop, wrapping from the last to the first.
    Forward,
    /// Shift-Tab: to the previous stop, wrapping from the first to the last.
    Backward,
}

/// The stop to focus next among `len` stops when `current` has focus, or
/// `None` when there are no stops. From outside the stops (`current` is
/// `None`), Tab goes to the first and shift-Tab to the last.
pub fn cycle(len: usize, current: Option<usize>, direction: TabDirection) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match (current, direction) {
        (None, TabDirection::Forward) => 0,
        (None, TabDirection::Backward) => len - 1,
        (Some(ix), TabDirection::Forward) => (ix + 1) % len,
        (Some(ix), TabDirection::Backward) => (ix + len - 1) % len,
    })
}

/// The element around a view's content that keeps Tab inside it. It is a
/// `div` (style it and add children as one), with the `UiFocusScope` key
/// context and the `FocusNext`/`FocusPrev` handlers, which read the view's
/// stops when the key arrives.
pub struct FocusScope<V: FocusStops + Focusable + 'static> {
    view: Entity<V>,
    base: Div,
}

impl<V: FocusStops + Focusable + 'static> FocusScope<V> {
    pub fn new(view: &Entity<V>) -> Self {
        Self { view: view.clone(), base: div() }
    }

    /// Moves focus one stop along. In a debug build, a focused handle that
    /// is neither a stop nor the view's own is a stop the view forgot to
    /// list, which Tab could never reach.
    fn step(view: &Entity<V>, direction: TabDirection, window: &mut Window, cx: &mut App) {
        let (stops, own) = {
            let view = view.read(cx);
            (view.stops(cx), view.focus_handle(cx))
        };
        let focused = window.focused(cx);
        let current = focused.as_ref().and_then(|focused| stops.iter().position(|stop| stop == focused));
        debug_assert!(
            current.is_some() || stops.is_empty() || focused.as_ref().is_none_or(|focused| *focused == own),
            "the focused handle {focused:?} in a focus scope isn't one of its stops: list it in FocusStops::stops",
        );
        if let Some(ix) = cycle(stops.len(), current, direction) {
            window.focus(&stops[ix]);
        }
    }
}

impl<V: FocusStops + Focusable + 'static> ParentElement for FocusScope<V> {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.base.extend(elements);
    }
}

impl<V: FocusStops + Focusable + 'static> Styled for FocusScope<V> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl<V: FocusStops + Focusable + 'static> IntoElement for FocusScope<V> {
    type Element = Div;

    fn into_element(self) -> Div {
        let Self { view, base } = self;
        base.key_context("UiFocusScope")
            .on_action({
                let view = view.clone();
                move |_: &FocusNext, window, cx| Self::step(&view, TabDirection::Forward, window, cx)
            })
            .on_action(move |_: &FocusPrev, window, cx| Self::step(&view, TabDirection::Backward, window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TabDirection::{Backward, Forward};

    #[test]
    fn wraps_forward_and_back() {
        assert_eq!(cycle(3, Some(0), Forward), Some(1));
        assert_eq!(cycle(3, Some(1), Forward), Some(2));
        assert_eq!(cycle(3, Some(2), Forward), Some(0));
        assert_eq!(cycle(3, Some(2), Backward), Some(1));
        assert_eq!(cycle(3, Some(1), Backward), Some(0));
        assert_eq!(cycle(3, Some(0), Backward), Some(2));
        // A single stop stays put either way.
        assert_eq!(cycle(1, Some(0), Forward), Some(0));
        assert_eq!(cycle(1, Some(0), Backward), Some(0));
    }

    #[test]
    fn from_outside_the_stops() {
        // Tab goes to the first stop and shift-Tab to the last.
        assert_eq!(cycle(3, None, Forward), Some(0));
        assert_eq!(cycle(3, None, Backward), Some(2));
        assert_eq!(cycle(1, None, Backward), Some(0));
    }

    #[test]
    fn no_stops() {
        assert_eq!(cycle(0, None, Forward), None);
        assert_eq!(cycle(0, None, Backward), None);
        assert_eq!(cycle(0, Some(0), Forward), None);
    }
}
