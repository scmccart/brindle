//! The overlay primitives: layering, dismissal, focus containment and
//! return. The decisions are pure functions; the GPUI glue around them is
//! thin.

pub mod layer;

pub use layer::{LayerEntry, LayerId, LayerStack};
