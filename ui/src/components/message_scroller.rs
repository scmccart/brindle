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
        }
    }

    /// The items in `old` were replaced by `count` items (an item that
    /// changed is replaced by itself, so it is measured again).
    pub fn splice(&mut self, old: Range<usize>, count: usize, cx: &mut Context<Self>) {
        self.state.splice(old, count);
        cx.notify();
    }

    /// Every item replaced by `count` new ones, scrolled to the end.
    pub fn reset(&mut self, count: usize, cx: &mut Context<Self>) {
        self.state.reset(count);
        self.follow = Follow::AtEnd;
        cx.notify();
    }

    pub fn item_count(&self) -> usize {
        self.state.item_count()
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
        let drawn = self.drawn.clone();
        let items = list(self.state.clone(), move |ix, window, cx| {
            let item = (render_item.borrow_mut())(ix, window, cx);
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
                        let last_bottom = {
                            let mut drawn = drawn.borrow_mut();
                            drawn.viewport = Some(bounds);
                            drawn
                                .items
                                .iter()
                                .find(|(ix, _)| *ix + 1 == count)
                                .map(|(_, b)| f32::from(b.bottom()))
                        };
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
}
