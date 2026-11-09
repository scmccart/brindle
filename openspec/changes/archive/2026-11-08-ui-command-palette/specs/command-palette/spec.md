## MODIFIED Requirements

### Requirement: Filtering and navigation
Typed text SHALL filter the list by a case-insensitive subsequence match on the command name, keeping the order. The text SHALL be edited with a text input, as defined by the `text-input` capability. The palette SHALL be operable with up/down (also ctrl-p / ctrl-n) to move the selection, wrapping from the last row to the first and back, enter to run the selected command, and escape or a click outside to dismiss. Home and end SHALL move the filter's caret, not the selection. Clicking a command SHALL run it.

#### Scenario: Filter and run
- **WHEN** the user opens the palette, types `clrsc` and presses enter
- **THEN** "Clear Scrollback" runs in the active tab

#### Scenario: No match
- **WHEN** the typed text matches no command
- **THEN** the palette says nothing matches, and enter does nothing

#### Scenario: Fix a typo in the filter
- **WHEN** the user types `splt`, presses left once and types `i`
- **THEN** the filter reads `split` and the list shows the commands matching it

#### Scenario: Wrapping
- **WHEN** the last listed command is selected and the user presses down
- **THEN** the first listed command is selected, and up from the first selects the last

#### Scenario: Home and end edit the filter
- **WHEN** the filter holds `split` and the user presses home and then types `x`
- **THEN** the filter reads `xsplit` and the selection is unchanged

## ADDED Requirements

### Requirement: Dismissal and focus
Keyboard focus SHALL stay inside an open palette: tab and shift-tab SHALL NOT move it to the tab behind. Escape SHALL dismiss the palette, and a click outside it SHALL dismiss it too, except for a palette that asks a question (a session-ended or open-elsewhere choice, and the prompt that follows one), which only escape, its close button or a choice closes. However a palette closes, keyboard focus SHALL return to the tab that had it, except when the closing makes another overlay open.

#### Scenario: Tab stays inside
- **WHEN** the command palette is open and the user presses tab
- **THEN** focus is still in the palette's filter

#### Scenario: Escape returns focus
- **WHEN** the user opens the command palette and presses escape
- **THEN** the palette closes and keyboard focus is on the tab

#### Scenario: Click outside a question
- **WHEN** the palette is asking what to do after a session ended and the user clicks on the terminal behind it
- **THEN** the palette stays open

#### Scenario: Click outside a plain palette
- **WHEN** the new-tab palette is open and the user clicks on the terminal behind it
- **THEN** the palette closes and keyboard focus is on the tab
