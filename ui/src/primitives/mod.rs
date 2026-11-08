//! The overlay primitives: layering, dismissal, focus containment and
//! return. The decisions are pure functions; the GPUI glue around them is
//! thin.

pub mod focus_scope;
pub mod layer;
pub mod nav;
pub mod overlay;
pub mod placement;

pub use focus_scope::{FocusScope, FocusStops, TabDirection, cycle};
pub use layer::{LayerEntry, LayerId, LayerStack};
pub use nav::{ListNav, TYPEAHEAD_TIMEOUT, Typeahead, typeahead_find};
pub use overlay::{Overlay, dismiss_on_request, initial_focus, restore_focus, wrap_in_deferred};
pub use placement::{Align, Anchored, Placement, Side, anchor_trigger, place};
