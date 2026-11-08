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
//! Where the owner renders it depends on the layer's depth (see
//! [`wrap_in_deferred`]). The bottom layer goes through `deferred`, which
//! lifts it above the window, so its owner renders it anywhere in its tree.
//! gpui refuses a `defer_draw` while it draws the deferred elements
//! (`window.rs`: "cannot call defer_draw during deferred drawing"), so a
//! layer opened while another is open renders inline, inside the open
//! layer's view, whose `deferred` already lifts it; within that subtree, tree
//! order gives z-order. So the owner of a nested overlay renders it inside
//! the parent layer's view, last in its tree and outside any clipping
//! container, or what the parent renders after it paints over it.
//!
//! Escape and outside presses never close the view directly: they dispatch
//! [`Dismiss`] to it, and the layer answers by emitting `DismissEvent` unless
//! the view handles `Dismiss` itself, which is how a view keeps itself open
//! (to show an error, say). Only the top layer acts, and the dismissal from
//! a press is deferred to the end of the event, so a press outside a
//! popover and the dialog under it closes the popover alone.

use gpui::{
    AnyElement, App, Context, DismissEvent, Entity, EventEmitter, Focusable, Hsla, InteractiveElement, IntoElement,
    ManagedView, ParentElement, Point, Styled, Subscription, WeakFocusHandle, Window, anchored, deferred, div, hsla,
};

use crate::actions::Dismiss;
use crate::primitives::focus_scope::{FocusScope, FocusStops};
use crate::primitives::layer::{self, LayerId};

/// A modal layer's backdrop: black at 50%, shadcn's `bg-black/50`. It isn't
/// a token.
fn backdrop() -> Hsla {
    hsla(0., 0., 0., 0.5)
}

/// The slot an owner keeps for one overlay view. Its view's `focus_handle`
/// must be tracked in the view's tree (`track_focus`), as GPUI's own focus
/// needs: the dismiss request from a press is dispatched to it.
pub struct Overlay<V: ManagedView + FocusStops> {
    modal: bool,
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
    /// A slot for a modal view: it covers the window with a backdrop that
    /// blocks the pointer, and centres the view over it.
    pub fn modal() -> Self {
        Self { modal: true, open: None }
    }

    /// A slot for a non-modal view (a popover): it blocks the pointer only
    /// within its own bounds.
    pub fn non_modal() -> Self {
        Self { modal: false, open: None }
    }

    pub fn is_modal(&self) -> bool {
        self.modal
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
        let layer = layer::push(window, self.modal, own.downgrade(), cx);
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

    /// The open view's element, or `None`: the view in its layer. The bottom
    /// layer is deferred, so it paints above everything; a layer above it
    /// renders inline, inside the layer below's view (see the module doc).
    /// A modal overlay comes with its full-window backdrop; a non-modal
    /// one is placed where it is rendered.
    pub fn render(&self, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.render_with(|panel| panel, window, cx)
    }

    /// [`Overlay::render`] with `place` applied to a non-modal view's panel
    /// first: a popover wraps the panel in an `Anchored` to put it beside
    /// its trigger. A modal view is centred by its backdrop, so `place` is
    /// skipped. The result is deferred only for the bottom layer and
    /// renders inline above it, so what `place` returns must work in both.
    pub fn render_with(
        &self,
        place: impl FnOnce(AnyElement) -> AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let open = self.open.as_ref()?;
        let id = open.layer;
        let depth = layer::depth(window, id, cx).unwrap_or(0);
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
                move |_, window, cx| {
                    if !layer::is_top(window, id, cx) {
                        return;
                    }
                    let own = view.focus_handle(cx);
                    let target = match window.focused(cx) {
                        Some(focused) if own.contains(&focused, window) => focused,
                        _ => own,
                    };
                    window.defer(cx, move |window, cx| target.dispatch_action(&Dismiss, window, cx));
                }
            })
            // Tab stays inside the view.
            .child(FocusScope::new(&view).child(view));
        let element = if self.modal {
            let size = window.viewport_size();
            // Absolute at the window's origin, so the backdrop covers the
            // window from wherever it is rendered, inline included.
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
                        .child(panel),
                )
                .into_any_element()
        } else {
            place(panel.into_any_element())
        };
        Some(if wrap_in_deferred(depth) { deferred(element).into_any_element() } else { element })
    }
}

/// Whether the layer at `depth` renders through `deferred`: only the bottom
/// one. gpui asserts that nothing calls `defer_draw` while it draws the
/// deferred elements, and a layer above the bottom one is rendered inside
/// the bottom layer's view, which is drawn then. Rendered inline there, it
/// is lifted above the window with that view, and paints after what the
/// view rendered before it.
pub fn wrap_in_deferred(depth: usize) -> bool {
    depth == 0
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
    fn initial_focus_prefers_the_designated_handle() {
        assert_eq!(initial_focus(Some("initial"), &["first", "second"], "own"), "initial");
        assert_eq!(initial_focus(None, &["first", "second"], "own"), "first");
        assert_eq!(initial_focus(None, &[], "own"), "own");
        // A designated handle wins even with no stops.
        assert_eq!(initial_focus(Some("initial"), &[], "own"), "initial");
    }

    /// Nested deferred draws panic in gpui, so only the bottom layer is
    /// deferred; every layer above it renders inline inside it.
    #[test]
    fn only_the_bottom_layer_is_deferred() {
        assert!(wrap_in_deferred(0));
        assert!(!wrap_in_deferred(1));
        assert!(!wrap_in_deferred(2));
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
