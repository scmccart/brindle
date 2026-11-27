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
//! The binding is reached only from focus inside the scope: with nothing
//! focused gpui dispatches keys from the window's root, above it. A view
//! that scopes a whole window tracks a handle of its own inside the scope
//! and focuses it at start, as a page's body.
//!
//! [`Overlay`]: crate::primitives::overlay::Overlay

use std::rc::Rc;

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
    on_step: Option<Rc<OnStep>>,
}

type OnStep = dyn Fn(&FocusHandle, &mut Window, &mut App);

impl<V: FocusStops + Focusable + 'static> FocusScope<V> {
    pub fn new(view: &Entity<V>) -> Self {
        Self { view: view.clone(), base: div(), on_step: None }
    }

    /// Calls `on_step` with the stop Tab or shift-Tab just focused (not a
    /// focus moved any other way), so the owner can bring it into view.
    pub fn on_step(mut self, on_step: impl Fn(&FocusHandle, &mut Window, &mut App) + 'static) -> Self {
        self.on_step = Some(Rc::new(on_step));
        self
    }

    /// Moves focus one stop along. In a debug build, a focused handle that
    /// is neither a stop nor the view's own is a stop the view forgot to
    /// list, which Tab could never reach.
    fn step(view: &Entity<V>, direction: TabDirection, on_step: Option<&OnStep>, window: &mut Window, cx: &mut App) {
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
            if let Some(on_step) = on_step {
                on_step(&stops[ix], window, cx);
            }
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
        let Self { view, base, on_step } = self;
        base.key_context("UiFocusScope")
            .on_action({
                let (view, on_step) = (view.clone(), on_step.clone());
                move |_: &FocusNext, window, cx| Self::step(&view, TabDirection::Forward, on_step.as_deref(), window, cx)
            })
            .on_action(move |_: &FocusPrev, window, cx| {
                Self::step(&view, TabDirection::Backward, on_step.as_deref(), window, cx)
            })
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

    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{Context, Modifiers, Render, VisualTestContext, px};

    /// Two stops in a scope that tells `seen` where Tab moved focus.
    struct Stops {
        a: FocusHandle,
        b: FocusHandle,
        seen: Rc<RefCell<Vec<FocusHandle>>>,
    }

    impl FocusStops for Stops {
        fn stops(&self, _: &App) -> Vec<FocusHandle> {
            vec![self.a.clone(), self.b.clone()]
        }
    }

    impl Focusable for Stops {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.a.clone()
        }
    }

    impl Render for Stops {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let seen = self.seen.clone();
            let stop = |handle: &FocusHandle, name: &'static str| {
                div().debug_selector(move || name.into()).track_focus(handle).w(px(100.)).h(px(40.))
            };
            FocusScope::new(&cx.entity())
                .on_step(move |handle, _, _| seen.borrow_mut().push(handle.clone()))
                .size_full()
                .child(stop(&self.a, "stop-a"))
                .child(stop(&self.b, "stop-b"))
        }
    }

    /// `FocusScope::on_step` hears Tab's and shift-Tab's stop, and not a
    /// focus a press moved.
    #[test]
    fn tab_tells_the_owner_which_stop_it_focused() {
        let mut app = crate::test_support::app();
        let seen: Rc<RefCell<Vec<FocusHandle>>> = Rc::default();
        let told = seen.clone();
        let (view, cx) = app.add_window_view(move |_, cx| Stops { a: cx.focus_handle(), b: cx.focus_handle(), seen: told });
        let cx: &mut VisualTestContext = cx;
        let (a, b) = view.read_with(cx, |view, _| (view.a.clone(), view.b.clone()));
        cx.update(|window, _| window.focus(&a));
        cx.run_until_parked();
        cx.simulate_keystrokes("tab");
        assert_eq!(seen.borrow().as_slice(), std::slice::from_ref(&b));
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(*seen.borrow(), [b.clone(), a.clone()]);
        let at = cx.debug_bounds("stop-b").expect("drawn").center();
        cx.simulate_click(at, Modifiers::none());
        assert!(cx.update(|window, _| b.is_focused(window)), "the press focused it");
        assert_eq!(seen.borrow().len(), 2, "a press isn't a step");
    }
}
