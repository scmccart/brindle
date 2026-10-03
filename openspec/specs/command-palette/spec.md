# command-palette Specification

## Purpose
Make every Brindle command discoverable and runnable by name from a searchable list over the active tab. A command that needs text, such as a new window name, asks for it as a second step.

## Requirements

### Requirement: Opening the command palette
ctrl-shift-p SHALL open the command palette over the active tab. Pressing it again while the palette is open SHALL close it. Only one palette SHALL be open at a time: opening the command palette or the new-tab palette SHALL close whichever one is already open.

#### Scenario: Toggle
- **WHEN** the user presses ctrl-shift-p twice
- **THEN** the palette opens and then closes, and keyboard focus returns to the tab

#### Scenario: Switching palettes
- **WHEN** the new-tab palette is open and the user presses ctrl-shift-p
- **THEN** the new-tab palette closes and the command palette opens

### Requirement: Commands listed for the focused tab
The palette SHALL list the commands that apply to the tab that had focus when it opened, each with a readable name. Commands that do not apply to that tab SHALL NOT be listed: for example, tmux commands SHALL appear only for control-mode tabs. Commands that only work inside a palette, and the "none" action, SHALL NOT be listed. The list SHALL be in a stable order.

#### Scenario: Local tab
- **WHEN** a local shell tab is active and the user opens the palette
- **THEN** commands such as "Reload Config" and "Clear Scrollback" are listed and no tmux command is listed

#### Scenario: Control-mode tab
- **WHEN** a control-mode tab is active and the user opens the palette
- **THEN** tmux commands such as "tmux: Split Right" and "tmux: Rename Window" are listed alongside the general ones

### Requirement: Shortcuts shown
Each listed command that has a keybinding in the focused tab SHALL show that binding, including the user's own overrides. A command without a binding SHALL show none.

#### Scenario: User rebinding
- **WHEN** the config binds `ctrl-alt-r` to `reload_config` and the user opens the palette
- **THEN** "Reload Config" is shown with `ctrl-alt-r`

### Requirement: Filtering and navigation
Typed text SHALL filter the list by a case-insensitive subsequence match on the command name, keeping the order. The palette SHALL be operable with up/down (also ctrl-p / ctrl-n) to move the selection, backspace to edit the text, enter to run the selected command, and escape or a click outside to dismiss. Clicking a command SHALL run it.

#### Scenario: Filter and run
- **WHEN** the user opens the palette, types `clrsc` and presses enter
- **THEN** "Clear Scrollback" runs in the active tab

#### Scenario: No match
- **WHEN** the typed text matches no command
- **THEN** the palette says nothing matches, and enter does nothing

### Requirement: Running a command
Running a command SHALL close the palette, return keyboard focus to the tab, and act exactly as the command's keybinding would in that tab.

#### Scenario: Same as the shortcut
- **WHEN** the user runs "tmux: Split Right" from the palette in a control-mode tab
- **THEN** the active pane is split, as with ctrl-shift-e, and focus moves to the new pane

### Requirement: Text prompt step
A command that needs text SHALL turn the palette into a prompt showing what is asked and, when the command provides one, a pre-filled value. Enter SHALL submit the text and escape SHALL cancel without running the command. The text SHALL be a single line: pasted line breaks SHALL become spaces. A command started from its keybinding SHALL open the same prompt directly.

#### Scenario: Prompt from the list
- **WHEN** the user picks "tmux: Rename Window" in the palette
- **THEN** the palette asks for the new name, pre-filled with the current one

#### Scenario: Prompt cancelled
- **WHEN** the user presses escape in a prompt
- **THEN** the palette closes and the command does not run

### Requirement: Prompt failures are shown
When a prompted command reports a failure, the palette SHALL stay open in the prompt, keep the entered text and show the failure message, so the user can correct the text and submit again. When it succeeds, the palette SHALL close.

#### Scenario: Retry after an error
- **WHEN** the user submits `selct-layout tiled` in the "tmux: Run Command" prompt
- **THEN** the prompt stays open with `selct-layout tiled` and shows tmux's error, and after correcting it to `select-layout tiled` and pressing enter the palette closes and the layout changes
