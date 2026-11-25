//! Brindle's shared UI components on GPUI, modelled on shadcn/ui.
//!
//! The crate depends on `gpui` only: it knows nothing of terminals, config
//! or the app's actions table. Apps call [`init`] once at startup and chain
//! [`default_bindings`] into their own keybindings.

// railway R3, types R2: no `_ =>` on one of Brindle's own enums. Non-test
// code only; `docs/architecture/README.md` lists the exceptions.
#![cfg_attr(not(test), warn(clippy::wildcard_enum_match_arm))]

use gpui::App;

pub mod actions;
pub mod components;
pub mod filter;
pub mod icon;
pub mod primitives;
pub mod style;
pub mod text_area;
pub mod text_input;
pub mod theme;
#[cfg(test)]
pub(crate) mod test_support;

pub use actions::*;
pub use components::*;
pub use icon::{Icon, IconElement, LEGIBLE_ICON_STROKE, stroke_width};
pub use primitives::*;
pub use style::{
    ButtonSize, ButtonStyle, ButtonVariant, CARD_BORDER, CARD_HEADER_HEIGHT, CardColors, CardStyle, CheckboxStyle,
    CommandStyle, DialogStyle, FIELD_HEIGHT, InputStyle, MenuStyle,
    PopoverStyle, SelectStyle, SwitchStyle, TabColors, TabDragStyle, TabListStyle, TabStyle, TabWidth,
    ToggleSize, ToggleStyle, ToggleVariant, TooltipStyle, button_style, checkbox_style, command_style, dialog_style,
    card_style, focus_ring, input_style, menu_style, popover_style, select_style, switch_style, switch_thumb_offset,
    tab_drag_style, tab_list_style, tab_style, toggle_style, tooltip_style,
};
pub use text_area::{DEFAULT_MAX_LINES, TextArea, TextAreaEvent};
pub use text_input::{TextInput, TextInputEvent};
pub use theme::{ActiveFont, ActiveTheme, Tokens, UiFont, UiTheme};

/// Sets the crate up in the app: it registers the keystroke observer that
/// tracks focus-visible modality (`primitives::focus_visible`), and nothing
/// else. It binds no keys, so a config reload that clears and rebinds every
/// key loses nothing of the crate's: the app takes [`default_bindings`]
/// into its own defaults instead. Call it once at startup; calling it
/// again is harmless.
pub fn init(cx: &mut App) {
    primitives::focus_visible::init(cx);
}
