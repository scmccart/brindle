## ADDED Requirements

### Requirement: Commands listed for the active tab
The palette SHALL list the commands that apply to the tab that had focus when it opened, each with a readable name. Commands that do not apply to that tab SHALL NOT be listed. Commands that only work inside a palette, and the "none" action, SHALL NOT be listed. The list SHALL be in a stable order. The `Rename Tab` command SHALL be listed for every tab.

#### Scenario: Shell tab
- **WHEN** a shell tab is active and the user opens the palette
- **THEN** commands such as "Reload Config", "Clear Scrollback", "Rename Tab" and "Split Zone Right" are listed, and no command whose name starts with `tmux:` is listed

### Requirement: Text prompt
A command that needs text SHALL turn the palette into a prompt showing what is asked and, when the command provides one, a pre-filled value. The prompt SHALL be a text input, as defined by the `text-input` capability, with the pre-filled value selected so typing replaces it. Enter SHALL submit the text and escape SHALL cancel without running the command. A command started from its keybinding SHALL open the same prompt directly.

#### Scenario: Rename tab prompt
- **WHEN** the user picks "Rename Tab" in the palette while the active tab is titled `bash`
- **THEN** the palette asks for the new tab title, pre-filled with `bash`

#### Scenario: Prompt cancelled
- **WHEN** the user presses escape in a prompt
- **THEN** the palette closes and the command does not run

#### Scenario: Edit the pre-filled value
- **WHEN** the rename prompt is pre-filled with `bash` and the user presses end and types `-2`
- **THEN** enter renames the tab to `bash-2`

### Requirement: Palette dismissal and focus
Keyboard focus SHALL stay inside an open palette: tab and shift-tab SHALL NOT move it to the tab behind. Escape SHALL dismiss the palette, and a click outside it SHALL dismiss it too. However a palette closes, keyboard focus SHALL return to the tab that had it, except when the closing makes another overlay open.

#### Scenario: Tab stays inside
- **WHEN** the command palette is open and the user presses tab
- **THEN** focus is still in the palette's filter

#### Scenario: Escape returns focus
- **WHEN** the user opens the command palette and presses escape
- **THEN** the palette closes and keyboard focus is on the tab

#### Scenario: Click outside a plain palette
- **WHEN** the new-tab palette is open and the user clicks on the terminal behind it
- **THEN** the palette closes and keyboard focus is on the tab

## MODIFIED Requirements

### Requirement: Running a command
Running a command SHALL close the palette, return keyboard focus to the tab, and act exactly as the command's keybinding would in that tab.

#### Scenario: Same as the shortcut
- **WHEN** the user runs "Split Zone Right" from the palette in a tab
- **THEN** the active zone is split, as with its shortcut, and focus moves to the new zone

## REMOVED Requirements

### Requirement: Commands listed for the focused tab
**Reason**: Rewritten without the tmux commands and control-mode listing. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Commands listed for the active tab".

### Requirement: Text prompt step
**Reason**: Rewritten without the tmux prompt scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Text prompt".

### Requirement: Dismissal and focus
**Reason**: Rewritten without the session-ended and open-elsewhere questions, which were tmux-only. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Palette dismissal and focus".

### Requirement: Prompt failures are shown
**Reason**: Only the `tmux: Run Command` prompt could report a failure back to the palette; `Rename Tab` cannot fail.
**Migration**: None.
