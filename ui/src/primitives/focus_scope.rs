//! Focus containment: the controls an overlay's keyboard focus moves among.
//!
//! GPUI's own tab order is private and window-wide, so a view lists its
//! stops itself through [`FocusStops`], in order, and [`Overlay`] uses them
//! for the initial focus when it opens.
//!
//! [`Overlay`]: crate::primitives::overlay::Overlay

use gpui::{App, FocusHandle};

/// The focusable controls of a view, in Tab order.
pub trait FocusStops {
    /// The handles of the enabled controls, first to last. A control that
    /// isn't listed can't be reached with Tab.
    fn stops(&self, cx: &App) -> Vec<FocusHandle>;

    /// The control to focus when the view opens as an overlay, when it
    /// isn't the first stop.
    fn initial_focus(&self, _cx: &App) -> Option<FocusHandle> {
        None
    }
}
