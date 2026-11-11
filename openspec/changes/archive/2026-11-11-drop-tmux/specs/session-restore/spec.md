## ADDED Requirements

### Requirement: Local tabs restore their profile, zones and directory
A restored local tab SHALL restore its recorded zone split tree, with each zone running the tab's recorded profile in that zone's recorded working directory, and focus the tab's recorded active zone. Each zone's recorded directory SHALL be the working directory of that zone's foreground process, the same directory a new tab or split would inherit. A recorded local tab without a split tree SHALL restore as a single zone in the tab's recorded working directory. When a recorded local tab has a manual title, that title SHALL be restored with the tab. A zone with a recorded terminal server session SHALL instead be attached to that session, or not restored, as defined by "Restored zones get their sessions back".

#### Scenario: Directory comes back
- **WHEN** a zone without a session record (on Windows, or in a state file from an earlier version) was in `/srv/www` when Brindle quit
- **THEN** after a launch that zone's new shell starts in `/srv/www`

#### Scenario: Split zone tree comes back
- **WHEN** a local tab was split right, with the left zone in `/srv/api` and the focused right zone in `/srv/web`, when Brindle quit
- **THEN** after a launch the tab opens with two side-by-side zones in `/srv/api` and `/srv/web`, and the right zone is focused

#### Scenario: Manual title comes back
- **WHEN** a local tab has manual title `server` when Brindle quits
- **THEN** after a launch that tab is restored with title `server`

### Requirement: Commands and messages are not restored
A tab that was opened with a command (`-e`, or the config editor tab) SHALL come back as its profile's program in the recorded directory, without the command. Tabs that only show a message, such as a failed connection, SHALL NOT be recorded.

#### Scenario: Command tab
- **WHEN** Brindle was started with `-e htop` in `~/src` and then quit
- **THEN** after a plain launch the tab runs the default profile's shell in `~/src`, not htop

#### Scenario: Message tab
- **WHEN** the only other tab showed a connection failure message when Brindle quit
- **THEN** that tab is not restored

### Requirement: Restoring falls back to defaults
Restoring SHALL succeed even when the recorded state no longer matches the system:
- a missing profile SHALL be replaced by the default profile, with a log line;
- a missing directory for a tab or zone SHALL be replaced by the profile's own directory, else the home directory.

#### Scenario: Profile removed from the config
- **WHEN** a recorded tab's profile `Work` has been removed from the config
- **THEN** the tab is restored with the default profile in its recorded directory

#### Scenario: Directory deleted
- **WHEN** a recorded tab's or zone's directory has been deleted since Brindle quit
- **THEN** that terminal is restored in the home directory (for a profile without its own directory)

### Requirement: Old tmux records are skipped
A state file written by a version that had tmux support MAY contain tmux entries (`kind = "tmux"`). Restoring SHALL skip them without an error, SHALL restore the other entries as usual, and SHALL NOT write such entries again.

#### Scenario: State file with a tmux entry
- **WHEN** the state file records a local tab, a `kind = "tmux"` entry for session `work`, and another local tab
- **THEN** after a launch the window shows the two local tabs in order, nothing attaches to tmux, and the next saved state has no tmux entry

#### Scenario: Only tmux entries
- **WHEN** the state file records only `kind = "tmux"` entries
- **THEN** after a launch the window opens with the default profile's tab, as it does with no state file

## MODIFIED Requirements

### Requirement: Restored zones get their sessions back
Each recorded local Linux zone SHALL record which terminal server session it showed. On a restoring launch, a zone whose recorded session is still running and detached SHALL be attached to it, showing its screen and scrollback, without bells or notifications from the replayed output. A zone whose recorded session is gone, attached elsewhere, or not the recorded one (the server was restarted in between and reused its id) SHALL NOT be restored: its split's sibling takes its place, a tab with no zone left SHALL NOT be restored, and a launch left with nothing to restore SHALL open one default-profile tab. A recorded zone without a session record SHALL restore as "Local tabs restore their profile, zones and directory" says.

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

## REMOVED Requirements

### Requirement: Local tabs restore their profile and directory
**Reason**: Rewritten without classic tmux tabs and their scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Local tabs restore their profile, zones and directory".

### Requirement: Commands and messages are not replayed
**Reason**: Rewritten without the tmux attach-failure scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Commands and messages are not restored".

### Requirement: Restoring falls back when things have changed
**Reason**: Rewritten without the control-mode session fallback and its scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Restoring falls back to defaults".

### Requirement: Control-mode sessions are reattached once
**Reason**: Brindle has no tmux control mode, so there are no control-mode sessions to record or reattach.
**Migration**: Old entries are skipped by "Old tmux records are skipped". A tmux running in an ordinary tab is a program in a terminal server session, restored like any other.
