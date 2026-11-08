## MODIFIED Requirements

### Requirement: Window closes with its last tab
Closing the last tab SHALL close the window, which quits Brindle. Closing the window, by its close button, the window manager or `Close Window`, SHALL quit Brindle, as defined by "Quitting keeps programs running".

#### Scenario: Last tab closed
- **WHEN** the window's only tab is closed
- **THEN** the window closes and Brindle exits

#### Scenario: Window closed with tabs
- **WHEN** the window has two local tabs running `sleep 1000` and the user clicks its close button
- **THEN** Brindle exits without asking, and both sessions are listed as detached

## REMOVED Requirements

### Requirement: Multiple windows
**Reason**: Brindle has one window (see "One window"), so the session it keeps and restores has one place.
**Migration**: Use tabs and zones. ctrl-shift-n no longer opens anything; a user binding to `new_window` is a config error naming the unknown action.

### Requirement: New tab with the default profile does not move between windows
**Reason**: There is no other window to move to; replaced by "New tab with the default profile".
**Migration**: None.

## ADDED Requirements

### Requirement: New tab with the default profile
ctrl-shift-t (or the `+` button) in a tab that is not a control-mode tab SHALL open the default profile. When the default profile is a control-mode profile whose session the window shows, its tab SHALL become active instead.

#### Scenario: Session in the window
- **WHEN** the window shows session `main` and a local tab is active, and the user presses ctrl-shift-t
- **THEN** a tab of `main` becomes active and no tab is added

### Requirement: One window
Brindle SHALL show a single window. No command SHALL open another window; opening a profile, a tmux session or an attached session SHALL add a tab to that window.

#### Scenario: No new window
- **WHEN** the user presses ctrl-shift-n
- **THEN** no window opens

#### Scenario: Not in the command palette
- **WHEN** the user opens the command palette and types `window`
- **THEN** "Close Window" is listed and "New Window" is not

### Requirement: Quitting keeps programs running
Quitting Brindle (`Quit`, ctrl-shift-q, closing the window) SHALL leave every open local Linux terminal's program running detached, so the next launch brings it back (see `session-restore`), except `-e` command terminals, which end. Brindle SHALL NOT ask. When Brindle ends without quitting (a crash, a signal, the desktop stopping it), the same terminals SHALL keep running.

#### Scenario: Quit
- **WHEN** the window holds three local tabs and the user presses ctrl-shift-q
- **THEN** Brindle exits without asking, and the three sessions are listed as detached with their pids unchanged

#### Scenario: Crash
- **WHEN** Brindle is killed with SIGKILL while a tab runs `sleep 1000`
- **THEN** the `sleep` keeps running, detached

#### Scenario: Command tab
- **WHEN** a tab runs `brindle -e sleep 1000`'s command and the user quits
- **THEN** that `sleep` is gone within a few seconds and no session is listed for it
