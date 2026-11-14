## MODIFIED Requirements

### Requirement: Remote target
A profile MAY define one remote Linux endpoint. An SSH endpoint SHALL accept `host` and optional `user`, `port`, `identity_file`, and `ssh_args`. A container endpoint SHALL accept `container`, optional `user`, and `engine` (`docker` by default, or `podman`). Either kind SHALL accept an optional `connect_at_launch` (default `false`), as defined by `remote-connections`. The profile's command, arguments, working directory, environment, theme, and font size SHALL otherwise retain their local meanings on that endpoint.

#### Scenario: SSH config alias
- **WHEN** a profile sets `remote = { type = "ssh", host = "A8" }`
- **THEN** Brindle invokes OpenSSH using `A8`, so the user's SSH configuration, agent, proxy, and host-key policy apply

#### Scenario: Existing container
- **WHEN** a profile sets `remote = { type = "container", container = "devbox" }`
- **THEN** Brindle opens the profile in the already-running `devbox` through `docker exec` and does not create or start a container

#### Scenario: Connect at launch
- **WHEN** a profile sets `remote = { type = "container", container = "devbox", connect_at_launch = true }`
- **THEN** Brindle connects to `devbox` at launch, and still does not start it when it is stopped
