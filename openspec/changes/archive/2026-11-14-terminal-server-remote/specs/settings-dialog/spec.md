## MODIFIED Requirements

### Requirement: Remote target in the profile editor
The profile editor SHALL show a "Remote" field with the choices `none`, `ssh` and `container`. For `ssh` it SHALL show host, user, port, identity file and ssh arguments; for `container` it SHALL show engine (`docker` or `podman`), container and user. For both it SHALL show a "Connect at launch" switch. Fields of the other kinds SHALL be hidden. Choosing `none` SHALL remove the profile's `remote` on save. Saving SHALL write the `remote` table with blank optional fields as absent keys, the switch only when on, and ssh arguments as shell words, as for the profile's other list fields.

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

#### Scenario: Connect at launch
- **WHEN** the user turns on "Connect at launch" for an `ssh` target with host `A8` and saves
- **THEN** the profile has `remote = { type = "ssh", host = "A8", connect_at_launch = true }`
