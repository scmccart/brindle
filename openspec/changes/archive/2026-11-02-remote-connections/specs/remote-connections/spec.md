# remote-connections Specification

## Purpose
Run profiles on a remote Linux host over SSH or inside an existing Docker or Podman container, with the same tabs, zones, titles, directories, notifications and tmux control mode as a local profile.

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

### Requirement: A vanished client ends the remote session
The remote helper SHALL end a session, hanging up its shell and the processes under it, when it receives nothing from the client for a bounded idle limit. The client SHALL send a heartbeat while it is otherwise silent, so an idle but connected session is kept. A client that is merely slow to read output SHALL NOT be treated as vanished.

#### Scenario: Network cut without a clean close
- **WHEN** the client stops sending anything while the transport stays open on the server (the connection was cut and the server did not notice)
- **THEN** the helper, the shell and the processes under it are gone within the idle limit plus a few seconds

#### Scenario: Idle session stays connected
- **WHEN** a remote tab sits idle for longer than the idle limit with the client running
- **THEN** the session is still alive and accepts input

#### Scenario: Slow reader during heavy output
- **WHEN** the client stops reading for longer than the idle limit while the remote program floods output
- **THEN** the helper does not end the session for idleness, and output resumes when the client reads again

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
When a remote profile specifies `tmux = "control"`, Brindle SHALL run `tmux -u -C` on the remote endpoint and show its windows and panes natively, as it does for a local control-mode session.

#### Scenario: Native tabs for remote tmux windows
- **WHEN** a remote profile with `tmux = "control"` attaches to a remote tmux session
- **THEN** each remote tmux window is rendered as a native tab in Brindle
