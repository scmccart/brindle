## MODIFIED Requirements

### Requirement: Sessions palette
The `Attach Session…` command (Linux and Windows) SHALL open a palette listing the detached sessions of the local terminal server and of every remote endpoint Brindle is connected to. The rows SHALL be grouped by endpoint, local first, then remote endpoints in profile order. Within a group they SHALL be newest first. Each row SHALL show the session's title (else its command), its working directory and how long ago it started. Each endpoint Brindle is connecting to, or that is offline, SHALL be listed as a row saying so. An offline endpoint's row SHALL say, on its one line, why its last attempt failed, with the SSH or container engine's own error when there was one. The palette SHALL open without waiting for an endpoint that is slow to answer. Until such an endpoint answers, it SHALL be listed as a row saying its sessions are being listed. Typed text SHALL filter the rows as in the command palette. Once every endpoint has answered, with no detached sessions anywhere the palette SHALL say so, after any endpoint's row. When the rows change while the palette is open, the filter text SHALL be kept, and the highlighted session SHALL stay highlighted while it is listed. Sessions attached to any client SHALL NOT be listed.

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

#### Scenario: Stopped container says why
- **WHEN** a container profile connects at launch, its container is stopped, and the user runs "Attach Session…"
- **THEN** the container's row says it is offline because the container is not running

#### Scenario: Slow host
- **WHEN** Brindle is connected to `A8`, `A8`'s link has gone silent but Brindle hasn't noticed yet, and the user runs "Attach Session…"
- **THEN** the palette opens at once with this machine's sessions and a row saying `A8`'s sessions are being listed, and that row shows `A8` as offline once its listing gives up

#### Scenario: No sessions note comes last
- **WHEN** nothing is detached on this machine, profile `A8` connects at launch, `A8` can't be reached, and the user runs "Attach Session…"
- **THEN** the palette's first row says `A8` is offline, and the row after it says there are no detached sessions

#### Scenario: Highlight kept on a refresh
- **WHEN** the palette lists three local sessions, the user moves the highlight to the second, and then an endpoint's state changes
- **THEN** the second session is still highlighted, and the filter text is unchanged

### Requirement: Sessions palette actions
In the sessions palette, enter SHALL attach the selected session into a new tab after the active tab, and make it active. A remote session's tab SHALL run on the session's endpoint. It SHALL use the first of these profiles that exists:
- the profile it was started with, when that profile targets the endpoint;
- the first profile that targets the endpoint;
- the profile of the first tab on the endpoint;
- the default profile.

`picker_kill` (ctrl-shift-delete) SHALL end the selected session, local or remote, and refresh the list, with the highlight on the row that takes the ended session's place.

#### Scenario: Attach a detached session
- **WHEN** a `--no-restore` launch left a session running `top` detached, and the user runs "Attach Session…" and presses enter on its row
- **THEN** a new active tab shows that `top`, with its screen as it was

#### Scenario: Kill from the palette
- **WHEN** the user selects a session's row and presses ctrl-shift-delete
- **THEN** the session's program ends and its row disappears

#### Scenario: Kill keeps the place
- **WHEN** the palette lists three detached sessions, the user highlights the second and presses ctrl-shift-delete, then presses enter
- **THEN** the second session's program ends, and enter attaches the session that was listed third

#### Scenario: Attach a remote session
- **WHEN** the user presses enter on a row listed under `A8`
- **THEN** a new active tab shows that session, and splitting its zone opens the new zone on `A8`

#### Scenario: Endpoint no profile targets
- **WHEN** no profile targets `old-host`, a tab of profile `Remote` runs on `old-host`, and the user presses enter on a row listed under `old-host`
- **THEN** the new tab has profile `Remote`, and it and its splits run on `old-host`
