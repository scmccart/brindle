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
        RenameTab,
        NewWindow,
        CloseWindow,
        Copy,
        Paste,
        PastePrimary,
        SelectAll,
        CopyLink,
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
        JumpToPreviousPrompt,
        JumpToNextPrompt,
        SelectLastOutput,
        CopyLastOutput,
        Find,
        FindNext,
        FindPrevious,
        FindClose,
        FindToggleCase,
        FindToggleRegex,
        OpenProfilePicker,
        OpenCommandPalette,
        ReloadConfig,
        OpenConfig,
        OpenSettings,
        SplitZoneRight,
        SplitZoneDown,
        CloseZone,
        FocusZoneLeft,
        FocusZoneRight,
        FocusZoneUp,
        FocusZoneDown,
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
        TmuxNewSession,
        Quit,
        /// Consumes a key without doing anything (see the ctrl-shift-d guard).
        Swallow,
        // Palette navigation.
        PickerUp,
        PickerDown,
        PickerConfirm,
        PickerCancel,
        // Settings dialog.
        SettingsUp,
        SettingsDown,
        SettingsConfirm,
        SettingsCancel,
        SettingsNextField,
        SettingsPrevField,
        SettingsNew,
        SettingsEdit,
        SettingsDelete,
        SettingsSectionNext,
        SettingsSectionPrev,
        SettingsDuplicate,
        SettingsMoveUp,
        SettingsMoveDown,
        SettingsChoiceNext,
        SettingsChoicePrev,
        SettingsToggle,
        // Text inputs.
        TextInputLeft,
        TextInputRight,
        TextInputWordLeft,
        TextInputWordRight,
        TextInputHome,
        TextInputEnd,
        TextInputSelectLeft,
        TextInputSelectRight,
        TextInputSelectWordLeft,
        TextInputSelectWordRight,
        TextInputSelectHome,
        TextInputSelectAll,
        TextInputSelectEnd,
        TextInputBackspace,
        TextInputDelete,
        TextInputDeleteWordBack,
        TextInputCopy,
        TextInputCut,
        TextInputPaste,
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
    ("rename_tab", Some("Rename Tab"), || Box::new(RenameTab)),
    ("new_window", Some("New Window"), || Box::new(NewWindow)),
    ("close_window", Some("Close Window"), || Box::new(CloseWindow)),
    ("copy", Some("Copy"), || Box::new(Copy)),
    ("paste", Some("Paste"), || Box::new(Paste)),
    ("paste_primary", Some("Paste Primary Selection"), || Box::new(PastePrimary)),
    ("select_all", Some("Select All"), || Box::new(SelectAll)),
    ("copy_link", Some("Copy Link Under Mouse"), || Box::new(CopyLink)),
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
    ("jump_to_previous_prompt", Some("Jump to Previous Prompt"), || Box::new(JumpToPreviousPrompt)),
    ("jump_to_next_prompt", Some("Jump to Next Prompt"), || Box::new(JumpToNextPrompt)),
    ("select_last_output", Some("Select Last Command Output"), || Box::new(SelectLastOutput)),
    ("copy_last_output", Some("Copy Last Command Output"), || Box::new(CopyLastOutput)),
    ("find", Some("Find"), || Box::new(Find)),
    // The find bar's own actions act on an open bar, so they stay out of
    // the palette.
    ("find_next", None, || Box::new(FindNext)),
    ("find_previous", None, || Box::new(FindPrevious)),
    ("find_close", None, || Box::new(FindClose)),
    ("find_toggle_case", None, || Box::new(FindToggleCase)),
    ("find_toggle_regex", None, || Box::new(FindToggleRegex)),
    ("open_profile_picker", Some("New Tab from Profile"), || Box::new(OpenProfilePicker)),
    ("open_command_palette", None, || Box::new(OpenCommandPalette)),
    ("reload_config", Some("Reload Config"), || Box::new(ReloadConfig)),
    ("open_config", Some("Open Config"), || Box::new(OpenConfig)),
    ("open_settings", Some("Open Settings"), || Box::new(OpenSettings)),
    ("split_zone_right", Some("Split Zone Right"), || Box::new(SplitZoneRight)),
    ("split_zone_down", Some("Split Zone Down"), || Box::new(SplitZoneDown)),
    ("close_zone", Some("Close Zone"), || Box::new(CloseZone)),
    ("focus_zone_left", Some("Focus Zone Left"), || Box::new(FocusZoneLeft)),
    ("focus_zone_right", Some("Focus Zone Right"), || Box::new(FocusZoneRight)),
    ("focus_zone_up", Some("Focus Zone Up"), || Box::new(FocusZoneUp)),
    ("focus_zone_down", Some("Focus Zone Down"), || Box::new(FocusZoneDown)),
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
    ("tmux_new_session", Some("tmux: New Session"), || Box::new(TmuxNewSession)),
    ("quit", Some("Quit"), || Box::new(Quit)),
    // The settings dialog's own actions: unlabelled, so they stay out of the
    // command palette, but `--action` and user bindings can reach them.
    ("settings_up", None, || Box::new(SettingsUp)),
    ("settings_down", None, || Box::new(SettingsDown)),
    ("settings_confirm", None, || Box::new(SettingsConfirm)),
    ("settings_cancel", None, || Box::new(SettingsCancel)),
    ("settings_next_field", None, || Box::new(SettingsNextField)),
    ("settings_prev_field", None, || Box::new(SettingsPrevField)),
    ("settings_new", None, || Box::new(SettingsNew)),
    ("settings_edit", None, || Box::new(SettingsEdit)),
    ("settings_delete", None, || Box::new(SettingsDelete)),
    ("settings_section_next", None, || Box::new(SettingsSectionNext)),
    ("settings_section_prev", None, || Box::new(SettingsSectionPrev)),
    ("settings_duplicate", None, || Box::new(SettingsDuplicate)),
    ("settings_move_up", None, || Box::new(SettingsMoveUp)),
    ("settings_move_down", None, || Box::new(SettingsMoveDown)),
    ("settings_choice_next", None, || Box::new(SettingsChoiceNext)),
    ("settings_choice_prev", None, || Box::new(SettingsChoicePrev)),
    ("settings_toggle", None, || Box::new(SettingsToggle)),
    // Palette navigation and text editing, for `--action` and user bindings.
    ("picker_up", None, || Box::new(PickerUp)),
    ("picker_down", None, || Box::new(PickerDown)),
    ("picker_confirm", None, || Box::new(PickerConfirm)),
    ("picker_cancel", None, || Box::new(PickerCancel)),
    ("text_input_left", None, || Box::new(TextInputLeft)),
    ("text_input_right", None, || Box::new(TextInputRight)),
    ("text_input_word_left", None, || Box::new(TextInputWordLeft)),
    ("text_input_word_right", None, || Box::new(TextInputWordRight)),
    ("text_input_home", None, || Box::new(TextInputHome)),
    ("text_input_end", None, || Box::new(TextInputEnd)),
    ("text_input_select_left", None, || Box::new(TextInputSelectLeft)),
    ("text_input_select_right", None, || Box::new(TextInputSelectRight)),
    ("text_input_select_word_left", None, || Box::new(TextInputSelectWordLeft)),
    ("text_input_select_word_right", None, || Box::new(TextInputSelectWordRight)),
    ("text_input_select_home", None, || Box::new(TextInputSelectHome)),
    ("text_input_select_all", None, || Box::new(TextInputSelectAll)),
    ("text_input_select_end", None, || Box::new(TextInputSelectEnd)),
    ("text_input_backspace", None, || Box::new(TextInputBackspace)),
    ("text_input_delete", None, || Box::new(TextInputDelete)),
    ("text_input_delete_word_back", None, || Box::new(TextInputDeleteWordBack)),
    ("text_input_copy", None, || Box::new(TextInputCopy)),
    ("text_input_cut", None, || Box::new(TextInputCut)),
    ("text_input_paste", None, || Box::new(TextInputPaste)),
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
    let z = Some("ZoneTree");
    let s = Some("SettingsDialog");
    let i = Some("TextInput");
    let c = Some("SettingsChoice");
    let f = Some("FindBar");
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
        // GPUI reports shift-, as `<` (without shift) on US-style layouts;
        // the readable spelling comes last so the palette shows it.
        KeyBinding::new("ctrl-<", OpenSettings, w),
        KeyBinding::new("ctrl-shift-,", OpenSettings, w),
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
        // Prompt jumps fall back to scrolling one line in a terminal with
        // no shell-integration marks.
        KeyBinding::new("ctrl-shift-up", JumpToPreviousPrompt, t),
        KeyBinding::new("ctrl-shift-down", JumpToNextPrompt, t),
        KeyBinding::new("shift-home", ScrollToTop, t),
        KeyBinding::new("shift-end", ScrollToBottom, t),
        KeyBinding::new("ctrl-shift-k", ClearScrollback, t),
        KeyBinding::new("ctrl-shift-f", Find, t),
        KeyBinding::new("enter", FindNext, f),
        KeyBinding::new("shift-enter", FindPrevious, f),
        KeyBinding::new("escape", FindClose, f),
        KeyBinding::new("alt-c", FindToggleCase, f),
        KeyBinding::new("alt-r", FindToggleRegex, f),
        KeyBinding::new("ctrl-shift-e", SplitZoneRight, z),
        KeyBinding::new("ctrl-shift-o", SplitZoneDown, z),
        KeyBinding::new("ctrl-shift-x", CloseZone, z),
        KeyBinding::new("alt-left", FocusZoneLeft, z),
        KeyBinding::new("alt-right", FocusZoneRight, z),
        KeyBinding::new("alt-up", FocusZoneUp, z),
        KeyBinding::new("alt-down", FocusZoneDown, z),
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
        KeyBinding::new("up", SettingsUp, s),
        KeyBinding::new("down", SettingsDown, s),
        KeyBinding::new("ctrl-p", SettingsUp, s),
        KeyBinding::new("ctrl-n", SettingsDown, s),
        KeyBinding::new("enter", SettingsConfirm, s),
        KeyBinding::new("escape", SettingsCancel, s),
        KeyBinding::new("tab", SettingsNextField, s),
        KeyBinding::new("shift-tab", SettingsPrevField, s),
        KeyBinding::new("ctrl-pagedown", SettingsSectionNext, s),
        KeyBinding::new("ctrl-pageup", SettingsSectionPrev, s),
        KeyBinding::new("ctrl-tab", SettingsSectionNext, s),
        KeyBinding::new("ctrl-shift-tab", SettingsSectionPrev, s),
        KeyBinding::new("alt-up", SettingsMoveUp, s),
        KeyBinding::new("alt-down", SettingsMoveDown, s),
        // The profile editor's theme and tmux choices and default checkbox.
        KeyBinding::new("left", SettingsChoicePrev, c),
        KeyBinding::new("right", SettingsChoiceNext, c),
        KeyBinding::new("space", SettingsToggle, c),
        // Text inputs leave enter, escape, tab, up/down and ctrl-p/ctrl-n
        // to the palette or dialog around them.
        KeyBinding::new("left", TextInputLeft, i),
        KeyBinding::new("right", TextInputRight, i),
        KeyBinding::new("ctrl-left", TextInputWordLeft, i),
        KeyBinding::new("ctrl-right", TextInputWordRight, i),
        KeyBinding::new("home", TextInputHome, i),
        KeyBinding::new("end", TextInputEnd, i),
        KeyBinding::new("ctrl-e", TextInputEnd, i),
        KeyBinding::new("shift-left", TextInputSelectLeft, i),
        KeyBinding::new("shift-right", TextInputSelectRight, i),
        KeyBinding::new("ctrl-shift-left", TextInputSelectWordLeft, i),
        KeyBinding::new("ctrl-shift-right", TextInputSelectWordRight, i),
        KeyBinding::new("shift-home", TextInputSelectHome, i),
        KeyBinding::new("shift-end", TextInputSelectEnd, i),
        KeyBinding::new("ctrl-shift-e", TextInputSelectEnd, i),
        // Select all, as in desktop text boxes; ctrl-shift-a is also the
        // terminal's select-all key.
        KeyBinding::new("ctrl-a", TextInputSelectAll, i),
        KeyBinding::new("ctrl-shift-a", TextInputSelectAll, i),
        KeyBinding::new("backspace", TextInputBackspace, i),
        KeyBinding::new("shift-backspace", TextInputBackspace, i),
        KeyBinding::new("delete", TextInputDelete, i),
        KeyBinding::new("ctrl-backspace", TextInputDeleteWordBack, i),
        KeyBinding::new("ctrl-w", TextInputDeleteWordBack, i),
        KeyBinding::new("ctrl-insert", TextInputCopy, i),
        KeyBinding::new("ctrl-shift-c", TextInputCopy, i),
        KeyBinding::new("ctrl-c", TextInputCopy, i),
        KeyBinding::new("shift-delete", TextInputCut, i),
        KeyBinding::new("ctrl-x", TextInputCut, i),
        KeyBinding::new("shift-insert", TextInputPaste, i),
        KeyBinding::new("ctrl-shift-v", TextInputPaste, i),
        KeyBinding::new("ctrl-v", TextInputPaste, i),
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
    fn default_bindings_parse() {
        // KeyBinding::new panics on a keystroke it can't parse.
        let bindings = default_bindings();
        let settings = bindings.iter().filter(|b| b.action().partial_eq(&OpenSettings)).count();
        assert_eq!(settings, 2);
        let find: Vec<String> = bindings
            .iter()
            .filter(|b| b.action().name().starts_with("brindle::Find"))
            .map(|b| format!("{:?}", b.keystrokes()))
            .collect();
        assert_eq!(find.len(), 6, "{find:?}");
        assert!(action_by_name("find").is_some_and(|a| a.partial_eq(&Find)));
        assert!(action_by_name("find_toggle_regex").is_some_and(|a| a.partial_eq(&FindToggleRegex)));
    }

    #[test]
    fn settings_key_matches_what_gpui_reports() {
        // ctrl-shift-, on a US layout: xkb gives `<`, and GPUI drops shift
        // for symbols.
        let typed = gpui::Keystroke {
            modifiers: gpui::Modifiers { control: true, ..Default::default() },
            key: "<".into(),
            key_char: Some("<".into()),
        };
        let matched = default_bindings()
            .iter()
            .filter(|b| b.action().partial_eq(&OpenSettings))
            .map(|b| (format!("{:?}", b.keystrokes()), b.match_keystrokes(&[typed.clone()])))
            .collect::<Vec<_>>();
        assert!(matched.iter().any(|(_, m)| *m == Some(false)), "{matched:#?}");
    }

    /// Real ctrl-shift-up/down presses (as GPUI reports them on Linux)
    /// resolve to the prompt jumps in a local tab and in a tmux pane alike;
    /// `--action` in the e2e cases skips the keymap, so this covers it.
    #[test]
    fn prompt_jump_keys_resolve_in_local_and_tmux_terminals() {
        use gpui::{KeyContext, Keymap, Keystroke, Modifiers};
        let keymap = Keymap::new(default_bindings());
        let press = |key: &str| Keystroke {
            modifiers: Modifiers { control: true, shift: true, ..Default::default() },
            key: key.into(),
            key_char: None,
        };
        let stacks = [vec!["Workspace", "Terminal"], vec!["Workspace", "TmuxWindow", "Terminal"]];
        for stack in stacks {
            let contexts: Vec<KeyContext> = stack.iter().map(|c| KeyContext::parse(c).unwrap()).collect();
            for (key, expected) in
                [("up", &JumpToPreviousPrompt as &dyn Action), ("down", &JumpToNextPrompt as &dyn Action)]
            {
                let (bindings, pending) = keymap.bindings_for_input(&[press(key)], &contexts);
                assert!(!pending, "{stack:?} {key}");
                let first = bindings.first().unwrap_or_else(|| panic!("{stack:?}: ctrl-shift-{key} is unbound"));
                assert!(first.action().partial_eq(expected), "{stack:?}: ctrl-shift-{key} -> {}", first.action().name());
            }
        }
    }

    #[test]
    fn zone_keys_resolve_in_zone_tree_and_tmux_keys_in_tmux_window() {
        use gpui::{KeyContext, Keymap, Keystroke, Modifiers};
        let keymap = Keymap::new(default_bindings());
        let ctrl_shift = |key: &str| Keystroke {
            modifiers: Modifiers { control: true, shift: true, ..Default::default() },
            key: key.into(),
            key_char: None,
        };
        let alt = |key: &str| Keystroke {
            modifiers: Modifiers { alt: true, ..Default::default() },
            key: key.into(),
            key_char: None,
        };
        let zone_ctx: Vec<KeyContext> =
            ["Workspace", "ZoneTree", "Terminal"].iter().map(|c| KeyContext::parse(c).unwrap()).collect();
        for (press, expected) in [
            (ctrl_shift("e"), &SplitZoneRight as &dyn Action),
            (ctrl_shift("o"), &SplitZoneDown as &dyn Action),
            (ctrl_shift("x"), &CloseZone as &dyn Action),
            (alt("left"), &FocusZoneLeft as &dyn Action),
            (alt("right"), &FocusZoneRight as &dyn Action),
            (alt("up"), &FocusZoneUp as &dyn Action),
            (alt("down"), &FocusZoneDown as &dyn Action),
        ] {
            let (bindings, pending) = keymap.bindings_for_input(&[press], &zone_ctx);
            assert!(!pending);
            assert!(bindings[0].action().partial_eq(expected));
        }
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
