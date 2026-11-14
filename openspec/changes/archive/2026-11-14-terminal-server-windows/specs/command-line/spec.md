## MODIFIED Requirements

### Requirement: Launch options
Brindle SHALL accept:
- `-p/--profile <NAME>` (matched case-insensitively; an unknown name falls back to the default profile with a warning);
- `-d/--working-directory <DIR>`;
- `--config <FILE>`;
- `-e/--command <CMD> [ARGS]...`, which runs CMD instead of the profile's program and must come last;
- `--no-restore`, which skips session restore for this launch;
- `--attach <ID>` (Linux and Windows), which opens a tab attached to the detached terminal server session `ID`.

An unknown argument SHALL print usage and exit with status 2.

#### Scenario: Run a command
- **WHEN** the user runs `brindle -e htop`
- **THEN** a tab running htop opens and becomes active, and when htop exits its tab closes

#### Scenario: Attach a session
- **WHEN** `brindle-remote list` shows a detached session 7 running `htop`, and the user runs `brindle --attach 7`
- **THEN** an active tab shows that `htop`, and the session is no longer detached

#### Scenario: Attach a missing session
- **WHEN** the user runs `brindle --attach 99` and there is no session 99, or it is attached elsewhere
- **THEN** an active tab's message names session 99 and the reason

### Requirement: Terminal server directory for tests
When the environment variable `BRINDLE_SERVER_DIR` is set and not empty, Brindle and `brindle-remote` SHALL use it as the terminal server's directory, as defined by the `terminal-server` capability, on Linux and on Windows. Programs in tabs inherit it, so `brindle-remote` commands run in a tab manage that same server.

#### Scenario: Isolated server
- **WHEN** a test runs Brindle with `BRINDLE_SERVER_DIR=/tmp/case-1`
- **THEN** its local tabs run in a server whose socket is in `/tmp/case-1`, and the user's own server is not touched

#### Scenario: Isolated server on Windows
- **WHEN** a test runs Brindle on Windows with `BRINDLE_SERVER_DIR=C:\Temp\case-1`
- **THEN** its local tabs run in a server whose lock and log files are in `C:\Temp\case-1`, and the user's own server is not touched

#### Scenario: Managing from a tab
- **WHEN** the user runs `brindle-remote list` in a local tab
- **THEN** it lists the sessions of the server that tab runs in, that tab's own session included

## ADDED Requirements

### Requirement: Terminal server helper override
When `BRINDLE_SERVER_HELPER` is set and not empty, Brindle SHALL use the file it names as the local terminal server's `brindle-remote`, on Linux and on Windows. Without it, Brindle SHALL look beside its own executable, and on Linux SHALL first honour `BRINDLE_REMOTE_AGENT`. On Windows, `BRINDLE_REMOTE_AGENT` SHALL keep naming only the Linux helper that remote profiles deploy.

#### Scenario: Override on Windows
- **WHEN** Brindle runs on Windows with `BRINDLE_SERVER_HELPER=C:\build\brindle-remote.exe` and `BRINDLE_REMOTE_AGENT` naming a Linux helper
- **THEN** local tabs run in a server started from a copy of `C:\build\brindle-remote.exe`, and remote profiles deploy the Linux helper

#### Scenario: Override names nothing
- **WHEN** `BRINDLE_SERVER_HELPER` names a file that doesn't exist and the user opens a local tab
- **THEN** the tab shows an error naming `BRINDLE_SERVER_HELPER` and the path
