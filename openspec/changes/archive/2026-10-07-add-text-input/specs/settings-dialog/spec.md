## MODIFIED Requirements

### Requirement: Keyboard and mouse operation
The dialog SHALL be fully operable from the keyboard:
- up and down (also ctrl-p and ctrl-n) move through the theme list;
- tab and shift-tab move keyboard focus between the editor's fields;
- enter applies in the list and saves in the editor;
- escape cancels in the list and discards in the editor.

The editor's name and color fields SHALL be text inputs, as defined by the `text-input` capability, including input-method support. Every list entry, button and field SHALL also work with the mouse, and clicking a field SHALL focus it.

#### Scenario: Keyboard only
- **WHEN** the user opens the dialog, presses down twice and then enter
- **THEN** the theme two places below the active one becomes the global theme

#### Scenario: Fix one digit of a color
- **WHEN** the background field holds `#1e1e2e`, and the user presses home, right twice, shift-right and types `f`
- **THEN** the field holds `#1f1e2e` and the terminals behind the dialog preview that background

#### Scenario: Tab moves focus
- **WHEN** the editor's name field has focus and the user presses tab
- **THEN** the foreground field has focus and shows the caret, and the name field shows none
