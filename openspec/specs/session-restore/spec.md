# session-restore Specification

## Purpose
Bring back the windows and tabs a user had open when Brindle last quit, so a logout, reboot or crash doesn't mean rebuilding the same tabs, directories and tmux attachments by hand. Restoring is opt-in.

## Requirements

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

### Requirement: Commands and messages are not replayed
A tab that was opened with a command (`-e`, or the config editor tab) SHALL come back as its profile's program in the recorded directory, without the command. Tabs that only show a message, such as an attach failure, SHALL NOT be recorded.

#### Scenario: Command tab
- **WHEN** Brindle was started with `-e htop` in `~/src` and then quit
- **THEN** after a plain launch the tab runs the default profile's shell in `~/src`, not htop

#### Scenario: Attach-failure tab
- **WHEN** the only other tab showed a tmux attach failure when Brindle quit
- **THEN** that tab is not restored

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

### Requirement: Restoring falls back when things have changed
Restoring SHALL succeed even when the recorded state no longer matches the system:
- a missing profile SHALL be replaced by the default profile, with a log line;
- a missing directory for a tab or zone SHALL be replaced by the profile's own directory, else the home directory;
- a missing control-mode session SHALL be created again by the attach, as opening the profile does. If tmux can't attach, the attach-failure tab of `tmux-control-mode` SHALL be shown.

#### Scenario: Profile removed from the config
- **WHEN** a recorded tab's profile `Work` has been removed from the config
- **THEN** the tab is restored with the default profile in its recorded directory

#### Scenario: Directory deleted
- **WHEN** a recorded tab's or zone's directory has been deleted since Brindle quit
- **THEN** that terminal is restored in the home directory (for a profile without its own directory)

#### Scenario: Session killed
- **WHEN** a recorded control-mode session was killed after Brindle quit
- **THEN** after a plain launch a new session with the same name is created and attached

### Requirement: Bad state files are ignored
A state file that cannot be read, is not valid, or has a format version Brindle doesn't know SHALL be ignored with a log line naming the file and the problem, and the launch SHALL proceed as if no file existed. A state file that records no windows SHALL also lead to a normal launch.

#### Scenario: Corrupt file
- **WHEN** the state file contains invalid data and the user launches `brindle`
- **THEN** a log line reports the problem, and one window opens with a single default-profile tab

#### Scenario: Newer format
- **WHEN** the state file declares a format version newer than this Brindle understands
- **THEN** it is ignored with a log line, and a normal launch happens

### Requirement: State file location and writes
The state SHALL be stored in `session.toml` in `$BRINDLE_STATE_DIR` when that is set. Otherwise it SHALL be stored in `$XDG_STATE_HOME/brindle/` on Linux, which defaults to `~/.local/state/brindle/`, and in `%LOCALAPPDATA%\brindle\` on Windows. Every write SHALL replace the file in one step, so an interrupted write never leaves a partial file. The file SHALL be written within a few seconds of a change to the recorded windows, tabs or directories, and again when Brindle quits.

#### Scenario: Crash after a change
- **WHEN** the user opens a tab, waits a few seconds, and Brindle is then killed
- **THEN** the next plain launch restores the new tab

#### Scenario: Override for tests
- **WHEN** `BRINDLE_STATE_DIR=/tmp/s` is set
- **THEN** the state is read from and written to `/tmp/s/session.toml`, and nothing under `~/.local/state` is touched

#### Scenario: State on Windows
- **WHEN** Brindle runs on Windows with restore on and `BRINDLE_STATE_DIR` unset
- **THEN** the state is written to `%LOCALAPPDATA%\brindle\session.toml`, and nothing under `%USERPROFILE%\.local` is created

#### Scenario: Directory comes back on Windows
- **WHEN** a Windows tab's PowerShell, with Brindle's integration script loaded, was in `C:\src` when Brindle quit
- **THEN** after a plain launch that tab's shell starts in `C:\src`

### Requirement: One process owns the state
Only one Brindle process at a time SHALL own the session state: the instance, which records and restores it and reattaches detached sessions. A launch while an instance runs SHALL hand its request to the instance, as defined by "A launch goes to the running Brindle" in `command-line`, and exit. A launch with debug options (`--send`, `--action`, `--dump-screen-after`, `--latency-log`, `--latency-bench`) while an instance runs SHALL instead run on its own, and SHALL neither restore, reattach, nor write the state file, and SHALL log that it doesn't.

#### Scenario: Second instance
- **WHEN** Brindle is running with three tabs and the user launches `brindle` again
- **THEN** no second window opens, the running window is raised, and its tabs are not duplicated

#### Scenario: Debug run beside the instance
- **WHEN** Brindle is running and a test runs `brindle --dump-screen-after 3`
- **THEN** a separate window opens with a single default-profile tab and exits after the dump, and the running Brindle's state file is unchanged

### Requirement: Remote endpoint identity
A recorded local tab backed by a remote profile SHALL store the SSH or container target identity used when its CWD and zone directories were observed. Restore SHALL reuse those directories only when the current profile resolves to the same endpoint. State written before endpoint identity was recorded, a missing/renamed profile, or a profile whose endpoint changed SHALL discard the recorded directories and use the current profile's remote directory default.

#### Scenario: SSH profile changes host
- **WHEN** a tab recorded `/srv/old` for profile `Remote` on SSH host `old-host`, and the profile now points to `new-host` with `cwd = "/home/new"`
- **THEN** restore starts on `new-host` in `/home/new` and never sends `/srv/old`

#### Scenario: Existing local state
- **WHEN** an older state file has no endpoint identity and resolves to a local profile
- **THEN** its local directory is restored under the existing local-directory fallback rules

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
