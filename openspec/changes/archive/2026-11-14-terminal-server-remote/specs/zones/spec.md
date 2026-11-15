## MODIFIED Requirements

### Requirement: Remote zone split
Splitting a zone whose profile has a remote target SHALL create the new session on the same SSH host or container the zone runs on, through that endpoint's existing connection, and SHALL inherit the active remote working directory. A local directory SHALL NOT be passed to a remote profile, and a directory from one remote target SHALL NOT be passed to another target. The split SHALL stay on the zone's endpoint even when the profile's `remote` target has changed since the tab opened.

#### Scenario: Split on the same SSH host
- **WHEN** an SSH-backed zone on host `A8` is in `/srv/app` and the user selects Split Zone Right
- **THEN** the new zone connects to `A8` and starts in `/srv/app`

#### Scenario: Open remote profile from a local tab
- **WHEN** the active local tab is in `C:\src` and the user opens an SSH profile
- **THEN** Brindle does not send `C:\src` as the remote working directory

#### Scenario: Split after the profile moved
- **WHEN** a zone of profile `Remote` runs on `old-host`, the profile's host is changed to `new-host` and saved, and the user splits the zone
- **THEN** the new zone runs on `old-host`

### Requirement: Closing ends the program
When the user closes a local or remote terminal (its zone or its tab), on Linux or on Windows, Brindle SHALL end its program in the terminal server, without asking, including a terminal attached to a session that an earlier Brindle left. Quitting Brindle SHALL NOT end it (see "Quitting keeps programs running" in `tabs`). A remote terminal closed while its endpoint is disconnected SHALL have its program ended once Brindle reconnects, if Brindle is still running then.

#### Scenario: Close a zone
- **WHEN** a zone runs `sleep 1000` and the user closes it
- **THEN** the zone closes, the `sleep` process is gone within a few seconds, and no session is listed for it

#### Scenario: Close a reattached tab
- **WHEN** a tab reattached at launch runs `sleep 1000` and the user closes it with ctrl-shift-w
- **THEN** the `sleep` process is gone within a few seconds

#### Scenario: Close a Windows zone
- **WHEN** a Windows zone runs `ping -t localhost` and the user closes it
- **THEN** the zone closes, the `ping` process is gone within a few seconds, and no session is listed for it

#### Scenario: Close a remote tab
- **WHEN** an SSH tab on `A8` runs `sleep 1000` and the user closes it
- **THEN** the tab closes, and on `A8` the `sleep` process is gone within a few seconds and no session is listed for it
