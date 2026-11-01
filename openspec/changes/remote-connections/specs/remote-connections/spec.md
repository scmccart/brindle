# remote-connections Specification

## ADDED Requirements

### Requirement: Remote target profile configuration
A profile SHALL be able to define an optional `remote` target specifying either an SSH host or a container.
For SSH targets, the configuration SHALL support:
- `host` (hostname or SSH config alias)
- optional `user`
- optional `port`
- optional `identity_file`
- optional `ssh_args`

For container targets, the configuration SHALL support:
- `engine` (`docker` or `podman`, defaulting to `docker`)
- `container` (container name or ID)
- optional `user`

#### Scenario: SSH remote profile
- **WHEN** a profile is configured with `remote = { type = "ssh", host = "A8" }`
- **THEN** launching tabs with this profile connects to the remote host `A8` over SSH

#### Scenario: Container remote profile
- **WHEN** a profile is configured with `remote = { type = "container", engine = "docker", container = "my-box" }`
- **THEN** launching tabs with this profile executes inside the `my-box` container via `docker exec`

### Requirement: Remote PTY lifecycle and I/O parity
A remote terminal session SHALL provide bidirectional PTY stream forwarding, resizing, and child exit notification with parity to local PTY sessions.
- Input typed into the terminal SHALL be transmitted to the remote PTY.
- Output from the remote PTY SHALL be streamed to the local terminal grid.
- Grid resizing SHALL be transmitted to the remote PTY via window size updates (`TIOCSWINSZ`).
- Termination of the remote shell or process SHALL trigger terminal exit locally.

#### Scenario: Interactive shell input and output
- **WHEN** the user types `echo hello` in a remote tab
- **THEN** the remote PTY receives the input and the terminal grid displays `hello`

#### Scenario: Window resizing
- **WHEN** the user resizes a remote terminal window to 120 columns and 40 rows
- **THEN** the remote PTY updates its terminal dimensions to 120x40

### Requirement: Process tracking and automatic titles
Remote Linux sessions SHALL inspect the remote foreground process name from `/proc` and provide it to Brindle's title refresh loop, matching local title behavior.

#### Scenario: Remote command execution title
- **WHEN** the user executes `htop` in a remote tab
- **THEN** the tab title updates to display `htop`

### Requirement: Current working directory tracking
Remote sessions SHALL track the remote working directory through `/proc/<pid>/cwd` and OSC 7 reports. When splitting a zone or launching a new tab from an active remote tab, the new session SHALL inherit the remote directory.

#### Scenario: Remote zone split inherits remote CWD
- **WHEN** the user is in `/var/log` on a remote host and triggers `Split Zone Right`
- **THEN** the newly opened split zone launches in `/var/log` on the same remote host

### Requirement: Passthrough of notifications and program status
Remote sessions SHALL pass through OSC 9, OSC 777, and OSC 7501 sequences intact in the output stream so that desktop notifications, bell indicators, and program status dots function identically to local tabs.

#### Scenario: Remote desktop notification
- **WHEN** a remote command prints `ESC ] 9 ; build complete BEL`
- **THEN** Brindle triggers a desktop notification for that tab

### Requirement: Remote tmux control mode
When a remote profile specifies `tmux = "control"`, Brindle SHALL spawn `tmux -u -C` on the remote endpoint and connect Brindle's native `TmuxSession` over the remote transport channel.

#### Scenario: Native tabs for remote tmux windows
- **WHEN** a remote profile with `tmux = "control"` attaches to a remote tmux session
- **THEN** each remote tmux window is rendered as a native tab in Brindle
