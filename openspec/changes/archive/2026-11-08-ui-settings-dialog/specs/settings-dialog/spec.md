## MODIFIED Requirements

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
