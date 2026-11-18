# Spec Delta

## MODIFIED Requirements

### Requirement: Restoring falls back to defaults
Restoring SHALL succeed even when the recorded state no longer matches the system:
- a missing profile SHALL be replaced by the default profile, and the restored local tab SHALL say, in one dim line, that its recorded profile is gone and which profile it uses, after any replay of its screen;
- a missing directory for a tab or zone SHALL be replaced by the profile's own directory, else the home directory.

#### Scenario: Profile removed from the config
- **WHEN** a recorded tab's profile `Work` has been removed from the config
- **THEN** the tab is restored with the default profile in its recorded directory, and says that profile `Work` is gone and the default profile is used

#### Scenario: Directory deleted
- **WHEN** a recorded tab's or zone's directory has been deleted since Brindle quit
- **THEN** that terminal is restored in the home directory (for a profile without its own directory)

### Requirement: State file location and writes
The state SHALL be stored in `session.toml` in `$BRINDLE_STATE_DIR` when that is set. Otherwise it SHALL be stored in `$XDG_STATE_HOME/brindle/` on Linux, which defaults to `~/.local/state/brindle/`, and in `%LOCALAPPDATA%\brindle\` on Windows. When none of these can be found, Brindle SHALL run on its own, neither restoring, reattaching nor writing state, and the window SHALL say why. Every write SHALL replace the file in one step, so an interrupted write never leaves a partial file. The file SHALL be written within a few seconds of a change to the recorded windows, tabs or directories, and again when Brindle quits. A state that can't be serialized SHALL NOT be written: the file SHALL keep what it last held, with one log line naming the problem.

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

#### Scenario: A state that can't be serialized keeps the file
- **WHEN** a tab's shell changes into a directory whose name isn't valid UTF-8, so the recorded state can't be serialized, and stays there for a minute
- **THEN** `session.toml` still holds what was last written, not an empty file, and the log has one line naming the problem, not one per check

#### Scenario: Quitting while the state can't be serialized
- **WHEN** Brindle quits while a tab's directory still can't be serialized
- **THEN** the quit leaves `session.toml` as it was last written, and the next plain launch restores from that file

#### Scenario: Writes resume once the state serializes
- **WHEN** a tab's shell has been in a directory whose name isn't valid UTF-8 and then changes to `/tmp`
- **THEN** within a few seconds `session.toml` is written with the current windows and tabs, that tab recorded in `/tmp`

#### Scenario: No state directory
- **WHEN** on Linux `BRINDLE_STATE_DIR`, `XDG_STATE_HOME` and `HOME` are unset, the user has no home directory, and Brindle starts in `/tmp`
- **THEN** the window opens with a default tab and says the session can't be saved, and nothing is written to `/tmp`
