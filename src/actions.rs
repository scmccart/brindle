//! Actions and default keybindings.

use gpui::{Action, App, KeyBinding, NoAction, actions};

use crate::settings::Settings;

actions!(
    brindle,
    [
        NewTab,
        CloseTab,
        NextTab,
        PrevTab,
        MoveTabLeft,
        MoveTabRight,
        NewWindow,
        CloseWindow,
        Copy,
        Paste,
        PastePrimary,
        SelectAll,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        ScrollLineUp,
        ScrollLineDown,
        ScrollPageUp,
        ScrollPageDown,
        ScrollToTop,
        ScrollToBottom,
        ClearScrollback,
        OpenProfilePicker,
        ReloadConfig,
        OpenConfig,
        TmuxDetach,
        TmuxSplitRight,
        TmuxSplitDown,
        TmuxClosePane,
        TmuxZoomPane,
        TmuxFocusLeft,
        TmuxFocusRight,
        TmuxFocusUp,
        TmuxFocusDown,
        Quit,
        // Profile picker navigation.
        PickerUp,
        PickerDown,
        PickerConfirm,
        PickerCancel,
    ]
);

/// Activate the tab at a 0-based index (`usize::MAX` = last tab).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = brindle, no_json)]
pub struct ActivateTab(pub usize);

/// Open a new tab using the profile at a 0-based index.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = brindle, no_json)]
pub struct NewTabWithProfile(pub usize);

/// Every bindable action, by config name.
pub fn action_by_name(name: &str) -> Option<Box<dyn Action>> {
    if let Some(n) = name.strip_prefix("activate_tab_") {
        let n: usize = n.parse().ok()?;
        return Some(Box::new(ActivateTab(n.checked_sub(1)?)));
    }
    if let Some(n) = name.strip_prefix("new_tab_with_profile_") {
        let n: usize = n.parse().ok()?;
        return Some(Box::new(NewTabWithProfile(n.checked_sub(1)?)));
    }
    Some(match name {
        "none" => Box::new(NoAction),
        "new_tab" => Box::new(NewTab),
        "close_tab" => Box::new(CloseTab),
        "next_tab" => Box::new(NextTab),
        "prev_tab" => Box::new(PrevTab),
        "last_tab" => Box::new(ActivateTab(usize::MAX)),
        "move_tab_left" => Box::new(MoveTabLeft),
        "move_tab_right" => Box::new(MoveTabRight),
        "new_window" => Box::new(NewWindow),
        "close_window" => Box::new(CloseWindow),
        "copy" => Box::new(Copy),
        "paste" => Box::new(Paste),
        "paste_primary" => Box::new(PastePrimary),
        "select_all" => Box::new(SelectAll),
        "increase_font_size" => Box::new(IncreaseFontSize),
        "decrease_font_size" => Box::new(DecreaseFontSize),
        "reset_font_size" => Box::new(ResetFontSize),
        "scroll_line_up" => Box::new(ScrollLineUp),
        "scroll_line_down" => Box::new(ScrollLineDown),
        "scroll_page_up" => Box::new(ScrollPageUp),
        "scroll_page_down" => Box::new(ScrollPageDown),
        "scroll_to_top" => Box::new(ScrollToTop),
        "scroll_to_bottom" => Box::new(ScrollToBottom),
        "clear_scrollback" => Box::new(ClearScrollback),
        "open_profile_picker" => Box::new(OpenProfilePicker),
        "reload_config" => Box::new(ReloadConfig),
        "open_config" => Box::new(OpenConfig),
        "tmux_detach" => Box::new(TmuxDetach),
        "tmux_split_right" => Box::new(TmuxSplitRight),
        "tmux_split_down" => Box::new(TmuxSplitDown),
        "tmux_close_pane" => Box::new(TmuxClosePane),
        "tmux_zoom_pane" => Box::new(TmuxZoomPane),
        "tmux_focus_left" => Box::new(TmuxFocusLeft),
        "tmux_focus_right" => Box::new(TmuxFocusRight),
        "tmux_focus_up" => Box::new(TmuxFocusUp),
        "tmux_focus_down" => Box::new(TmuxFocusDown),
        "quit" => Box::new(Quit),
        _ => return None,
    })
}

pub const ACTION_NAMES: &[&str] = &[
    "new_tab",
    "new_tab_with_profile_<N>",
    "close_tab",
    "next_tab",
    "prev_tab",
    "activate_tab_<N>",
    "last_tab",
    "move_tab_left",
    "move_tab_right",
    "new_window",
    "close_window",
    "copy",
    "paste",
    "paste_primary",
    "select_all",
    "increase_font_size",
    "decrease_font_size",
    "reset_font_size",
    "scroll_line_up",
    "scroll_line_down",
    "scroll_page_up",
    "scroll_page_down",
    "scroll_to_top",
    "scroll_to_bottom",
    "clear_scrollback",
    "open_profile_picker",
    "reload_config",
    "open_config",
    "tmux_detach",
    "tmux_split_right",
    "tmux_split_down",
    "tmux_close_pane",
    "tmux_zoom_pane",
    "tmux_focus_left",
    "tmux_focus_right",
    "tmux_focus_up",
    "tmux_focus_down",
    "quit",
    "none",
];

/// Default bindings follow GNOME Terminal / kitty conventions so plain
/// ctrl-<letter> always reaches the shell (and tmux's prefix).
fn default_bindings() -> Vec<KeyBinding> {
    let w = Some("Workspace");
    let t = Some("Terminal");
    let p = Some("ProfilePicker");
    let x = Some("TmuxWindow");
    let mut bindings = vec![
        KeyBinding::new("ctrl-shift-t", NewTab, w),
        KeyBinding::new("ctrl-shift-w", CloseTab, w),
        KeyBinding::new("ctrl-tab", NextTab, w),
        KeyBinding::new("ctrl-shift-tab", PrevTab, w),
        KeyBinding::new("ctrl-pagedown", NextTab, w),
        KeyBinding::new("ctrl-pageup", PrevTab, w),
        KeyBinding::new("ctrl-shift-pagedown", MoveTabRight, w),
        KeyBinding::new("ctrl-shift-pageup", MoveTabLeft, w),
        KeyBinding::new("ctrl-shift-n", NewWindow, w),
        KeyBinding::new("ctrl-shift-q", Quit, w),
        KeyBinding::new("ctrl-shift-p", OpenProfilePicker, w),
        KeyBinding::new("ctrl-shift-r", ReloadConfig, w),
        KeyBinding::new("ctrl-,", OpenConfig, w),
        KeyBinding::new("ctrl-shift-d", TmuxDetach, w),
        KeyBinding::new("ctrl-=", IncreaseFontSize, w),
        KeyBinding::new("ctrl-+", IncreaseFontSize, w),
        KeyBinding::new("ctrl-shift-=", IncreaseFontSize, w),
        KeyBinding::new("ctrl--", DecreaseFontSize, w),
        KeyBinding::new("ctrl-0", ResetFontSize, w),
        KeyBinding::new("ctrl-shift-c", Copy, t),
        KeyBinding::new("ctrl-shift-v", Paste, t),
        KeyBinding::new("shift-insert", Paste, t),
        KeyBinding::new("ctrl-insert", Copy, t),
        KeyBinding::new("ctrl-shift-a", SelectAll, t),
        KeyBinding::new("shift-pageup", ScrollPageUp, t),
        KeyBinding::new("shift-pagedown", ScrollPageDown, t),
        KeyBinding::new("ctrl-shift-up", ScrollLineUp, t),
        KeyBinding::new("ctrl-shift-down", ScrollLineDown, t),
        KeyBinding::new("shift-home", ScrollToTop, t),
        KeyBinding::new("shift-end", ScrollToBottom, t),
        KeyBinding::new("ctrl-shift-k", ClearScrollback, t),
        KeyBinding::new("ctrl-shift-e", TmuxSplitRight, x),
        KeyBinding::new("ctrl-shift-o", TmuxSplitDown, x),
        KeyBinding::new("ctrl-shift-x", TmuxClosePane, x),
        KeyBinding::new("ctrl-shift-z", TmuxZoomPane, x),
        KeyBinding::new("alt-left", TmuxFocusLeft, x),
        KeyBinding::new("alt-right", TmuxFocusRight, x),
        KeyBinding::new("alt-up", TmuxFocusUp, x),
        KeyBinding::new("alt-down", TmuxFocusDown, x),
        KeyBinding::new("up", PickerUp, p),
        KeyBinding::new("down", PickerDown, p),
        KeyBinding::new("ctrl-p", PickerUp, p),
        KeyBinding::new("ctrl-n", PickerDown, p),
        KeyBinding::new("enter", PickerConfirm, p),
        KeyBinding::new("escape", PickerCancel, p),
    ];
    for n in 1..=9 {
        let index = if n == 9 { usize::MAX } else { n - 1 };
        bindings.push(KeyBinding::new(&format!("alt-{n}"), ActivateTab(index), w));
        bindings.push(KeyBinding::new(&format!("ctrl-alt-{n}"), NewTabWithProfile(n - 1), w));
    }
    bindings
}

pub fn bind_keys(cx: &mut App) {
    cx.clear_key_bindings();
    cx.bind_keys(default_bindings());
    let user: Vec<(String, String)> = Settings::get(cx)
        .config
        .keybindings
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (keys, name) in user {
        let Some(action) = action_by_name(&name) else {
            log::warn!("unknown action {name:?} bound to {keys:?}");
            continue;
        };
        // Bound in the innermost context so user bindings beat the defaults.
        let context = gpui::KeyBindingContextPredicate::parse("Terminal").unwrap();
        match KeyBinding::load(
            &keys,
            action,
            Some(context.into()),
            false,
            None,
            &gpui::DummyKeyboardMapper,
        ) {
            Ok(binding) => cx.bind_keys([binding]),
            Err(err) => log::warn!("invalid keybinding {keys:?}: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_documented_action_resolves() {
        for name in ACTION_NAMES {
            let name = name.replace("<N>", "3");
            assert!(action_by_name(&name).is_some(), "{name}");
        }
        assert!(action_by_name("activate_tab_0").is_none());
        assert!(action_by_name("bogus").is_none());
        assert_eq!(
            action_by_name("activate_tab_2").unwrap().as_any().downcast_ref::<ActivateTab>(),
            Some(&ActivateTab(1))
        );
    }
}
