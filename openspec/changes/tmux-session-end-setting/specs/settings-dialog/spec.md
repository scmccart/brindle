## MODIFIED Requirements

### Requirement: Opening the settings dialog
ctrl-shift-, and the "Open Settings" command SHALL open the settings dialog as a modal overlay over the current window. Keyboard focus SHALL move to the dialog. Pressing the shortcut while the dialog is open SHALL close it, as cancel does.

#### Scenario: Open from the shortcut
- **WHEN** the user presses ctrl-shift-, in a tab
- **THEN** the settings dialog opens over the window, with the General section showing

#### Scenario: Open from the command palette
- **WHEN** the user runs "Open Settings" from the command palette
- **THEN** the palette closes and the settings dialog opens

### Requirement: Sections
The dialog SHALL show a list of sections beside the selected section's content. The sections SHALL be "General", "Profiles" and "Theme", in that order, and the dialog SHALL open on "General". Leaving the Theme section SHALL drop an unapplied theme preview and select the active theme again. The section SHALL NOT change while an editor has unsaved edits.

#### Scenario: General section first
- **WHEN** the dialog opens
- **THEN** the section list shows "General", "Profiles" and "Theme", and "General" is selected

#### Scenario: Theme section
- **WHEN** the dialog opens and the user selects "Theme"
- **THEN** the theme list is shown with the active theme selected

#### Scenario: Switch to Profiles
- **WHEN** the user previews `tokyo-night` in the theme list and then selects "Profiles"
- **THEN** the tabs show the active theme again and the profile list is shown

#### Scenario: Editor holds the section
- **WHEN** the profile editor has unsaved edits and the user clicks "Theme"
- **THEN** the profile editor stays open with its edits

### Requirement: Theme list
The Theme section SHALL list every built-in theme followed by the user's custom themes, in a stable order. The active global theme SHALL be marked, and selected whenever the Theme section is shown, and custom themes SHALL be marked as custom. The custom themes SHALL be read from the config file even when another part of the file has an error. A custom theme that cannot be read SHALL be listed with its error and SHALL NOT be selectable.

#### Scenario: Custom theme listed
- **WHEN** the config defines `[themes.mine]` and `theme = "mine"`, and the user opens the dialog and selects "Theme"
- **THEN** "mine" is listed after the built-ins, marked custom and active, and is selected

#### Scenario: Error elsewhere in the config
- **WHEN** the config has an unknown key `fontsize = 3` and defines `[themes.mine]`
- **THEN** "mine" is still listed in the dialog

### Requirement: Keyboard and mouse operation
The dialog SHALL be fully operable from the keyboard:
- up and down (also ctrl-p and ctrl-n) move through the theme list and the profile list;
- alt-up and alt-down move the selected profile up or down in the profile list;
- tab and shift-tab move keyboard focus between an editor's fields;
- left and right change the focused choice field (the General section's settings, theme, tmux mode), and space toggles the focused checkbox;
- enter applies in the theme list, edits in the profile list, saves in an editor, and closes the dialog in the General section;
- escape cancels in a list and in the General section, and discards in an editor.

The editors' text fields SHALL be text inputs, as defined by the `text-input` capability, including input-method support. Every list entry, button and field SHALL also work with the mouse, and clicking a field SHALL focus it.

#### Scenario: Keyboard only
- **WHEN** the user opens the dialog, selects "Theme", presses down twice and then enter
- **THEN** the theme two places below the active one becomes the global theme

#### Scenario: Fix one digit of a color
- **WHEN** the background field holds `#1e1e2e`, and the user presses home, right twice, shift-right and types `f`
- **THEN** the field holds `#1f1e2e` and the terminals behind the dialog preview that background

#### Scenario: Tab moves focus
- **WHEN** the editor's name field has focus and the user presses tab
- **THEN** the foreground field has focus and shows the caret, and the name field shows none

#### Scenario: Profile editing by keyboard
- **WHEN** the user selects "Profiles", moves to `Work`, presses enter, tabs to the theme field, presses right and then enter
- **THEN** `Work`'s theme is saved as the theme after its previous one in the choice list, and the profile list is shown

#### Scenario: General setting by keyboard
- **WHEN** the user opens the dialog and presses right
- **THEN** the session-end setting shows "Open a new tab" and the config has `tmux_session_end = "new_tab"`

## ADDED Requirements

### Requirement: General section
The General section SHALL show the `tmux_session_end` setting as a choice field: "Close window", "Open a new tab" or "Ask". It SHALL be focused when the section is shown, and labelled with what it governs. Changing the choice SHALL write the key to the config at once, keeping the rest of the file, and SHALL apply it without waiting for the file watcher. Cancel SHALL NOT revert a change already written. A failed write SHALL show the reason and keep the previous value.

#### Scenario: Current value shown
- **WHEN** the config has `tmux_session_end = "ask"` and the user opens the dialog
- **THEN** the General section shows "Ask"

#### Scenario: Change is saved at once
- **WHEN** the config has comments and no `tmux_session_end`, and the user changes the choice to "Ask" and closes the dialog
- **THEN** the config has `tmux_session_end = "ask"` and its comments, and a session ending in this window then asks

#### Scenario: Malformed config file
- **WHEN** the config file has a TOML syntax error and the user changes the choice
- **THEN** the section shows that the config file must be fixed first, the choice stays as it was, and the file is unchanged
