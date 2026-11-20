## MODIFIED Requirements

### Requirement: Installed helper stays replaceable on Windows
On Windows, no server role SHALL run from the installed `brindle-remote.exe`. Each SHALL run from a copy for its build under `%LOCALAPPDATA%\brindle\server\` (inside the server directory when `BRINDLE_SERVER_DIR` is set; for an elevated process, under its elevated files directory, see "Elevated server files are out of an unelevated process's reach on Windows"), so replacing or removing the installed files while sessions run succeeds. A copy SHALL be removed once no server role runs from it and a newer build has started.

#### Scenario: Replace the helper while sessions run
- **WHEN** local tabs are running on Windows and `brindle-remote.exe` in Brindle's install directory is replaced with another build
- **THEN** the replacement succeeds, and the tabs keep working

## ADDED Requirements

### Requirement: Elevated server files are out of an unelevated process's reach on Windows
On Windows, every directory an elevated Brindle or elevated server role runs, loads, writes or removes files in (its server directory, its helper copies and its ConPTY files) SHALL be one an unelevated process of the same user can't write or replace. One that is a reparse point, or that it didn't make that way, SHALL be refused or replaced before use, and nothing inside SHALL be followed through a reparse point.

#### Scenario: Planted helper copy
- **WHEN** an unelevated process writes a `brindle-remote.exe` where the elevated server's helper copy for the current build goes, and an elevated Brindle then opens a local tab
- **THEN** the planted file is never run, and the tab works

#### Scenario: Unelevated write into the elevated directories
- **WHEN** an elevated Brindle has a local tab open, and an unelevated process of the same user tries to create, replace or delete a file in the elevated server directory, helper copies or ConPTY files
- **THEN** each attempt fails with access denied

#### Scenario: Junction where the copies go
- **WHEN** an unelevated process makes the elevated copies directory a junction to another directory before an elevated Brindle starts
- **THEN** the elevated Brindle doesn't use or prune through the junction, and nothing in the junction's target is removed

### Requirement: Bare program names resolve through PATH on Windows
On Windows, a session's program named without a directory SHALL be found through `PATH`, never in the server role's own directory.

#### Scenario: Program planted beside the helper
- **WHEN** a `cmd.exe` is placed in the directory of the helper copy the server runs from, and a profile with `command = "cmd.exe"` opens a tab
- **THEN** the tab runs `cmd.exe` from `PATH`, not the placed file
