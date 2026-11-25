//! Actions and default keybindings.

use gpui::{Action, App, KeyBinding, NoAction, SharedString, actions};

use crate::config::ConfigFinding;
use crate::settings::Settings;
use crate::workspace::TabTarget;

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
        CopyLastCommand,
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
        /// Dismisses the window's transient notices on its config line (a
        /// link Brindle didn't open), leaving the config's errors and
        /// findings. Available while one shows.
        DismissNotices,
        SplitZoneRight,
        SplitZoneDown,
        CloseZone,
        FocusZoneLeft,
        FocusZoneRight,
        FocusZoneUp,
        FocusZoneDown,
        Quit,
        AttachSession,
        /// Puts keyboard focus on the tab strip, a tab list: arrows, home and
        /// end then switch tabs, and enter or escape return focus to the
        /// terminal. Hidden (no label, no default key).
        FocusTabStrip,
        /// Opens the profile menu below the strip's `⌄` button (closes it
        /// when it reaches an open one, as from `--action`). Hidden (no
        /// label, no default key).
        OpenProfileMenu,
        /// Opens the active tab's menu (what a right-click on its face opens)
        /// below the tab. Hidden (no label, no default key).
        OpenTabMenu,
        /// Opens the active zone's menu (what a right-click on its header
        /// opens) below the header. Hidden (no label, no default key).
        OpenZoneMenu,
        // The sessions palette: end the highlighted session. (The palettes'
        // other keys are the command list's and the overlay's.)
        PickerKill,
        // Settings dialog.
        SettingsUp,
        SettingsDown,
        SettingsFirst,
        SettingsLast,
        SettingsConfirm,
        SettingsNew,
        SettingsEdit,
        SettingsDelete,
        SettingsSectionNext,
        SettingsSectionPrev,
        SettingsSectionFirst,
        SettingsSectionLast,
        SettingsDuplicate,
        SettingsMoveUp,
        SettingsMoveDown,
        // An agent zone's (`AgentView`), available only while one has
        // focus.
        /// Sends the prompt input's text, as enter does.
        AgentSubmit,
        /// Stops the agent's running turn, as escape does.
        AgentCancel,
        /// Removes the queued prompt, or discards a held one.
        AgentUnqueue,
        /// Sends the held prompt again.
        AgentResend,
        /// Continues the conversation of an agent that exited or was lost,
        /// as the Continue conversation button does.
        AgentContinue,
        /// Copies the queued prompt's text, waiting or held.
        CopyQueuedPrompt,
        /// Copies the agent's latest message.
        CopyLastAgentMessage,
        /// Copies the last code block of the agent's latest message that
        /// has one.
        CopyLastCodeBlock,
        /// Opens every thought when one is closed, else closes them all.
        ToggleAgentThoughts,
        /// Scrolls the conversation to its end, which it then follows.
        AgentScrollToBottom,
    ]
);

/// Activate a tab: the nth from the left (from 0), or the last.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = brindle, no_json)]
pub struct ActivateTab(pub TabTarget);

/// Open a new tab using the profile at a 0-based index.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = brindle, no_json)]
pub struct NewTabWithProfile(pub usize);

/// Builds the action of a table entry.
type ActionBuilder = fn() -> Box<dyn Action>;

/// Builds the action of a numbered entry from its 1-based number.
type NumberedBuilder = fn(usize) -> Box<dyn Action>;

/// Every bindable action without a parameter: config name, command palette
/// label (`None` keeps it out of the palette) and constructor.
const ACTIONS: &[(&str, Option<&str>, ActionBuilder)] = &[
    ("new_tab", Some("New Tab"), || Box::new(NewTab)),
    ("close_tab", Some("Close Tab"), || Box::new(CloseTab)),
    ("next_tab", Some("Next Tab"), || Box::new(NextTab)),
    ("prev_tab", Some("Previous Tab"), || Box::new(PrevTab)),
    ("last_tab", Some("Go to Last Tab"), || Box::new(ActivateTab(TabTarget::Last))),
    ("move_tab_left", Some("Move Tab Left"), || Box::new(MoveTabLeft)),
    ("move_tab_right", Some("Move Tab Right"), || Box::new(MoveTabRight)),
    ("rename_tab", Some("Rename Tab"), || Box::new(RenameTab)),
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
    ("copy_last_command", Some("Copy Last Command Line"), || Box::new(CopyLastCommand)),
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
    ("dismiss_notices", Some("Dismiss Notices"), || Box::new(DismissNotices)),
    ("split_zone_right", Some("Split Zone Right"), || Box::new(SplitZoneRight)),
    ("split_zone_down", Some("Split Zone Down"), || Box::new(SplitZoneDown)),
    ("close_zone", Some("Close Zone"), || Box::new(CloseZone)),
    ("focus_zone_left", Some("Focus Zone Left"), || Box::new(FocusZoneLeft)),
    ("focus_zone_right", Some("Focus Zone Right"), || Box::new(FocusZoneRight)),
    ("focus_zone_up", Some("Focus Zone Up"), || Box::new(FocusZoneUp)),
    ("focus_zone_down", Some("Focus Zone Down"), || Box::new(FocusZoneDown)),
    ("attach_session", Some("Attach Session…"), || Box::new(AttachSession)),
    ("quit", Some("Quit"), || Box::new(Quit)),
    // An agent zone's: the palette lists them only while one has focus.
    ("agent_submit", Some("Send Prompt"), || Box::new(AgentSubmit)),
    ("agent_cancel", Some("Stop Agent Turn"), || Box::new(AgentCancel)),
    ("agent_unqueue", Some("Remove Queued Prompt"), || Box::new(AgentUnqueue)),
    ("agent_resend", Some("Send Held Prompt Again"), || Box::new(AgentResend)),
    ("agent_continue", Some("Continue Agent Conversation"), || Box::new(AgentContinue)),
    ("copy_queued_prompt", Some("Copy Queued Prompt"), || Box::new(CopyQueuedPrompt)),
    ("copy_last_agent_message", Some("Copy Last Agent Message"), || Box::new(CopyLastAgentMessage)),
    ("copy_last_code_block", Some("Copy Last Code Block"), || Box::new(CopyLastCodeBlock)),
    ("toggle_agent_thoughts", Some("Toggle Agent Thoughts"), || Box::new(ToggleAgentThoughts)),
    ("agent_scroll_to_bottom", Some("Scroll Agent to Bottom"), || Box::new(AgentScrollToBottom)),
    // The tab strip as a keyboard stop: unlabelled, for `--action` and user
    // bindings (its keys are the tab list's own).
    ("focus_tab_strip", None, || Box::new(FocusTabStrip)),
    // The `⌄` button's profile menu: unlabelled (the palette has "New Tab
    // from Profile"), for `--action` and user bindings.
    ("open_profile_menu", None, || Box::new(OpenProfileMenu)),
    // The tab and zone header menus (a right-click opens them): unlabelled,
    // for `--action` and user bindings (GPUI has no menu key).
    ("open_tab_menu", None, || Box::new(OpenTabMenu)),
    ("open_zone_menu", None, || Box::new(OpenZoneMenu)),
    // The settings dialog's own actions: unlabelled, so they stay out of the
    // command palette, but `--action` and user bindings can reach them.
    ("settings_up", None, || Box::new(SettingsUp)),
    ("settings_down", None, || Box::new(SettingsDown)),
    ("settings_first", None, || Box::new(SettingsFirst)),
    ("settings_last", None, || Box::new(SettingsLast)),
    ("settings_confirm", None, || Box::new(SettingsConfirm)),
    // Escape and Tab in the dialog are the overlay's: a dismiss request
    // (the dialog answers it as cancel) and its focus scope's stops.
    ("settings_cancel", None, || Box::new(brindle_ui::Dismiss)),
    ("settings_next_field", None, || Box::new(brindle_ui::FocusNext)),
    ("settings_prev_field", None, || Box::new(brindle_ui::FocusPrev)),
    ("settings_new", None, || Box::new(SettingsNew)),
    ("settings_edit", None, || Box::new(SettingsEdit)),
    ("settings_delete", None, || Box::new(SettingsDelete)),
    ("settings_section_next", None, || Box::new(SettingsSectionNext)),
    ("settings_section_prev", None, || Box::new(SettingsSectionPrev)),
    ("settings_section_first", None, || Box::new(SettingsSectionFirst)),
    ("settings_section_last", None, || Box::new(SettingsSectionLast)),
    ("settings_duplicate", None, || Box::new(SettingsDuplicate)),
    ("settings_move_up", None, || Box::new(SettingsMoveUp)),
    ("settings_move_down", None, || Box::new(SettingsMoveDown)),
    // A list's keys (a select's trigger and menu, the command list), for
    // `--action`: a choice field changes only through its menu.
    ("list_up", None, || Box::new(brindle_ui::ListUp)),
    ("list_down", None, || Box::new(brindle_ui::ListDown)),
    ("list_first", None, || Box::new(brindle_ui::ListFirst)),
    ("list_last", None, || Box::new(brindle_ui::ListLast)),
    ("list_confirm", None, || Box::new(brindle_ui::ListConfirm)),
    // Palette navigation and text editing, for `--action` and user bindings.
    // A palette's keys are its command list's and its overlay's, so these
    // names stand for the shared actions (enter in a prompt is
    // `ListConfirm` too); `picker_kill` is the sessions palette's own.
    ("picker_up", None, || Box::new(brindle_ui::ListUp)),
    ("picker_down", None, || Box::new(brindle_ui::ListDown)),
    ("picker_confirm", None, || Box::new(brindle_ui::ListConfirm)),
    ("picker_cancel", None, || Box::new(brindle_ui::Dismiss)),
    ("picker_kill", None, || Box::new(PickerKill)),
    ("text_input_left", None, || Box::new(brindle_ui::TextInputLeft)),
    ("text_input_right", None, || Box::new(brindle_ui::TextInputRight)),
    ("text_input_word_left", None, || Box::new(brindle_ui::TextInputWordLeft)),
    ("text_input_word_right", None, || Box::new(brindle_ui::TextInputWordRight)),
    ("text_input_home", None, || Box::new(brindle_ui::TextInputHome)),
    ("text_input_end", None, || Box::new(brindle_ui::TextInputEnd)),
    ("text_input_select_left", None, || Box::new(brindle_ui::TextInputSelectLeft)),
    ("text_input_select_right", None, || Box::new(brindle_ui::TextInputSelectRight)),
    ("text_input_select_word_left", None, || Box::new(brindle_ui::TextInputSelectWordLeft)),
    ("text_input_select_word_right", None, || Box::new(brindle_ui::TextInputSelectWordRight)),
    ("text_input_select_home", None, || Box::new(brindle_ui::TextInputSelectHome)),
    ("text_input_select_all", None, || Box::new(brindle_ui::TextInputSelectAll)),
    ("text_input_select_end", None, || Box::new(brindle_ui::TextInputSelectEnd)),
    ("text_input_backspace", None, || Box::new(brindle_ui::TextInputBackspace)),
    ("text_input_delete", None, || Box::new(brindle_ui::TextInputDelete)),
    ("text_input_delete_word_back", None, || Box::new(brindle_ui::TextInputDeleteWordBack)),
    ("text_input_copy", None, || Box::new(brindle_ui::TextInputCopy)),
    ("text_input_cut", None, || Box::new(brindle_ui::TextInputCut)),
    ("text_input_paste", None, || Box::new(brindle_ui::TextInputPaste)),
    ("none", None, || Box::new(NoAction)),
];

/// Actions taking a 1-based number, written as `<prefix><N>`.
const NUMBERED_ACTIONS: &[(&str, NumberedBuilder)] = &[
    ("activate_tab_", |n| Box::new(ActivateTab(TabTarget::Nth(n)))),
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
/// labelled action (the hidden ones, such as the menus' openers, have no
/// label). The workspace keeps those available from the focused tab.
pub fn palette_commands() -> Vec<(&'static str, Box<dyn Action>)> {
    ACTIONS.iter().filter_map(|(_, label, build)| Some(((*label)?, build()))).collect()
}

/// What the active zone says about which commands make sense for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteFacts {
    /// What the active zone holds.
    pub zone: PaletteZone,
    /// It has sent a prompt mark (OSC 133/633 `A`): a shell with
    /// integration runs in it. Never in an agent zone.
    pub prompt_marks: bool,
}

/// What the active zone holds, for the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteZone {
    Terminal,
    Agent,
}

/// Whether `action` acts on a terminal: its selection, clipboard, links,
/// scrollback, prompts or find bar. An agent zone answers some of the
/// scrolling ones (its own keys reach them), but the palette lists the
/// agent's own commands there instead.
fn acts_on_terminal(action: &dyn Action) -> bool {
    let terminal: [&dyn Action; 18] = [
        &Copy,
        &Paste,
        &PastePrimary,
        &SelectAll,
        &CopyLink,
        &ScrollLineUp,
        &ScrollLineDown,
        &ScrollPageUp,
        &ScrollPageDown,
        &ScrollToTop,
        &ScrollToBottom,
        &ClearScrollback,
        &JumpToPreviousPrompt,
        &JumpToNextPrompt,
        &SelectLastOutput,
        &CopyLastOutput,
        &CopyLastCommand,
        &Find,
    ];
    terminal.iter().any(|t| action.partial_eq(*t))
}

/// Whether `action` reads the marks of a shell with integration, and so does
/// nothing without them: the commands on the last command's line and output.
/// The jump-to-prompt actions aren't among them: they scroll a line instead.
fn needs_prompt_marks(action: &dyn Action) -> bool {
    action.partial_eq(&CopyLastCommand) || action.partial_eq(&SelectLastOutput) || action.partial_eq(&CopyLastOutput)
}

/// Whether the palette lists `action` for a zone with these `facts`. In a
/// terminal zone, the commands that read shell-integration marks are hidden
/// until a prompt mark has arrived (without one they would do nothing); in
/// an agent zone, every command that acts on a terminal is. Every other
/// action is the focused tab's to answer.
pub fn palette_lists(action: &dyn Action, facts: PaletteFacts) -> bool {
    match facts.zone {
        PaletteZone::Terminal => facts.prompt_marks || !needs_prompt_marks(action),
        PaletteZone::Agent => !acts_on_terminal(action),
    }
}

/// An action's label as a menu item shows it: its command palette label.
/// `None` for an action without a label.
pub fn menu_label(action: &dyn Action) -> Option<&'static str> {
    ACTIONS.iter().find(|(_, _, build)| build().partial_eq(action)).and_then(|(_, label, _)| *label)
}

/// A row of a menu whose items run actions, such as a tab's or a zone
/// header's menu.
pub enum ActionRow {
    Item { action: Box<dyn Action>, enabled: bool },
    Separator,
}

impl ActionRow {
    pub fn item(action: impl Action, enabled: bool) -> Self {
        Self::Item { action: Box::new(action), enabled }
    }

    /// The action an item runs; a separator runs none.
    pub fn action(&self) -> Option<&dyn Action> {
        match self {
            Self::Item { action, .. } => Some(&**action),
            Self::Separator => None,
        }
    }
}

impl Clone for ActionRow {
    fn clone(&self) -> Self {
        match self {
            Self::Item { action, enabled } => Self::Item { action: action.boxed_clone(), enabled: *enabled },
            Self::Separator => Self::Separator,
        }
    }
}

/// A menu of actions: its entries, each item labelled from the `ACTIONS`
/// table ([`menu_label`]) and keyed by its action's name, with the keys
/// bound to its action, and, by entry index (separators counted), the
/// action each item runs. A `MenuEvent::Selected`'s key is resolved with
/// [`ActionMenu::chosen`].
pub struct ActionMenu {
    pub entries: Vec<brindle_ui::MenuEntry>,
    pub actions: Vec<Option<Box<dyn Action>>>,
}

impl ActionMenu {
    /// The menu of `rows`, item `i` showing `shortcuts[i]` (the keys the
    /// owner found bound to its action; a row past the slice shows none).
    pub fn new(rows: Vec<ActionRow>, shortcuts: &[Option<SharedString>]) -> Self {
        let mut menu = Self { entries: Vec::with_capacity(rows.len()), actions: Vec::with_capacity(rows.len()) };
        for (ix, row) in rows.into_iter().enumerate() {
            match row {
                ActionRow::Item { action, enabled } => {
                    let label = menu_label(&*action).unwrap_or_else(|| action.name());
                    let mut item = brindle_ui::MenuItem::new(label).key(action.name()).disabled(!enabled);
                    item.shortcut = shortcuts.get(ix).cloned().flatten();
                    menu.entries.push(item.into());
                    menu.actions.push(Some(action));
                }
                ActionRow::Separator => {
                    menu.entries.push(brindle_ui::MenuEntry::Separator);
                    menu.actions.push(None);
                }
            }
        }
        menu
    }

    /// The action of entry `ix`, when it is an enabled item.
    pub fn action(&self, ix: usize) -> Option<&dyn Action> {
        self.entries.get(ix).filter(|entry| entry.is_enabled())?;
        self.actions.get(ix)?.as_deref()
    }

    /// The action of the enabled item keyed `key`, as a
    /// `MenuEvent::Selected` names it.
    pub fn chosen(&self, key: &brindle_ui::ItemKey) -> Option<&dyn Action> {
        self.action(brindle_ui::entry_keyed(&self.entries, key)?)
    }
}

/// Default bindings follow GNOME Terminal / kitty conventions so plain
/// ctrl-<letter> always reaches the shell (and the programs in it).
pub(crate) fn default_bindings() -> Vec<KeyBinding> {
    let w = Some("Workspace");
    let t = Some("Terminal");
    let p = Some("Palette");
    let z = Some("ZoneTree");
    let s = Some("SettingsDialog");
    let l = Some("SettingsList");
    let sl = Some("SettingsSections");
    let f = Some("FindBar");
    let a = Some("AgentZone");
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
        KeyBinding::new("ctrl-shift-q", Quit, w),
        KeyBinding::new("ctrl-shift-p", OpenCommandPalette, w),
        KeyBinding::new("ctrl-shift-space", OpenProfilePicker, w),
        KeyBinding::new("ctrl-shift-r", ReloadConfig, w),
        KeyBinding::new("ctrl-,", OpenConfig, w),
        // GPUI reports shift-, as `<` (without shift) on US-style layouts;
        // the readable spelling comes last so the palette shows it.
        KeyBinding::new("ctrl-<", OpenSettings, w),
        KeyBinding::new("ctrl-shift-,", OpenSettings, w),
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
        // An agent zone: escape stops the turn (the prompt input's own
        // escape does while it shows the stop button, and passes it on
        // otherwise), and the page keys scroll the conversation. Enter and
        // shift-enter are the prompt input's and its text area's.
        KeyBinding::new("escape", AgentCancel, a),
        KeyBinding::new("shift-pageup", ScrollPageUp, a),
        KeyBinding::new("shift-pagedown", ScrollPageDown, a),
        // A palette's list keys and escape come from its command list
        // (`UiList`, `UiCommand`) and overlay (`UiLayer`). The host adds
        // the sessions palette's kill, and enter for a prompt, which submits
        // as a one-field form does (a list's `UiList` enter is nearer).
        KeyBinding::new("ctrl-shift-delete", PickerKill, p),
        KeyBinding::new("enter", brindle_ui::ListConfirm, p),
        // The section list: the dialog's first Tab stop, a vertical tab
        // list whose keys switch sections at once (Radix's vertical Tabs).
        KeyBinding::new("up", SettingsSectionPrev, sl),
        KeyBinding::new("down", SettingsSectionNext, sl),
        KeyBinding::new("home", SettingsSectionFirst, sl),
        KeyBinding::new("end", SettingsSectionLast, sl),
        // The theme and profile lists: one Tab stop each, whose keys move
        // the selection.
        KeyBinding::new("up", SettingsUp, l),
        KeyBinding::new("down", SettingsDown, l),
        KeyBinding::new("ctrl-p", SettingsUp, l),
        KeyBinding::new("ctrl-n", SettingsDown, l),
        KeyBinding::new("home", SettingsFirst, l),
        KeyBinding::new("end", SettingsLast, l),
        // Escape and tab come from the overlay around the dialog (`UiLayer`,
        // `UiFocusScope`). Enter on a focused button reaches the button.
        KeyBinding::new("enter", SettingsConfirm, s),
        KeyBinding::new("ctrl-pagedown", SettingsSectionNext, s),
        KeyBinding::new("ctrl-pageup", SettingsSectionPrev, s),
        KeyBinding::new("ctrl-tab", SettingsSectionNext, s),
        KeyBinding::new("ctrl-shift-tab", SettingsSectionPrev, s),
        KeyBinding::new("alt-up", SettingsMoveUp, s),
        KeyBinding::new("alt-down", SettingsMoveDown, s),
        // The dialog's selects and checkbox keep their own keys (enter,
        // space, up and down open a select's menu; space ticks a checkbox),
        // and left and right do nothing on them.
    ];
    // The text inputs' keys, in the `TextInput` context.
    bindings.extend(brindle_ui::default_bindings());
    for n in 1..=9 {
        let target = if n == 9 { TabTarget::Last } else { TabTarget::Nth(n - 1) };
        bindings.push(KeyBinding::new(&format!("alt-{n}"), ActivateTab(target), w));
        bindings.push(KeyBinding::new(&format!("ctrl-alt-{n}"), NewTabWithProfile(n - 1), w));
    }
    bindings
}

/// Binds the default keys, then the config's `[keybindings]` over them, and
/// returns what it couldn't bind (an unknown action, keys that don't parse)
/// for the window's config line.
pub fn bind_keys(cx: &mut App) -> Vec<ConfigFinding> {
    cx.clear_key_bindings();
    cx.bind_keys(default_bindings());
    let user: Vec<(String, String)> = Settings::get(cx)
        .config
        .keybindings
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let (bindings, findings) = user_bindings(&user);
    cx.bind_keys(bindings);
    findings
}

/// The bindings for the config's `(keys, action name)` pairs, in the
/// innermost context so they beat the defaults, and a finding for each pair
/// that can't be bound. Binding a key to `none` is a binding (it frees the
/// key for the program), not a finding.
pub fn user_bindings(user: &[(String, String)]) -> (Vec<KeyBinding>, Vec<ConfigFinding>) {
    let mut bindings = Vec::new();
    let mut findings = Vec::new();
    for (keys, name) in user {
        let Some(action) = action_by_name(name) else {
            findings.push(ConfigFinding::UnknownAction { keys: keys.clone(), action: name.clone() });
            continue;
        };
        let context = gpui::KeyBindingContextPredicate::parse("Terminal").unwrap();
        match KeyBinding::load(keys, action, Some(context.into()), false, None, &gpui::DummyKeyboardMapper) {
            Ok(binding) => bindings.push(binding),
            Err(err) => findings.push(ConfigFinding::InvalidKeys { keys: keys.clone(), error: err.to_string() }),
        }
    }
    (bindings, findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
    }

    #[test]
    fn an_unknown_action_is_a_finding() {
        let (bindings, findings) =
            user_bindings(&pairs(&[("ctrl-shift-y", "split_zone_rihgt"), ("ctrl-shift-u", "split_zone_right")]));
        assert_eq!(bindings.len(), 1);
        assert_eq!(
            findings,
            [ConfigFinding::UnknownAction { keys: "ctrl-shift-y".into(), action: "split_zone_rihgt".into() }]
        );
    }

    #[test]
    fn invalid_keys_are_a_finding() {
        let (bindings, findings) = user_bindings(&pairs(&[("ctrl-a-b", "split_zone_right")]));
        assert!(bindings.is_empty());
        assert!(
            matches!(findings.as_slice(), [ConfigFinding::InvalidKeys { keys, .. }] if keys == "ctrl-a-b"),
            "{findings:?}"
        );
    }

    #[test]
    fn none_unbinds_and_is_not_a_finding() {
        let (bindings, findings) = user_bindings(&pairs(&[("ctrl-shift-t", "none")]));
        assert!(findings.is_empty(), "{findings:?}");
        assert_eq!(bindings.len(), 1);
        assert!(bindings[0].action().partial_eq(&NoAction));
    }

    #[test]
    fn every_listed_action_resolves() {
        for name in action_names() {
            let name = name.replace("<N>", "3");
            assert!(action_by_name(&name).is_some(), "{name}");
        }
        assert!(action_by_name("activate_tab_0").is_none());
        assert!(action_by_name("bogus").is_none());
        // Removed actions are unknown names now (user bindings to them are
        // logged and ignored).
        for removed in ["swallow", "tmux_detach", "tmux_new_session"] {
            assert!(action_by_name(removed).is_none(), "{removed}");
        }
        assert!(action_names().all(|n| !n.starts_with("tmux_")));
        assert_eq!(
            action_by_name("activate_tab_2").unwrap().as_any().downcast_ref::<ActivateTab>(),
            Some(&ActivateTab(TabTarget::Nth(1)))
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
        assert!(palette_commands().iter().all(|(_, a)| !a.partial_eq(&OpenCommandPalette) && !a.partial_eq(&NoAction)));
    }

    /// The tab and zone menus' hidden openers resolve by name and stay out
    /// of the palette; the menus' items take their labels from the table.
    #[test]
    fn menu_openers_are_hidden_and_menu_labels_come_from_the_table() {
        for (name, action) in [("open_tab_menu", &OpenTabMenu as &dyn Action), ("open_zone_menu", &OpenZoneMenu)] {
            assert!(action_by_name(name).is_some_and(|a| a.partial_eq(action)), "{name}");
            assert!(action_names().any(|n| n == name), "{name} isn't listed");
            assert!(palette_commands().iter().all(|(_, a)| !a.partial_eq(action)), "{name} is in the palette");
            assert_eq!(menu_label(action), None, "{name}");
        }
        assert_eq!(menu_label(&RenameTab), Some("Rename Tab"));
        assert_eq!(menu_label(&SplitZoneDown), Some("Split Zone Down"));

        let menu = ActionMenu::new(
            vec![ActionRow::item(RenameTab, true), ActionRow::Separator, ActionRow::item(CloseTab, false)],
            &[Some("ctrl-shift-r".into()), None, None],
        );
        let labels: Vec<Option<&str>> = menu.entries.iter().map(|e| e.label()).collect();
        assert_eq!(labels, [Some("Rename Tab"), None, Some("Close Tab")]);
        assert_eq!(menu.entries[0].item().and_then(|i| i.shortcut.clone()), Some("ctrl-shift-r".into()));
        assert!(menu.entries[2].item().is_some_and(|i| i.disabled && i.shortcut.is_none()));
        assert!(menu.action(0).is_some_and(|a| a.partial_eq(&RenameTab)));
        // A separator and a disabled item run nothing.
        assert!(menu.action(1).is_none() && menu.action(2).is_none() && menu.action(3).is_none());
    }

    /// Items are keyed by their action's name, so a chosen key finds its
    /// action whatever its label or place; a disabled item's or an unknown
    /// key runs nothing. Shortcuts are taken as given, by row.
    #[test]
    fn action_menu_items_are_keyed_by_action_name() {
        let rows = vec![ActionRow::item(RenameTab, true), ActionRow::Separator, ActionRow::item(CloseTab, false)];
        let shortcuts: Vec<Option<SharedString>> = rows.iter().map(|row| row.action().map(|a| a.name().into())).collect();
        let menu = ActionMenu::new(rows, &shortcuts);
        let keys: Vec<Option<&str>> = menu.entries.iter().map(|e| e.key().map(|k| k.as_str())).collect();
        assert_eq!(keys, [Some(RenameTab.name()), None, Some(CloseTab.name())]);
        let shown: Vec<Option<SharedString>> = menu.entries.iter().map(|e| e.item().and_then(|i| i.shortcut.clone())).collect();
        assert_eq!(shown, shortcuts);
        assert!(menu.chosen(&RenameTab.name().into()).is_some_and(|a| a.partial_eq(&RenameTab)));
        assert!(menu.chosen(&CloseTab.name().into()).is_none(), "disabled");
        assert!(menu.chosen(&"Rename Tab".into()).is_none(), "a label isn't a key");
        // Fewer shortcuts than rows: the rest show none.
        let menu = ActionMenu::new(vec![ActionRow::item(RenameTab, true), ActionRow::item(CloseTab, true)], &[]);
        assert!(menu.entries.iter().all(|e| e.item().is_some_and(|i| i.shortcut.is_none())));
    }

    /// alt-1 to alt-8 go to the nth tab, alt-9 and `last_tab` to the last;
    /// `activate_tab_N` to the Nth.
    #[test]
    fn tab_keys_name_their_target() {
        let bindings = default_bindings();
        let target = |keys: &str| {
            bindings
                .iter()
                .filter(|b| b.keystrokes().iter().map(|k| k.unparse()).eq([keys.to_string()]))
                .find_map(|b| b.action().as_any().downcast_ref::<ActivateTab>().cloned())
        };
        for n in 1..=8 {
            assert_eq!(target(&format!("alt-{n}")), Some(ActivateTab(TabTarget::Nth(n - 1))), "alt-{n}");
        }
        assert_eq!(target("alt-9"), Some(ActivateTab(TabTarget::Last)));
        let built = |name: &str| action_by_name(name).and_then(|a| a.as_any().downcast_ref::<ActivateTab>().cloned());
        assert_eq!(built("last_tab"), Some(ActivateTab(TabTarget::Last)));
        assert_eq!(built("activate_tab_1"), Some(ActivateTab(TabTarget::Nth(0))));
        assert_eq!(built("activate_tab_12"), Some(ActivateTab(TabTarget::Nth(11))));
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
            .map(|b| (format!("{:?}", b.keystrokes()), b.match_keystrokes(std::slice::from_ref(&typed))))
            .collect::<Vec<_>>();
        assert!(matched.iter().any(|(_, m)| *m == Some(false)), "{matched:#?}");
    }

    /// Real ctrl-shift-up/down presses (as GPUI reports them on Linux)
    /// resolve to the prompt jumps in a terminal, alone or in a zone tree;
    /// `--action` in the e2e cases skips the keymap, so this covers it.
    #[test]
    fn prompt_jump_keys_resolve_in_terminals() {
        use gpui::{KeyContext, Keymap, Keystroke, Modifiers};
        let keymap = Keymap::new(default_bindings());
        let press = |key: &str| Keystroke {
            modifiers: Modifiers { control: true, shift: true, ..Default::default() },
            key: key.into(),
            key_char: None,
        };
        let stacks = [vec!["Workspace", "Terminal"], vec!["Workspace", "ZoneTree", "Terminal"]];
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
    fn zone_keys_resolve_in_zone_tree() {
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

    /// The palette's commands are every labelled action in table order; the
    /// workspace keeps those available from the focused tab, in that order.
    #[test]
    fn palette_lists_only_available_actions() {
        use std::any::TypeId;
        let commands = palette_commands();
        let labelled: Vec<&str> = ACTIONS.iter().filter_map(|(_, label, _)| *label).collect();
        assert_eq!(commands.iter().map(|(l, _)| *l).collect::<Vec<_>>(), labelled);
        assert!(commands.iter().any(|(l, _)| *l == "Split Zone Right"));
        assert!(commands.iter().all(|(l, _)| !l.starts_with("tmux")));

        // As if focus were in a local terminal: workspace + terminal handlers,
        // a global listener, and ActivateTab (no_json) on the workspace.
        let available = [
            TypeId::of::<ActivateTab>(),
            TypeId::of::<ClearScrollback>(),
            TypeId::of::<ReloadConfig>(),
            TypeId::of::<OpenCommandPalette>(),
            TypeId::of::<NoAction>(),
        ];
        let labels: Vec<&str> = commands
            .into_iter()
            .filter(|(_, a)| available.contains(&a.as_any().type_id()))
            .map(|(l, _)| l)
            .collect();
        assert_eq!(labels, ["Go to Last Tab", "Clear Scrollback", "Reload Config"]);
    }

    /// The commands on the last command's line and output need a shell that
    /// marks its prompts, so the palette lists them only once the terminal has
    /// sent a prompt mark. The jump-to-prompt actions stay listed.
    #[test]
    fn the_last_command_actions_are_listed_only_after_a_prompt_mark() {
        let (without, with) = (
            PaletteFacts { zone: PaletteZone::Terminal, prompt_marks: false },
            PaletteFacts { zone: PaletteZone::Terminal, prompt_marks: true },
        );
        let listed = |facts| -> Vec<&'static str> {
            palette_commands().into_iter().filter(|(_, a)| palette_lists(&**a, facts)).map(|(l, _)| l).collect()
        };
        let hidden = ["Copy Last Command Line", "Select Last Command Output", "Copy Last Command Output"];
        for label in hidden {
            assert!(!listed(without).contains(&label), "{label} without a prompt mark");
            assert!(listed(with).contains(&label), "{label} with one");
        }
        // Nothing else depends on it.
        let all = palette_commands().len();
        assert_eq!(listed(without).len(), all - hidden.len());
        assert_eq!(listed(with).len(), all);
        let jumps = palette_commands().into_iter().filter(|(_, a)| a.partial_eq(&JumpToPreviousPrompt) || a.partial_eq(&JumpToNextPrompt));
        for (label, action) in jumps {
            assert!(palette_lists(&*action, without), "{label} stays listed");
        }
    }

    /// In an agent zone the palette keeps the agent's, zone, tab and window
    /// commands and drops every command that acts on a terminal, the
    /// scrolling ones the agent view answers included.
    #[test]
    fn an_agent_zone_lists_no_terminal_commands() {
        let agent = PaletteFacts { zone: PaletteZone::Agent, prompt_marks: false };
        let listed: Vec<&str> =
            palette_commands().into_iter().filter(|(_, a)| palette_lists(&**a, agent)).map(|(l, _)| l).collect();
        for label in [
            "Stop Agent Turn",
            "Remove Queued Prompt",
            "Send Held Prompt Again",
            "Continue Agent Conversation",
            "Copy Queued Prompt",
            "Copy Last Agent Message",
            "Copy Last Code Block",
            "Toggle Agent Thoughts",
            "Scroll Agent to Bottom",
            "Split Zone Right",
            "Close Zone",
            "New Tab",
            "Close Tab",
            "Quit",
        ] {
            assert!(listed.contains(&label), "{label} is listed");
        }
        for label in ["Clear Scrollback", "Find", "Copy", "Paste", "Scroll to Bottom", "Scroll Page Up", "Jump to Next Prompt"] {
            assert!(!listed.contains(&label), "{label} isn't listed");
        }
    }

    /// ctrl-shift-d has no default binding anywhere, so it reaches the
    /// program in the terminal like any other unbound chord.
    #[test]
    fn ctrl_shift_d_is_unbound() {
        let d = gpui::Keystroke::parse("ctrl-shift-d").unwrap();
        assert!(default_bindings().iter().all(|b| b.match_keystrokes(std::slice::from_ref(&d)) != Some(false)));
    }

    /// On a select in the settings dialog the select's own keys win over
    /// the dialog's (up and down open it rather than move the dialog), and
    /// left and right reach nothing: a choice changes only through its menu.
    #[test]
    fn settings_select_keys_are_the_selects() {
        use gpui::{KeyContext, Keymap, Keystroke};
        let keymap = Keymap::new(default_bindings());
        let contexts: Vec<KeyContext> = ["Workspace", "UiLayer", "UiFocusScope", "SettingsDialog", "UiList"]
            .iter()
            .map(|c| KeyContext::parse(c).unwrap())
            .collect();
        let first = |key: &str| {
            let (bindings, pending) = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &contexts);
            assert!(!pending, "{key}");
            bindings.first().map(|b| b.action().name())
        };
        assert_eq!(first("up"), Some("brindle_ui::ListUp"));
        assert_eq!(first("down"), Some("brindle_ui::ListDown"));
        assert_eq!(first("enter"), Some("brindle_ui::ListConfirm"));
        assert_eq!(first("home"), Some("brindle_ui::ListFirst"));
        assert_eq!(first("end"), Some("brindle_ui::ListLast"));
        assert_eq!(first("left"), None);
        assert_eq!(first("right"), None);
        assert_eq!(first("space"), None);
    }

    /// A focused theme or profile list takes the list keys; elsewhere in
    /// the dialog (a footer button, say) up, down, home and end are unbound.
    #[test]
    fn settings_list_keys_are_the_lists() {
        use gpui::{KeyContext, Keymap, Keystroke};
        let keymap = Keymap::new(default_bindings());
        let stack = |extra: &[&str]| -> Vec<KeyContext> {
            ["Workspace", "UiLayer", "UiFocusScope", "SettingsDialog"]
                .iter()
                .chain(extra)
                .map(|c| KeyContext::parse(c).unwrap())
                .collect()
        };
        let first = |key: &str, contexts: &[KeyContext]| {
            let (bindings, _) = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], contexts);
            bindings.first().map(|b| b.action().name())
        };
        let list = stack(&["SettingsList"]);
        assert_eq!(first("up", &list), Some("brindle::SettingsUp"));
        assert_eq!(first("down", &list), Some("brindle::SettingsDown"));
        assert_eq!(first("ctrl-p", &list), Some("brindle::SettingsUp"));
        assert_eq!(first("ctrl-n", &list), Some("brindle::SettingsDown"));
        assert_eq!(first("home", &list), Some("brindle::SettingsFirst"));
        assert_eq!(first("end", &list), Some("brindle::SettingsLast"));
        assert_eq!(first("enter", &list), Some("brindle::SettingsConfirm"));
        let button = stack(&[]);
        for key in ["up", "down", "home", "end"] {
            assert_eq!(first(key, &button), None, "{key}");
        }
        // The section list switches sections; the dialog's shortcuts stay.
        let sections = stack(&["SettingsSections"]);
        assert_eq!(first("up", &sections), Some("brindle::SettingsSectionPrev"));
        assert_eq!(first("down", &sections), Some("brindle::SettingsSectionNext"));
        assert_eq!(first("home", &sections), Some("brindle::SettingsSectionFirst"));
        assert_eq!(first("end", &sections), Some("brindle::SettingsSectionLast"));
        for contexts in [&sections, &list, &button] {
            assert_eq!(first("ctrl-tab", contexts), Some("brindle::SettingsSectionNext"));
            assert_eq!(first("ctrl-shift-tab", contexts), Some("brindle::SettingsSectionPrev"));
            assert_eq!(first("ctrl-pagedown", contexts), Some("brindle::SettingsSectionNext"));
            assert_eq!(first("ctrl-pageup", contexts), Some("brindle::SettingsSectionPrev"));
        }
    }

    /// A palette's keys, as gpui resolves them with focus in a list's
    /// filter and in a prompt's input: each `picker_*` name stands for the
    /// action its key resolves to, so `--action picker_*` and a user's
    /// binding of one do what the key does.
    #[test]
    fn palette_keys_and_picker_aliases() {
        use gpui::{KeyContext, Keymap, Keystroke};
        let keymap = Keymap::new(default_bindings());
        let stack = |names: &[&str]| -> Vec<KeyContext> {
            ["Workspace", "UiLayer", "UiFocusScope", "Palette"]
                .iter()
                .chain(names)
                .map(|c| KeyContext::parse(c).unwrap())
                .collect()
        };
        let list = stack(&["UiCommand", "UiList", "TextInput"]);
        let prompt = stack(&["TextInput"]);
        let first = |key: &str, contexts: &[KeyContext]| -> Box<dyn Action> {
            let (bindings, pending) = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], contexts);
            assert!(!pending, "{key}");
            bindings.first().unwrap_or_else(|| panic!("{key} is unbound")).action().boxed_clone()
        };
        let alias = |name: &str| action_by_name(name).unwrap_or_else(|| panic!("{name} does not resolve"));
        for (key, name) in [
            ("up", "picker_up"),
            ("down", "picker_down"),
            ("ctrl-p", "picker_up"),
            ("ctrl-n", "picker_down"),
            ("enter", "picker_confirm"),
            ("escape", "picker_cancel"),
            ("ctrl-shift-delete", "picker_kill"),
        ] {
            let action = first(key, &list);
            assert!(action.partial_eq(&*alias(name)), "list: {key} -> {}, {name} -> {}", action.name(), alias(name).name());
        }
        assert!(alias("picker_up").partial_eq(&brindle_ui::ListUp));
        assert!(alias("picker_down").partial_eq(&brindle_ui::ListDown));
        assert!(alias("picker_confirm").partial_eq(&brindle_ui::ListConfirm));
        assert!(alias("picker_cancel").partial_eq(&brindle_ui::Dismiss));
        assert!(alias("picker_kill").partial_eq(&PickerKill));
        // Home and end edit the filter; tab stays in the palette.
        assert!(first("home", &list).partial_eq(&brindle_ui::TextInputHome));
        assert!(first("end", &list).partial_eq(&brindle_ui::TextInputEnd));
        assert!(first("tab", &list).partial_eq(&brindle_ui::FocusNext));
        assert!(first("shift-tab", &list).partial_eq(&brindle_ui::FocusPrev));
        // A prompt: enter submits and escape dismisses, by the same names.
        assert!(first("enter", &prompt).partial_eq(&*alias("picker_confirm")));
        assert!(first("escape", &prompt).partial_eq(&*alias("picker_cancel")));
        assert!(first("tab", &prompt).partial_eq(&brindle_ui::FocusNext));
    }

    /// Enter on a focused button in the dialog: its only binding is
    /// `SettingsConfirm`, whose handler propagates when a button has focus
    /// (`settings_dialog::enter_confirms`), so gpui goes on to the button's
    /// own key-down, which takes a bare enter. Nothing else in the stack
    /// binds enter, so nothing else runs.
    #[test]
    fn enter_on_a_dialog_button_reaches_the_button() {
        use gpui::{KeyContext, Keymap, Keystroke, Modifiers};
        let keymap = Keymap::new(default_bindings());
        let contexts: Vec<KeyContext> = ["Workspace", "UiLayer", "UiFocusScope", "SettingsDialog"]
            .iter()
            .map(|c| KeyContext::parse(c).unwrap())
            .collect();
        let (bindings, pending) = keymap.bindings_for_input(&[Keystroke::parse("enter").unwrap()], &contexts);
        assert!(!pending);
        let names: Vec<_> = bindings.iter().map(|b| b.action().name()).collect();
        assert_eq!(names, ["brindle::SettingsConfirm"]);
        assert_eq!(
            brindle_ui::key_activation("enter", Modifiers::default(), false, &brindle_ui::ACTIVATION_KEYS),
            Some(gpui::KeyboardButton::Enter)
        );
    }
}
