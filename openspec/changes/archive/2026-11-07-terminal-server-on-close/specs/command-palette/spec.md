## ADDED Requirements

### Requirement: Sessions palette
The `Attach Session…` command (Linux) SHALL open a palette listing the local terminal server's detached sessions, newest first, each row showing the session's title (else its command), its working directory and how long ago it started. Typed text SHALL filter the rows as in the command palette. With no detached sessions the palette SHALL say so. Sessions attached to any client SHALL NOT be listed.

#### Scenario: Nothing detached
- **WHEN** every session is attached and the user runs "Attach Session…"
- **THEN** the palette says there are no detached sessions

### Requirement: Sessions palette actions
In the sessions palette, enter SHALL attach the selected session into a new tab after the active tab, and make it active. `picker_kill` (ctrl-shift-delete) SHALL end the selected session and refresh the list.

#### Scenario: Attach a detached session
- **WHEN** a `--no-restore` launch left a session running `top` detached, and the user runs "Attach Session…" and presses enter on its row
- **THEN** a new active tab shows that `top`, with its screen as it was

#### Scenario: Kill from the palette
- **WHEN** the user selects a session's row and presses ctrl-shift-delete
- **THEN** the session's program ends and its row disappears
