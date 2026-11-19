## MODIFIED Requirements

### Requirement: Opening the settings dialog
ctrl-shift-,, the "Open Settings" command and the profile menu's "Settings" item SHALL open the settings dialog as a modal overlay over the current window. Keyboard focus SHALL move to the dialog. Pressing the shortcut while the dialog is open SHALL close it, as cancel does.

#### Scenario: Open from the shortcut
- **WHEN** the user presses ctrl-shift-, in a tab
- **THEN** the settings dialog opens over the window, with the Profiles section showing

#### Scenario: Open from the command palette
- **WHEN** the user runs "Open Settings" from the command palette
- **THEN** the palette closes and the settings dialog opens

#### Scenario: Open from the profile menu
- **WHEN** the user chooses "Settings" in the strip's `⌄` menu
- **THEN** the menu closes and the settings dialog opens, with the Profiles section showing

### Requirement: Theme list
The Theme section SHALL list every built-in theme followed by the user's custom themes, in a stable order. The active global theme SHALL be marked, and selected whenever the Theme section is shown. Each built-in theme SHALL be marked as built-in, with a lock icon whose tooltip says the theme is built in and New makes an editable copy, and each custom theme as custom. The custom themes SHALL be read from the config file even when another part of the file has an error. A custom theme that cannot be read SHALL be listed with its error and SHALL NOT be selectable. When the file can't be read, isn't valid TOML, or has a `themes` key that isn't a table, the section SHALL say so, naming the file and the cause, and list the custom themes of the config in effect.

#### Scenario: Custom theme listed
- **WHEN** the config defines `[themes.mine]` and `theme = "mine"`, and the user opens the dialog and selects "Theme"
- **THEN** "mine" is listed after the built-ins, marked custom and active, and is selected

#### Scenario: Built-in themes marked
- **WHEN** the user opens the dialog and selects "Theme"
- **THEN** `brindle-dark`, `brindle-light`, `tokyo-night`, `gruvbox-dark` and `solarized-dark` are each marked built-in with a lock icon, and no custom theme is

#### Scenario: Error elsewhere in the config
- **WHEN** the config has an unknown key `fontsize = 3` and defines `[themes.mine]`
- **THEN** "mine" is still listed in the dialog

#### Scenario: Themes of the wrong shape
- **WHEN** the config sets `themes = 3`, and the user opens the dialog and selects "Theme"
- **THEN** the section says `themes` in the config file is not a table, and lists the built-in themes

### Requirement: Apply and cancel
Apply (enter in the theme list, or the Apply button) SHALL save the selected theme as the global `theme` in the config, close the dialog, and apply the theme to all windows without waiting for the file watcher. Cancel (escape in a list, the Cancel button, the profile list's Close button, or the dialog's close button in any view) SHALL close the dialog, restore every previewed tab to its theme and font size from before the preview, and leave the config file untouched by anything not already saved. A click outside the dialog SHALL NOT close it, in a list or an editor; only these explicit actions close it.

#### Scenario: Apply
- **WHEN** the user selects `gruvbox-dark` and presses enter
- **THEN** the config's `theme` becomes `"gruvbox-dark"`, the dialog closes, and every window shows `gruvbox-dark`

#### Scenario: Cancel reverts
- **WHEN** the user previews `tokyo-night` and presses escape
- **THEN** the dialog closes, every tab shows its previous theme again, and the config file is unchanged

#### Scenario: Close button reverts an editor's preview
- **WHEN** the theme editor has an unsaved background of `#000000` previewing on the tabs, and the user presses the dialog's close button
- **THEN** the dialog closes, every tab shows its theme from before the dialog opened, and the config file is unchanged

#### Scenario: Stray click in a list
- **WHEN** the theme list is previewing `tokyo-night` and the user clicks on the terminal behind the dialog
- **THEN** the dialog stays open and `tokyo-night` is still previewed

#### Scenario: Stray click while editing
- **WHEN** the theme editor is open and the user clicks on the terminal behind the dialog
- **THEN** the dialog stays open with the edits intact

#### Scenario: Stray click in the profile editor
- **WHEN** the profile editor is open and the user clicks on the terminal behind the dialog
- **THEN** the dialog stays open with the edits intact

### Requirement: Creating a custom theme
"New" SHALL open the theme editor with a copy of the selected theme's colors and an unused name based on it. Built-in themes SHALL NOT be editable: while a built-in theme is selected, Edit SHALL be unavailable, and its key `e` SHALL open nothing and say that the theme is built in and that New makes an editable copy.

#### Scenario: Copy a built-in
- **WHEN** `tokyo-night` is selected and the user chooses New
- **THEN** the editor opens with `tokyo-night`'s colors and a name such as `tokyo-night-custom`

#### Scenario: Edit unavailable on a built-in
- **WHEN** `tokyo-night` is selected in the theme list
- **THEN** the Edit button is unavailable, and New and Apply are available

#### Scenario: Edit key on a built-in
- **WHEN** `tokyo-night` is selected and the user presses `e`
- **THEN** the list stays shown with `tokyo-night` selected, and the dialog says `tokyo-night` is built in and New makes an editable copy

#### Scenario: Edit a custom theme
- **WHEN** the custom theme `mine` is selected and the user presses `e`
- **THEN** the editor opens on `mine` under its own name

### Requirement: Deleting a custom theme
Delete SHALL remove a custom theme's table from the config after the user confirms. Delete SHALL be refused, with the reason shown, while the global `theme` or any profile `theme` refers to the theme. Built-in themes SHALL NOT be deletable: while a built-in theme is selected, Delete SHALL be unavailable, and its keys SHALL ask nothing and say that the theme is built in.

#### Scenario: Delete an unused theme
- **WHEN** the user deletes the custom theme `old`, which nothing refers to, and confirms
- **THEN** `[themes.old]` is removed from the config and from the list

#### Scenario: Theme in use
- **WHEN** the user tries to delete the custom theme that is the global `theme`
- **THEN** the dialog says the theme is in use and nothing is removed

#### Scenario: Built-in theme
- **WHEN** `gruvbox-dark` is selected and the user presses `d`
- **THEN** no confirmation is asked, the Delete button is unavailable, and the dialog says `gruvbox-dark` is built in

### Requirement: Keyboard and mouse use
The dialog SHALL be fully operable from the keyboard:
- the theme list and the profile list are each a single Tab stop: while one has focus, up and down (also ctrl-p and ctrl-n) move its selection, home and end select its first and last entry, and enter does its list action. Up and down also open the menu of a focused choice field. They do nothing in an editor's text fields: tab moves between fields;
- alt-up and alt-down move the selected profile up or down in the profile list;
- tab and shift-tab move keyboard focus through the dialog's controls (the section list; then the section's list, or an editor's fields; then the footer buttons; then the dialog's close button) and wrap from the last to the first without leaving the dialog. The section list and every list, choice field, text field, checkbox, switch and button in every section and mode is a stop;
- a list, choice field, checkbox, switch or button that has focus SHALL show a visible focus indicator when focus came from the keyboard (a key press, Tab included, or a section change by key), and none after a pointer press until the next key press; a focused text field always shows it. When a list or an editor opens it takes focus on its first stop;
- a choice field (theme, remote kind, engine) behaves as a select does: enter, space, up or down on it opens its menu on the current option, and in the menu up and down move, enter picks, and escape closes it. Left and right do nothing on a choice field. Space toggles the focused checkbox or switch;
- enter does nothing on the section list (its section is already shown); it applies in the theme list, edits in the profile list, and saves in an editor from a text field or a checkbox; on a focused choice field it opens the menu instead;
- escape closes an open choice menu first; otherwise it cancels in a list and discards in an editor.

The dialog's close button SHALL close the dialog from any view, as the dialog's shortcut does: every preview ends, an editor's or a form's unsaved changes are dropped, and nothing is written. It SHALL NOT merely step back to the list as escape does.

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

#### Scenario: Close button in an editor
- **WHEN** the profile editor has an unsaved rename of `Shell` to `work`, and the user presses the dialog's close button (or focuses it and presses enter)
- **THEN** the dialog closes, keyboard focus is on the tab, and the config file still names the profile `Shell`

#### Scenario: Close button in the delete question
- **WHEN** the dialog asks whether to delete the custom theme `old` and the user presses the dialog's close button
- **THEN** the dialog closes and `[themes.old]` is still in the config

#### Scenario: Escape still steps back
- **WHEN** the theme editor is open and the user presses escape
- **THEN** the editor closes and the theme list is shown, and the dialog stays open

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

### Requirement: Dialog sections
The dialog SHALL show a list of sections beside the selected section's content. The sections SHALL be "Profiles", "Theme", "Fonts" and "Clipboard", in that order, and the dialog SHALL open on "Profiles". The section list SHALL be reachable from the keyboard as a vertical tab list: it is a single Tab stop, the first of the dialog's, and while it has focus up and down select the previous and next section (wrapping from the last to the first and back), home and end the first and last, each shown at once while focus stays on the list; the shown section SHALL show a focus indicator while the list has focus from the keyboard. ctrl-tab and ctrl-pagedown (next) and ctrl-shift-tab and ctrl-pageup (previous) SHALL also change the section from anywhere in the dialog. Leaving the Theme section SHALL drop an unapplied theme preview and select the active theme again, leaving the Fonts section SHALL drop its unapplied changes and their preview, and leaving the Clipboard section SHALL drop its unapplied changes. The section SHALL NOT change while an editor has unsaved edits, by any of these keys or a click.

#### Scenario: Profiles section first
- **WHEN** the dialog opens
- **THEN** the section list shows "Profiles", "Theme", "Fonts" and "Clipboard", and "Profiles" is selected

#### Scenario: Theme section
- **WHEN** the dialog opens and the user selects "Theme"
- **THEN** the theme list is shown with the active theme selected

#### Scenario: Switch to Profiles
- **WHEN** the user previews `tokyo-night` in the theme list and then selects "Profiles"
- **THEN** the tabs show the active theme again and the profile list is shown

#### Scenario: Leave Fonts unapplied
- **WHEN** the user previews size 18 in the Fonts section and then selects "Theme"
- **THEN** the terminals return to the config's size and the theme list is shown

#### Scenario: Leave Clipboard unapplied
- **WHEN** the user turns on "Right-click pastes" in the Clipboard section and then selects "Profiles"
- **THEN** nothing is written, and the Clipboard section shows the config's values when it is selected again

#### Scenario: Last section wraps
- **WHEN** the section list has focus on "Clipboard" and the user presses down
- **THEN** "Profiles" is shown

#### Scenario: Editor holds the section
- **WHEN** the profile editor has unsaved edits and the user clicks "Theme", or moves focus to the section list and presses down
- **THEN** the profile editor stays open with its edits

#### Scenario: Keyboard-only section change
- **WHEN** the user opens the dialog, presses shift-tab, then down
- **THEN** focus is on the section list with its focus indicator, Theme is shown, and tab then moves focus into the Theme section's list

#### Scenario: Section shortcuts from anywhere
- **WHEN** a choice field, a list, a text field or a button in the dialog has focus and the user presses ctrl-tab
- **THEN** the next section is shown

## ADDED Requirements

### Requirement: Clipboard section
The Clipboard section SHALL show two switches: "Copy on select" (`copy_on_select`) and "Right-click pastes" (on when `right_click` is `"paste"`). They SHALL show the config file's values even when another part of it has an error, and the config in effect's, read-only, when the file can't be parsed. Space SHALL toggle the focused switch, and enter SHALL apply, as Apply does.

#### Scenario: Current values shown
- **WHEN** the config has `copy_on_select = true` and no `right_click`, and the user opens the Clipboard section
- **THEN** "Copy on select" is on and "Right-click pastes" is off

#### Scenario: Unparseable config
- **WHEN** the config file has a TOML syntax error and the user opens the Clipboard section
- **THEN** the switches show the config in effect's values, the section says the file must be fixed first, and Apply is unavailable

### Requirement: Applying clipboard settings
Apply SHALL write the Clipboard section's changed values to the config file, keeping its comments and everything else in it, and close the dialog; the reload SHALL apply them to every open tab. A switch left unchanged SHALL write nothing. Cancel, escape, the close button, or leaving the section SHALL drop the changes.

#### Scenario: Turn on right-click paste
- **WHEN** the config has no `right_click`, and the user turns on "Right-click pastes" and applies
- **THEN** the config has `right_click = "paste"`, its comments are kept, the dialog closes, and a right-click in an open tab with no text selected pastes

#### Scenario: Unchanged switch
- **WHEN** the user turns on "Copy on select" and applies, leaving "Right-click pastes" alone
- **THEN** the config gains `copy_on_select = true` and no `right_click` key

#### Scenario: Cancel drops the change
- **WHEN** the user turns on both switches and presses escape
- **THEN** the dialog closes and the config file is unchanged
