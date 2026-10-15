# Spec Delta

## ADDED Requirements

### Requirement: No subscriptions on tmux with the subscription crash
On tmux older than 3.7, or when the version tmux reports can't be read, Brindle SHALL NOT register control-mode format subscriptions. It SHALL instead ask tmux for each pane's geometry, title line and pane styles, and each window's border status and border styles, about once a second while attached, so these keep updating as they do with subscriptions. tmux 3.7 and newer SHALL be followed with subscriptions.

#### Scenario: Session ended from inside tmux
- **WHEN** Brindle is attached to a session on tmux 3.6, another session exists on the same server, and `exit` is run in the attached session's last shell, repeated ten times
- **THEN** the tmux server keeps running and the other session still exists

#### Scenario: No subscription commands
- **WHEN** Brindle attaches to a session on tmux 3.6, then detaches
- **THEN** it sends no `refresh-client -B` command at any point

#### Scenario: Title changes on old tmux
- **WHEN** Brindle is attached on tmux 3.6, the window has `pane-border-status top`, and a program renames its pane with `select-pane -T`
- **THEN** the title line shows the new name within about a second

#### Scenario: Border status turned on while attached on old tmux
- **WHEN** Brindle is attached on tmux 3.6 and `set -w pane-border-status top` is run in the attached window
- **THEN** within about a second each pane is one row shorter, matches `capture-pane`, and shows its title line

#### Scenario: Newer tmux
- **WHEN** Brindle attaches to a session on tmux 3.7 or newer
- **THEN** it registers its subscriptions as before and does not ask for these values once a second

#### Scenario: Unreadable version
- **WHEN** tmux's reported version is not of the form `<major>.<minor>` with an optional letter or `next-` prefix (for example a distribution's own version string)
- **THEN** Brindle behaves as on tmux 3.6

## MODIFIED Requirements

### Requirement: Subscriptions are removed before leaving a session
When Brindle uses control-mode format subscriptions, before it detaches from a session, quits, or ends the session itself by closing its last window or killing its last pane, it SHALL remove the subscriptions it registered. tmux up to 3.6 can crash when a control client with subscriptions leaves or loses its session.

#### Scenario: Detach
- **WHEN** the user detaches a control-mode session that uses subscriptions with ctrl-shift-d
- **THEN** Brindle sends `refresh-client -B <name>` for each of its subscriptions before `detach-client`, and the tmux server keeps running

#### Scenario: Quit
- **WHEN** Brindle quits while attached to a control-mode session that uses subscriptions
- **THEN** it removes its subscriptions before its control connection closes, and the tmux server keeps running

#### Scenario: Closing the session's last window
- **WHEN** the user closes the tab of the only window of a session that uses subscriptions
- **THEN** Brindle removes its subscriptions before sending `kill-window`
