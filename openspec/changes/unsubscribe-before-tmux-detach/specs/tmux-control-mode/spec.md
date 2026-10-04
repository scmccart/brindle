# Spec Delta

## ADDED Requirements

### Requirement: Subscriptions are removed before leaving a session
Before Brindle detaches from a session, quits, or ends the session itself by closing its last window or killing its last pane, it SHALL remove the control-mode format subscriptions it registered. tmux up to 3.6 can crash when a control client with subscriptions leaves or loses its session.

#### Scenario: Detach
- **WHEN** the user detaches a control-mode session with ctrl-shift-d
- **THEN** Brindle sends `refresh-client -B <name>` for each of its subscriptions before `detach-client`, and the tmux server keeps running

#### Scenario: Quit
- **WHEN** Brindle quits while attached to a control-mode session
- **THEN** it removes its subscriptions before its control connection closes, and the tmux server keeps running

#### Scenario: Closing the session's last window
- **WHEN** the user closes the tab of a session's only window
- **THEN** Brindle removes its subscriptions before sending `kill-window`
