# Spec Delta

## MODIFIED Requirements

### Requirement: Multiple windows
The user SHALL be able to open additional windows (ctrl-shift-n), each with its own tabs, and quit everything with ctrl-shift-q. A new window SHALL open with one tab using the default profile. When the default profile is a control-mode profile whose session is already shown in any window, ctrl-shift-n SHALL NOT open a window directly; it SHALL ask what to do, as defined by "Choosing when a session is open elsewhere" in `tmux-control-mode`.

#### Scenario: New window
- **WHEN** the user presses ctrl-shift-n
- **THEN** a second window opens with one tab using the default profile

#### Scenario: Default session already open
- **WHEN** the default profile is the control-mode profile for session `main`, a window shows `main`, and the user presses ctrl-shift-n
- **THEN** no window opens yet, the current window offers "Go to main", "New tmux session…" and "New window: <profile>" rows, and `tmux list-clients` still shows one Brindle client for `main`

## ADDED Requirements

### Requirement: New tab with the default profile does not move between windows
ctrl-shift-t (or the `+` button) in a tab that is not a control-mode tab SHALL open the default profile. When the default profile is a control-mode profile whose session is shown in the current window, its tab SHALL become active. When that session is shown in another window, Brindle SHALL ask what to do, as defined by "Choosing when a session is open elsewhere" in `tmux-control-mode`, and SHALL NOT switch windows unless the user chooses to.

#### Scenario: Session in another window
- **WHEN** the default profile's session `main` is shown in window A, and the user presses ctrl-shift-t in a local tab of window B
- **THEN** window B asks what to do, offering "New tab: <profile>" rows, and window A is not raised

#### Scenario: Session in this window
- **WHEN** the current window shows session `main` and a local tab is active, and the user presses ctrl-shift-t
- **THEN** a tab of `main` becomes active and no tab is added
