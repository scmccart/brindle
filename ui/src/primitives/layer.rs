//! The layer stack: which open overlay acts on an Escape or an outside press.
//!
//! [`LayerStack`] is pure and per window: every open overlay pushes a layer
//! when it opens and removes it when it closes, and only the top layer acts.
//! Dismissals are deferred to the end of the event, so every layer decides
//! against the stack as it was when the event started: a press outside both
//! a popover and the dialog under it closes the popover alone, and the
//! dialog, which wasn't top when it looked, stays.
//!
//! The registry at the bottom keeps one stack per [`WindowId`] in a crate
//! global, and it is the only thing that touches the global. The crate's
//! overlays go through it, and so can an app's own overlay code, so that
//! both agree on which layer is top.

use std::collections::HashMap;

use gpui::{App, Global, WeakFocusHandle, Window, WindowId};

/// One open overlay in its window's stack. Ids are never reused within a
/// window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerId(u64);

/// A layer and whether it is modal: a modal layer covers the window with a
/// backdrop, a non-modal one blocks the pointer only within its own bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerEntry {
    pub id: LayerId,
    pub modal: bool,
}

/// The open overlays of one window, bottom first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerStack {
    layers: Vec<LayerEntry>,
    next_id: u64,
}

impl LayerStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a layer on top of the stack.
    pub fn push(&mut self, modal: bool) -> LayerId {
        let id = LayerId(self.next_id);
        self.next_id += 1;
        self.layers.push(LayerEntry { id, modal });
        id
    }

    /// Closes a layer, wherever it is in the stack. Returns whether it was
    /// there.
    pub fn remove(&mut self, id: LayerId) -> bool {
        let before = self.layers.len();
        self.layers.retain(|layer| layer.id != id);
        self.layers.len() < before
    }

    /// Whether `id` is the layer that acts on Escape and outside presses.
    pub fn is_top(&self, id: LayerId) -> bool {
        self.top() == Some(id)
    }

    pub fn top(&self) -> Option<LayerId> {
        self.layers.last().map(|layer| layer.id)
    }

    /// The layer's position from the bottom, which is its paint priority:
    /// deeper layers paint above.
    pub fn depth(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|layer| layer.id == id)
    }

    pub fn contains(&self, id: LayerId) -> bool {
        self.depth(id).is_some()
    }

    pub fn get(&self, id: LayerId) -> Option<LayerEntry> {
        self.layers.iter().find(|layer| layer.id == id).copied()
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// The layers, bottom first.
    pub fn layers(&self) -> &[LayerEntry] {
        &self.layers
    }
}

// ---- per-window registry ----------------------------------------------------

/// A window's stack, with each layer's view focus handle: a layer whose
/// handle no longer upgrades belongs to an overlay that was dropped without
/// closing (its owner went away), and is pruned on the next change or query.
#[derive(Default)]
struct WindowLayers {
    stack: LayerStack,
    handles: HashMap<LayerId, WeakFocusHandle>,
}

impl WindowLayers {
    fn prune(&mut self) {
        let dead: Vec<LayerId> = self
            .stack
            .layers()
            .iter()
            .map(|layer| layer.id)
            .filter(|id| self.handles.get(id).is_none_or(|handle| handle.upgrade().is_none()))
            .collect();
        for id in dead {
            self.stack.remove(id);
            self.handles.remove(&id);
        }
    }
}

/// The crate global: one stack per window.
#[derive(Default)]
struct Layers {
    windows: HashMap<WindowId, WindowLayers>,
}

impl Global for Layers {}

/// Opens a layer in `window` for the view whose focus handle is `handle`,
/// which also tells the registry when the layer's overlay is gone.
pub fn push(window: &Window, modal: bool, handle: WeakFocusHandle, cx: &mut App) -> LayerId {
    let layers = cx.default_global::<Layers>().windows.entry(window.window_handle().window_id()).or_default();
    layers.prune();
    let id = layers.stack.push(modal);
    layers.handles.insert(id, handle);
    id
}

/// Closes a layer. Nothing happens when it is already gone.
pub fn remove(window: &Window, id: LayerId, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    let Some(layers) = cx.default_global::<Layers>().windows.get_mut(&window_id) else {
        return;
    };
    layers.stack.remove(id);
    layers.handles.remove(&id);
    layers.prune();
    if layers.stack.is_empty() {
        cx.global_mut::<Layers>().windows.remove(&window_id);
    }
}

/// Whether `id` is the top layer of `window`, so it acts on an Escape or an
/// outside press. Called from event handlers, where the stack is the one the
/// event started with, since dismissals apply only after the event.
pub fn is_top(window: &Window, id: LayerId, cx: &mut App) -> bool {
    let window_id = window.window_handle().window_id();
    let Some(layers) = cx.default_global::<Layers>().windows.get_mut(&window_id) else {
        return false;
    };
    layers.prune();
    layers.stack.is_top(id)
}

/// The paint priority of `id` in `window`, or `None` when it isn't open.
/// Read-only, so it can be called while rendering.
pub fn depth(window: &Window, id: LayerId, cx: &App) -> Option<usize> {
    cx.try_global::<Layers>()?
        .windows
        .get(&window.window_handle().window_id())?
        .stack
        .depth(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_top_layer_acts() {
        let mut stack = LayerStack::new();
        assert!(stack.is_empty());
        assert_eq!(stack.top(), None);
        let dialog = stack.push(true);
        assert!(stack.is_top(dialog));
        let popover = stack.push(false);
        assert!(stack.is_top(popover));
        assert!(!stack.is_top(dialog));
        assert_eq!(stack.depth(dialog), Some(0));
        assert_eq!(stack.depth(popover), Some(1));
        assert_eq!(stack.get(dialog), Some(LayerEntry { id: dialog, modal: true }));
        assert_eq!(stack.get(popover), Some(LayerEntry { id: popover, modal: false }));
        // Closing the top layer hands the top back.
        assert!(stack.remove(popover));
        assert!(stack.is_top(dialog));
        assert!(!stack.is_top(popover));
        assert!(!stack.contains(popover));
        assert_eq!(stack.depth(popover), None);
        // Removing twice is harmless.
        assert!(!stack.remove(popover));
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn removing_a_middle_layer_keeps_the_order() {
        let mut stack = LayerStack::new();
        let a = stack.push(true);
        let b = stack.push(false);
        let c = stack.push(false);
        assert!(stack.remove(b));
        assert_eq!(stack.layers().iter().map(|l| l.id).collect::<Vec<_>>(), [a, c]);
        assert!(stack.is_top(c));
        assert_eq!(stack.depth(c), Some(1));
        // Ids are never reused.
        let d = stack.push(false);
        assert_ne!(d, b);
        assert_eq!(stack.depth(d), Some(2));
    }

    /// A press outside both a popover and the dialog it was opened from
    /// reaches both layers' outside-press handlers in one event. Each decides
    /// against the stack as it was when the event started, and the
    /// dismissals apply afterwards, so only the popover closes. Deciding and
    /// removing one layer at a time would close the dialog too.
    #[test]
    fn decisions_against_a_snapshot_dont_cascade() {
        let mut stack = LayerStack::new();
        let dialog = stack.push(true);
        let popover = stack.push(false);
        let handlers = [popover, dialog];

        let snapshot = stack.clone();
        let dismiss: Vec<LayerId> = handlers.into_iter().filter(|id| snapshot.is_top(*id)).collect();
        for id in &dismiss {
            stack.remove(*id);
        }
        assert_eq!(dismiss, [popover]);
        assert!(stack.contains(dialog));
        assert!(stack.is_top(dialog));

        // The cascade that deferring avoids.
        let mut eager = LayerStack::new();
        let dialog = eager.push(true);
        let popover = eager.push(false);
        for id in [popover, dialog] {
            if eager.is_top(id) {
                eager.remove(id);
            }
        }
        assert!(eager.is_empty());
    }
}
