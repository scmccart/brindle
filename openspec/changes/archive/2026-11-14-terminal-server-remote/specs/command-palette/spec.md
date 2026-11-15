## MODIFIED Requirements

### Requirement: Sessions palette
The `Attach Session…` command (Linux and Windows) SHALL open a palette listing the detached sessions of the local terminal server and of every remote endpoint Brindle is connected to. The rows SHALL be grouped by endpoint, local first, then remote endpoints in profile order. Within a group they SHALL be newest first. Each row SHALL show the session's title (else its command), its working directory and how long ago it started. Each endpoint Brindle is connecting to, or that is offline, SHALL be listed as a row saying so. Typed text SHALL filter the rows as in the command palette. With no detached sessions anywhere the palette SHALL say so. Sessions attached to any client SHALL NOT be listed.

#### Scenario: Nothing detached
- **WHEN** every session is attached and the user runs "Attach Session…"
- **THEN** the palette says there are no detached sessions

#### Scenario: Detached Windows session
- **WHEN** on Windows a `--no-restore` launch left a PowerShell session detached, and the user runs "Attach Session…"
- **THEN** the palette lists that session with its title and directory

#### Scenario: Remote host's sessions
- **WHEN** Brindle is connected to `A8`, which has a detached session titled `build`, and the user runs "Attach Session…"
- **THEN** the palette lists `build` under `A8`, after the local sessions

#### Scenario: Offline host
- **WHEN** profile `A8` connects at launch, `A8` can't be reached, and the user runs "Attach Session…"
- **THEN** the palette has a row saying `A8` is offline, and choosing it opens nothing

### Requirement: Sessions palette actions
In the sessions palette, enter SHALL attach the selected session into a new tab after the active tab, and make it active. A remote session's tab SHALL use the profile it was started with when that profile targets its endpoint, else the first profile that does. `picker_kill` (ctrl-shift-delete) SHALL end the selected session, local or remote, and refresh the list.

#### Scenario: Attach a detached session
- **WHEN** a `--no-restore` launch left a session running `top` detached, and the user runs "Attach Session…" and presses enter on its row
- **THEN** a new active tab shows that `top`, with its screen as it was

#### Scenario: Kill from the palette
- **WHEN** the user selects a session's row and presses ctrl-shift-delete
- **THEN** the session's program ends and its row disappears

#### Scenario: Attach a remote session
- **WHEN** the user presses enter on a row listed under `A8`
- **THEN** a new active tab shows that session, and splitting its zone opens the new zone on `A8`
