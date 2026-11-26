//! A part of a view drawn in a token set of its own.
//!
//! [`ThemeScope`] makes its set the active one (what
//! [`ActiveTheme::ui_theme`](crate::ActiveTheme::ui_theme) returns) while
//! its child is laid out, prepainted and painted, and restores the outer
//! set after each phase, so a scope inside another wins inside it.
//! Components read the active set in all three phases (a `RenderOnce`
//! renders in its layout, a list renders the items that come into view in
//! its prepaint, a text area paints its cursor), so all of them follow it.
//!
//! The set is swapped through an immutable borrow of the
//! [`UiTheme`](crate::UiTheme) global, so entering or leaving a scope
//! notifies no observer and redraws nothing outside it.
//!
//! What a scope can't reach: a `deferred` child, which is prepainted and
//! painted after the scope has ended, and a tooltip, which gpui draws
//! outside the window's tree. So a scope goes inside a deferred element,
//! never round one (the overlay's [`Overlay::tokens`](crate::Overlay::tokens)
//! does this), and a component's tooltip carries the set it was built in
//! (`Tooltip::tokens`, which `Button`, `Badge` and `Response` fill).

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    Window,
};

use crate::theme::{Tokens, UiTheme};

/// Draws `child` with `tokens` as the active set. Build it with
/// [`ThemeScope::new`].
pub struct ThemeScope {
    tokens: Tokens,
    child: AnyElement,
}

impl ThemeScope {
    pub fn new(tokens: Tokens, child: impl IntoElement) -> Self {
        Self { tokens, child: child.into_any_element() }
    }

    /// Runs `phase` with the scope's set active, restoring the outer one.
    fn within<R>(tokens: Tokens, cx: &mut App, phase: impl FnOnce(&mut App) -> R) -> R {
        let outer = cx.global::<UiTheme>().replace_scope(Some(tokens));
        let result = phase(cx);
        cx.global::<UiTheme>().replace_scope(outer);
        result
    }
}

impl IntoElement for ThemeScope {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for ThemeScope {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let child = &mut self.child;
        (Self::within(self.tokens, cx, |cx| child.request_layout(window, cx)), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let child = &mut self.child;
        Self::within(self.tokens, cx, |cx| child.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let child = &mut self.child;
        Self::within(self.tokens, cx, |cx| child.paint(window, cx));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{
        AppContext, Context, Entity, ParentElement, Render, Styled, TestAppContext, VisualTestContext, div, px,
    };

    use super::*;
    use crate::components::MessageScroller;
    use crate::test_support;
    use crate::theme::ActiveTheme;

    /// Which phase a probe read the active set in.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Phase {
        Layout,
        Prepaint,
        Paint,
    }

    /// What probes read: (probe, phase, the active set's background).
    pub(crate) type Readings = Rc<RefCell<Vec<(&'static str, Phase, gpui::Hsla)>>>;

    /// An element that records the active set in each of its phases.
    pub(crate) struct Probe {
        name: &'static str,
        readings: Readings,
    }

    impl Probe {
        pub(crate) fn new(name: &'static str, readings: &Readings) -> Self {
            Self { name, readings: readings.clone() }
        }

        fn record(&self, phase: Phase, cx: &App) {
            self.readings.borrow_mut().push((self.name, phase, cx.ui_theme().background));
        }
    }

    impl IntoElement for Probe {
        type Element = Self;

        fn into_element(self) -> Self {
            self
        }
    }

    impl Element for Probe {
        type RequestLayoutState = ();
        type PrepaintState = ();

        fn id(&self) -> Option<ElementId> {
            None
        }

        fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
            None
        }

        fn request_layout(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            window: &mut Window,
            cx: &mut App,
        ) -> (LayoutId, ()) {
            self.record(Phase::Layout, cx);
            let style = gpui::Style { size: gpui::size(px(10.).into(), px(10.).into()), ..Default::default() };
            (window.request_layout(style, None, cx), ())
        }

        fn prepaint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            _: Bounds<Pixels>,
            _: &mut (),
            _: &mut Window,
            cx: &mut App,
        ) {
            self.record(Phase::Prepaint, cx);
        }

        fn paint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            _: Bounds<Pixels>,
            _: &mut (),
            _: &mut (),
            _: &mut Window,
            cx: &mut App,
        ) {
            self.record(Phase::Paint, cx);
        }
    }

    /// The backgrounds the probe `name` read, one per phase it ran.
    pub(crate) fn read(readings: &Readings, name: &str) -> Vec<(Phase, gpui::Hsla)> {
        readings.borrow().iter().filter(|(n, ..)| *n == name).map(|&(_, phase, bg)| (phase, bg)).collect()
    }

    fn every_phase(bg: gpui::Hsla) -> Vec<(Phase, gpui::Hsla)> {
        vec![(Phase::Layout, bg), (Phase::Prepaint, bg), (Phase::Paint, bg)]
    }

    /// A probe before a scope, one inside it, one in a scope nested in it,
    /// one in the outer scope after the nested one, and one after both.
    struct Nested {
        readings: Readings,
        outer: Tokens,
        inner: Tokens,
    }

    impl Render for Nested {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let r = &self.readings;
            div()
                .flex()
                .child(Probe::new("before", r))
                .child(ThemeScope::new(
                    self.outer,
                    div()
                        .flex()
                        .child(Probe::new("outer", r))
                        .child(ThemeScope::new(self.inner, Probe::new("inner", r)))
                        .child(Probe::new("outer-after", r)),
                ))
                .child(Probe::new("after", r))
        }
    }

    /// The spec's "Nested scopes", and a scope leaving the window's set to
    /// what is drawn beside it: in every phase, the innermost scope's set
    /// is the active one, and the outer set comes back after each scope.
    #[test]
    fn the_innermost_scope_wins_in_every_phase_and_the_outer_set_returns() {
        let mut app: TestAppContext = test_support::app();
        let readings = Readings::default();
        let window = Tokens::neutral_light();
        let outer = Tokens { background: gpui::red(), ..Tokens::neutral_dark() };
        let inner = Tokens { background: gpui::blue(), ..Tokens::neutral_dark() };
        let r = readings.clone();
        let (_, cx) = app.add_window_view(move |_, _| Nested { readings: r, outer, inner });
        cx.run_until_parked();
        assert_eq!(read(&readings, "before"), every_phase(window.background));
        assert_eq!(read(&readings, "outer"), every_phase(outer.background));
        assert_eq!(read(&readings, "inner"), every_phase(inner.background));
        assert_eq!(read(&readings, "outer-after"), every_phase(outer.background));
        assert_eq!(read(&readings, "after"), every_phase(window.background));
        // Nothing is left set once the frame is drawn.
        cx.update(|_, cx| {
            assert_eq!(cx.scoped_theme(), None);
            assert_eq!(cx.ui_theme(), window);
            assert_eq!(cx.window_theme(), window);
        });
    }

    /// Entering and leaving a scope notifies no observer of the global.
    #[test]
    fn a_scope_notifies_no_one() {
        let mut app: TestAppContext = test_support::app();
        let notified = Rc::new(RefCell::new(0));
        let n = notified.clone();
        app.update(|cx| cx.observe_global::<UiTheme>(move |_| *n.borrow_mut() += 1).detach());
        let readings = Readings::default();
        let r = readings.clone();
        let dark = Tokens::neutral_dark();
        let (_, cx) = app.add_window_view(move |_, _| Nested { readings: r, outer: dark, inner: dark });
        cx.run_until_parked();
        assert!(!readings.borrow().is_empty(), "the frame was drawn");
        assert_eq!(*notified.borrow(), 0);
    }

    /// A scroller of twenty 50px items in a 300px view, inside a scope.
    struct Scrolled {
        scroller: Entity<MessageScroller>,
        tokens: Tokens,
    }

    impl Render for Scrolled {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            ThemeScope::new(
                self.tokens,
                div().w(px(400.)).h(px(300.)).flex().flex_col().child(self.scroller.clone()),
            )
        }
    }

    /// The spec's "A list item scrolled into view": an item the list lays
    /// out only when it comes into view, in its prepaint, is rendered with
    /// the scope's set.
    #[test]
    fn a_list_item_scrolled_into_view_takes_the_scopes_set() {
        let mut app: TestAppContext = test_support::app();
        let rendered: Rc<RefCell<Vec<(usize, gpui::Hsla)>>> = Rc::default();
        let tokens = Tokens { background: gpui::red(), ..Tokens::neutral_dark() };
        let r = rendered.clone();
        let (host, cx) = app.add_window_view(move |_, cx| {
            let scroller = cx.new(|cx| {
                MessageScroller::new(
                    20,
                    move |ix, _, cx| {
                        r.borrow_mut().push((ix, cx.ui_theme().background));
                        div().h(px(50.)).w_full().into_any_element()
                    },
                    cx,
                )
            });
            Scrolled { scroller, tokens }
        });
        let cx: &mut VisualTestContext = cx;
        cx.run_until_parked();
        assert!(!rendered.borrow().iter().any(|(ix, _)| *ix == 0), "the first item starts out of view");
        let scroller = host.read_with(cx, |h, _| h.scroller.clone());
        for _ in 0..4 {
            cx.update(|_, cx| scroller.update(cx, |s, cx| s.scroll_page(crate::PageDirection::Up, cx)));
            cx.run_until_parked();
        }
        let rendered = rendered.borrow();
        assert!(rendered.iter().any(|(ix, _)| *ix == 0), "the first item came into view");
        assert!(rendered.iter().all(|(_, bg)| *bg == tokens.background), "{rendered:?}");
    }
}
