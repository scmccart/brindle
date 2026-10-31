# session-restore Specification

## Purpose
Bring back the windows and tabs a user had open when Brindle last quit, so a logout, reboot or crash doesn't mean rebuilding the same tabs, directories and tmux attachments by hand. Restoring is opt-in.

## Requirements

### Requirement: Restore is opt-in
Session recording and restoring SHALL happen only when the config sets `restore_session = true`. With the setting off (the default), Brindle SHALL NOT read or write a session state file, and every launch SHALL start as it does without this feature.

#### Scenario: Default config
- **WHEN** the config does not mention `restore_session` and the user quits Brindle with three tabs open
- **THEN** no state file is written, and the next launch opens a single tab with the default profile

#### Scenario: Turned on while running
- **WHEN** the user sets `restore_session = true` and saves the config while Brindle is running
- **THEN** within a few seconds Brindle starts recording its windows and tabs, without a restart

### Requirement: What a launch restores
On a plain launch (see `command-line`) with the setting on and a usable state file, Brindle SHALL open one window for each recorded window, in order, instead of the usual single tab. Each window SHALL get:
- its recorded size, maximized if it was maximized;
- its recorded tabs, in order;
- the recorded active tab, as the active tab.

#### Scenario: Two tabs come back
- **WHEN** Brindle quit with two local tabs, the second active, and the user launches `brindle` again
- **THEN** one window opens with two tabs, and the second tab is active

#### Scenario: Two windows come back
- **WHEN** Brindle quit with two windows open
- **THEN** the next plain launch opens two windows, each with its own recorded tabs

### Requirement: Local tabs restore their profile and directory
A restored local tab SHALL run its recorded profile's program, starting in the recorded working directory. The recorded directory SHALL be the working directory of the tab's foreground process, the same directory a new tab would inherit. Classic tmux tabs SHALL rerun their profile, which reattaches to their session. When a recorded local tab has a manual title, that title SHALL be restored with the tab.

#### Scenario: Directory comes back
- **WHEN** a tab's shell was in `/srv/www` when Brindle quit
- **THEN** after a plain launch that tab's shell starts in `/srv/www`

#### Scenario: Classic tmux tab
- **WHEN** a classic tmux tab for session `main` was open when Brindle quit
- **THEN** after a plain launch a tab runs `tmux new-session -A -s main` and shows that session

#### Scenario: Manual title comes back
- **WHEN** a local tab has manual title `server` when Brindle quits
- **THEN** after a plain launch that tab is restored with title `server`

### Requirement: Commands and messages are not replayed
A tab that was opened with a command (`-e`, or the config editor tab) SHALL come back as its profile's program in the recorded directory, without the command. Tabs that only show a message, such as an attach failure, SHALL NOT be recorded.

#### Scenario: Command tab
- **WHEN** Brindle was started with `-e htop` in `~/src` and then quit
- **THEN** after a plain launch the tab runs the default profile's shell in `~/src`, not htop

#### Scenario: Attach-failure tab
- **WHEN** the only other tab showed a tmux attach failure when Brindle quit
- **THEN** that tab is not restored

### Requirement: Control-mode sessions are reattached once
For tmux control-mode tabs, Brindle SHALL record the session (profile and current session name), once per window, at the position of the session's first tab. Restoring SHALL attach each recorded session once across all restored windows: when a state file records the same session in more than one window, only the first of those windows SHALL attach it, and a window left with nothing to restore SHALL NOT be opened. The session's windows SHALL then arrive as tabs at the position the session had among the other tabs, in tmux's window order. Restoring SHALL NOT select any tmux window or pane: the requirements of `tmux-control-mode` for attaching apply unchanged. Brindle-local manual titles on tmux control-mode tabs SHALL NOT be recorded or restored.

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

#### Scenario: Brindle-local tmux title is not restored
- **WHEN** a tmux control-mode tab has Brindle-local manual title `review` and tmux window name `editor` when Brindle quits
- **THEN** after a plain launch that tab is titled from tmux state, not `review`

### Requirement: Restoring falls back when things have changed
Restoring SHALL succeed even when the recorded state no longer matches the system:
- a missing profile SHALL be replaced by the default profile, with a log line;
- a missing directory SHALL be replaced by the profile's own directory, else the home directory;
- a missing control-mode session SHALL be created again by the attach, as opening the profile does. If tmux can't attach, the attach-failure tab of `tmux-control-mode` SHALL be shown.

#### Scenario: Profile removed from the config
- **WHEN** a recorded tab's profile `Work` has been removed from the config
- **THEN** the tab is restored with the default profile in its recorded directory

#### Scenario: Directory deleted
- **WHEN** a recorded tab's directory has been deleted since Brindle quit
- **THEN** the tab is restored in the home directory (for a profile without its own directory)

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

### Requirement: Closing a window forgets it
A window that closes because its last tab was closed or exited SHALL be removed from the recorded state. A window that the user closes while other windows remain SHALL also be removed. When the application quits, including by closing the last window while it still has tabs, the windows open at that moment SHALL be kept.

#### Scenario: Last tab exits
- **WHEN** the only window's only tab runs `exit`, so Brindle quits
- **THEN** the next plain launch opens a single default-profile tab

#### Scenario: Window closed with its tabs
- **WHEN** the user closes the only window with its close button while it has three tabs
- **THEN** the next plain launch restores the three tabs

### Requirement: One process owns the state
Only one Brindle process at a time SHALL record and restore the session. A process started while another one owns the state SHALL neither restore nor write the state file, and SHALL log that it doesn't.

#### Scenario: Second instance
- **WHEN** Brindle is running with restore on and the user launches `brindle` again
- **THEN** the new process opens a single default-profile tab, and the first process's tabs are not duplicated
