# Spec Delta

## Purpose

Let users change Brindle's settings from inside the app, see the effect live on their own terminals before committing, and save the result to the config file. The theme is the first setting covered, including creating and editing custom themes.

## ADDED Requirements

### Requirement: Opening the settings dialog
ctrl-shift-, and the "Open Settings" command SHALL open the settings dialog as a modal overlay over the current window. Keyboard focus SHALL move to the dialog. Pressing the shortcut while the dialog is open SHALL close it, as cancel does.

#### Scenario: Open from the shortcut
- **WHEN** the user presses ctrl-shift-, in a tab
- **THEN** the settings dialog opens over the window, with the Theme section showing

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
The dialog SHALL show a list of sections beside the selected section's content. This change SHALL provide one section, "Theme".

#### Scenario: Theme section
- **WHEN** the dialog opens
- **THEN** the section list shows "Theme" and it is selected

### Requirement: Theme list
The Theme section SHALL list every built-in theme followed by the user's custom themes, in a stable order. The active global theme SHALL be marked and selected when the dialog opens, and custom themes SHALL be marked as custom. The custom themes SHALL be read from the config file even when another part of the file has an error. A custom theme that cannot be read SHALL be listed with its error and SHALL NOT be selectable.

#### Scenario: Custom theme listed
- **WHEN** the config defines `[themes.mine]` and `theme = "mine"`, and the user opens the dialog
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
Apply (enter in the theme list, or the Apply button) SHALL save the selected theme as the global `theme` in the config, close the dialog, and apply the theme to all windows without waiting for the file watcher. Cancel (escape, the Cancel button, or a click outside the dialog while the theme list is showing) SHALL close the dialog, restore every previewed tab to its theme from before the preview, and leave the config file untouched. A click outside SHALL NOT close the dialog while the theme editor is open, so edits aren't lost.

#### Scenario: Apply
- **WHEN** the user selects `gruvbox-dark` and presses enter
- **THEN** the config's `theme` becomes `"gruvbox-dark"`, the dialog closes, and every window shows `gruvbox-dark`

#### Scenario: Cancel reverts
- **WHEN** the user previews `tokyo-night` and presses escape
- **THEN** the dialog closes, every tab shows its previous theme again, and the config file is unchanged

#### Scenario: Stray click while editing
- **WHEN** the theme editor is open and the user clicks on the terminal behind the dialog
- **THEN** the dialog stays open with the edits intact

### Requirement: Creating a custom theme
"New" SHALL open the theme editor with a copy of the selected theme's colors and an unused name based on it. Built-in themes SHALL NOT be editable in place: editing one SHALL start from such a copy.

#### Scenario: Copy a built-in
- **WHEN** `tokyo-night` is selected and the user chooses New
- **THEN** the editor opens with `tokyo-night`'s colors and a name such as `tokyo-night-custom`

### Requirement: Theme editor colors
The editor SHALL show every theme color with a swatch and an editable hex field: foreground, background, cursor, cursor text, selection background, selection foreground, accent, and the 16 ANSI colors labelled by name. Fields SHALL accept `#rrggbb` and `#rgb`. Each valid edit SHALL be previewed at once, as in the theme list. An invalid field SHALL be marked, SHALL keep the last valid color in the preview, and SHALL block saving.

#### Scenario: Edit previews immediately
- **WHEN** the user changes the background field to `#000000`
- **THEN** the swatch and the terminals behind the dialog turn black

#### Scenario: Invalid color
- **WHEN** the user types `#12345` in the cursor field
- **THEN** the field is marked invalid, the preview keeps the previous cursor color, and Save is unavailable

### Requirement: Optional selection foreground
Selection foreground SHALL be clearable. A cleared selection foreground SHALL mean that selected cells keep their own colors. Cursor text and accent SHALL always hold an explicit color, pre-filled with the effective color of the theme the editor was opened from.

#### Scenario: Clear selection foreground
- **WHEN** the user clears the selection foreground and saves
- **THEN** the saved theme has no `selection_foreground`, and selected text keeps its colors

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
- up and down (also ctrl-p and ctrl-n) move through the theme list;
- tab and shift-tab move between the editor's fields;
- enter applies in the list and saves in the editor;
- escape cancels in the list and discards in the editor.

Every list entry, button and field SHALL also work with the mouse.

#### Scenario: Keyboard only
- **WHEN** the user opens the dialog, presses down twice and then enter
- **THEN** the theme two places below the active one becomes the global theme
