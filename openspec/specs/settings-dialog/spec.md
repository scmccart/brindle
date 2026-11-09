# settings-dialog Specification

## Purpose
Let users change Brindle's settings from inside the app, see the effect live on their own terminals before committing, and save the result to the config file. The theme is the first setting covered, including creating and editing custom themes.

## Requirements

### Requirement: Opening the settings dialog
ctrl-shift-, and the "Open Settings" command SHALL open the settings dialog as a modal overlay over the current window. Keyboard focus SHALL move to the dialog. Pressing the shortcut while the dialog is open SHALL close it, as cancel does.

#### Scenario: Open from the shortcut
- **WHEN** the user presses ctrl-shift-, in a tab
- **THEN** the settings dialog opens over the window, with the General section showing

#### Scenario: Open from the command palette
- **WHEN** the user runs "Open Settings" from the command palette
- **THEN** the palette closes and the settings dialog opens

### Requirement: One overlay at a time
At most one overlay, the settings dialog or a palette, SHALL be open at a time. Opening a palette SHALL cancel an open settings dialog, and opening the settings dialog SHALL close an open palette. While the dialog is open, window shortcuts other than its own and the palettes' SHALL NOT act, so nothing such as a new tab is created during a preview.

#### Scenario: Palette replaces the dialog
- **WHEN** the settings dialog is previewing a theme and the user presses ctrl-shift-p
- **THEN** the dialog closes, the previous theme is restored, and the command palette opens

#### Scenario: Shortcuts held while open
- **WHEN** the settings dialog is open and the user presses ctrl-shift-t
- **THEN** no tab is opened

### Requirement: Sections
The dialog SHALL show a list of sections beside the selected section's content. The sections SHALL be "General", "Profiles" and "Theme", in that order, and the dialog SHALL open on "General". The section list SHALL be reachable from the keyboard as a vertical tab list: it is a single Tab stop, the first of the dialog's, and while it has focus up and down select the previous and next section (wrapping from the last to the first and back), home and end the first and last, each shown at once while focus stays on the list; the shown section SHALL show a focus indicator while the list has focus from the keyboard. ctrl-tab and ctrl-pagedown (next) and ctrl-shift-tab and ctrl-pageup (previous) SHALL also change the section from anywhere in the dialog. Leaving the Theme section SHALL drop an unapplied theme preview and select the active theme again. The section SHALL NOT change while an editor has unsaved edits, by any of these keys or a click.

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
- **WHEN** the profile editor has unsaved edits and the user clicks "Theme", or moves focus to the section list and presses down
- **THEN** the profile editor stays open with its edits

#### Scenario: Keyboard-only section change
- **WHEN** the user opens the dialog, presses shift-tab, then down, then down
- **THEN** focus is on the section list with its focus indicator, the first down shows Profiles and the second shows Theme, and tab then moves focus into the Theme section's list

#### Scenario: Section shortcuts from anywhere
- **WHEN** a choice field, a list, a text field or a button in the dialog has focus and the user presses ctrl-tab
- **THEN** the next section is shown

### Requirement: Theme list
The Theme section SHALL list every built-in theme followed by the user's custom themes, in a stable order. The active global theme SHALL be marked, and selected whenever the Theme section is shown, and custom themes SHALL be marked as custom. The custom themes SHALL be read from the config file even when another part of the file has an error. A custom theme that cannot be read SHALL be listed with its error and SHALL NOT be selectable.

#### Scenario: Custom theme listed
- **WHEN** the config defines `[themes.mine]` and `theme = "mine"`, and the user opens the dialog and selects "Theme"
- **THEN** "mine" is listed after the built-ins, marked custom and active, and is selected

#### Scenario: Error elsewhere in the config
- **WHEN** the config has an unknown key `fontsize = 3` and defines `[themes.mine]`
- **THEN** "mine" is still listed in the dialog

### Requirement: Live theme preview
Selecting a theme in the list SHALL apply it at once to every tab in the window that follows the global theme. A tab follows the global theme when its profile sets no theme of its own. The preview SHALL also cover window chrome, tmux panes and the tab strip. Tabs whose profile sets a theme SHALL keep it. The preview SHALL NOT change the config file or other windows.

#### Scenario: Preview while browsing
- **WHEN** the global theme is `brindle-dark` and the user moves the selection to `tokyo-night`
- **THEN** the terminals and tab strip behind the dialog redraw in `tokyo-night` immediately

#### Scenario: Profile override untouched
- **WHEN** one tab's profile sets `theme = "gruvbox-dark"` and the user previews `tokyo-night`
- **THEN** that tab stays `gruvbox-dark` and the other tabs show `tokyo-night`

### Requirement: Apply and cancel
Apply (enter in the theme list, or the Apply button) SHALL save the selected theme as the global `theme` in the config, close the dialog, and apply the theme to all windows without waiting for the file watcher. Cancel (escape in a list, the Cancel button, or the Close button) SHALL close the dialog, restore every previewed tab to its theme and font size from before the preview, and leave the config file untouched by anything not already saved. A click outside the dialog SHALL NOT close it, in a list or an editor; only these explicit actions close it.

#### Scenario: Apply
- **WHEN** the user selects `gruvbox-dark` and presses enter
- **THEN** the config's `theme` becomes `"gruvbox-dark"`, the dialog closes, and every window shows `gruvbox-dark`

#### Scenario: Cancel reverts
- **WHEN** the user previews `tokyo-night` and presses escape
- **THEN** the dialog closes, every tab shows its previous theme again, and the config file is unchanged

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
"New" SHALL open the theme editor with a copy of the selected theme's colors and an unused name based on it. Built-in themes SHALL NOT be editable in place: editing one SHALL start from such a copy.

#### Scenario: Copy a built-in
- **WHEN** `tokyo-night` is selected and the user chooses New
- **THEN** the editor opens with `tokyo-night`'s colors and a name such as `tokyo-night-custom`

### Requirement: Theme editor colors
The editor SHALL show every theme color with a swatch and an editable hex field: foreground, background, cursor, cursor text, selection background, selection foreground, accent, the four search highlight colors (match background and foreground, focused match background and foreground), and the 16 ANSI colors labelled by name. Fields SHALL accept `#rrggbb` and `#rgb`. Each valid edit SHALL be previewed at once, as in the theme list. An invalid field SHALL be marked, SHALL keep the last valid color in the preview, and SHALL block saving.

#### Scenario: Edit previews immediately
- **WHEN** the user changes the background field to `#000000`
- **THEN** the swatch and the terminals behind the dialog turn black

#### Scenario: Invalid color
- **WHEN** the user types `#12345` in the cursor field
- **THEN** the field is marked invalid, the preview keeps the previous cursor color, and Save is unavailable

#### Scenario: Search colors listed
- **WHEN** the user opens the editor on `tokyo-night`
- **THEN** the search highlight fields are shown, pre-filled with the colors `tokyo-night` derives for them

### Requirement: Optional selection foreground
Selection foreground SHALL be clearable. A cleared selection foreground SHALL mean that selected cells keep their own colors. Cursor text, accent and the four search highlight colors SHALL always hold an explicit color, pre-filled with the effective color of the theme the editor was opened from.

#### Scenario: Clear selection foreground
- **WHEN** the user clears the selection foreground and saves
- **THEN** the saved theme has no `selection_foreground`, and selected text keeps its colors

#### Scenario: Search colors saved explicitly
- **WHEN** the user copies a built-in theme and saves it without touching the search fields
- **THEN** the saved theme has all four `search_*` keys, holding the colors the built-in theme derived

### Requirement: Theme names
A custom theme's name SHALL be non-empty after trimming, SHALL differ from every built-in theme name, and SHALL differ from every other custom theme's name. An invalid name SHALL be reported in the editor and SHALL block saving.

#### Scenario: Built-in name refused
- **WHEN** the user names a custom theme `tokyo-night`
- **THEN** the editor reports that the name belongs to a built-in theme and Save is unavailable

### Requirement: Saving a custom theme
Save in the editor SHALL write the theme to the config file as `[themes.<name>]`, with every color it holds written explicitly. The editor SHALL then return to the theme list with that theme selected and still previewed. Saving a theme SHALL NOT by itself change the global `theme`. Discard SHALL return to the list without writing, restoring the preview of the theme selected before editing.

#### Scenario: Save then apply
- **WHEN** the user creates `night-owl`, saves it, then presses enter in the list
- **THEN** the config contains `[themes.night-owl]` with all its colors and `theme = "night-owl"`

#### Scenario: Discard edits
- **WHEN** the user edits colors and chooses Discard
- **THEN** nothing is written, and the list shows and previews the previously selected theme

### Requirement: Renaming a custom theme
Changing a saved custom theme's name in the editor and saving SHALL rename its table in the config. The rename SHALL update the global `theme` and every profile `theme` that referred to the old name.

#### Scenario: Rename the active theme
- **WHEN** `theme = "mine"` and a profile sets `theme = "mine"`, and the user renames `mine` to `ours`
- **THEN** the config has `[themes.ours]`, `theme = "ours"` and the profile's `theme = "ours"`, and no `[themes.mine]`

### Requirement: Deleting a custom theme
Delete SHALL remove a custom theme's table from the config after the user confirms. Delete SHALL be refused, with the reason shown, while the global `theme` or any profile `theme` refers to the theme. Built-in themes SHALL NOT be deletable.

#### Scenario: Delete an unused theme
- **WHEN** the user deletes the custom theme `old`, which nothing refers to, and confirms
- **THEN** `[themes.old]` is removed from the config and from the list

#### Scenario: Theme in use
- **WHEN** the user tries to delete the custom theme that is the global `theme`
- **THEN** the dialog says the theme is in use and nothing is removed

### Requirement: Save failures are shown
When a save, rename or delete cannot be written, the dialog SHALL stay open, keep the user's edits and preview, and show the reason. This includes a config file that is not valid TOML and a write error.

#### Scenario: Malformed config file
- **WHEN** the config file has a TOML syntax error and the user presses Apply
- **THEN** the dialog stays open showing that the config file must be fixed first, and the file is unchanged

### Requirement: Keyboard and mouse operation
The dialog SHALL be fully operable from the keyboard:
- the theme list and the profile list are each a single Tab stop: while one has focus, up and down (also ctrl-p and ctrl-n) move its selection, home and end select its first and last entry, and enter does its list action. Up and down also open the menu of a focused choice field. They do nothing in an editor's text fields: tab moves between fields;
- alt-up and alt-down move the selected profile up or down in the profile list;
- tab and shift-tab move keyboard focus through the dialog's controls (the section list; then the section's list, or an editor's fields, or General's choice field; then the footer buttons; then the dialog's close button) and wrap from the last to the first without leaving the dialog. The section list and every list, choice field, text field, checkbox and button in every section and mode is a stop;
- a list, choice field, checkbox or button that has focus SHALL show a visible focus indicator when focus came from the keyboard (a key press, Tab included, or a section change by key), and none after a pointer press until the next key press; a focused text field always shows it. When a list or an editor opens it takes focus on its first stop;
- a choice field (the General section's setting, theme, tmux mode, remote kind, engine) behaves as a select does: enter, space, up or down on it opens its menu on the current option, and in the menu up and down move, enter picks, and escape closes it. Left and right do nothing on a choice field. Space toggles the focused checkbox;
- enter does nothing on the section list (its section is already shown); it applies in the theme list, edits in the profile list, and saves in an editor from a text field or a checkbox; on a focused choice field it opens the menu instead, and in the General section it closes the dialog from anywhere but a choice field or the section list;
- escape closes an open choice menu first; otherwise it cancels in a list and in the General section, and discards in an editor.

The dialog's close button SHALL act as escape does (it cancels in a list and in General and discards in an editor, so it never loses an editor's edits silently).

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

#### Scenario: The General select shows focus after switching sections
- **WHEN** the user opens the dialog, presses ctrl-pagedown to go to Profiles and ctrl-pageup to come back to General
- **THEN** General's choice field has focus and shows its focus indicator

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

#### Scenario: General setting by keyboard
- **WHEN** the user opens the dialog and presses down, down and enter
- **THEN** the session-end setting shows "Open a new tab" and the config has `tmux_session_end = "new_tab"`

#### Scenario: Focus returns to the tab
- **WHEN** the dialog is closed by any means
- **THEN** keyboard focus is on the tab that had it when the dialog opened

### Requirement: Profile list
The Profiles section SHALL list every profile in config order, each with its ctrl-alt-N number (for the first nine), a short description of what it runs, and a marker on the default profile. When the config defines no profiles, the implicit "Shell" profile SHALL be listed. The profiles SHALL be read from the config file even when another part of the file has an error. A profile that cannot be read SHALL be listed with its error and SHALL NOT be editable.

#### Scenario: Profiles in order
- **WHEN** the config defines `Shell`, `tmux` and `Work`, with `default_profile = "work"`
- **THEN** the list shows `1 Shell`, `2 tmux` and `3 Work`, and `Work` is marked default

#### Scenario: Error elsewhere in the config
- **WHEN** the config has an unknown top-level key and defines a profile `Work`
- **THEN** `Work` is still listed and editable

### Requirement: Profile editor fields
The profile editor SHALL edit:
- name;
- command (blank means the user's shell);
- arguments, as one line of shell words;
- working directory (blank means the default);
- environment, as one line of `KEY=value` shell words;
- theme: "follow global", or any built-in or custom theme;
- font size (blank means the global size);
- tmux mode: none, plain or control;
- tmux session (blank means `main`) and tmux arguments, as shell words, shown only when the tmux mode isn't none;
- whether the profile is the default.

The editor SHALL say that command, arguments, directory, environment and tmux settings apply to new tabs only.

#### Scenario: Shell words
- **WHEN** the user enters `-L work -f "~/my tmux.conf"` as tmux arguments and saves
- **THEN** the saved `tmux_args` is `["-L", "work", "-f", "~/my tmux.conf"]`

#### Scenario: Existing values shown as shell words
- **WHEN** a profile has `env = { PAGER = "less -R" }` and the user edits it
- **THEN** the environment field reads `PAGER='less -R'`

#### Scenario: tmux fields hidden
- **WHEN** the tmux mode is none
- **THEN** the session and tmux arguments fields are not shown

### Requirement: Profile field validation
The editor SHALL mark invalid fields and SHALL block saving while any field is invalid:
- a name that is empty after trimming, or that matches another profile's name ignoring case;
- arguments, environment or tmux arguments that are not valid shell words, such as an unclosed quote;
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

### Requirement: Profile preview
While the profile editor is open, a valid theme or font size SHALL apply at once to the open tabs of the profile being edited, in the dialog's window, and to no other tabs. Choosing "follow global" SHALL show the global theme on those tabs. Discard, cancel, or leaving the editor without saving SHALL restore those tabs.

#### Scenario: Preview a profile theme
- **WHEN** two tabs use profile `Work`, one uses `Shell`, and the user sets `Work`'s theme to `tokyo-night` in the editor
- **THEN** both `Work` tabs show `tokyo-night` and the `Shell` tab is unchanged

#### Scenario: Discard restores
- **WHEN** the user previews font size 18 on `Work` and chooses Discard
- **THEN** the `Work` tabs return to their previous font size, and nothing is written

### Requirement: Saving a profile
Save in the profile editor SHALL write that profile's table in the config, keeping the file's comments and every other table, and return to the profile list with the profile selected. Blank fields SHALL be written as absent keys. Making a profile the default SHALL set `default_profile` to its name. When the config defined no profiles, saving SHALL write the implicit "Shell" profile as well, so it stays listed. The dialog SHALL stay open, and the config SHALL be reloaded at once.

#### Scenario: Save keeps comments
- **WHEN** the user changes `Shell`'s working directory to `~/src` and saves
- **THEN** the `Shell` table has `cwd = "~/src"`, and the comments around it are unchanged

#### Scenario: First profile edit on an empty config
- **WHEN** the config has no `[[profiles]]` and the user creates a profile `Work`
- **THEN** the config has a `Shell` profile followed by a `Work` profile

#### Scenario: Config changed underneath
- **WHEN** the profile being saved is no longer at its position with its name in the config file, because the file was edited since the dialog read it
- **THEN** nothing is written, and the dialog asks the user to reopen the section

### Requirement: Renaming a profile
Changing a profile's name and saving SHALL rename it in the config and SHALL update `default_profile` when it named the old name, ignoring case.

#### Scenario: Rename the default
- **WHEN** `default_profile = "work"` and the user renames `Work` to `Job`
- **THEN** the config has the profile `Job` and `default_profile = "Job"`

### Requirement: New and duplicate profiles
New SHALL open the editor for a profile with default settings and an unused name, `Profile N`. Duplicate SHALL open the editor with a copy of the selected profile's settings, named `<name> copy` (or `<name> copy N` when that is taken), and not marked default. The new profile SHALL be written after the last profile.

#### Scenario: Duplicate
- **WHEN** the user duplicates `Work`
- **THEN** the editor opens with `Work`'s settings, named `Work copy`

### Requirement: Deleting a profile
Delete SHALL remove the selected profile from the config after the user confirms. Delete SHALL be refused, with the reason shown, for the only profile. When the deleted profile is the default, the confirmation SHALL say so, and deleting it SHALL remove `default_profile`. Open tabs of a deleted profile SHALL keep running unchanged.

#### Scenario: Delete the default
- **WHEN** `Work` is the default and the user deletes it and confirms
- **THEN** the `Work` table and `default_profile` are removed, and the first profile becomes the default

#### Scenario: Last profile
- **WHEN** only `Shell` is defined and the user tries to delete it
- **THEN** the dialog says the last profile can't be deleted and nothing is removed

### Requirement: Reordering profiles
alt-up and alt-down, and the move buttons, SHALL move the selected profile one place in the config and write the change at once, keeping each profile's comments with it. The list SHALL show the new ctrl-alt-N numbers.

#### Scenario: Move up
- **WHEN** the list is `Shell`, `tmux`, `Work` and the user selects `Work` and presses alt-up
- **THEN** the config order is `Shell`, `Work`, `tmux`, and ctrl-alt-2 opens `Work`

### Requirement: General section
The General section SHALL show the `tmux_session_end` setting as a choice field: "Close window", "Open a new tab" or "Ask". It SHALL be focused when the section is shown, and labelled with what it governs. Changing the choice SHALL write the key to the config at once, keeping the rest of the file, and SHALL apply it without waiting for the file watcher. Cancel SHALL NOT revert a change already written. A failed write SHALL show the reason and keep the previous value. The section SHALL have a Close button that closes the dialog, as escape does there.

#### Scenario: Current value shown
- **WHEN** the config has `tmux_session_end = "ask"` and the user opens the dialog
- **THEN** the General section shows "Ask"

#### Scenario: Change is saved at once
- **WHEN** the config has comments and no `tmux_session_end`, and the user changes the choice to "Ask" and closes the dialog
- **THEN** the config has `tmux_session_end = "ask"` and its comments, and a session ending in this window then asks

#### Scenario: Malformed config file
- **WHEN** the config file has a TOML syntax error and the user changes the choice
- **THEN** the section shows that the config file must be fixed first, the choice stays as it was, and the file is unchanged

#### Scenario: Closing with the mouse
- **WHEN** the General section is showing and the user clicks Close
- **THEN** the dialog closes, and a choice already changed stays saved

### Requirement: Remote target in the profile editor
The profile editor SHALL show a "Remote" field with the choices `none`, `ssh` and `container`. For `ssh` it SHALL show host, user, port, identity file and ssh arguments; for `container` it SHALL show engine (`docker` or `podman`), container and user. Fields of the other kinds SHALL be hidden. Choosing `none` SHALL remove the profile's `remote` on save. Saving SHALL write the `remote` table with blank optional fields as absent keys and ssh arguments as shell words, as for the profile's other list fields.

#### Scenario: Add an SSH target
- **WHEN** the user sets Remote to `ssh`, host to `A8` and user to `ubuntu` on profile `Work` and saves
- **THEN** the `Work` table has `remote = { type = "ssh", host = "A8", user = "ubuntu" }` and no other remote keys

#### Scenario: Container target
- **WHEN** the user sets Remote to `container`, engine `podman` and container `my-box` and saves
- **THEN** the profile has `remote = { type = "container", engine = "podman", container = "my-box" }`

#### Scenario: Remove a target
- **WHEN** the user opens a remote profile, sets Remote to `none` and saves
- **THEN** the profile's table no longer has `remote`

#### Scenario: Existing target shown
- **WHEN** the user opens a profile whose config has `remote = { type = "ssh", host = "A8", port = 2222 }`
- **THEN** the editor shows Remote `ssh`, host `A8` and port `2222`

### Requirement: Remote target validation
An `ssh` target SHALL need a non-blank host and a container target a non-blank container. A port SHALL be blank or a number from 1 to 65535. An invalid field SHALL be marked and SHALL block saving.

#### Scenario: Empty host
- **WHEN** Remote is `ssh` and the host is blank
- **THEN** the host field is marked invalid and Save is unavailable

#### Scenario: Bad port
- **WHEN** the user types `70000` as the port
- **THEN** the port field is marked invalid and Save is unavailable

### Requirement: Profile list shows the remote
The profile list SHALL show a remote profile's target (`ssh:<host>` or `container:<name>`) beside its name.

#### Scenario: Remote marked in the list
- **WHEN** profile `Work` has an SSH target with host `A8`
- **THEN** its row shows `ssh:A8`
