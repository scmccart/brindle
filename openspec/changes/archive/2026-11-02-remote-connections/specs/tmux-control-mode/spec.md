## ADDED Requirements

### Requirement: Remote control client
A control-mode profile with a remote target SHALL run `tmux -u -C` on that target while preserving the existing native tab, pane, input, resize, restore, style, notification, status, palette-query, subscription/polling, detach, and session-lifecycle behavior. Remote target identity SHALL be part of session identity, so identically named servers and sessions on different hosts or containers do not collide.

#### Scenario: Same session name on two hosts
- **WHEN** SSH profiles for hosts `build-a` and `build-b` both attach to tmux session `main`
- **THEN** Brindle treats them as two independent sessions and may show both

#### Scenario: Container tmux windows
- **WHEN** a Docker remote profile with `tmux = "control"` attaches to a session containing two windows
- **THEN** both remote windows appear as native Brindle tabs and their panes accept input and resize as local control-mode panes do
