//! Brindle's shared UI components on GPUI, modelled on shadcn/ui.
//!
//! The crate depends on `gpui` only: it knows nothing of terminals, config
//! or the app's actions table. Apps chain [`default_bindings`] into their
//! own keybindings.

pub mod actions;
pub mod text_input;

pub use actions::*;
pub use text_input::{TextInput, TextInputEvent};
