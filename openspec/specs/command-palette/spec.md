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
The palette SHALL list the commands that apply to the tab that had focus when it opened, each with a readable name. Commands that do not apply to that tab SHALL NOT be listed: for example, zone commands ("Split Zone Right", "Split Zone Down", "Close Zone", "Focus Zone Left", "Focus Zone Right", "Focus Zone Up", "Focus Zone Down") SHALL appear only for local tabs, and tmux commands SHALL appear only for control-mode tabs, except "tmux: New Session", which is listed in every tab while the config has a control-mode profile. Commands that only work inside a palette, and the "none" action, SHALL NOT be listed. The list SHALL be in a stable order. The general `Rename Tab` command SHALL be listed for both local and control-mode tabs.

#### Scenario: Local tab
- **WHEN** a local shell tab is active and the user opens the palette
- **THEN** commands such as "Reload Config", "Clear Scrollback", "Rename Tab" and "Split Zone Right" are listed, and no tmux command other than "tmux: New Session" is listed

#### Scenario: Control-mode tab
- **WHEN** a control-mode tab is active and the user opens the palette
- **THEN** tmux commands such as "tmux: Split Right" and "tmux: Rename Window" are listed alongside general commands such as "Rename Tab", and no zone commands are listed

### Requirement: Shortcuts shown
Each listed command that has a keybinding in the focused tab SHALL show that binding, including the user's own overrides. A command without a binding SHALL show none.

#### Scenario: User rebinding
- **WHEN** the config binds `ctrl-alt-r` to `reload_config` and the user opens the palette
- **THEN** "Reload Config" is shown with `ctrl-alt-r`

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

### Requirement: Running a command
Running a command SHALL close the palette, return keyboard focus to the tab, and act exactly as the command's keybinding would in that tab.

#### Scenario: Same as the shortcut
- **WHEN** the user runs "tmux: Split Right" from the palette in a control-mode tab
- **THEN** the active pane is split, as with ctrl-shift-e, and focus moves to the new pane

### Requirement: Text prompt step
A command that needs text SHALL turn the palette into a prompt showing what is asked and, when the command provides one, a pre-filled value. The prompt SHALL be a text input, as defined by the `text-input` capability, with the pre-filled value selected so typing replaces it. Enter SHALL submit the text and escape SHALL cancel without running the command. A command started from its keybinding SHALL open the same prompt directly.

#### Scenario: Prompt from the list
- **WHEN** the user picks "tmux: Rename Window" in the palette
- **THEN** the palette asks for the new name, pre-filled with the current one

#### Scenario: Rename tab prompt
- **WHEN** the user picks "Rename Tab" in the palette while the active tab is titled `bash`
- **THEN** the palette asks for the new tab title, pre-filled with `bash`

#### Scenario: Prompt cancelled
- **WHEN** the user presses escape in a prompt
- **THEN** the palette closes and the command does not run

#### Scenario: Edit the pre-filled value
- **WHEN** the rename prompt is pre-filled with `editor` and the user presses end and types `-2`
- **THEN** enter renames the window to `editor-2`

### Requirement: Prompt failures are shown
When a prompted command reports a failure, the palette SHALL stay open in the prompt, keep the entered text and show the failure message, so the user can correct the text and submit again. When it succeeds, the palette SHALL close.

#### Scenario: Retry after an error
- **WHEN** the user submits `selct-layout tiled` in the "tmux: Run Command" prompt
- **THEN** the prompt stays open with `selct-layout tiled` and shows tmux's error, and after correcting it to `select-layout tiled` and pressing enter the palette closes and the layout changes

### Requirement: Sessions palette
The `Attach Session…` command (Linux) SHALL open a palette listing the local terminal server's detached sessions, newest first, each row showing the session's title (else its command), its working directory and how long ago it started. Typed text SHALL filter the rows as in the command palette. With no detached sessions the palette SHALL say so. Sessions attached to any client SHALL NOT be listed.

#### Scenario: Nothing detached
- **WHEN** every session is attached and the user runs "Attach Session…"
- **THEN** the palette says there are no detached sessions

### Requirement: Sessions palette actions
In the sessions palette, enter SHALL attach the selected session into a new tab after the active tab, and make it active. `picker_kill` (ctrl-shift-delete) SHALL end the selected session and refresh the list.

#### Scenario: Attach a detached session
- **WHEN** a `--no-restore` launch left a session running `top` detached, and the user runs "Attach Session…" and presses enter on its row
- **THEN** a new active tab shows that `top`, with its screen as it was

#### Scenario: Kill from the palette
- **WHEN** the user selects a session's row and presses ctrl-shift-delete
- **THEN** the session's program ends and its row disappears

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
