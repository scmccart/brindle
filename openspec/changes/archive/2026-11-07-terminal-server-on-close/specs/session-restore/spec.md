## REMOVED Requirements

### Requirement: Restore is opt-in
**Reason**: Terminals now outlive Brindle, and reattaching them without the recorded layout flattens split tabs. Restore is always on.
**Migration**: Remove `restore_session` from the config; it is ignored with a warning. Use `--no-restore` to skip restore for one launch.

### Requirement: Closing a window forgets it
**Reason**: Brindle has one window, and closing it is quitting (see `tabs`), which keeps the state. Replaced by "Quitting keeps the state".
**Migration**: None.

## ADDED Requirements

### Requirement: Restore is always on
Brindle SHALL always record its window and tabs and restore them when it starts as the instance (see "One process owns the state"). No config setting SHALL turn this off; `--no-restore` (see `command-line`) skips it for one launch, and `BRINDLE_STATE_DIR` points it elsewhere.

#### Scenario: Default config
- **WHEN** the config is empty and the user quits Brindle with three tabs, two of them split
- **THEN** the next launch opens the window with the same three tabs and splits, each zone showing its program as it was

#### Scenario: Skipped once
- **WHEN** the user quits with three tabs and runs `brindle --no-restore`
- **THEN** the window opens with a single default-profile tab, and the three tabs' sessions stay detached

### Requirement: Quitting keeps the state
When Brindle quits, including by closing its window, the window and tabs open at that moment SHALL be kept. When the last tab closes or exits, so the window closes with no tabs, the recorded state SHALL be empty.

#### Scenario: Window closed with its tabs
- **WHEN** the user closes the window with its close button while it has three tabs
- **THEN** the next launch restores the three tabs

#### Scenario: Last tab exits
- **WHEN** the only tab runs `exit`, so Brindle quits
- **THEN** the next launch opens a single default-profile tab

### Requirement: Restored zones get their sessions back
Each recorded local Linux zone SHALL record which terminal server session it showed. On a restoring launch, a zone whose recorded session is still running and detached SHALL be attached to it, showing its screen and scrollback, without bells or notifications from the replayed output. A zone whose recorded session is gone, attached elsewhere, or not the recorded one (the server was restarted in between and reused its id) SHALL NOT be restored: its split's sibling takes its place, a tab with no zone left SHALL NOT be restored, and a launch left with nothing to restore SHALL open one default-profile tab. A recorded zone without a session record SHALL restore as "Local tabs restore their profile and directory" says.

#### Scenario: Programs survive a quit
- **WHEN** a tab runs `htop`, and the user quits and relaunches
- **THEN** the restored tab shows the same `htop` process, with the same pid

#### Scenario: Programs survive a crash
- **WHEN** Brindle is killed with SIGKILL while a split tab's zones run `vim` and a shell with scrollback
- **THEN** the next launch restores the split with `vim` running and the shell's scrollback intact

#### Scenario: Session gone from a split
- **WHEN** one zone of a two-zone tab had its program ended with `brindle-remote kill` before the relaunch
- **THEN** the tab is restored with only the other zone, reattached

#### Scenario: Every session gone
- **WHEN** Brindle quit with two local tabs and `brindle-remote stop` ended their sessions before the relaunch
- **THEN** the next launch opens one default-profile tab, and neither recorded tab comes back

#### Scenario: Reused id
- **WHEN** the server stopped after Brindle quit, and a new server gave the recorded id to a different session
- **THEN** the zone is not restored, and that other session opens as its own tab

### Requirement: Launch reattaches detached terminals
On every restoring launch, Brindle SHALL attach every detached local terminal server session it doesn't restore into a zone, each as a new tab, in the order the sessions were started, with the profile its session was started with (the default profile when that profile is gone). The active tab SHALL stay the one the launch would otherwise make active. Replayed output SHALL raise no bells or notifications.

#### Scenario: Session left by a skipped launch
- **WHEN** a session is detached because the last launch was `--no-restore`, and the user quits and launches `brindle`
- **THEN** the window shows the recorded tabs, the recorded active tab still active, followed by a tab showing that session

#### Scenario: No bell from the replay
- **WHEN** a detached session's output contained a bell before the launch
- **THEN** reattaching it marks no bell on its tab and sends no notification

## MODIFIED Requirements

### Requirement: What a launch restores
On a restoring launch with a usable state file, Brindle SHALL open its window with:
- the recorded window's size, maximized if it was maximized;
- the recorded tabs, in order;
- the recorded active tab, as the active tab.

A state file that records several windows (written by earlier versions) SHALL restore as one window: the tabs of every recorded window in order, with the first window's size and active tab.

#### Scenario: Two tabs come back
- **WHEN** Brindle quit with two local tabs, the second active, and the user launches `brindle` again
- **THEN** the window opens with two tabs, and the second tab is active

#### Scenario: Two windows come back
- **WHEN** the state file records two windows with two tabs each
- **THEN** the next launch opens one window with the four tabs, in order, and the first window's active tab is active

### Requirement: Local tabs restore their profile and directory
A restored local tab SHALL restore its recorded zone split tree, with each zone running the tab's recorded profile in that zone's recorded working directory, and focus the tab's recorded active zone. Each zone's recorded directory SHALL be the working directory of that zone's foreground process, the same directory a new tab or split would inherit. A recorded local tab without a split tree SHALL restore as a single zone in the tab's recorded working directory. Classic tmux tabs SHALL rerun their profile, which reattaches to their session. When a recorded local tab has a manual title, that title SHALL be restored with the tab. A zone with a recorded terminal server session SHALL instead be attached to that session, or not restored, as defined by "Restored zones get their sessions back".

#### Scenario: Directory comes back
- **WHEN** a zone without a session record (on Windows, or in a state file from an earlier version) was in `/srv/www` when Brindle quit
- **THEN** after a launch that zone's new shell starts in `/srv/www`

#### Scenario: Split zone tree comes back
- **WHEN** a local tab was split right, with the left zone in `/srv/api` and the focused right zone in `/srv/web`, when Brindle quit
- **THEN** after a launch the tab opens with two side-by-side zones in `/srv/api` and `/srv/web`, and the right zone is focused

#### Scenario: Classic tmux tab
- **WHEN** a classic tmux tab for session `main` was open when Brindle quit
- **THEN** after a launch a tab runs `tmux new-session -A -s main` and shows that session

#### Scenario: Manual title comes back
- **WHEN** a local tab has manual title `server` when Brindle quits
- **THEN** after a launch that tab is restored with title `server`

### Requirement: Control-mode sessions are reattached once
For tmux control-mode tabs, Brindle SHALL record the session (profile and current session name) at the position of the session's first tab. Restoring SHALL attach each recorded session once: when a state file records the same session more than once (in several windows written by earlier versions), only the first SHALL be attached. The session's windows SHALL then arrive as tabs at the position the session had among the other tabs, in tmux's window order. Restoring SHALL NOT select any tmux window or pane: the requirements of `tmux-control-mode` for attaching apply unchanged. Brindle-local manual titles on tmux control-mode tabs SHALL NOT be recorded or restored.

#### Scenario: Session with three windows
- **WHEN** Brindle quit showing a local tab followed by the three windows of control-mode session `work`
- **THEN** after a launch the session is attached once, and the window shows the local tab followed by three tabs for `work`'s windows

#### Scenario: tmux's current window is kept
- **WHEN** the recorded active tab was a tab of session `work`, and another client has since made a different window of `work` current
- **THEN** after a launch the tab for tmux's current window is active, and tmux's current window is unchanged

#### Scenario: Local tab was active
- **WHEN** the recorded active tab was a local tab, and the window also had control-mode tabs
- **THEN** after the session attaches, the local tab is still the active tab

#### Scenario: Same session recorded in two windows
- **WHEN** the state file records two windows that both show session `main`, the second also with a local tab
- **THEN** after a launch the window shows `main` once, followed by the local tab

#### Scenario: Window with only a duplicate session
- **WHEN** the state file records two windows that each show only session `main`
- **THEN** after a launch the window shows `main` once

#### Scenario: Brindle-local tmux title is not restored
- **WHEN** a tmux control-mode tab has Brindle-local manual title `review` and tmux window name `editor` when Brindle quits
- **THEN** after a launch that tab is titled from tmux state, not `review`

### Requirement: One process owns the state
Only one Brindle process at a time SHALL own the session state: the instance, which records and restores it and reattaches detached sessions. A launch while an instance runs SHALL hand its request to the instance, as defined by "A launch goes to the running Brindle" in `command-line`, and exit. A launch with debug options (`--send`, `--action`, `--dump-screen-after`, `--latency-log`, `--latency-bench`) while an instance runs SHALL instead run on its own, and SHALL neither restore, reattach, nor write the state file, and SHALL log that it doesn't.

#### Scenario: Second instance
- **WHEN** Brindle is running with three tabs and the user launches `brindle` again
- **THEN** no second window opens, the running window is raised, and its tabs are not duplicated

#### Scenario: Debug run beside the instance
- **WHEN** Brindle is running and a test runs `brindle --dump-screen-after 3`
- **THEN** a separate window opens with a single default-profile tab and exits after the dump, and the running Brindle's state file is unchanged
