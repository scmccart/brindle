# Spec Delta

## MODIFIED Requirements

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
