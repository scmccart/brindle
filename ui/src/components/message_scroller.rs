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
//! don't count it. GPUI's list draws its top padding above whichever item
//! is at its scroll position, not above its first item, so the row can't
//! be drawn there. Whether the row was in view in the last frame is
//! [`MessageScroller::top_row_in_view`], and a change of it calls the
//! owner's [`MessageScroller::on_top_row_view`] callback.
//!
//! Where the view is, as items change (OpenSpec change
//! `agents-zone-focus-and-scroll`, decision 2), in list indices (the top
//! row, while shown, is item 0):
//! 1. Following the end, the list keeps no position and lays out from the
//!    bottom: any splice or remeasure keeps the end in view.
//! 2. Away, a splice wholly before the item at the top moves its index by
//!    the change in count and keeps its offset.
//! 3. Away, a splice wholly after it leaves it.
//! 4. Away, a splice that replaces it keeps its index while that is among
//!    the new items, else takes the last new one, and keeps the offset, so
//!    an item growing at the top keeps its lines in view.
//! 5. Away, a splice that removes it puts the next item at the top, whole.
//! 6. An insert at the owner's start leaves whether the top row is in view
//!    to the next frame drawn. Away with the row the first item in view,
//!    the item that was first goes to the view's top (moving by at most the
//!    row's height), the row and the new items above it.
//! 7. An item at the top that shrank to its offset or less gives the top to
//!    the next item, once a frame has measured it.
//! 8. [`MessageScroller::remeasure_all`] measures every item again; away,
//!    the item at the top stays there at the same fraction of its height,
//!    once a frame has measured it.
//!
//! Rules 2 to 5 are [`top_after_splice`], 7 and 8 [`settle_anchor`] (a
//! settle that moves the view draws one more frame), 6 [`inserts_at_start`].

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

/// The item at the view's top and how far into it the view starts, in
/// pixels (a list index and offset, as gpui's `ListOffset`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Top {
    pub item: usize,
    pub offset: f32,
}

impl Top {
    fn of(offset: ListOffset) -> Self {
        Self { item: offset.item_ix, offset: f32::from(offset.offset_in_item) }
    }

    fn list_offset(self) -> ListOffset {
        ListOffset { item_ix: self.item, offset_in_item: px(self.offset) }
    }
}

/// Where the view's top goes when the items in `old` (list indices) are
/// replaced by `count` items, while scrolled away (rules 2 to 5): a splice
/// wholly before the top item moves its index by the change in count and
/// keeps its offset, and one wholly after it leaves it; a splice that
/// replaces the top item keeps its index while that is among the new items,
/// else takes the last new item, and keeps the offset; one that removes it
/// puts the item after the removed run at the top.
pub fn top_after_splice(top: Top, old: Range<usize>, count: usize) -> Top {
    if old.end <= top.item {
        Top { item: top.item - old.len() + count, ..top }
    } else if old.start > top.item {
        top
    } else if count > 0 {
        Top { item: top.item.min(old.start + count - 1), ..top }
    } else {
        Top { item: old.start, offset: 0. }
    }
}

/// What the view's top waits on until a frame has measured its item
/// again: that the item didn't shrink under the offset (a splice that
/// replaced it, rule 7), or the offset at the same fraction of the item's
/// new height (every item measured again, rule 8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    Offset { item: usize, offset: f32 },
    Fraction { item: usize, fraction: f32 },
}

impl Anchor {
    fn item(self) -> usize {
        match self {
            Anchor::Offset { item, .. } | Anchor::Fraction { item, .. } => item,
        }
    }

    fn with_item(self, item: usize) -> Self {
        match self {
            Anchor::Offset { offset, .. } => Anchor::Offset { item, offset },
            Anchor::Fraction { fraction, .. } => Anchor::Fraction { item, fraction },
        }
    }
}

/// The top once the anchor's item measures `height`: the item after it
/// when it shrank to its offset or less, the fraction's offset in it, or
/// nothing to change.
pub fn settle_anchor(anchor: Anchor, height: f32) -> Option<Top> {
    match anchor {
        Anchor::Offset { item, offset } => (height <= offset).then_some(Top { item: item + 1, offset: 0. }),
        Anchor::Fraction { item, fraction } => Some(Top { item, offset: fraction * height }),
    }
}

/// Whether a splice of the owner's items `old` by `count` inserts items at
/// the owner's start (rule 6): it starts at the first item and adds some.
pub fn inserts_at_start(old: &Range<usize>, count: usize) -> bool {
    old.start == 0 && count > old.len()
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
    /// What the view's top waits on from the next frame (rules 7 and 8),
    /// while scrolled away.
    anchor: Option<Anchor>,
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
            anchor: None,
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
            self.splice_list(0..0, 1);
        } else {
            self.splice_list(0..1, 0);
            self.top_in_view = false;
        }
        self.top_shown = shown;
        cx.notify();
    }

    /// The top row changed (its height may have): it is measured again,
    /// the view kept where it was (rule 4).
    pub fn refresh_top_row(&mut self, cx: &mut Context<Self>) {
        if !self.top_shown {
            return;
        }
        self.splice_list(0..1, 1);
        cx.notify();
    }

    /// The list's items `old` replaced by `count`, the view's top kept by
    /// rules 2 to 5 while scrolled away (gpui's own rewrite of it, which
    /// puts a replaced top item's start at the top, is never relied on).
    /// A replaced top item waits on the next frame for rule 7; a pending
    /// anchor follows its item.
    fn splice_list(&mut self, old: Range<usize>, count: usize) {
        let was = Top::of(self.state.logical_scroll_top());
        self.state.splice(old.clone(), count);
        if self.follow == Follow::AtEnd {
            self.anchor = None;
            return;
        }
        let top = top_after_splice(was, old.clone(), count);
        self.state.scroll_to(top.list_offset());
        let replaced = old.contains(&was.item);
        self.anchor = match (replaced, count > 0, self.anchor) {
            // Every item measured again, and this one replaced once more:
            // still the fraction of its height last drawn.
            (true, true, Some(anchor @ Anchor::Fraction { .. })) if anchor.item() == was.item => {
                Some(anchor.with_item(top.item))
            }
            (true, true, _) => Some(Anchor::Offset { item: top.item, offset: top.offset }),
            // The top item removed: the next is at the top, whole.
            (true, false, _) => None,
            (false, _, Some(anchor)) if anchor.item() == was.item => Some(anchor.with_item(top.item)),
            (false, _, Some(_) | None) => None,
        };
    }

    /// Whether the top row was in view in the last frame.
    pub fn top_row_in_view(&self) -> bool {
        self.top_in_view
    }

    /// The owner's items in `old` were replaced by `count` items (an item
    /// that changed is replaced by itself, so it is measured again), the
    /// view kept by the module's rules. Items inserted at the start while
    /// the top row is the first item in view put the item that was first at
    /// the view's top (it moves up by what showed of the row), so the
    /// inserted items and the row are above. Any insert at the start leaves
    /// whether the row is in view to the next frame (rule 6).
    pub fn splice(&mut self, old: Range<usize>, count: usize, cx: &mut Context<Self>) {
        let top = self.state.logical_scroll_top();
        let row_first = self.top_shown && self.follow == Follow::Away && top.item_ix == 0 && old == (0..0) && count > 0;
        if row_first {
            self.state.splice(self.list_ix(0)..self.list_ix(0), count);
            self.state.scroll_to(ListOffset { item_ix: self.list_ix(count), offset_in_item: px(0.) });
            self.anchor = None;
        } else {
            self.splice_list(self.list_ix(old.start)..self.list_ix(old.end), count);
        }
        if inserts_at_start(&old, count) {
            self.top_in_view = false;
        }
        cx.notify();
    }

    /// Every item, the top row included, measured again (rule 8): their
    /// heights changed with something only the owner knows (a text size, a
    /// font). Scrolled away, the item at the top stays there at the same
    /// fraction of its height once the next frame has measured it; at the
    /// end, the end stays in view.
    pub fn remeasure_all(&mut self, cx: &mut Context<Self>) {
        let count = self.state.item_count();
        let was = Top::of(self.state.logical_scroll_top());
        self.state.splice(0..count, count);
        if self.follow == Follow::Away {
            let drawn = self.drawn.borrow().items.iter().find(|(ix, _)| *ix == was.item).map(|(_, b)| f32::from(b.size.height));
            self.state.scroll_to(was.list_offset());
            self.anchor = Some(match drawn {
                Some(height) if height > 0. => Anchor::Fraction { item: was.item, fraction: was.offset / height },
                // Not drawn in the last frame: its offset, kept.
                Some(_) | None => Anchor::Offset { item: was.item, offset: was.offset },
            });
        } else {
            self.anchor = None;
        }
        cx.notify();
    }

    /// After a frame: the pending anchor, settled against the height its
    /// item was drawn at. A settle that moves the view draws again.
    fn settle(&mut self, cx: &mut Context<Self>) {
        let Some(anchor) = self.anchor.take() else { return };
        if self.follow == Follow::AtEnd {
            return;
        }
        let now = Top::of(self.state.logical_scroll_top());
        if now.item != anchor.item() {
            return;
        }
        let height = self.drawn.borrow().items.iter().find(|(ix, _)| *ix == now.item).map(|(_, b)| f32::from(b.size.height));
        let Some(height) = height else { return };
        // A replaced item is checked at the offset the view is at now.
        let anchor = match anchor {
            Anchor::Offset { item, .. } => Anchor::Offset { item, offset: now.offset },
            fraction @ Anchor::Fraction { .. } => fraction,
        };
        if let Some(top) = settle_anchor(anchor, height).filter(|top| *top != now) {
            self.state.scroll_to(top.list_offset());
            cx.notify();
        }
    }

    /// Every item replaced by `count` new ones, scrolled to the end.
    pub fn reset(&mut self, count: usize, cx: &mut Context<Self>) {
        self.state.reset(self.list_ix(count));
        self.follow = Follow::AtEnd;
        self.top_in_view = false;
        self.anchor = None;
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
        self.anchor = None;
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
        let settling = self.anchor.is_some();
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
                        } else if settling {
                            // After the frame, so its notify draws another.
                            let settle = after_draw.clone();
                            cx.defer(move |cx| {
                                settle.update(cx, |this, cx| this.settle(cx)).ok();
                            });
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

    fn top(item: usize, offset: f32) -> Top {
        Top { item, offset }
    }

    /// Rules 2 to 5, in list indices (`top_after_splice`).
    #[test]
    fn the_top_after_a_splice() {
        // a_splice_before_the_top_shifts_it_and_keeps_its_offset
        assert_eq!(top_after_splice(top(5, 12.), 1..3, 4), top(7, 12.));
        assert_eq!(top_after_splice(top(5, 12.), 5..5, 2), top(7, 12.), "an insert at the top's index goes before it");
        assert_eq!(top_after_splice(top(5, 12.), 0..2, 0), top(3, 12.));
        // a_splice_after_the_top_leaves_it
        assert_eq!(top_after_splice(top(5, 12.), 6..8, 1), top(5, 12.));
        assert_eq!(top_after_splice(top(5, 12.), 9..9, 3), top(5, 12.));
        // a_splice_that_replaces_the_top_keeps_its_item_and_offset
        assert_eq!(top_after_splice(top(5, 12.), 5..6, 1), top(5, 12.));
        assert_eq!(top_after_splice(top(5, 12.), 3..8, 6), top(5, 12.));
        assert_eq!(top_after_splice(top(0, 12.), 0..1, 1), top(0, 12.), "the top row replaced by itself");
        // a_splice_that_shortens_past_the_top_keeps_the_last_new_item
        assert_eq!(top_after_splice(top(6, 12.), 3..8, 2), top(4, 12.));
        // a_splice_that_removes_the_top_puts_the_next_item_at_the_top
        assert_eq!(top_after_splice(top(5, 12.), 4..6, 0), top(4, 0.));
        assert_eq!(top_after_splice(top(0, 12.), 0..1, 0), top(0, 0.));
    }

    /// Rules 7 and 8 (`settle_anchor`).
    #[test]
    fn settling_an_anchor() {
        // an_item_that_grew_needs_no_settling
        assert_eq!(super::settle_anchor(Anchor::Offset { item: 3, offset: 200. }, 600.), None);
        assert_eq!(super::settle_anchor(Anchor::Offset { item: 3, offset: 200. }, 200.5), None);
        // an_item_shrunk_under_its_offset_gives_the_top_to_the_next_item
        assert_eq!(super::settle_anchor(Anchor::Offset { item: 3, offset: 400. }, 100.), Some(top(4, 0.)));
        assert_eq!(super::settle_anchor(Anchor::Offset { item: 3, offset: 100. }, 100.), Some(top(4, 0.)));
        // a_fraction_settles_at_the_new_height
        assert_eq!(super::settle_anchor(Anchor::Fraction { item: 5, fraction: 0.4 }, 200.), Some(top(5, 80.)));
        assert_eq!(super::settle_anchor(Anchor::Fraction { item: 5, fraction: 0.4 }, 50.), Some(top(5, 20.)));
    }

    /// Rule 6, in the owner's indices (`inserts_at_start`).
    #[test]
    fn an_insert_at_the_start() {
        // only_an_insert_at_the_owners_first_item_counts
        assert!(super::inserts_at_start(&(0..0), 3));
        assert!(super::inserts_at_start(&(0..2), 5), "a replay that adds items");
        assert!(!super::inserts_at_start(&(0..3), 1), "a replay that shortens");
        assert!(!super::inserts_at_start(&(0..1), 1), "the first item replaced by itself");
        assert!(!super::inserts_at_start(&(4..4), 2), "an insert further on");
        assert!(!super::inserts_at_start(&(0..0), 0));
    }

    /// Twenty 50px items with item 3 `tall`, scrolled `into` px into item 3.
    fn away_in_a_tall_item(tall: f32, into: f32) -> (Entity<MessageScroller>, Rc<RefCell<Vec<f32>>>, &'static mut VisualTestContext) {
        let mut items = vec![50.; 20];
        items[3] = tall;
        let heights = Rc::new(RefCell::new(items));
        let (scroller, cx) = host(heights.clone());
        scroll_to(&scroller, ListOffset { item_ix: 3, offset_in_item: px(into) }, cx);
        (scroller, heights, cx)
    }

    /// The spec's "The item at the top grows" (rule 4): scrolled 200px into
    /// item 3, which is replaced by a taller one; its lines in view stay.
    #[test]
    fn a_top_item_that_grows_while_away_keeps_its_lines() {
        let (scroller, heights, cx) = away_in_a_tall_item(400., 200.);
        let before = item_bounds(&scroller, 3, cx).expect("item 3 at the top");
        assert_eq!(f32::from(before.top()), -200.);
        heights.borrow_mut()[3] = 600.;
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.splice(3..4, 1, cx)));
        cx.run_until_parked();
        let after = item_bounds(&scroller, 3, cx).expect("item 3 at the top");
        assert_eq!((f32::from(after.top()), f32::from(after.size.height)), (-200., 600.));
        assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::Away);
    }

    /// The spec's "The item at the top shrinks above the view" (rule 7):
    /// 400px into a 600px item, replaced by a 100px one; once the frames
    /// settle, the item after it is at the view's top.
    #[test]
    fn a_top_item_that_shrinks_under_the_view_gives_the_top_to_the_next() {
        let (scroller, heights, cx) = away_in_a_tall_item(600., 400.);
        heights.borrow_mut()[3] = 100.;
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.splice(3..4, 1, cx)));
        cx.run_until_parked();
        let next = item_bounds(&scroller, 4, cx).expect("item 4 in view");
        assert_eq!(f32::from(next.top()), 0.);
        let top = scroller.read_with(cx, |s, _| s.scroll_top());
        assert_eq!((top.item_ix, top.offset_in_item), (4, px(0.)));
    }

    /// The spec's "Items inserted at the start" (rule 6): following the end
    /// with the top row in view, a prepend makes the row's fact unknown
    /// until a frame shows it again.
    #[test]
    fn a_prepend_while_following_clears_the_top_row_until_a_frame_shows_it() {
        let heights = Rc::new(RefCell::new(vec![50.; 3]));
        let (scroller, seen, cx) = host_with_top(heights.clone());
        assert!(scroller.read_with(cx, |s, _| s.top_row_in_view()), "a short list shows its row");
        assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::AtEnd);
        heights.borrow_mut().insert(0, 50.);
        // Read before the test context draws the frame the splice asks for.
        let in_view = cx.update(|_, cx| {
            scroller.update(cx, |s, cx| {
                s.splice(0..0, 1, cx);
                s.top_row_in_view()
            })
        });
        assert!(!in_view, "unknown until a frame shows it");
        cx.run_until_parked();
        assert!(scroller.read_with(cx, |s, _| s.top_row_in_view()), "the frame shows it again");
        assert_eq!(*seen.borrow(), [true, true], "the owner hears it come into view again");
    }

    /// The spec's "Every item measured again" (rule 8): 40% into item 5,
    /// every height doubled and every item measured again; item 5 is at the
    /// top at the same fraction, and an item above has its new height once
    /// scrolled to.
    #[test]
    fn remeasuring_every_item_keeps_the_top_item_at_its_fraction() {
        let heights = Rc::new(RefCell::new(vec![100.; 20]));
        let (scroller, cx) = host(heights.clone());
        scroll_to(&scroller, ListOffset { item_ix: 5, offset_in_item: px(40.) }, cx);
        assert_eq!(item_bounds(&scroller, 5, cx).map(|b| f32::from(b.top())), Some(-40.));
        heights.borrow_mut().iter_mut().for_each(|h| *h *= 2.);
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.remeasure_all(cx)));
        cx.run_until_parked();
        let fifth = item_bounds(&scroller, 5, cx).expect("item 5 at the top");
        assert_eq!((f32::from(fifth.top()), f32::from(fifth.size.height)), (-80., 200.));
        assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::Away);
        scroll_to(&scroller, ListOffset { item_ix: 1, offset_in_item: px(0.) }, cx);
        assert_eq!(item_bounds(&scroller, 1, cx).map(|b| f32::from(b.size.height)), Some(200.));
    }

    /// Rule 8 while following: the end stays in view.
    #[test]
    fn remeasuring_while_following_keeps_the_end() {
        let heights = Rc::new(RefCell::new(vec![50.; 20]));
        let (scroller, cx) = host(heights.clone());
        heights.borrow_mut().iter_mut().for_each(|h| *h *= 2.);
        cx.update(|_, cx| scroller.update(cx, |s, cx| s.remeasure_all(cx)));
        cx.run_until_parked();
        let last = item_bounds(&scroller, 19, cx).expect("the end in view");
        assert_eq!((f32::from(last.bottom()), f32::from(last.size.height)), (300., 100.));
        assert_eq!(scroller.read_with(cx, |s, _| s.follow()), Follow::AtEnd);
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
