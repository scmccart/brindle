//! `MessageScroller`: AI Elements' conversation, a scrolled list of
//! messages of different heights that sticks to its end.
//!
//! It is GPUI's `list` with `ListAlignment::Bottom`, so it starts at its
//! end and lays out only the items in view and the overdraw around them.
//! The owner keeps the items and tells the scroller what changed
//! ([`MessageScroller::splice`], as on a `ListState`); the scroller asks the
//! owner's `render_item` for an item by index when it needs one.
//!
//! While it is at its end ([`Follow::AtEnd`]) it stays there as items are
//! added or grow: the list keeps no scroll position then, and lays out
//! from the bottom. Once the user scrolls away ([`Follow::Away`]) the list
//! keeps its position, so the items in view stay where they are as items
//! are added below, and a scroll button (lucide `arrow-down`) shows; it
//! scrolls to the end and follows again. [`MessageScroller::scroll_page`]
//! is the owner's page keys' way in. The button takes no focus: its
//! keyboard path is the owner's (a "scroll to bottom" action calling
//! [`MessageScroller::scroll_to_end`]).
//!
//! A top row (an agent zone's "Earlier messages") is an item of its own
//! before the owner's, shown while the owner says so
//! ([`MessageScroller::show_top_row`]) and rendered by the owner's
//! `render_top` ([`MessageScroller::with_top_row`]); the owner's indices
//! don't count it. Items spliced in at the owner's start while the row is
//! the first item in view re-anchor the view on the item that was first,
//! which moves it by at most the row's height. GPUI's list draws its top
//! padding above whichever item is at its scroll position, not above its
//! first item, so the row can't be drawn there. Whether the row was in view
//! in the last frame is [`MessageScroller::top_row_in_view`], and a change
//! of it calls the owner's [`MessageScroller::on_top_row_view`] callback.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Context, IntoElement, ListAlignment, ListOffset, ListState, ParentElement, Pixels,
    Render, Styled, Window, canvas, div, list, px,
};

use crate::components::Button;
use crate::icon::Icon;
use crate::style::{ButtonSize, ButtonVariant};

/// Whether a message scroller follows its end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Follow {
    /// At the end: it stays there as items are added or grow.
    #[default]
    AtEnd,
    /// Scrolled away: the items in view stay put, and the scroll button
    /// shows.
    Away,
}

impl Follow {
    /// After a scroll the list reports: at its end when the list let go of
    /// its scroll position, which a bottom-aligned list does exactly when
    /// it is scrolled to its end.
    pub fn after_scroll(is_scrolled: bool) -> Follow {
        if is_scrolled { Follow::Away } else { Follow::AtEnd }
    }

    /// After a frame: away from the end, but showing the last item's end
    /// (`last_bottom`, when the last item was drawn) at or above the view's
    /// bottom, it is at the end again (a page key brought it there).
    pub fn after_draw(self, last_bottom: Option<f32>, view_bottom: f32) -> Follow {
        match (self, last_bottom) {
            (Follow::Away, Some(bottom)) if bottom <= view_bottom + 0.5 => Follow::AtEnd,
            (Follow::Away, Some(_) | None) | (Follow::AtEnd, _) => self,
        }
    }
}

/// Which way a page key scrolls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageDirection {
    Up,
    Down,
}

/// What a page key does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageScroll {
    /// Scroll by this many pixels (up is negative), away from the end.
    By(f32),
    /// Scroll to the end, and follow it.
    ToEnd,
    /// Nothing to scroll that way.
    Nothing,
}

/// A page key's scroll: `page` pixels, or what is left that way. `above`
/// and `below` are how much of the items is hidden above and below the
/// view, `None` when that isn't known (an item there isn't in view, so it
/// is more than nothing). Down to within a page of the end goes to the end
/// and follows it.
pub fn page_scroll(dir: PageDirection, page: f32, above: Option<f32>, below: Option<f32>) -> PageScroll {
    match dir {
        PageDirection::Down => match below {
            Some(below) if below <= page => PageScroll::ToEnd,
            Some(_) | None => PageScroll::By(page),
        },
        PageDirection::Up => match above {
            Some(above) if above <= 0.5 => PageScroll::Nothing,
            Some(above) => PageScroll::By(-above.min(page)),
            None => PageScroll::By(-page),
        },
    }
}

type RenderItem = dyn FnMut(usize, &mut Window, &mut App) -> AnyElement;
type RenderTop = dyn FnMut(&mut Window, &mut App) -> AnyElement;
type TopRowView = dyn Fn(bool, &mut App);

/// Where the last frame drew the view and the items in it.
#[derive(Default)]
struct Drawn {
    viewport: Option<Bounds<Pixels>>,
    /// The items drawn in view, by index, in the order drawn.
    items: Vec<(usize, Bounds<Pixels>)>,
}

/// The scrolled list. Create it with the item count and the owner's item
/// renderer, render it filling the room it is given, and tell it of
/// changes with [`splice`](Self::splice).
pub struct MessageScroller {
    state: ListState,
    follow: Follow,
    render_item: Rc<RefCell<Box<RenderItem>>>,
    drawn: Rc<RefCell<Drawn>>,
    /// The owner's top row, when it has one, and whether it is shown (the
    /// list's item 0).
    render_top: Option<Rc<RefCell<Box<RenderTop>>>>,
    top_shown: bool,
    /// Whether the top row was in view in the last frame.
    top_in_view: bool,
    on_top_row_view: Option<Rc<TopRowView>>,
}

/// How far beyond the view items are laid out ahead of a scroll.
const OVERDRAW: Pixels = px(512.);

impl MessageScroller {
    pub fn new(
        count: usize,
        render_item: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = ListState::new(count, ListAlignment::Bottom, OVERDRAW);
        let scroller = cx.weak_entity();
        state.set_scroll_handler(move |event, _, cx| {
            let follow = Follow::after_scroll(event.is_scrolled);
            scroller
                .update(cx, |this, cx| {
                    if this.follow != follow {
                        this.follow = follow;
                        cx.notify();
                    }
                })
                .ok();
        });
        Self {
            state,
            follow: Follow::AtEnd,
            render_item: Rc::new(RefCell::new(Box::new(render_item))),
            drawn: Rc::default(),
            render_top: None,
            top_shown: false,
            top_in_view: false,
            on_top_row_view: None,
        }
    }

    /// The owner's top row, rendered by `render_top` while shown
    /// ([`show_top_row`](Self::show_top_row)).
    pub fn with_top_row(mut self, render_top: impl FnMut(&mut Window, &mut App) -> AnyElement + 'static) -> Self {
        self.render_top = Some(Rc::new(RefCell::new(Box::new(render_top))));
        self
    }

    /// Calls `on_view` with whether the top row is in view, after each
    /// frame that changed it.
    pub fn on_top_row_view(mut self, on_view: impl Fn(bool, &mut App) + 'static) -> Self {
        self.on_top_row_view = Some(Rc::new(on_view));
        self
    }

    /// The list's index of the owner's item `ix`.
    fn list_ix(&self, ix: usize) -> usize {
        ix + usize::from(self.top_shown)
    }

    /// Shows or hides the top row (a no-op without one). Shown, it comes
    /// in before the items, out of view unless the list shows its start;
    /// hidden while it was the first item in view, the view moves to the
    /// first item's top.
    pub fn show_top_row(&mut self, shown: bool, cx: &mut Context<Self>) {
        let shown = shown && self.render_top.is_some();
        if shown == self.top_shown {
            return;
        }
        if shown {
            self.state.splice(0..0, 1);
        } else {
            self.state.splice(0..1, 0);
            self.top_in_view = false;
        }
        self.top_shown = shown;
        cx.notify();
    }

    /// The top row changed (its height may have): it is measured again,
    /// the view kept where it was.
    pub fn refresh_top_row(&mut self, cx: &mut Context<Self>) {
        if !self.top_shown {
            return;
        }
        let top = self.state.logical_scroll_top();
        self.state.splice(0..1, 1);
        if self.follow == Follow::Away && top.item_ix == 0 {
            self.state.scroll_to(top);
        }
        cx.notify();
    }

    /// Whether the top row was in view in the last frame.
    pub fn top_row_in_view(&self) -> bool {
        self.top_in_view
    }

    /// The items in `old` were replaced by `count` items (an item that
    /// changed is replaced by itself, so it is measured again). Items
    /// inserted at the start while the top row is the first item in view
    /// put the item that was first at the view's top (it moves up by what
    /// showed of the row), so the inserted items and the row are above.
    pub fn splice(&mut self, old: Range<usize>, count: usize, cx: &mut Context<Self>) {
        let top = self.state.logical_scroll_top();
        let anchor = self.top_shown && self.follow == Follow::Away && top.item_ix == 0 && old == (0..0) && count > 0;
        self.state.splice(self.list_ix(old.start)..self.list_ix(old.end), count);
        if anchor {
            self.state.scroll_to(ListOffset { item_ix: self.list_ix(count), offset_in_item: px(0.) });
        }
        cx.notify();
    }

    /// Every item replaced by `count` new ones, scrolled to the end.
    pub fn reset(&mut self, count: usize, cx: &mut Context<Self>) {
        self.state.reset(self.list_ix(count));
        self.follow = Follow::AtEnd;
        self.top_in_view = false;
        cx.notify();
    }

    /// The owner's items (the top row isn't one).
    pub fn item_count(&self) -> usize {
        self.state.item_count() - usize::from(self.top_shown)
    }

    pub fn follow(&self) -> Follow {
        self.follow
    }

    /// The list's scroll position (the end's while it follows).
    pub fn scroll_top(&self) -> ListOffset {
        self.state.logical_scroll_top()
    }

    /// Scrolls to the end and follows it.
    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.state.scroll_to(ListOffset { item_ix: self.state.item_count(), offset_in_item: px(0.) });
        self.follow = Follow::AtEnd;
        cx.notify();
    }

    /// A page up or down, from what the last frame showed: a view's height
    /// less a line's worth, so a line stays in view across the page.
    pub fn scroll_page(&mut self, dir: PageDirection, cx: &mut Context<Self>) {
        let (top, above, below, page) = {
            let drawn = self.drawn.borrow();
            let Some(viewport) = drawn.viewport else { return };
            let count = self.state.item_count();
            let top = drawn.items.iter().find(|(_, b)| b.bottom() > viewport.top()).copied();
            let above = drawn
                .items
                .iter()
                .find(|(ix, _)| *ix == 0)
                .map(|(_, b)| f32::from(viewport.top() - b.top()).max(0.));
            let below = drawn
                .items
                .iter()
                .find(|(ix, _)| *ix + 1 == count)
                .map(|(_, b)| f32::from(b.bottom() - viewport.bottom()).max(0.));
            let page = (f32::from(viewport.size.height) - 24.).max(24.);
            (top, above, below, page)
        };
        match page_scroll(dir, page, above, below) {
            PageScroll::Nothing => {}
            PageScroll::ToEnd => self.scroll_to_end(cx),
            PageScroll::By(distance) => {
                // From where the view shows now: a list at its end keeps no
                // position of its own to scroll from.
                if let (Some((ix, bounds)), Some(viewport)) = (top, self.drawn.borrow().viewport) {
                    self.state.scroll_to(ListOffset { item_ix: ix, offset_in_item: viewport.top() - bounds.top() });
                }
                self.state.scroll_by(px(distance));
                self.follow = Follow::Away;
                cx.notify();
            }
        }
    }
}

impl Render for MessageScroller {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drawn.borrow_mut().items.clear();
        let render_item = self.render_item.clone();
        let render_top = self.render_top.clone().filter(|_| self.top_shown);
        let drawn = self.drawn.clone();
        let items = list(self.state.clone(), move |ix, window, cx| {
            let item = match (&render_top, ix) {
                (Some(top), 0) => (top.borrow_mut())(window, cx),
                (Some(_), ix) => (render_item.borrow_mut())(ix - 1, window, cx),
                (None, ix) => (render_item.borrow_mut())(ix, window, cx),
            };
            let drawn = drawn.clone();
            // Each item drawn in view records where, for the page keys.
            div()
                .relative()
                .child(item)
                .child(
                    canvas(move |bounds, _, _| drawn.borrow_mut().items.push((ix, bounds)), |_, _, _, _| {})
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                )
                .into_any_element()
        })
        .size_full();
        let drawn = self.drawn.clone();
        let scroller = cx.weak_entity();
        let follow = self.follow;
        let count = self.state.item_count();
        let after_draw = scroller.clone();
        let top_shown = self.top_shown;
        let top_was_in_view = self.top_in_view;
        let on_top_row_view = self.on_top_row_view.clone();
        div()
            .relative()
            .size_full()
            .min_h_0()
            .child(items)
            // After the items (drawn first), the view: where it is, and
            // whether a page key brought the list back to its end.
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let (last_bottom, top_in_view) = {
                            let mut drawn = drawn.borrow_mut();
                            drawn.viewport = Some(bounds);
                            let last_bottom = drawn
                                .items
                                .iter()
                                .find(|(ix, _)| *ix + 1 == count)
                                .map(|(_, b)| f32::from(b.bottom()));
                            let top_in_view = top_shown
                                && drawn.items.iter().any(|(ix, b)| *ix == 0 && b.bottom() > bounds.top());
                            (last_bottom, top_in_view)
                        };
                        if top_in_view != top_was_in_view {
                            after_draw
                                .update(cx, |this, _| this.top_in_view = top_in_view)
                                .ok();
                            if let Some(on_view) = on_top_row_view.clone() {
                                cx.defer(move |cx| on_view(top_in_view, cx));
                            }
                        }
                        if follow.after_draw(last_bottom, f32::from(bounds.bottom())) != follow {
                            after_draw.update(cx, |this, cx| this.scroll_to_end(cx)).ok();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .children((self.follow == Follow::Away).then(|| {
                div().absolute().bottom_4().left_0().right_0().flex().justify_center().child(
                    Button::icon_only("message-scroller-end", Icon::ArrowDown)
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::IconSm)
                        .rounded_full()
                        .on_click(move |_, _, cx| {
                            scroller.update(cx, |this, cx| this.scroll_to_end(cx)).ok();
                        }),
                )
            }))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext, Entity, Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase, VisualTestContext, point};

    use super::*;
    use crate::test_support;

    #[test]
    fn follow_comes_from_the_lists_scroll_position() {
        assert_eq!(Follow::after_scroll(false), Follow::AtEnd);
        assert_eq!(Follow::after_scroll(true), Follow::Away);
        assert_eq!(Follow::default(), Follow::AtEnd);
    }

    #[test]
    fn page_keys_scroll_a_page_or_to_the_end() {
        use PageDirection::{Down, Up};
        // Down within a page of the end goes there and follows it.
        assert_eq!(page_scroll(Down, 300., None, Some(120.)), PageScroll::ToEnd);
        assert_eq!(page_scroll(Down, 300., None, Some(0.)), PageScroll::ToEnd);
        assert_eq!(page_scroll(Down, 300., None, Some(900.)), PageScroll::By(300.));
        assert_eq!(page_scroll(Down, 300., Some(0.), None), PageScroll::By(300.));
        // Up a page, what is left, or nothing at the top.
        assert_eq!(page_scroll(Up, 300., None, None), PageScroll::By(-300.));
        assert_eq!(page_scroll(Up, 300., Some(120.), None), PageScroll::By(-120.));
        assert_eq!(page_scroll(Up, 300., Some(0.), Some(50.)), PageScroll::Nothing);
    }

    /// Twenty items 50px tall in a 300px view, each growable.
    struct Host {
        scroller: Entity<MessageScroller>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(400.)).h(px(300.)).flex().flex_col().child(self.scroller.clone())
        }
    }

    fn host(heights: Rc<RefCell<Vec<f32>>>) -> (Entity<MessageScroller>, &'static mut VisualTestContext) {
        let mut app = test_support::app();
        let (host, cx) = app.add_window_view(move |_, cx| {
            let count = heights.borrow().len();
            let heights = heights.clone();
            let scroller = cx.new(|cx| {
                MessageScroller::new(
                    count,
                    move |ix, _, _| div().h(px(heights.borrow()[ix])).w_full().into_any_element(),
                    cx,
                )
            });
            Host { scroller }
        });
        let scroller = host.read_with(cx, |host, _| host.scroller.clone());
        let cx: &'static mut VisualTestContext = Box::leak(Box::new(cx.clone()));
        cx.run_until_parked();
        (scroller, cx)
    }

    fn item_bounds(scroller: &Entity<MessageScroller>, ix: usize, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
        scroller.read_with(cx, |s, _| s.drawn.borrow().items.iter().find(|(i, _)| *i == ix).map(|(_, b)| *b))
    }

    fn redraw(scroller: &Entity<MessageScroller>, cx: &mut VisualTestContext) {
        cx.update(|_, cx| scroller.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
    }

    /// The spec's "Growing last item": at the end, the last item grows by
    /// ten lines and its end stays in view.
    #[test]
    fn a_growing_last_item_stays_in_view() {
        let heights = Rc::new(RefCell::new(vec![50.; 20]));
        let (scroller, cx) = host(heights.clone());
        let last = item_bounds(&scroller, 19, cx).expect("the end shows first");
        assert_eq!(f32::from(last.bottom()), 300.);
        heights.borrow_mut()[19] = 50. + 10. * 20.;
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.splice(19..20, 1, cx)));
        cx.run_until_parked();
        let last = item_bounds(&scroller, 19, cx).expect("the last item in view");
        assert_eq!((f32::from(last.bottom()), f32::from(last.size.height)), (300., 250.));
        assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::AtEnd);
    }

    /// The spec's "Scrolled away": scrolled up, an item added at the end
    /// moves nothing in view, and the scroll button shows; it goes back to
    /// the end.
    #[test]
    fn an_item_added_while_away_leaves_the_view_alone() {
        let heights = Rc::new(RefCell::new(vec![50.; 20]));
        let (scroller, cx) = host(heights.clone());
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(200.), px(150.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(120.))),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        redraw(&scroller, cx);
        assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::Away);
        let offset = |s: &MessageScroller| {
            let top = s.scroll_top();
            (top.item_ix, top.offset_in_item)
        };
        let top = scroller.read_with(cx, |s, _| offset(s));
        let shown = item_bounds(&scroller, 15, cx);
        heights.borrow_mut().push(80.);
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.splice(20..20, 1, cx)));
        cx.run_until_parked();
        assert_eq!(scroller.read_with(cx, |s, _| offset(s)), top);
        assert_eq!(item_bounds(&scroller, 15, cx), shown);
        assert!(shown.is_some());
        assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::Away);
        // The button: a press scrolls to the end, which follows again.
        let button = point(px(200.), px(300. - 16. - 14.));
        cx.simulate_click(button, Modifiers::none());
        redraw(&scroller, cx);
        assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::AtEnd);
        assert_eq!(item_bounds(&scroller, 20, cx).map(|b| f32::from(b.bottom())), Some(300.));
    }

    /// Page up leaves the end and page down comes back to it.
    #[test]
    fn page_keys_leave_and_reach_the_end() {
        let heights = Rc::new(RefCell::new(vec![50.; 20]));
        let (scroller, cx) = host(heights);
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.scroll_page(PageDirection::Up, cx)));
        redraw(&scroller, cx);
        assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::Away);
        // 276px up from the end: the last item is out of view, and the
        // one that was at the top is at the bottom.
        assert_eq!(item_bounds(&scroller, 19, cx), None);
        let fifteenth = item_bounds(&scroller, 14, cx).map(|b| f32::from(b.top()));
        assert_eq!(fifteenth, Some(276.));
        // Down a page shows the end again, and follows it.
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.scroll_page(PageDirection::Down, cx)));
        cx.run_until_parked();
        assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::AtEnd);
        assert_eq!(item_bounds(&scroller, 19, cx).map(|b| f32::from(b.bottom())), Some(300.));
    }

    #[test]
    fn a_page_key_that_reaches_the_end_follows_it() {
        assert_eq!(Follow::Away.after_draw(Some(300.), 300.), Follow::AtEnd);
        assert_eq!(Follow::Away.after_draw(Some(299.), 300.), Follow::AtEnd);
        assert_eq!(Follow::Away.after_draw(Some(420.), 300.), Follow::Away);
        assert_eq!(Follow::Away.after_draw(None, 300.), Follow::Away);
        assert_eq!(Follow::AtEnd.after_draw(Some(420.), 300.), Follow::AtEnd);
    }

    /// A 40px top row over [`host`]'s items, shown; gives the scroller and
    /// the top row's view changes as the callback saw them.
    fn host_with_top(
        heights: Rc<RefCell<Vec<f32>>>,
    ) -> (Entity<MessageScroller>, Rc<RefCell<Vec<bool>>>, &'static mut VisualTestContext) {
        let mut app = test_support::app();
        let seen: Rc<RefCell<Vec<bool>>> = Rc::default();
        let views = seen.clone();
        let (host, cx) = app.add_window_view(move |_, cx| {
            let count = heights.borrow().len();
            let heights = heights.clone();
            let views = views.clone();
            let scroller = cx.new(|cx| {
                let mut scroller = MessageScroller::new(
                    count,
                    move |ix, _, _| div().h(px(heights.borrow()[ix])).w_full().into_any_element(),
                    cx,
                )
                .with_top_row(|_, _| div().h(px(40.)).w_full().into_any_element())
                .on_top_row_view(move |in_view, _| views.borrow_mut().push(in_view));
                scroller.show_top_row(true, cx);
                scroller
            });
            Host { scroller }
        });
        let scroller = host.read_with(cx, |host, _| host.scroller.clone());
        let cx: &'static mut VisualTestContext = Box::leak(Box::new(cx.clone()));
        cx.run_until_parked();
        (scroller, seen, cx)
    }

    /// Items spliced in at the owner's start, with heights of 50px.
    fn prepend(scroller: &Entity<MessageScroller>, heights: &Rc<RefCell<Vec<f32>>>, count: usize, cx: &mut VisualTestContext) {
        heights.borrow_mut().splice(0..0, std::iter::repeat_n(50., count));
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.splice(0..0, count, cx)));
        cx.run_until_parked();
    }

    fn scroll_to(scroller: &Entity<MessageScroller>, top: ListOffset, cx: &mut VisualTestContext) {
        cx.update(|_, cx| {
            scroller.update(cx, |s, cx| {
                s.state.scroll_to(top);
                s.follow = Follow::Away;
                cx.notify();
            })
        });
        cx.run_until_parked();
    }

    /// Task 5.1's spike, with its outcome: GPUI's list draws its top padding
    /// above whichever item is at its scroll position, so the top row is the
    /// list's item 0. Items spliced in at the owner's start keep every item
    /// in view where it was, following the end or away from it, with or
    /// without a last item standing for the working row; with the row
    /// itself in view, the item that was first goes to the view's top,
    /// moving by at most the row's height.
    #[test]
    fn items_spliced_in_at_the_start_keep_the_view() {
        for working in [false, true] {
            let mut items = vec![50.; 20];
            if working {
                items.push(30.);
            }
            let last = items.len();
            let heights = Rc::new(RefCell::new(items));
            let (scroller, _, cx) = host_with_top(heights.clone());
            assert_eq!(scroller.read_with(cx, |s, _| s.item_count()), last);

            // Following the end: the last item stays at the bottom.
            let end = item_bounds(&scroller, last, cx).expect("the end shows first");
            prepend(&scroller, &heights, 3, cx);
            assert_eq!(item_bounds(&scroller, last + 3, cx), Some(end), "working: {working}");
            assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::AtEnd);

            // Away, the row out of view: nothing in view moves.
            scroll_to(&scroller, ListOffset { item_ix: 8, offset_in_item: px(20.) }, cx);
            let shown = item_bounds(&scroller, 10, cx).expect("item 10 in view");
            assert!(!scroller.read_with(cx, |s, _| s.top_row_in_view()));
            prepend(&scroller, &heights, 3, cx);
            assert_eq!(item_bounds(&scroller, 13, cx), Some(shown), "working: {working}");
            assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::Away);

            // Away, the row in view (scrolled 10px into it): the first item
            // goes to the top, 30px up, and the row and the new items are
            // above the view.
            scroll_to(&scroller, ListOffset { item_ix: 0, offset_in_item: px(10.) }, cx);
            assert!(scroller.read_with(cx, |s, _| s.top_row_in_view()));
            let first = item_bounds(&scroller, 1, cx).expect("the first item in view");
            assert_eq!(f32::from(first.top()), 30.);
            prepend(&scroller, &heights, 3, cx);
            let moved = item_bounds(&scroller, 4, cx).expect("the item that was first");
            assert_eq!((f32::from(moved.top()), moved.size), (0., first.size), "working: {working}");
            assert_eq!(item_bounds(&scroller, 0, cx), None);
            assert!(!scroller.read_with(cx, |s, _| s.top_row_in_view()));
        }
    }

    /// Task 5.2: a page up from the first item's top brings the top row
    /// into view, and the owner is told when it comes and goes.
    #[test]
    fn a_page_up_reaches_the_top_row() {
        let heights = Rc::new(RefCell::new(vec![50.; 20]));
        let (scroller, seen, cx) = host_with_top(heights);
        scroll_to(&scroller, ListOffset { item_ix: 1, offset_in_item: px(0.) }, cx);
        assert!(!scroller.read_with(cx, |s, _| s.top_row_in_view()));
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.scroll_page(PageDirection::Up, cx)));
        redraw(&scroller, cx);
        assert!(scroller.read_with(cx, |s, _| s.top_row_in_view()));
        assert_eq!(item_bounds(&scroller, 0, cx).map(|b| f32::from(b.top())), Some(0.));
        // A page down takes it out of view again; hiding it takes it out
        // of the list.
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.scroll_page(PageDirection::Down, cx)));
        redraw(&scroller, cx);
        assert!(!scroller.read_with(cx, |s, _| s.top_row_in_view()));
        assert_eq!(*seen.borrow(), [true, false]);
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.show_top_row(false, cx)));
        redraw(&scroller, cx);
        assert_eq!(scroller.read_with(cx, |s, _| (s.item_count(), s.state.item_count())), (20, 20));
    }
}
