## ADDED Requirements

### Requirement: Fonts section
The Fonts section SHALL show four fields:
- **Terminal font**: "Default", then the installed families, monospace ones first;
- **Size**: a number from 4 to 96;
- **Line height**: a number from 0.8 to 3.0;
- **UI font**: "Default", then the installed families.

A family choice SHALL show the first installed family of the config's list for that font, or "Default" when none is installed. Values SHALL be read from the config file even when another part of it has an error.

#### Scenario: Current fonts shown
- **WHEN** the config has `font.family = ["Nonexistent", "Cascadia Mono"]`, `font.size = 13` and no `[ui_font]`, and the user opens the Fonts section
- **THEN** Terminal font shows "Cascadia Mono", Size shows 13, Line height shows the default 1.25, and UI font shows "Default"

### Requirement: Font field validation
A size or line height that is not a number in its range SHALL be marked invalid with the range, and Apply SHALL be disabled while any field is invalid.

#### Scenario: Size out of range
- **WHEN** the user types 200 into Size
- **THEN** Size is marked invalid with "4 to 96", and Apply is disabled

### Requirement: Font preview
While the Fonts section is shown, a valid change to any of its fields SHALL apply at once in the dialog's window: a terminal font, size or line height change to every tab's terminals, which re-fit their grids to the new cell size, and a UI font change to the chrome, the dialog included. A profile's own font size SHALL still win over Size on that profile's tabs. Cancel, closing the dialog, or leaving the Fonts section without applying SHALL restore the fonts the config gives.

#### Scenario: Preview a terminal font
- **WHEN** the user chooses another Terminal font
- **THEN** every tab's terminal redraws in that family and its grid re-fits, before anything is written

#### Scenario: Cancel restores
- **WHEN** the user previews UI font "Inter" and size 18, then presses escape
- **THEN** the chrome and the terminals return to the config's fonts, and nothing is written

### Requirement: Applying fonts
Apply SHALL write the Fonts section's changed values to the config file, keeping its comments and everything else in it, and close the dialog. A field left unchanged SHALL write nothing. When the config file can't be parsed, Apply SHALL write nothing and show the error.

#### Scenario: Choose a UI font
- **WHEN** the config has no `[ui_font]` and the user chooses UI font "Inter" and applies
- **THEN** the config gains `[ui_font]` with `family = ["Inter"]`, its comments are kept, and the chrome uses Inter after the dialog closes

### Requirement: Family lists on apply
Applying a chosen family SHALL move it to the front of that font's `family` list (`[font]` for the terminal font, `[ui_font]` for the UI font), keeping the list's other entries after it as fallbacks. Applying "Default" SHALL remove that font's `family` key.

#### Scenario: Choose a listed fallback
- **WHEN** the config has `font.family = ["JetBrains Mono", "Cascadia Mono"]`, JetBrains Mono isn't installed, and the user chooses "Consolas" and applies
- **THEN** the config has `font.family = ["Consolas", "JetBrains Mono", "Cascadia Mono"]`

## MODIFIED Requirements

### Requirement: Dialog sections
The dialog SHALL show a list of sections beside the selected section's content. The sections SHALL be "Profiles", "Theme" and "Fonts", in that order, and the dialog SHALL open on "Profiles". The section list SHALL be reachable from the keyboard as a vertical tab list: it is a single Tab stop, the first of the dialog's, and while it has focus up and down select the previous and next section (wrapping from the last to the first and back), home and end the first and last, each shown at once while focus stays on the list; the shown section SHALL show a focus indicator while the list has focus from the keyboard. ctrl-tab and ctrl-pagedown (next) and ctrl-shift-tab and ctrl-pageup (previous) SHALL also change the section from anywhere in the dialog. Leaving the Theme section SHALL drop an unapplied theme preview and select the active theme again, and leaving the Fonts section SHALL drop its unapplied changes and their preview. The section SHALL NOT change while an editor has unsaved edits, by any of these keys or a click.

#### Scenario: Profiles section first
- **WHEN** the dialog opens
- **THEN** the section list shows "Profiles", "Theme" and "Fonts", and "Profiles" is selected

#### Scenario: Theme section
- **WHEN** the dialog opens and the user selects "Theme"
- **THEN** the theme list is shown with the active theme selected

#### Scenario: Switch to Profiles
- **WHEN** the user previews `tokyo-night` in the theme list and then selects "Profiles"
- **THEN** the tabs show the active theme again and the profile list is shown

#### Scenario: Leave Fonts unapplied
- **WHEN** the user previews size 18 in the Fonts section and then selects "Theme"
- **THEN** the terminals return to the config's size and the theme list is shown

#### Scenario: Editor holds the section
- **WHEN** the profile editor has unsaved edits and the user clicks "Theme", or moves focus to the section list and presses down
- **THEN** the profile editor stays open with its edits

#### Scenario: Keyboard-only section change
- **WHEN** the user opens the dialog, presses shift-tab, then down
- **THEN** focus is on the section list with its focus indicator, Theme is shown, and tab then moves focus into the Theme section's list

#### Scenario: Section shortcuts from anywhere
- **WHEN** a choice field, a list, a text field or a button in the dialog has focus and the user presses ctrl-tab
- **THEN** the next section is shown
