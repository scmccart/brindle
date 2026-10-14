# Spec Delta

## MODIFIED Requirements

### Requirement: Control-mode sessions are reattached once
For tmux control-mode tabs, Brindle SHALL record the session (profile and current session name), once per window, at the position of the session's first tab. Restoring SHALL attach each recorded session once across all restored windows: when a state file records the same session in more than one window, only the first of those windows SHALL attach it, and a window left with nothing to restore SHALL NOT be opened. The session's windows SHALL then arrive as tabs at the position the session had among the other tabs, in tmux's window order. Restoring SHALL NOT select any tmux window or pane: the requirements of `tmux-control-mode` for attaching apply unchanged.

#### Scenario: Session with three windows
- **WHEN** Brindle quit showing a local tab followed by the three windows of control-mode session `work`
- **THEN** after a plain launch the session is attached once, and the window shows the local tab followed by three tabs for `work`'s windows

#### Scenario: tmux's current window is kept
- **WHEN** the recorded active tab was a tab of session `work`, and another client has since made a different window of `work` current
- **THEN** after a plain launch the tab for tmux's current window is active, and tmux's current window is unchanged

#### Scenario: Local tab was active
- **WHEN** the recorded active tab was a local tab, and the window also had control-mode tabs
- **THEN** after the session attaches, the local tab is still the active tab

#### Scenario: Same session recorded in two windows
- **WHEN** the state file records two windows that both show session `main`, the second also with a local tab
- **THEN** after a plain launch two windows open, only the first shows `main`, and the second shows only the local tab

#### Scenario: Window with only a duplicate session
- **WHEN** the state file records two windows that each show only session `main`
- **THEN** after a plain launch one window opens, showing `main`
