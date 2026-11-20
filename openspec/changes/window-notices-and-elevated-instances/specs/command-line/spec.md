# Spec Delta

## MODIFIED Requirements

### Requirement: A launch goes to the running Brindle
A launch without debug options while an instance runs (one with the same state directory) SHALL send its request to the instance and exit with status 0 once the instance has accepted it. The instance SHALL raise its window and open what the launch asked for, as "A launch restores, then opens what it asks for" says, without restoring again; a `-e` command SHALL start in the launching process's working directory. When the instance doesn't accept the request within 5 seconds, the launch SHALL print why and exit with status 1. The request channel SHALL be reachable only by the same user. Before sending anything, the launch SHALL check that the instance runs as the same user and, on Windows, at the same integrity level, and SHALL give it no right to act as the launching user; otherwise it SHALL send nothing, print why and exit with status 1. On Windows, a launch with no console that gives up (refused, or not accepted within 5 seconds) SHALL also show why in a message box. An elevated and an unelevated Brindle on Windows have different default state directories, so each is its own instance.

#### Scenario: Second plain launch
- **WHEN** Brindle runs with two tabs and the user launches `brindle` from the desktop
- **THEN** the window is raised, it still has two tabs, and no second window opens

#### Scenario: Command from a shell
- **WHEN** Brindle runs and the user runs `brindle -e htop` in a shell in `/srv`
- **THEN** the `brindle` command exits, and Brindle's window gains an active tab running htop in `/srv`

#### Scenario: Attach from a shell
- **WHEN** Brindle runs, session 7 is detached, and the user runs `brindle --attach 7`
- **THEN** Brindle's window gains an active tab showing session 7

#### Scenario: Pipe served by someone else on Windows
- **WHEN** another account's process has created the instance pipe's name, and the user runs `brindle -e pwsh`
- **THEN** the launch sends nothing to that process, prints that the pipe's server is not this user's Brindle, and exits with status 1

#### Scenario: Elevated launch beside an unelevated Brindle on Windows
- **WHEN** an unelevated Brindle runs, and the user starts `brindle` elevated without `BRINDLE_STATE_DIR`
- **THEN** a second, elevated window opens with the elevated Brindle's own session, and the unelevated window is unchanged

#### Scenario: Shared state directory across levels on Windows
- **WHEN** an unelevated Brindle runs with `BRINDLE_STATE_DIR=C:\s`, and the user starts `brindle` elevated with the same `BRINDLE_STATE_DIR`
- **THEN** the elevated launch sends nothing, says that the running Brindle is at another integrity level, and exits with status 1

#### Scenario: Refused launch from the Start Menu
- **WHEN** a launch with no console is refused by the check above
- **THEN** a message box says why before the launch exits with status 1
