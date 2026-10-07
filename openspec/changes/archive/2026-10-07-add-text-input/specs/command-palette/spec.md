## MODIFIED Requirements

### Requirement: Filtering and navigation
Typed text SHALL filter the list by a case-insensitive subsequence match on the command name, keeping the order. The text SHALL be edited with a text input, as defined by the `text-input` capability. The palette SHALL be operable with up/down (also ctrl-p / ctrl-n) to move the selection, enter to run the selected command, and escape or a click outside to dismiss. Clicking a command SHALL run it.

#### Scenario: Filter and run
- **WHEN** the user opens the palette, types `clrsc` and presses enter
- **THEN** "Clear Scrollback" runs in the active tab

#### Scenario: No match
- **WHEN** the typed text matches no command
- **THEN** the palette says nothing matches, and enter does nothing

#### Scenario: Fix a typo in the filter
- **WHEN** the user types `splt`, presses left once and types `i`
- **THEN** the filter reads `split` and the list shows the commands matching it

### Requirement: Text prompt step
A command that needs text SHALL turn the palette into a prompt showing what is asked and, when the command provides one, a pre-filled value. The prompt SHALL be a text input, as defined by the `text-input` capability, with the pre-filled value selected so typing replaces it. Enter SHALL submit the text and escape SHALL cancel without running the command. A command started from its keybinding SHALL open the same prompt directly.

#### Scenario: Prompt from the list
- **WHEN** the user picks "tmux: Rename Window" in the palette
- **THEN** the palette asks for the new name, pre-filled with the current one

#### Scenario: Prompt cancelled
- **WHEN** the user presses escape in a prompt
- **THEN** the palette closes and the command does not run

#### Scenario: Edit the pre-filled value
- **WHEN** the rename prompt is pre-filled with `editor` and the user presses end and types `-2`
- **THEN** enter renames the window to `editor-2`
