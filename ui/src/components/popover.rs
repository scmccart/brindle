//! `Popover`: shadcn's popover, a non-modal overlay anchored to its trigger.
//!
//! A `RenderOnce` builder that renders the trigger and, while open, the
//! content beside it. Its state lives in keyed state (by the popover's
//! id): a non-modal [`Overlay`] holding the content view, the trigger's
//! bounds from the last frame, and whether the popover was open when the
//! last pointer press on the trigger came.
//!
//! The trigger is built by a closure that gets a [`PopoverTrigger`]: the
//! closure wires [`PopoverTrigger::on_click`] into the trigger's own click
//! handler, so the trigger toggles the popover however it activates (a
//! button's click, or its enter and space). The content is a view of the
//! app's own, a `ManagedView` that also implements [`FocusStops`], built
//! afresh by the content closure each time the popover opens. It usually
//! renders a [`PopoverContent`] panel:
//!
//! ```ignore
//! // The closures are kept until the popover renders, so they own what
//! // they use: a clone of the trigger's focus handle, not `self`.
//! let handle = self.trigger.clone();
//! Popover::new(
//!     "font-size",
//!     move |trigger| {
//!         Button::new("font-size-trigger", "Font size")
//!             .variant(ButtonVariant::Outline)
//!             .focus(&handle)
//!             .on_click(trigger.on_click())
//!     },
//!     |_, cx| cx.new(FontSize::new),
//! )
//!
//! impl Render for FontSize {
//!     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
//!         div().track_focus(&self.focus).child(PopoverContent::new().child(/* … */))
//!     }
//! }
//! ```
//!
//! The panel goes below the trigger, centred, by default ([`Anchored`]
//! flips it to the other side when it doesn't fit, and keeps it in the
//! window). Escape and a press outside the panel close it, and so does the
//! content emitting `DismissEvent`; focus then goes back to what had it
//! when the popover opened, the trigger when it was opened from there. That
//! holds for an outside press too: one on another control runs its click
//! but leaves focus on the trigger (design.md, "Overlay lifecycle").
//!
//! The overlay is deferred, which lifts it above the window from wherever
//! the popover is, a dialog's body included. A popover opened over another
//! popover renders inline instead (see `Overlay`'s module doc), so it
//! should be rendered last in that popover's content and outside any
//! clipping container.
//!
//! [`Overlay`]: crate::primitives::overlay::Overlay
//! [`FocusStops`]: crate::primitives::focus_scope::FocusStops
//! [`Anchored`]: crate::primitives::placement::Anchored

use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, ElementId, Entity, InteractiveElement, IntoElement, ManagedView,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Refineable, RenderOnce, StyleRefinement, Styled, Window, div,
};

use crate::primitives::focus_scope::FocusStops;
use crate::primitives::overlay::Overlay;
use crate::primitives::placement::{Align, Anchored, Side, anchor_trigger};
use crate::style::{POPOVER_WIDTH, popover_style};
use crate::theme::ActiveTheme;

/// Toggles the popover, given the click behind it (`None` from code).
type Toggle = Rc<dyn Fn(Option<&ClickEvent>, &mut Window, &mut App)>;

/// Builds the content view on each opening.
type ContentBuilder<V> = Rc<dyn Fn(&mut Window, &mut App) -> Entity<V>>;

/// Builds the trigger, given its toggle.
type TriggerBuilder = Box<dyn FnOnce(PopoverTrigger) -> AnyElement>;

/// What activating a popover's trigger does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerAction {
    Open,
    Close,
    /// Nothing: the pointer press behind this click already closed the
    /// popover, as a press outside it.
    Stay,
}

/// What activating the trigger does, when the popover is `open` now, the
/// activation is a `mouse` click (not a keyboard click or a call from
/// code), and the popover was `pressed_open` when that click's press came.
///
/// A press on the trigger is outside the popover, so it asks the popover to
/// dismiss, and the dismissal applies before the click that follows on
/// release. Without the press's record, that click would find the popover
/// closed and open it again. Only a mouse click has such a press, so the
/// record counts for nothing else. A popover still open (its content kept
/// itself open) closes, since activating the trigger again closes it.
pub fn trigger_action(open: bool, mouse: bool, pressed_open: bool) -> TriggerAction {
    if open {
        TriggerAction::Close
    } else if mouse && pressed_open {
        TriggerAction::Stay
    } else {
        TriggerAction::Open
    }
}

/// Whether the popover must draw again after recording the trigger's new
/// `bounds`: when it is open and the trigger moved, since the panel was
/// placed against `previous`.
pub fn follows_trigger(previous: Option<Bounds<Pixels>>, bounds: Bounds<Pixels>, open: bool) -> bool {
    open && previous != Some(bounds)
}

/// What the trigger closure gets: whether the popover is open, and the
/// handler that toggles it.
#[derive(Clone)]
pub struct PopoverTrigger {
    open: bool,
    toggle: Toggle,
}

impl PopoverTrigger {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// A click handler that toggles the popover, for the trigger's
    /// `on_click`.
    pub fn on_click(&self) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let toggle = self.toggle.clone();
        move |event, window, cx| toggle(Some(event), window, cx)
    }

    /// Toggles the popover from code.
    pub fn toggle(&self, window: &mut Window, cx: &mut App) {
        (self.toggle)(None, window, cx)
    }
}

/// A popover's keyed state.
struct PopoverState<V: ManagedView + FocusStops> {
    overlay: Overlay<V>,
    /// The trigger's bounds, in window coordinates, from the last frame.
    trigger: Option<Bounds<Pixels>>,
    /// Whether the popover was open when the last pointer press on the
    /// trigger came (see [`trigger_action`]).
    pressed_open: bool,
}

/// A popover: a trigger, and the content view it opens beside it. Refine
/// the trigger's wrapper through `Styled` (a width, say).
#[derive(IntoElement)]
pub struct Popover<V: ManagedView + FocusStops> {
    id: ElementId,
    trigger: TriggerBuilder,
    content: ContentBuilder<V>,
    side: Side,
    align: Align,
    offset: Option<Pixels>,
    style: StyleRefinement,
}

impl<V: ManagedView + FocusStops> Popover<V> {
    /// A popover whose state is kept under `id`, unique among the
    /// popovers of one parent. `trigger` builds the trigger and wires the
    /// toggle into it; `content` builds the content view on each opening.
    /// Both are kept past the call (`trigger` until the popover renders,
    /// `content` until its trigger is activated), so they must own what
    /// they capture: clones of focus handles and entities, not borrows of
    /// the owner.
    pub fn new<E: IntoElement>(
        id: impl Into<ElementId>,
        trigger: impl FnOnce(PopoverTrigger) -> E + 'static,
        content: impl Fn(&mut Window, &mut App) -> Entity<V> + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            trigger: Box::new(move |toggle| trigger(toggle).into_any_element()),
            content: Rc::new(content),
            side: Side::default(),
            align: Align::default(),
            offset: None,
            style: StyleRefinement::default(),
        }
    }

    /// The side of the trigger the panel goes on, below by default. It
    /// flips to the opposite side when it doesn't fit.
    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    /// How the panel lines up with the trigger, centred by default.
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// The gap between the trigger and the panel ([`Anchored`]'s 4px by
    /// default).
    pub fn offset(mut self, offset: Pixels) -> Self {
        self.offset = Some(offset);
        self
    }
}

impl<V: ManagedView + FocusStops> Styled for Popover<V> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<V: ManagedView + FocusStops> RenderOnce for Popover<V> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, trigger, content, side, align, offset, style } = self;
        let state = window.use_keyed_state(id, cx, |_, _| PopoverState::<V> {
            overlay: Overlay::non_modal(),
            trigger: None,
            pressed_open: false,
        });

        let toggle: Toggle = {
            let state = state.clone();
            Rc::new(move |click, window, cx| {
                let mouse = matches!(click, Some(ClickEvent::Mouse(_)));
                state.update(cx, |s, cx| {
                    let pressed_open = std::mem::take(&mut s.pressed_open);
                    match trigger_action(s.overlay.is_open(), mouse, pressed_open) {
                        TriggerAction::Open => {
                            let view = content(window, cx);
                            s.overlay.open(view, |s| &mut s.overlay, window, cx);
                        }
                        TriggerAction::Close => {
                            s.overlay.close(window, cx);
                            cx.notify();
                        }
                        TriggerAction::Stay => {}
                    }
                });
            })
        };
        let open = state.read(cx).overlay.is_open();
        let trigger = trigger(PopoverTrigger { open, toggle });

        // The trigger's bounds each frame, for placing the panel. Recorded
        // without a notify, since this runs while the frame is drawn; a
        // trigger that moved under an open panel asks for one more frame.
        let record = {
            let state = state.clone();
            move |bounds, window: &mut Window, cx: &mut App| {
                let follow = state.update(cx, |s, _| {
                    let follow = follows_trigger(s.trigger, bounds, s.overlay.is_open());
                    s.trigger = Some(bounds);
                    follow
                });
                if follow {
                    window.request_animation_frame();
                }
            }
        };
        // Bubbles after the open panel's outside-press handler has asked it
        // to dismiss, and before that dismissal applies.
        let pressed = {
            let state = state.clone();
            move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                state.update(cx, |s, _| s.pressed_open = s.overlay.is_open());
            }
        };

        let panel = state.update(cx, |s, cx| {
            let at = s.trigger.unwrap_or_default();
            s.overlay.render_with(
                move |panel| {
                    let anchored = Anchored::new(at).side(side).align(align);
                    let anchored = match offset {
                        Some(offset) => anchored.offset(offset),
                        None => anchored,
                    };
                    anchored.child(panel).into_any_element()
                },
                window,
                cx,
            )
        });

        // A flex row, so the trigger keeps its own size and its bounds are
        // its own. The panel, absolute, takes no room in it.
        let mut wrapper =
            anchor_trigger(trigger, record).flex().on_mouse_down(MouseButton::Left, pressed).children(panel);
        // The owner's layout refinements win over the component's.
        wrapper.style().refine(&style);
        wrapper
    }
}

/// shadcn's PopoverContent: the panel a popover's content view renders, in
/// the popover colours, w-72 with p-4. Its children are the content;
/// refine its layout through `Styled`.
#[derive(Default, IntoElement)]
pub struct PopoverContent {
    children: Vec<AnyElement>,
    style: StyleRefinement,
}

impl PopoverContent {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ParentElement for PopoverContent {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for PopoverContent {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for PopoverContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let style = popover_style(cx.ui_theme());
        // w-72 rounded-md border p-4 shadow-md
        let mut panel = div()
            .w(POPOVER_WIDTH)
            .p_4()
            .rounded(style.radius)
            .border_1()
            .border_color(style.border)
            .bg(style.background)
            .text_color(style.foreground)
            .shadow_md()
            .children(self.children);
        // The owner's layout refinements win over the component's.
        panel.style().refine(&self.style);
        panel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{bounds, point, px, size};
    use TriggerAction::{Close, Open, Stay};

    /// Every activation of the trigger, as (open now, a mouse click,
    /// pressed while open) → what it does.
    #[test]
    fn the_trigger_toggles_once_per_activation() {
        let cases = [
            // Closed: a click or a key opens it.
            ((false, true, false), Open),
            ((false, false, false), Open),
            // A click whose press closed it, as an outside press: stays shut,
            // instead of opening again on release.
            ((false, true, true), Stay),
            // A key or a call from code has no press: a record left by a
            // press that never became a click (dragged off) counts for
            // nothing.
            ((false, false, true), Open),
            // Open (its content refused the press's dismissal, or no press
            // came): activating the trigger again closes it.
            ((true, true, true), Close),
            ((true, true, false), Close),
            ((true, false, false), Close),
            ((true, false, true), Close),
        ];
        for ((open, mouse, pressed_open), expected) in cases {
            let case = format!("open={open} mouse={mouse} pressed_open={pressed_open}");
            assert_eq!(trigger_action(open, mouse, pressed_open), expected, "{case}");
        }
    }

    #[test]
    fn an_open_popover_follows_a_moved_trigger() {
        let at = bounds(point(px(10.), px(10.)), size(px(40.), px(20.)));
        let moved = bounds(point(px(30.), px(10.)), size(px(40.), px(20.)));
        assert!(follows_trigger(Some(at), moved, true));
        assert!(follows_trigger(None, at, true));
        // Unmoved, or closed: no extra frame.
        assert!(!follows_trigger(Some(at), at, true));
        assert!(!follows_trigger(Some(at), moved, false));
    }
}
