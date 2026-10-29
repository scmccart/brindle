# Spec Delta

## ADDED Requirements

### Requirement: Program status indicator
A tab SHALL show a status dot while its terminal, or any pane of its tmux control-mode window, holds a program status record (see the `program-status` capability) other than `idle`. The dot's colour SHALL follow the most pressing state across those records, in the order `blocked` (the theme's yellow, ANSI 3), `error` (red, ANSI 1), `done` (green, ANSI 2), `working` (the muted foreground). It SHALL sit beside the bell dot when both apply.

#### Scenario: Blocked in a background tab
- **WHEN** a program in a background tab reports `state=blocked:kind=permission`
- **THEN** that tab shows a yellow dot

#### Scenario: Pane in a tmux window
- **WHEN** a program in one pane of a tmux window reports `state=error`, and another pane reports `state=working`
- **THEN** that window's tab shows a red dot

### Requirement: Status indicator lifetime
The status dot SHALL stay until the terminal's records change (a report, a clear, a new prompt, the process exiting, or the user typing into the terminal), whether or not the tab is active. Activating the tab SHALL NOT clear it.

#### Scenario: Done survives the prompt
- **WHEN** a program in the active tab reports `state=done`, exits, and the shell prints its prompt
- **THEN** the tab shows a green dot until the user types a key into that tab

#### Scenario: Activated while blocked
- **WHEN** a background tab shows the yellow dot and the user activates it
- **THEN** the dot remains until the program reports again
