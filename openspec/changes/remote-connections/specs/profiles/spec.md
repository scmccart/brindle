## ADDED Requirements

### Requirement: Remote target
A profile MAY define one remote Linux endpoint. An SSH endpoint SHALL accept `host` and optional `user`, `port`, `identity_file`, and `ssh_args`. A container endpoint SHALL accept `container`, optional `user`, and `engine` (`docker` by default, or `podman`). The profile's command, arguments, working directory, environment, theme, font size, and tmux settings SHALL otherwise retain their local meanings on that endpoint.

#### Scenario: SSH config alias
- **WHEN** a profile sets `remote = { type = "ssh", host = "A8" }`
- **THEN** Brindle invokes OpenSSH using `A8`, so the user's SSH configuration, agent, proxy, and host-key policy apply

#### Scenario: Existing container
- **WHEN** a profile sets `remote = { type = "container", container = "devbox" }`
- **THEN** Brindle opens the profile in the already-running `devbox` through `docker exec` and does not create or start a container
