## MODIFIED Requirements

### Requirement: A launch goes to the running Brindle
A launch without debug options while an instance runs (one with the same state directory) SHALL send its request to the instance and exit with status 0 once the instance has accepted it. The instance SHALL raise its window and open what the launch asked for, as "A launch restores, then opens what it asks for" says, without restoring again; a `-e` command SHALL start in the launching process's working directory. When the instance doesn't accept the request within 5 seconds, the launch SHALL print why and exit with status 1. The request channel SHALL be reachable only by the same user. Before sending anything, the launch SHALL check that the instance runs as the same user and, on Windows, at the same integrity level, and SHALL give it no right to act as the launching user; otherwise it SHALL send nothing, print why and exit with status 1.

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
- **WHEN** an unelevated Brindle runs, and the user starts `brindle` elevated with the same state directory
- **THEN** the elevated launch sends nothing, prints that the running Brindle is at another integrity level, and exits with status 1
