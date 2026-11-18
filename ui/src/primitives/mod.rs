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
pub use nav::{ListNav, TYPEAHEAD_TIMEOUT, Typeahead, typeahead_find, typeahead_find_labelled};
pub use overlay::{
    DismissTarget, ENTER_DURATION, Overlay, OverlayKind, deferred_priority, dismiss_on_request, dismiss_target,
    enter_animation_id, initial_focus, restore_focus, wrap_in_deferred,
};
pub use placement::{
    Align, Anchored, EDGE_MARGIN, Placement, SIDE_OFFSET, Side, anchor_trigger, available_height, place,
};
