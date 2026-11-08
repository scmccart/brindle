//! Brindle's shared UI components on GPUI, modelled on shadcn/ui.
//!
//! The crate depends on `gpui` only: it knows nothing of terminals, config
//! or the app's actions table. Apps call [`init`] once at startup and chain
//! [`default_bindings`] into their own keybindings.

use gpui::App;

pub mod actions;
pub mod components;
pub mod filter;
pub mod icon;
pub mod primitives;
pub mod style;
pub mod text_input;
pub mod theme;

pub use actions::*;
pub use components::*;
pub use icon::{Icon, IconElement};
pub use primitives::*;
pub use style::{
    ButtonSize, ButtonStyle, ButtonVariant, CheckboxStyle, InputStyle, SwitchStyle, button_style, checkbox_style,
    focus_ring, input_style, switch_style, switch_thumb_offset,
};
pub use text_input::{TextInput, TextInputEvent};
pub use theme::{ActiveTheme, Tokens, UiTheme};

/// Sets the crate up in the app: it registers the keystroke observer that
/// tracks focus-visible modality (`primitives::focus_visible`), and nothing
/// else. It binds no keys, so a config reload that clears and rebinds every
/// key loses nothing of the crate's: the app takes [`default_bindings`]
/// into its own defaults instead. Call it once at startup; calling it
/// again is harmless.
pub fn init(cx: &mut App) {
    primitives::focus_visible::init(cx);
}
