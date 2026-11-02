## ADDED Requirements

### Requirement: Remote target in the profile editor
The profile editor SHALL show a "Remote" field with the choices `none`, `ssh` and `container`. For `ssh` it SHALL show host, user, port, identity file and ssh arguments; for `container` it SHALL show engine (`docker` or `podman`), container and user. Fields of the other kinds SHALL be hidden. Choosing `none` SHALL remove the profile's `remote` on save. Saving SHALL write the `remote` table with blank optional fields as absent keys and ssh arguments as shell words, as for the profile's other list fields.

#### Scenario: Add an SSH target
- **WHEN** the user sets Remote to `ssh`, host to `A8` and user to `ubuntu` on profile `Work` and saves
- **THEN** the `Work` table has `remote = { type = "ssh", host = "A8", user = "ubuntu" }` and no other remote keys

#### Scenario: Container target
- **WHEN** the user sets Remote to `container`, engine `podman` and container `my-box` and saves
- **THEN** the profile has `remote = { type = "container", engine = "podman", container = "my-box" }`

#### Scenario: Remove a target
- **WHEN** the user opens a remote profile, sets Remote to `none` and saves
- **THEN** the profile's table no longer has `remote`

#### Scenario: Existing target shown
- **WHEN** the user opens a profile whose config has `remote = { type = "ssh", host = "A8", port = 2222 }`
- **THEN** the editor shows Remote `ssh`, host `A8` and port `2222`

### Requirement: Remote target validation
An `ssh` target SHALL need a non-blank host and a container target a non-blank container. A port SHALL be blank or a number from 1 to 65535. An invalid field SHALL be marked and SHALL block saving.

#### Scenario: Empty host
- **WHEN** Remote is `ssh` and the host is blank
- **THEN** the host field is marked invalid and Save is unavailable

#### Scenario: Bad port
- **WHEN** the user types `70000` as the port
- **THEN** the port field is marked invalid and Save is unavailable

### Requirement: Profile list shows the remote
The profile list SHALL show a remote profile's target (`ssh:<host>` or `container:<name>`) beside its name.

#### Scenario: Remote marked in the list
- **WHEN** profile `Work` has an SSH target with host `A8`
- **THEN** its row shows `ssh:A8`
