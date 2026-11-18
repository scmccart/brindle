# Spec Delta

## ADDED Requirements

### Requirement: A failed listing is shown in the sessions palette
When this machine's terminal server can't be listed, the sessions palette SHALL show a disabled row saying that this machine's sessions could not be listed, and why, in place of its sessions. It SHALL NOT say that there are no detached sessions on account of that failure. A row that reports an endpoint's failure SHALL name what was being done, such as listing that endpoint's sessions.

#### Scenario: This machine can't be listed
- **WHEN** this machine's terminal server can't be reached, starting one fails, and the user runs "Attach Session…"
- **THEN** the palette's row for this machine says its sessions could not be listed and why, and no row says there are no detached sessions

### Requirement: A failed kill is shown in the sessions palette
When ending a session from the sessions palette fails, the session's row SHALL stay listed, and a disabled row for its endpoint SHALL say that the session could not be ended, and why, until that endpoint is listed again.

#### Scenario: Kill fails
- **WHEN** the user presses ctrl-shift-delete on a session's row and the terminal server answers that it can't end it
- **THEN** the session's row is still listed, and a row says that session could not be ended, with the server's answer
