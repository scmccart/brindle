## MODIFIED Requirements

### Requirement: Sessions palette
The `Attach Session…` command (Linux and Windows) SHALL open a palette listing the local terminal server's detached sessions, newest first, each row showing the session's title (else its command), its working directory and how long ago it started. Typed text SHALL filter the rows as in the command palette. With no detached sessions the palette SHALL say so. Sessions attached to any client SHALL NOT be listed.

#### Scenario: Nothing detached
- **WHEN** every session is attached and the user runs "Attach Session…"
- **THEN** the palette says there are no detached sessions

#### Scenario: Detached Windows session
- **WHEN** on Windows a `--no-restore` launch left a PowerShell session detached, and the user runs "Attach Session…"
- **THEN** the palette lists that session with its title and directory
