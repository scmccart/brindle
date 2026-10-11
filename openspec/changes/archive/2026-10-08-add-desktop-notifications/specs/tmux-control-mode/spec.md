# Spec Delta

## ADDED Requirements

### Requirement: Bells and notifications from panes
Bells, OSC 9 and OSC 777 from a pane SHALL be handled as they are for a local tab, with the pane's tab as their tab, for panes in every window of the session, whether or not that window's tab is active. tmux sends no separate bell notification to control clients; these arrive in the pane's output.

#### Scenario: Notification from a background window
- **WHEN** a pane in a tmux window whose tab is not active prints `ESC ] 777 ; notify ; Build ; done BEL`
- **THEN** a desktop notification titled `Build — <window name>` with body `done` appears, and clicking it activates that window's tab
