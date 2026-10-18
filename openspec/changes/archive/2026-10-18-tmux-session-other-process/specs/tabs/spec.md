# Spec Delta

## MODIFIED Requirements

### Requirement: Multiple windows
The user SHALL be able to open additional windows (ctrl-shift-n), each with its own tabs, and quit everything with ctrl-shift-q. A new window SHALL open with one tab using the default profile. When the default profile is a control-mode profile whose session is already shown in any window of this Brindle, ctrl-shift-n SHALL NOT open a window directly; it SHALL ask what to do, as defined by "Choosing when a session is open elsewhere" in `tmux-control-mode`. When another Brindle shows it instead, the new window SHALL open and ask there, as defined by "Session shown by another Brindle".

#### Scenario: New window
- **WHEN** the user presses ctrl-shift-n
- **THEN** a second window opens with one tab using the default profile

#### Scenario: Default session already open
- **WHEN** the default profile is the control-mode profile for session `main`, a window shows `main`, and the user presses ctrl-shift-n
- **THEN** no window opens yet, the current window offers "Go to main", "New tmux session…" and "New window: <profile>" rows, and `tmux list-clients` still shows one Brindle client for `main`

#### Scenario: Default session shown by another Brindle
- **WHEN** the default profile is the control-mode profile for session `main`, another Brindle shows `main`, and the user presses ctrl-shift-n
- **THEN** a new window opens with no tabs and asks about `main`, and the current window stays as it was
