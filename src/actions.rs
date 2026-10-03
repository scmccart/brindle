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
        OpenCommandPalette,
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
        TmuxNewWindow,
        TmuxLayoutEvenHorizontal,
        TmuxLayoutEvenVertical,
        TmuxLayoutMainVertical,
        TmuxLayoutTiled,
        TmuxNextLayout,
        TmuxRotatePanes,
        TmuxSwapPanePrev,
        TmuxSwapPaneNext,
        TmuxBreakPane,
        TmuxRenameWindow,
        TmuxCommand,
        Quit,
        /// Consumes a key without doing anything (see the ctrl-shift-d guard).
        Swallow,
        // Palette navigation.
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

/// Every bindable action without a parameter: config name, command palette
/// label (`None` keeps it out of the palette) and constructor.
const ACTIONS: &[(&str, Option<&str>, fn() -> Box<dyn Action>)] = &[
    ("new_tab", Some("New Tab"), || Box::new(NewTab)),
    ("close_tab", Some("Close Tab"), || Box::new(CloseTab)),
    ("next_tab", Some("Next Tab"), || Box::new(NextTab)),
    ("prev_tab", Some("Previous Tab"), || Box::new(PrevTab)),
    ("last_tab", Some("Go to Last Tab"), || Box::new(ActivateTab(usize::MAX))),
    ("move_tab_left", Some("Move Tab Left"), || Box::new(MoveTabLeft)),
    ("move_tab_right", Some("Move Tab Right"), || Box::new(MoveTabRight)),
    ("new_window", Some("New Window"), || Box::new(NewWindow)),
    ("close_window", Some("Close Window"), || Box::new(CloseWindow)),
    ("copy", Some("Copy"), || Box::new(Copy)),
    ("paste", Some("Paste"), || Box::new(Paste)),
    ("paste_primary", Some("Paste Primary Selection"), || Box::new(PastePrimary)),
    ("select_all", Some("Select All"), || Box::new(SelectAll)),
    ("increase_font_size", Some("Increase Font Size"), || Box::new(IncreaseFontSize)),
    ("decrease_font_size", Some("Decrease Font Size"), || Box::new(DecreaseFontSize)),
    ("reset_font_size", Some("Reset Font Size"), || Box::new(ResetFontSize)),
    ("scroll_line_up", Some("Scroll Line Up"), || Box::new(ScrollLineUp)),
    ("scroll_line_down", Some("Scroll Line Down"), || Box::new(ScrollLineDown)),
    ("scroll_page_up", Some("Scroll Page Up"), || Box::new(ScrollPageUp)),
    ("scroll_page_down", Some("Scroll Page Down"), || Box::new(ScrollPageDown)),
    ("scroll_to_top", Some("Scroll to Top"), || Box::new(ScrollToTop)),
    ("scroll_to_bottom", Some("Scroll to Bottom"), || Box::new(ScrollToBottom)),
    ("clear_scrollback", Some("Clear Scrollback"), || Box::new(ClearScrollback)),
    ("open_profile_picker", Some("New Tab from Profile"), || Box::new(OpenProfilePicker)),
    ("open_command_palette", None, || Box::new(OpenCommandPalette)),
    ("reload_config", Some("Reload Config"), || Box::new(ReloadConfig)),
    ("open_config", Some("Open Config"), || Box::new(OpenConfig)),
    ("tmux_detach", Some("tmux: Detach"), || Box::new(TmuxDetach)),
    ("tmux_split_right", Some("tmux: Split Right"), || Box::new(TmuxSplitRight)),
    ("tmux_split_down", Some("tmux: Split Down"), || Box::new(TmuxSplitDown)),
    ("tmux_close_pane", Some("tmux: Close Pane"), || Box::new(TmuxClosePane)),
    ("tmux_zoom_pane", Some("tmux: Toggle Zoom"), || Box::new(TmuxZoomPane)),
    ("tmux_focus_left", Some("tmux: Focus Pane Left"), || Box::new(TmuxFocusLeft)),
    ("tmux_focus_right", Some("tmux: Focus Pane Right"), || Box::new(TmuxFocusRight)),
    ("tmux_focus_up", Some("tmux: Focus Pane Up"), || Box::new(TmuxFocusUp)),
    ("tmux_focus_down", Some("tmux: Focus Pane Down"), || Box::new(TmuxFocusDown)),
    ("tmux_new_window", Some("tmux: New Window"), || Box::new(TmuxNewWindow)),
    ("tmux_layout_even_horizontal", Some("tmux: Layout Even Horizontal"), || Box::new(TmuxLayoutEvenHorizontal)),
    ("tmux_layout_even_vertical", Some("tmux: Layout Even Vertical"), || Box::new(TmuxLayoutEvenVertical)),
    ("tmux_layout_main_vertical", Some("tmux: Layout Main Vertical"), || Box::new(TmuxLayoutMainVertical)),
    ("tmux_layout_tiled", Some("tmux: Layout Tiled"), || Box::new(TmuxLayoutTiled)),
    ("tmux_next_layout", Some("tmux: Next Layout"), || Box::new(TmuxNextLayout)),
    ("tmux_rotate_panes", Some("tmux: Rotate Panes"), || Box::new(TmuxRotatePanes)),
    ("tmux_swap_pane_prev", Some("tmux: Swap Pane with Previous"), || Box::new(TmuxSwapPanePrev)),
    ("tmux_swap_pane_next", Some("tmux: Swap Pane with Next"), || Box::new(TmuxSwapPaneNext)),
    ("tmux_break_pane", Some("tmux: Break Pane to New Tab"), || Box::new(TmuxBreakPane)),
    ("tmux_rename_window", Some("tmux: Rename Window"), || Box::new(TmuxRenameWindow)),
    ("tmux_command", Some("tmux: Run Command"), || Box::new(TmuxCommand)),
    ("quit", Some("Quit"), || Box::new(Quit)),
    ("none", None, || Box::new(NoAction)),
];

/// Actions taking a 1-based number, written as `<prefix><N>`.
const NUMBERED_ACTIONS: &[(&str, fn(usize) -> Box<dyn Action>)] = &[
    ("activate_tab_", |n| Box::new(ActivateTab(n))),
    ("new_tab_with_profile_", |n| Box::new(NewTabWithProfile(n))),
];

pub fn action_by_name(name: &str) -> Option<Box<dyn Action>> {
    for (prefix, build) in NUMBERED_ACTIONS {
        if let Some(n) = name.strip_prefix(prefix) {
            return Some(build(n.parse::<usize>().ok()?.checked_sub(1)?));
        }
    }
    ACTIONS.iter().find(|(n, _, _)| *n == name).map(|(_, _, build)| build())
}

/// Bindable action names, for `--list-actions`.
pub fn action_names() -> impl Iterator<Item = String> {
    NUMBERED_ACTIONS
        .iter()
        .map(|(prefix, _)| format!("{prefix}<N>"))
        .chain(ACTIONS.iter().map(|(name, _, _)| name.to_string()))
}

/// The command palette's entries, as (label, action) in table order: every
/// labelled action for which `available` holds.
pub fn palette_commands(mut available: impl FnMut(&dyn Action) -> bool) -> Vec<(&'static str, Box<dyn Action>)> {
    ACTIONS
        .iter()
        .filter_map(|(_, label, build)| Some(((*label)?, build())))
        .filter(|(_, action)| available(&**action))
        .collect()
}

/// Default bindings follow GNOME Terminal / kitty conventions so plain
/// ctrl-<letter> always reaches the shell (and tmux's prefix).
fn default_bindings() -> Vec<KeyBinding> {
    let w = Some("Workspace");
    let t = Some("Terminal");
    let p = Some("Palette");
    let x = Some("TmuxWindow");
    let mut bindings = vec![
        KeyBinding::new("ctrl-shift-t", NewTab, w),
        KeyBinding::new("ctrl-shift-w", CloseTab, w),
        // For the same action and context the last binding is the one the
        // command palette shows, so the main key comes last.
        KeyBinding::new("ctrl-pagedown", NextTab, w),
        KeyBinding::new("ctrl-pageup", PrevTab, w),
        KeyBinding::new("ctrl-tab", NextTab, w),
        KeyBinding::new("ctrl-shift-tab", PrevTab, w),
        KeyBinding::new("ctrl-shift-pagedown", MoveTabRight, w),
        KeyBinding::new("ctrl-shift-pageup", MoveTabLeft, w),
        KeyBinding::new("ctrl-shift-n", NewWindow, w),
        KeyBinding::new("ctrl-shift-q", Quit, w),
        KeyBinding::new("ctrl-shift-p", OpenCommandPalette, w),
        KeyBinding::new("ctrl-shift-space", OpenProfilePicker, w),
        KeyBinding::new("ctrl-shift-r", ReloadConfig, w),
        KeyBinding::new("ctrl-,", OpenConfig, w),
        // Detach only means something in a control-mode tab; elsewhere the
        // key is swallowed so habit never sends ^D to a shell.
        KeyBinding::new("ctrl-shift-d", Swallow, w),
        KeyBinding::new("ctrl-shift-d", TmuxDetach, x),
        KeyBinding::new("ctrl-+", IncreaseFontSize, w),
        KeyBinding::new("ctrl-shift-=", IncreaseFontSize, w),
        KeyBinding::new("ctrl-=", IncreaseFontSize, w),
        KeyBinding::new("ctrl--", DecreaseFontSize, w),
        KeyBinding::new("ctrl-0", ResetFontSize, w),
        KeyBinding::new("shift-insert", Paste, t),
        KeyBinding::new("ctrl-insert", Copy, t),
        KeyBinding::new("ctrl-shift-c", Copy, t),
        KeyBinding::new("ctrl-shift-v", Paste, t),
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
        KeyBinding::new("ctrl-shift-v", Paste, p),
        KeyBinding::new("shift-insert", Paste, p),
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
    fn every_listed_action_resolves() {
        for name in action_names() {
            let name = name.replace("<N>", "3");
            assert!(action_by_name(&name).is_some(), "{name}");
        }
        assert!(action_by_name("activate_tab_0").is_none());
        assert!(action_by_name("bogus").is_none());
        assert!(action_by_name("swallow").is_none());
        assert_eq!(
            action_by_name("activate_tab_2").unwrap().as_any().downcast_ref::<ActivateTab>(),
            Some(&ActivateTab(1))
        );
    }

    #[test]
    fn palette_labels_are_unique_and_resolve() {
        let mut seen = std::collections::HashSet::new();
        for (name, label, build) in ACTIONS {
            let Some(label) = label else { continue };
            assert!(seen.insert(*label), "duplicate label {label:?}");
            let action = action_by_name(name).unwrap_or_else(|| panic!("{name} does not resolve"));
            assert!(action.partial_eq(&*build()), "{name}");
        }
        assert!(palette_commands(|_| true).iter().all(|(_, a)| !a.partial_eq(&OpenCommandPalette) && !a.partial_eq(&NoAction)));
    }

    #[test]
    fn palette_lists_only_available_actions() {
        use std::any::TypeId;
        // As if focus were in a local terminal: workspace + terminal handlers,
        // a global listener, and ActivateTab (no_json) on the workspace.
        let available = [
            TypeId::of::<ActivateTab>(),
            TypeId::of::<ClearScrollback>(),
            TypeId::of::<ReloadConfig>(),
            TypeId::of::<OpenCommandPalette>(),
            TypeId::of::<NoAction>(),
        ];
        let labels: Vec<&str> =
            palette_commands(|a| available.contains(&a.as_any().type_id())).into_iter().map(|(l, _)| l).collect();
        assert_eq!(labels, ["Go to Last Tab", "Clear Scrollback", "Reload Config"]);
        assert!(palette_commands(|_| true).iter().any(|(l, _)| *l == "tmux: Split Right"));
    }
}
