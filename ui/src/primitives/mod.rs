//! The overlay primitives: layering, dismissal, focus containment and
//! return, placement, list navigation and focus-visible tracking. The
//! decisions are pure functions; the GPUI glue around them is thin.

pub mod focus_scope;
pub mod focus_visible;
pub mod layer;
pub mod nav;
pub mod overlay;
pub mod placement;

pub use focus_scope::{FocusScope, FocusStops, TabDirection, cycle};
pub use focus_visible::{Modality, focus_from_pointer, modality, set_modality, shows_ring};
pub use layer::{LayerEntry, LayerId, LayerStack};
pub use nav::{ListNav, TYPEAHEAD_TIMEOUT, Typeahead, typeahead_find};
pub use overlay::{Overlay, dismiss_on_request, initial_focus, restore_focus, wrap_in_deferred};
pub use placement::{Align, Anchored, Placement, Side, anchor_trigger, place};
