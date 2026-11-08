//! The overlay primitives: layering, dismissal, focus containment and
//! return. The decisions are pure functions; the GPUI glue around them is
//! thin.

pub mod focus_scope;
pub mod layer;
pub mod overlay;

pub use focus_scope::{FocusScope, FocusStops, TabDirection, cycle};
pub use layer::{LayerEntry, LayerId, LayerStack};
pub use overlay::{Overlay, dismiss_on_request, initial_focus, restore_focus, wrap_in_deferred};
