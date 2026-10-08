## MODIFIED Requirements

### Requirement: Sections
The dialog SHALL show a list of sections beside the selected section's content. The sections SHALL be "Theme" and "Profiles", in that order. Leaving the Theme section SHALL drop an unapplied theme preview and select the active theme again. The section SHALL NOT change while an editor has unsaved edits.

#### Scenario: Theme section
- **WHEN** the dialog opens
- **THEN** the section list shows "Theme" and "Profiles", and "Theme" is selected

#### Scenario: Switch to Profiles
- **WHEN** the user previews `tokyo-night` in the theme list and then selects "Profiles"
- **THEN** the tabs show the active theme again and the profile list is shown

#### Scenario: Editor holds the section
- **WHEN** the profile editor has unsaved edits and the user clicks "Theme"
- **THEN** the profile editor stays open with its edits

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

### Requirement: Keyboard and mouse operation
The dialog SHALL be fully operable from the keyboard:
- up and down (also ctrl-p and ctrl-n) move through the theme list and the profile list;
- alt-up and alt-down move the selected profile up or down in the profile list;
- tab and shift-tab move keyboard focus between an editor's fields;
- left and right change the focused choice field (theme, tmux mode), and space toggles the focused checkbox;
- enter applies in the theme list, edits in the profile list, and saves in an editor;
- escape cancels in a list and discards in an editor.

The editors' text fields SHALL be text inputs, as defined by the `text-input` capability, including input-method support. Every list entry, button and field SHALL also work with the mouse, and clicking a field SHALL focus it.

#### Scenario: Keyboard only
- **WHEN** the user opens the dialog, presses down twice and then enter
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

## ADDED Requirements

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
