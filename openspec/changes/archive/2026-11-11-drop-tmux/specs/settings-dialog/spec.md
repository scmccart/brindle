## MODIFIED Requirements

### Requirement: Opening the settings dialog
ctrl-shift-, and the "Open Settings" command SHALL open the settings dialog as a modal overlay over the current window. Keyboard focus SHALL move to the dialog. Pressing the shortcut while the dialog is open SHALL close it, as cancel does.

#### Scenario: Open from the shortcut
- **WHEN** the user presses ctrl-shift-, in a tab
- **THEN** the settings dialog opens over the window, with the Profiles section showing

#### Scenario: Open from the command palette
- **WHEN** the user runs "Open Settings" from the command palette
- **THEN** the palette closes and the settings dialog opens

## ADDED Requirements

### Requirement: Dialog sections
The dialog SHALL show a list of sections beside the selected section's content. The sections SHALL be "Profiles" and "Theme", in that order, and the dialog SHALL open on "Profiles". The section list SHALL be reachable from the keyboard as a vertical tab list: it is a single Tab stop, the first of the dialog's, and while it has focus up and down select the previous and next section (wrapping from the last to the first and back), home and end the first and last, each shown at once while focus stays on the list; the shown section SHALL show a focus indicator while the list has focus from the keyboard. ctrl-tab and ctrl-pagedown (next) and ctrl-shift-tab and ctrl-pageup (previous) SHALL also change the section from anywhere in the dialog. Leaving the Theme section SHALL drop an unapplied theme preview and select the active theme again. The section SHALL NOT change while an editor has unsaved edits, by any of these keys or a click.

#### Scenario: Profiles section first
- **WHEN** the dialog opens
- **THEN** the section list shows "Profiles" and "Theme", and "Profiles" is selected

#### Scenario: Theme section
- **WHEN** the dialog opens and the user selects "Theme"
- **THEN** the theme list is shown with the active theme selected

#### Scenario: Switch to Profiles
- **WHEN** the user previews `tokyo-night` in the theme list and then selects "Profiles"
- **THEN** the tabs show the active theme again and the profile list is shown

#### Scenario: Editor holds the section
- **WHEN** the profile editor has unsaved edits and the user clicks "Theme", or moves focus to the section list and presses down
- **THEN** the profile editor stays open with its edits

#### Scenario: Keyboard-only section change
- **WHEN** the user opens the dialog, presses shift-tab, then down
- **THEN** focus is on the section list with its focus indicator, Theme is shown, and tab then moves focus into the Theme section's list

#### Scenario: Section shortcuts from anywhere
- **WHEN** a choice field, a list, a text field or a button in the dialog has focus and the user presses ctrl-tab
- **THEN** the next section is shown

## MODIFIED Requirements

### Requirement: Live theme preview
Selecting a theme in the list SHALL apply it at once to every tab in the window that follows the global theme. A tab follows the global theme when its profile sets no theme of its own. The preview SHALL also cover window chrome and the tab strip. Tabs whose profile sets a theme SHALL keep it. The preview SHALL NOT change the config file or other windows.

#### Scenario: Preview while browsing
- **WHEN** the global theme is `brindle-dark` and the user moves the selection to `tokyo-night`
- **THEN** the terminals and tab strip behind the dialog redraw in `tokyo-night` immediately

#### Scenario: Profile override untouched
- **WHEN** one tab's profile sets `theme = "gruvbox-dark"` and the user previews `tokyo-night`
- **THEN** that tab stays `gruvbox-dark` and the other tabs show `tokyo-night`

## ADDED Requirements

### Requirement: Keyboard and mouse use
The dialog SHALL be fully operable from the keyboard:
- the theme list and the profile list are each a single Tab stop: while one has focus, up and down (also ctrl-p and ctrl-n) move its selection, home and end select its first and last entry, and enter does its list action. Up and down also open the menu of a focused choice field. They do nothing in an editor's text fields: tab moves between fields;
- alt-up and alt-down move the selected profile up or down in the profile list;
- tab and shift-tab move keyboard focus through the dialog's controls (the section list; then the section's list, or an editor's fields; then the footer buttons; then the dialog's close button) and wrap from the last to the first without leaving the dialog. The section list and every list, choice field, text field, checkbox and button in every section and mode is a stop;
- a list, choice field, checkbox or button that has focus SHALL show a visible focus indicator when focus came from the keyboard (a key press, Tab included, or a section change by key), and none after a pointer press until the next key press; a focused text field always shows it. When a list or an editor opens it takes focus on its first stop;
- a choice field (theme, remote kind, engine) behaves as a select does: enter, space, up or down on it opens its menu on the current option, and in the menu up and down move, enter picks, and escape closes it. Left and right do nothing on a choice field. Space toggles the focused checkbox;
- enter does nothing on the section list (its section is already shown); it applies in the theme list, edits in the profile list, and saves in an editor from a text field or a checkbox; on a focused choice field it opens the menu instead;
- escape closes an open choice menu first; otherwise it cancels in a list and discards in an editor.

The dialog's close button SHALL act as escape does (it cancels in a list and discards in an editor, so it never loses an editor's edits silently).

Enter or space on a focused button SHALL activate that button alone: it SHALL NOT also run what enter does for the section (enter on a focused Cancel button cancels and does not save).

The editors' text fields SHALL be text inputs, as defined by the `text-input` capability, including input-method support. Every list entry, button and field SHALL also work with the mouse, and clicking a field SHALL focus it. A click outside the dialog SHALL NOT close it.

#### Scenario: Keyboard only
- **WHEN** the user opens the dialog, selects "Theme", presses down twice and then enter
- **THEN** the theme two places below the active one becomes the global theme

#### Scenario: Fix one digit of a color
- **WHEN** the background field holds `#1e1e2e`, and the user presses home, right twice, shift-right and types `f`
- **THEN** the field holds `#1f1e2e` and the terminals behind the dialog preview that background

#### Scenario: Tab moves focus
- **WHEN** the editor's name field has focus and the user presses tab
- **THEN** the foreground field has focus and shows the caret, and the name field shows none

#### Scenario: Up and down stay in a text field
- **WHEN** the editor's name field has focus and the user presses down
- **THEN** the name field still has focus

#### Scenario: Tab reaches the list
- **WHEN** the user goes to the Theme section by keyboard and presses end, then tab
- **THEN** the theme list had focus with its focus indicator and end selected the last theme, and tab moved focus to the first enabled footer button

#### Scenario: Tab reaches a select and shows focus
- **WHEN** the profile editor's name field has focus and the user presses tab
- **THEN** the Remote choice field has focus and shows its focus indicator

#### Scenario: Focus shows again after a pointer press
- **WHEN** the user pressed a control with the pointer, and then presses tab
- **THEN** the newly focused control shows its focus indicator

#### Scenario: Tab wraps inside the dialog
- **WHEN** the dialog's last control (its close button) has focus and the user presses tab
- **THEN** its first control, the section list, has focus, and no tab behind the dialog is focused

#### Scenario: Enter on a focused button
- **WHEN** the theme editor has unsaved edits, the Discard button has focus, and the user presses enter
- **THEN** the editor discards its edits and nothing is written, and the dialog does not also save

#### Scenario: Profile editing by keyboard
- **WHEN** the user selects "Profiles", moves to `Work`, presses enter, tabs to the theme field, presses enter to open its menu, then down and enter to pick the theme after the shown one, tabs to Save and presses enter
- **THEN** `Work`'s theme is saved as the theme after its previous one in the choice list, and the profile list is shown

#### Scenario: Choice menu by keyboard
- **WHEN** the profile editor's theme field has focus and the user presses down, down and enter
- **THEN** the first down opens the menu on the shown theme, the second moves to the theme after it, enter selects it, the menu is closed, and the editor is still open

#### Scenario: Left and right do nothing on a choice field
- **WHEN** a choice field has focus with its menu closed and the user presses left or right
- **THEN** the shown option is unchanged, no menu opens and nothing is written

#### Scenario: Escape closes the menu first
- **WHEN** a choice menu is open in the profile editor and the user presses escape
- **THEN** the menu closes, the editor stays open with its edits, and the field is unchanged

#### Scenario: Focus returns to the tab
- **WHEN** the dialog is closed by any means
- **THEN** keyboard focus is on the tab that had it when the dialog opened

### Requirement: Profile editor field list
The profile editor SHALL edit:
- name;
- command (blank means the user's shell);
- arguments, as one line of shell words;
- working directory (blank means the default);
- environment, as one line of `KEY=value` shell words;
- theme: "follow global", or any built-in or custom theme;
- font size (blank means the global size);
- whether the profile is the default.

The editor SHALL say that command, arguments, directory and environment apply to new tabs only. Saving a profile SHALL NOT write `tmux`, `tmux_session` or `tmux_args`, and SHALL leave those keys out of the profile's table when the file still has them.

#### Scenario: Shell words
- **WHEN** the user enters `new-session -A -s "my work"` as arguments and saves
- **THEN** the saved `args` is `["new-session", "-A", "-s", "my work"]`

#### Scenario: Existing values shown as shell words
- **WHEN** a profile has `env = { PAGER = "less -R" }` and the user edits it
- **THEN** the environment field reads `PAGER='less -R'`

#### Scenario: Old tmux keys are dropped on save
- **WHEN** a profile table still has `tmux = "plain"` and the user edits and saves that profile
- **THEN** the saved table has no `tmux`, `tmux_session` or `tmux_args` key

## MODIFIED Requirements

### Requirement: Profile field validation
The editor SHALL mark invalid fields and SHALL block saving while any field is invalid:
- a name that is empty after trimming, or that matches another profile's name ignoring case;
- arguments or environment that are not valid shell words, such as an unclosed quote;
- an environment word without `=`, or whose key is empty or not a valid variable name;
- a font size that is not a number from 4 to 96.

#### Scenario: Duplicate name
- **WHEN** profiles `Shell` and `Work` exist and the user renames `Work` to `shell`
- **THEN** the name field is marked as already used, and Save is unavailable

#### Scenario: Unclosed quote
- **WHEN** the user types `-c "echo hi` as arguments
- **THEN** the arguments field is marked invalid and Save is unavailable

#### Scenario: Bad environment word
- **WHEN** the user types `EDITOR=nvim verbose` as the environment
- **THEN** the environment field is marked invalid because `verbose` has no `=`

## REMOVED Requirements

### Requirement: Sections
**Reason**: Rewritten for two sections, Profiles and Theme, without the General section and its scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Dialog sections".

### Requirement: Keyboard and mouse operation
**Reason**: Rewritten without the General section's choice field and the tmux mode field, and their scenarios. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Keyboard and mouse use".

### Requirement: Profile editor fields
**Reason**: Rewritten without the tmux fields and their scenarios. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Profile editor field list".

### Requirement: General section
**Reason**: The General section's only control was `tmux_session_end`, which is removed with tmux support, so the section has nothing left to show.
**Migration**: None. The dialog has the Profiles and Theme sections. A `tmux_session_end` line in an old config is ignored with a warning.
