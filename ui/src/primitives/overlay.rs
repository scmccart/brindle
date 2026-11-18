//! `Overlay<V>`: the open/close lifecycle of a managed view shown above
//! everything else, such as a dialog or a popover's content.
//!
//! The owner keeps an `Overlay` in a field and renders it in its tree.
//! [`Overlay::open`] records what has focus, pushes a layer, moves focus
//! into the view and subscribes to its `DismissEvent`; on that event, or
//! [`Overlay::close`], the layer is popped, the view dropped and focus given
//! back if what had it still exists. While open, the view sits in a
//! [`FocusScope`], so Tab cycles through its stops and never leaves it.
//!
//! Where the owner renders it depends on the layer's kind, because gpui
//! refuses a `defer_draw` while it draws the deferred elements
//! (`window.rs`: "cannot call defer_draw during deferred drawing"), so the
//! layers can't all go through `deferred` (see [`wrap_in_deferred`]). A
//! modal view is never deferred: the app renders it last in its window's
//! tree, where tree order paints it above the rest and its backdrop blocks
//! what is beneath. A non-modal view (a popover) is deferred, which lifts it
//! above the window from wherever its trigger is, a dialog's body included.
//! Only a non-modal view opened over another non-modal layer renders
//! inline, since that layer's view is drawn deferred: its owner renders it
//! last in the parent layer's view and outside any clipping container, or
//! what the parent renders after it paints over it.
//!
//! Escape and outside presses never close the view directly: they dispatch
//! [`Dismiss`] to it, and the layer answers by emitting `DismissEvent` unless
//! the view handles `Dismiss` itself, which is how a view keeps itself open
//! (to show an error, say). Only the top layer acts, and the dismissal from
//! a press is deferred to the end of the event, so a press outside a
//! popover and the dialog under it closes the popover alone.
//!
//! An outside press on a non-modal view still reaches what it lands on (a
//! popover's), unless the slot blocks the pointer outside its view
//! ([`OverlayKind::NonModal`] with `blocks_outside`, a select's menu): a
//! transparent cover beneath the view then takes the press, which only asks
//! the view to close.
//! Outside presses are found by position, not by what the press hit, so
//! the cover changes none of the rules above.
//!
//! Every opening fades in over [`ENTER_DURATION`], a modal layer's backdrop
//! with it. The animation is keyed by the slot's open count
//! ([`enter_animation_id`]), so each opening replays it, even one that
//! reopens the slot within a frame. There is no exit animation, and no
//! zoom: gpui 0.2.2 can't transform a `div`.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, App, Context, DismissEvent, ElementId, Entity, EventEmitter, Focusable, Hsla,
    InteractiveElement, IntoElement, ManagedView, ParentElement, Point, Styled, Subscription,
    WeakFocusHandle, Window, anchored, deferred, div, ease_in_out, hsla,
};

use crate::actions::Dismiss;
use crate::primitives::focus_scope::{FocusScope, FocusStops};
use crate::primitives::focus_visible::{Modality, set_modality};
use crate::primitives::layer::{self, LayerId, LayerStack};

/// A modal layer's backdrop: black at 50%, shadcn's `bg-black/50`. It isn't
/// a token.
fn backdrop() -> Hsla {
    hsla(0., 0., 0., 0.5)
}

/// How long an opening overlay takes to fade in (design.md's Animation:
/// shadcn's `fade-in-0`, at 150 ms).
pub const ENTER_DURATION: Duration = Duration::from_millis(150);

/// The id of the entry animation for a slot's `opens`-th opening. gpui
/// keeps an animation's start time in the element state of its id, so a
/// new id per opening starts it over.
pub fn enter_animation_id(opens: u64) -> ElementId {
    ("ui-layer-enter", opens).into()
}

/// What kind of layer a slot opens, which decides how it covers the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    /// A dialog: a backdrop over the window blocks the pointer, and the view
    /// is centred over it.
    Modal,
    /// A popover: the view blocks the pointer within its own bounds, and
    /// also outside them while open when `blocks_outside` is set, as
    /// Radix's select content does (`disableOutsidePointerEvents`): a
    /// transparent cover over the window, just beneath the view, takes
    /// every press, release and hover outside it. An outside press still
    /// requests dismissal (it is outside the view), but reaches nothing
    /// beneath, and the view's own outside-release handlers still run. The
    /// view stays where it is placed, unlike a modal one; no backdrop is
    /// drawn.
    NonModal { blocks_outside: bool },
}

impl OverlayKind {
    pub fn is_modal(self) -> bool {
        match self {
            OverlayKind::Modal => true,
            OverlayKind::NonModal { .. } => false,
        }
    }

    /// Whether the open layer keeps the pointer from what is outside its
    /// view: a modal one always does, with its backdrop.
    pub fn blocks_outside(self) -> bool {
        match self {
            OverlayKind::Modal => true,
            OverlayKind::NonModal { blocks_outside } => blocks_outside,
        }
    }
}

/// The slot an owner keeps for one overlay view. Its view's `focus_handle`
/// must be tracked in the view's tree (`track_focus`), as GPUI's own focus
/// needs: the dismiss request from a press is dispatched to it.
pub struct Overlay<V: ManagedView + FocusStops> {
    kind: OverlayKind,
    /// Whether a press outside the view requests its dismissal (the
    /// default). A slot that holds work in progress turns it off.
    outside_press_dismisses: bool,
    /// How many times the slot has opened, which keys the entry animation.
    opens: u64,
    open: Option<Open<V>>,
}

struct Open<V> {
    view: Entity<V>,
    layer: LayerId,
    /// What had focus when the overlay opened, to give it back on close.
    restore: Option<WeakFocusHandle>,
    _subscription: Subscription,
}

impl<V: ManagedView + FocusStops> Overlay<V> {
    /// A slot for a view of `kind`.
    pub fn new(kind: OverlayKind) -> Self {
        Self { kind, outside_press_dismisses: true, opens: 0, open: None }
    }

    /// A slot for a modal view: it covers the window with a backdrop that
    /// blocks the pointer, and centres the view over it.
    pub fn modal() -> Self {
        Self::new(OverlayKind::Modal)
    }

    /// Makes a press outside the view, on a modal slot's backdrop, request
    /// nothing. The backdrop still blocks the pointer, and Escape and the
    /// view's own controls close the view as before. Overlays opened above
    /// it keep their own outside presses.
    pub fn ignore_outside_press(mut self) -> Self {
        self.set_ignore_outside_press(true);
        self
    }

    /// [`Overlay::ignore_outside_press`] on a slot the owner already
    /// keeps, for a slot whose views differ (a palette that asks a
    /// question ignores outside presses, a plain one doesn't). It takes
    /// effect from the next frame, so it can be set before `open` or while
    /// a view is shown.
    pub fn set_ignore_outside_press(&mut self, ignore: bool) {
        self.outside_press_dismisses = !ignore;
    }

    /// Whether a press outside the view requests its dismissal.
    pub fn outside_press_dismisses(&self) -> bool {
        self.outside_press_dismisses
    }

    /// A slot for a non-modal view (a popover): it blocks the pointer only
    /// within its own bounds. One that blocks it outside too (a select's
    /// menu) is `Overlay::new(OverlayKind::NonModal { blocks_outside: true })`.
    pub fn non_modal() -> Self {
        Self::new(OverlayKind::NonModal { blocks_outside: false })
    }

    pub fn kind(&self) -> OverlayKind {
        self.kind
    }

    pub fn is_modal(&self) -> bool {
        self.kind.is_modal()
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn view(&self) -> Option<&Entity<V>> {
        self.open.as_ref().map(|open| &open.view)
    }

    pub fn layer(&self) -> Option<LayerId> {
        self.open.as_ref().map(|open| open.layer)
    }

    /// Shows `view`: records what has focus, pushes the layer, focuses the
    /// view's initial handle (else its first stop, else its own) and
    /// subscribes to its `DismissEvent`. `slot` finds this overlay in the
    /// owner again when the event arrives. A view already open is closed
    /// first.
    pub fn open<O: 'static>(
        &mut self,
        view: Entity<V>,
        slot: fn(&mut O) -> &mut Overlay<V>,
        window: &mut Window,
        cx: &mut Context<O>,
    ) {
        self.close(window, cx);
        let restore = window.focused(cx).map(|handle| handle.downgrade());
        let own = view.focus_handle(cx);
        let layer = layer::push(window, self.kind.is_modal(), own.downgrade(), cx);
        let target = {
            let v = view.read(cx);
            initial_focus(v.initial_focus(cx), &v.stops(cx), own)
        };
        window.focus(&target);
        let subscription = cx.subscribe_in(&view, window, move |owner, view, _: &DismissEvent, window, cx| {
            let overlay = slot(owner);
            if overlay.view() == Some(view) {
                overlay.close(window, cx);
                cx.notify();
            }
        });
        self.opens += 1;
        self.open = Some(Open { view, layer, restore, _subscription: subscription });
        cx.notify();
    }

    /// Closes the view: pops the layer, drops the view and gives focus back
    /// to what had it when the view opened, if that still exists. Otherwise
    /// focus is left for the owner to set.
    pub fn close(&mut self, window: &mut Window, cx: &mut App) {
        let Some(open) = self.open.take() else { return };
        layer::remove(window, open.layer, cx);
        if let Some(handle) = restore_focus(open.restore.as_ref(), WeakFocusHandle::upgrade) {
            window.focus(&handle);
        }
    }

    /// The open view's element, or `None`: the view in its layer. A modal
    /// overlay comes with its full-window backdrop and renders inline, so
    /// its owner places it last in the window's tree; a non-modal one is
    /// deferred, which lifts it above the window, unless a non-modal layer
    /// is open below it (see the module doc and [`wrap_in_deferred`]).
    pub fn render(&self, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.render_with(|panel| panel, window, cx)
    }

    /// [`Overlay::render`] with `place` applied to a non-modal view's panel
    /// first: a popover wraps the panel in an [`Anchored`] to put it beside
    /// its trigger. A modal view is centred by its backdrop, so `place` is
    /// skipped. The result is deferred or inline by [`wrap_in_deferred`],
    /// so what `place` returns must work in both.
    ///
    /// [`Anchored`]: crate::primitives::placement::Anchored
    pub fn render_with(
        &self,
        place: impl FnOnce(AnyElement) -> AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let open = self.open.as_ref()?;
        let id = open.layer;
        let lift = layer::stack(window, cx).and_then(|stack| deferred_priority(stack, id));
        let view = open.view.clone();
        let panel = div()
            .key_context("UiLayer")
            .occlude()
            // The default answer to a dismiss request. A view that handles
            // `Dismiss` itself runs first and this never sees it.
            .on_action({
                let view = view.clone();
                move |_: &Dismiss, window, cx| {
                    if layer::is_top(window, id, cx) {
                        view.update(cx, |_, cx| cx.emit(DismissEvent));
                    }
                }
            })
            // An outside press asks the top layer only. The request goes to
            // the view through the focused handle, chosen now because the
            // press moves focus to what it hit, and after the event, so
            // every layer sees the stack the event started with.
            .on_mouse_down_out({
                let view = view.clone();
                let dismisses = self.outside_press_dismisses;
                move |_, window, cx| {
                    if !outside_press_requests_dismissal(dismisses, layer::is_top(window, id, cx)) {
                        return;
                    }
                    let own = view.focus_handle(cx);
                    let focused = window.focused(cx);
                    let inside = focused.as_ref().is_some_and(|focused| own.contains(focused, window));
                    let target = match (dismiss_target(inside), focused) {
                        (DismissTarget::Focused, Some(focused)) => focused,
                        (DismissTarget::Focused, None) | (DismissTarget::Own, _) => own,
                    };
                    window.defer(cx, move |window, cx| target.dispatch_action(&Dismiss, window, cx));
                }
            })
            // Tab stays inside the view.
            .child(FocusScope::new(&view).child(view));
        // Opacity applies to a div's whole subtree: a modal layer fades in
        // with its backdrop, a non-modal one by itself.
        let enter = Animation::new(ENTER_DURATION).with_easing(ease_in_out);
        let enter_id = enter_animation_id(self.opens);
        let element = match self.kind {
            OverlayKind::Modal => {
                let size = window.viewport_size();
                // Absolute at the window's origin, so the backdrop covers the
                // window from wherever in the tree it is rendered.
                anchored()
                    .position(Point::default())
                    .child(
                        div()
                            .w(size.width)
                            .h(size.height)
                            .occlude()
                            .bg(backdrop())
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(panel)
                            .with_animation(enter_id, enter, |backdrop, delta| backdrop.opacity(delta)),
                    )
                    .into_any_element()
            }
            OverlayKind::NonModal { blocks_outside } => {
                let placed = place(panel.with_animation(enter_id, enter, |panel, delta| panel.opacity(delta)).into_any_element());
                if blocks_outside {
                    // The cover first, so the panel paints (and hit-tests) above
                    // it. A press on it, with any button, is pointer input; its
                    // default (moving focus) is prevented, so focus stays in the
                    // view until the dismissal gives it back. Both are out of
                    // flow.
                    let size = window.viewport_size();
                    let cover = anchored().position(Point::default()).child(
                        div().w(size.width).h(size.height).occlude().on_any_mouse_down(|_, window, cx| {
                            set_modality(window, Modality::Pointer, cx);
                            window.prevent_default();
                        }),
                    );
                    div().absolute().child(cover).child(placed).into_any_element()
                } else {
                    placed
                }
            }
        };
        Some(match lift {
            Some(priority) => deferred(element).with_priority(priority).into_any_element(),
            None => element,
        })
    }
}

/// The paint priority layer `id` of `stack` is deferred at, its depth, or
/// `None` when it renders inline ([`wrap_in_deferred`]): a layer not in the
/// stack is never lifted, so it has no depth to be drawn at.
pub fn deferred_priority(stack: &LayerStack, id: LayerId) -> Option<usize> {
    if wrap_in_deferred(stack, id) { stack.depth(id) } else { None }
}

/// Which handle an outside press's dismiss request goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DismissTarget {
    /// What has focus inside the view, so a view's own `Dismiss` handler
    /// below its root (a command list's) gets the request.
    Focused,
    /// The view's own handle.
    Own,
}

/// Where an outside press sends the dismiss request: to the focused handle
/// when focus is inside the view (`focused_inside`), else to the view's own.
pub fn dismiss_target(focused_inside: bool) -> DismissTarget {
    if focused_inside { DismissTarget::Focused } else { DismissTarget::Own }
}

/// Whether the layer `id` of `stack` renders through `deferred`, which lifts
/// it above the window, or inline, where tree order places it.
///
/// gpui asserts that nothing calls `defer_draw` while it draws the deferred
/// elements, so the layers can't all be deferred:
/// - A modal layer never is. Its owner, the app, renders it last in the
///   window's tree, so it paints above the rest, and its backdrop blocks
///   what is beneath. A layer opened from inside it can then be deferred.
/// - A non-modal layer is deferred unless a non-modal layer is open below
///   it, whose view is drawn deferred and would hold the nested call. Then
///   it renders inline in the parent layer's view, which its owner renders
///   last there.
///
/// A layer not in the stack renders inline, which is legal anywhere.
pub fn wrap_in_deferred(stack: &LayerStack, id: LayerId) -> bool {
    stack.get(id).is_some_and(|layer| !layer.modal && !stack.non_modal_below(id))
}

/// Whether a press outside a layer's view requests its dismissal: only the
/// top layer asks, and only when its slot doesn't ignore outside presses.
pub fn outside_press_requests_dismissal(slot_dismisses: bool, is_top: bool) -> bool {
    slot_dismisses && is_top
}

/// The default answer to a dismiss request, for a view shown outside an
/// [`Overlay`]: `.on_action(dismiss_on_request(cx))` emits `DismissEvent`.
/// Inside an `Overlay` the layer does this itself when the view doesn't
/// handle `Dismiss`.
pub fn dismiss_on_request<V: EventEmitter<DismissEvent> + 'static>(
    cx: &mut Context<V>,
) -> impl Fn(&Dismiss, &mut Window, &mut App) + 'static {
    cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent))
}

/// Where focus goes when an overlay opens: the view's designated handle,
/// else its first stop, else the view's own handle.
pub fn initial_focus<H: Clone>(initial: Option<H>, stops: &[H], own: H) -> H {
    initial.or_else(|| stops.first().cloned()).unwrap_or(own)
}

/// Where focus goes when an overlay closes: what had it when the overlay
/// opened (`recorded`), if `upgrade` still finds it. `None` leaves focus
/// alone, for the owner to set.
pub fn restore_focus<W, H>(recorded: Option<&W>, upgrade: impl FnOnce(&W) -> Option<H>) -> Option<H> {
    upgrade(recorded?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_can_ignore_outside_presses() {
        assert!(outside_press_requests_dismissal(true, true));
        // Not the top layer: the layer above handles the press.
        assert!(!outside_press_requests_dismissal(true, false));
        // An ignoring slot asks for nothing, even on top.
        assert!(!outside_press_requests_dismissal(false, true));
    }

    /// The setter has the constructor flag's effect, and can be turned
    /// back off on the same slot.
    #[test]
    fn the_outside_press_flag_can_be_set_on_a_kept_slot() {
        struct Never;
        impl gpui::EventEmitter<DismissEvent> for Never {}
        impl Focusable for Never {
            fn focus_handle(&self, _: &App) -> gpui::FocusHandle {
                unreachable!()
            }
        }
        impl gpui::Render for Never {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
            }
        }
        impl FocusStops for Never {
            fn stops(&self, _: &App) -> Vec<gpui::FocusHandle> {
                Vec::new()
            }
        }
        let mut slot = Overlay::<Never>::modal();
        assert!(slot.outside_press_dismisses());
        slot.set_ignore_outside_press(true);
        assert!(!slot.outside_press_dismisses());
        assert_eq!(slot.outside_press_dismisses(), Overlay::<Never>::modal().ignore_outside_press().outside_press_dismisses());
        slot.set_ignore_outside_press(false);
        assert!(slot.outside_press_dismisses());
        assert!(outside_press_requests_dismissal(slot.outside_press_dismisses(), true));
    }

    #[test]
    fn initial_focus_prefers_the_designated_handle() {
        assert_eq!(initial_focus(Some("initial"), &["first", "second"], "own"), "initial");
        assert_eq!(initial_focus(None, &["first", "second"], "own"), "first");
        assert_eq!(initial_focus(None, &[], "own"), "own");
        // A designated handle wins even with no stops.
        assert_eq!(initial_focus(Some("initial"), &[], "own"), "initial");
    }

    /// Nested deferred draws panic in gpui, so a modal layer, rendered last
    /// in the window's tree, is never deferred, and a non-modal one is
    /// unless a non-modal layer below it is already drawn deferred.
    #[test]
    fn modal_layers_are_never_deferred() {
        let mut stack = LayerStack::new();
        let dialog = stack.push(true);
        assert!(!wrap_in_deferred(&stack, dialog));
        // Nor above a non-modal layer.
        let mut stack = LayerStack::new();
        stack.push(false);
        let dialog = stack.push(true);
        assert!(!wrap_in_deferred(&stack, dialog));
    }

    #[test]
    fn a_non_modal_layer_alone_is_deferred() {
        let mut stack = LayerStack::new();
        let popover = stack.push(false);
        assert!(wrap_in_deferred(&stack, popover));
    }

    /// A select's menu in a dialog: the dialog isn't deferred, so the menu
    /// can be, and it paints above the dialog's footer.
    #[test]
    fn a_non_modal_layer_above_a_modal_one_is_deferred() {
        let mut stack = LayerStack::new();
        let dialog = stack.push(true);
        let menu = stack.push(false);
        assert!(wrap_in_deferred(&stack, menu));
        assert!(!wrap_in_deferred(&stack, dialog));
    }

    /// A popover opened from inside a popover renders inline, inside the
    /// parent popover's deferred view.
    #[test]
    fn a_non_modal_layer_above_a_non_modal_one_renders_inline() {
        let mut stack = LayerStack::new();
        let popover = stack.push(false);
        let nested = stack.push(false);
        assert!(wrap_in_deferred(&stack, popover));
        assert!(!wrap_in_deferred(&stack, nested));
        // With a dialog between them too: the lower popover is still drawn deferred.
        let mut stack = LayerStack::new();
        let popover = stack.push(false);
        stack.push(true);
        let menu = stack.push(false);
        assert!(wrap_in_deferred(&stack, popover));
        assert!(!wrap_in_deferred(&stack, menu));
        // Closing the lower popover lifts the one above it.
        stack.remove(popover);
        assert!(wrap_in_deferred(&stack, menu));
        // A layer that isn't open renders inline.
        assert!(!wrap_in_deferred(&stack, popover));
    }

    /// b42 #2: the kind says what blocks the pointer outside the view: a
    /// modal layer always does, a non-modal one only when its slot asks.
    #[test]
    fn a_modal_overlay_blocks_outside() {
        let cases = [
            (OverlayKind::Modal, true, true),
            (OverlayKind::NonModal { blocks_outside: true }, false, true),
            (OverlayKind::NonModal { blocks_outside: false }, false, false),
        ];
        for (kind, modal, blocks) in cases {
            assert_eq!((kind.is_modal(), kind.blocks_outside()), (modal, blocks), "{kind:?}");
        }
    }

    /// b42: a lifted layer is deferred at its depth; one that renders
    /// inline, or isn't in the stack at all, has no priority, and is drawn
    /// unlifted at the base.
    #[test]
    fn an_overlay_missing_from_the_stack_is_not_lifted() {
        let mut stack = LayerStack::new();
        let dialog = stack.push(true);
        let menu = stack.push(false);
        let nested = stack.push(false);
        assert_eq!(deferred_priority(&stack, dialog), None);
        assert_eq!(deferred_priority(&stack, menu), Some(1));
        assert_eq!(deferred_priority(&stack, nested), None);
        stack.remove(menu);
        assert_eq!(deferred_priority(&stack, menu), None);
        assert_eq!(deferred_priority(&stack, nested), Some(1));
        assert_eq!(deferred_priority(&LayerStack::new(), menu), None);
    }

    /// b42 #3: an outside press asks what has focus inside the view, so a
    /// handler below the view's root sees it, and else the view itself.
    #[test]
    fn an_outside_press_asks_the_focus_inside_the_view() {
        assert_eq!(dismiss_target(true), DismissTarget::Focused);
        assert_eq!(dismiss_target(false), DismissTarget::Own);
    }

    /// Each opening gets its own animation id, so the fade replays even
    /// when a slot reopens before a frame drops the old one's state.
    #[test]
    fn each_opening_has_its_own_entry_animation() {
        assert_ne!(enter_animation_id(1), enter_animation_id(2));
        assert_eq!(enter_animation_id(3), enter_animation_id(3));
        assert_eq!(ENTER_DURATION, Duration::from_millis(150));
    }

    #[test]
    fn restore_focus_only_when_the_handle_upgrades() {
        let alive = |recorded: &u32| Some(*recorded);
        let gone = |_: &u32| -> Option<u32> { None };
        assert_eq!(restore_focus(Some(&7), alive), Some(7));
        assert_eq!(restore_focus(Some(&7), gone), None);
        // Nothing had focus when the overlay opened.
        assert_eq!(restore_focus(None, alive), None);
    }
}
