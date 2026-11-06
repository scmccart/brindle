# Spec Delta

## Purpose

A per-user terminal server, `brindle-remote` in server mode, owns terminal sessions on its host, so a session can outlive the client that started it and be attached again with its screen intact.

## ADDED Requirements

### Requirement: One server per user per host
At most one terminal server SHALL run per user per server directory. The directory is `$BRINDLE_SERVER_DIR` when set, else `$XDG_RUNTIME_DIR/brindle`, else `~/.cache/brindle/run`, and is created user-only (mode 0700). A server that finds another live server for its directory SHALL exit without disturbing it.

#### Scenario: Two launches race
- **WHEN** two Brindle processes start at the same moment with no server running
- **THEN** exactly one server runs afterwards, and both Brindles' tabs are served by it

#### Scenario: Isolated directory
- **WHEN** a server runs with `BRINDLE_SERVER_DIR=/tmp/a` and another client starts one with `BRINDLE_SERVER_DIR=/tmp/b`
- **THEN** two servers run, and neither lists the other's sessions

### Requirement: Only the owning user can connect
The server's socket SHALL be reachable only by the user who runs it. The server SHALL check each connecting peer's user id and close the connection, without reading a request, when it differs from its own.

#### Scenario: Another user connects
- **WHEN** a process running as a different user connects to the server's socket
- **THEN** the connection is closed and no session is listed or created

### Requirement: Server starts on demand and exits when unused
A client SHALL start the server when none is running, detached from the client's process group and session, so that the server survives the client. The server SHALL exit once it has had neither sessions nor connected clients for a short grace period.

#### Scenario: First local tab starts the server
- **WHEN** no server is running and Brindle opens a local Linux tab
- **THEN** a server is running, and its parent is not the Brindle process

#### Scenario: Unused server exits
- **WHEN** the last client disconnects and the server has no sessions left
- **THEN** the server process exits within a few seconds, and removes its socket

### Requirement: Sessions
A client SHALL be able to start a session with a command, arguments, working directory, environment, grid size and scrollback limit. The server SHALL run the program on a new PTY with exactly that environment, not its own. It SHALL report the session's id and the program's pid, or the reason the program could not be started.

#### Scenario: Session uses the client's environment
- **WHEN** the server was started by a client whose environment had `FOO=1`, and a later client with `FOO=2` starts a session
- **THEN** the program in that session sees `FOO=2`

#### Scenario: Program cannot start
- **WHEN** a client starts a session whose command does not exist
- **THEN** the server replies with an error naming the program and the OS error, and no session is listed

### Requirement: Session I/O
For an attached session, the server SHALL carry the client's input and resizes to the PTY, stream its output to the client in order, and report the program's exit with its exit status. One session's slow or stalled client SHALL NOT delay other sessions.

#### Scenario: Exit status
- **WHEN** the program in an attached session runs `exit 3`
- **THEN** the client receives the session's exit with status 3, and the session is no longer listed

#### Scenario: A stalled client
- **WHEN** one client stops reading while its program floods output
- **THEN** sessions of other clients keep streaming output without delay

### Requirement: Close policies
Each session SHALL have a close policy, chosen when it is started. With `kill`, the session SHALL end, hanging up its program and the processes under it, when its client closes it or its client's connection ends for any reason, a crash included. With `detach`, the session SHALL keep running unattached when its client's connection ends.

#### Scenario: Client crashes with kill
- **WHEN** a client that started a `kill` session is killed with SIGKILL
- **THEN** the session's program and the processes under it are gone within a few seconds

#### Scenario: Client leaves with detach
- **WHEN** a client that started a `detach` session disconnects
- **THEN** the session is listed as detached and its program keeps running

### Requirement: Attach replays the screen
A client SHALL be able to attach to a detached session. Before any live output, the server SHALL send a replay that recreates the session's screen: a reset, the terminal modes, the scrollback within the session's limit, the visible grid with its colors and attributes, the cursor position and shape, and the title. The replay SHALL be delimited, so the client can tell it from live output.

#### Scenario: Full-screen program
- **WHEN** a client attaches to a detached session that is running `htop`
- **THEN** after the replay the client's screen matches the session's screen, in the alternate screen with mouse reporting as `htop` set it

#### Scenario: Scrollback
- **WHEN** a detached session has printed 500 numbered lines with a scrollback limit of 1000, and a client attaches
- **THEN** the client's scrollback holds all 500 lines in order

#### Scenario: Attached sessions can't be taken
- **WHEN** a client asks to attach to a session another client is attached to
- **THEN** the server refuses with an error saying the session is attached

### Requirement: Program status and prompt marks survive attach
The server SHALL keep each session's program status records (OSC 7501) and shell-integration prompt marks (OSC 133), and send them to an attaching client after the replay, so the client's status dots and prompt navigation match the session.

#### Scenario: Status after attach
- **WHEN** a program in a detached session reported `working`, and a client attaches
- **THEN** the client shows the session's status as working

#### Scenario: Prompt marks after attach
- **WHEN** a detached session's shell emitted prompt marks for three commands, and a client attaches
- **THEN** the client can jump between those three prompts

### Requirement: The server never answers terminal queries
The server's per-session emulator SHALL NOT write anything to the PTY. Device attribute, cursor position and color queries SHALL be answered only by the attached client.

#### Scenario: Cursor position query
- **WHEN** a program in an attached session sends `ESC [ 6 n`
- **THEN** it receives exactly one reply, from the client

### Requirement: Protocol versions
A client and the server SHALL agree on a protocol version when they connect. A server SHALL speak every protocol version from this change's first one up to its own. A client SHALL refuse a server that speaks none of its versions, with an error naming both versions.

#### Scenario: Older client, newer server
- **WHEN** a Brindle from an older release connects to a server from a newer release
- **THEN** its tabs open and work through that server

### Requirement: Upgrades hand sessions over
When a client whose server binary is newer than the running server connects, the server SHALL replace itself with that binary in place. Every session SHALL carry over with its program, pid, screen, scrollback, status and marks, and every client connection, including other clients' older protocol versions, SHALL stay open.

#### Scenario: Upgrade with a running Brindle
- **WHEN** a Brindle of version N has local tabs open on a server of version N, and a Brindle of version N+1 starts
- **THEN** the server runs version N+1 with the same pid, the version N Brindle's tabs keep working with the same programs and screens, and nothing is lost

#### Scenario: Detached session survives an upgrade
- **WHEN** a detached session is running `htop` and the server is upgraded
- **THEN** attaching afterwards replays `htop`'s screen, and `htop` still has the same pid

### Requirement: A failed upgrade keeps the old server
The server SHALL check that the new binary accepts its handover state before replacing itself. If the check fails, the server SHALL keep running unchanged, and the client SHALL use it when they share a protocol version. Otherwise the client reports the error in its tabs, as described in `terminal-emulation`.

#### Scenario: Incompatible binary
- **WHEN** the offered binary cannot read the running server's handover state
- **THEN** the old server keeps running with all its sessions, and logs why the upgrade was refused

### Requirement: Failures stay within one session
A failure in one session's emulator SHALL NOT end the server or other sessions. That session's program SHALL keep running. Its replay SHALL then be only a reset, and live output continues.

#### Scenario: Emulator fault
- **WHEN** the emulator of one session panics on its output
- **THEN** that session's program keeps running and its client keeps receiving output, and every other session is unaffected

### Requirement: Management commands
`brindle-remote` SHALL provide `list`, `kill <id>`, `kill --all`, `status` and `stop` for the server of the current server directory. `list` and `status` SHALL accept `--json`. When no server is running, `list` prints nothing and `status` says so, and both exit successfully. `kill` with an unknown id SHALL fail with an error naming the id.

#### Scenario: List
- **WHEN** a server has two sessions and the user runs `brindle-remote list`
- **THEN** one line per session shows its id, pid, attached or detached state, command and working directory

#### Scenario: Kill one
- **WHEN** the user runs `brindle-remote kill <id>` for an attached session
- **THEN** that session's program is hung up and its client receives the session's exit

#### Scenario: Status
- **WHEN** the user runs `brindle-remote status --json`
- **THEN** it prints the server's version, protocol version, pid, uptime, session count and client count

#### Scenario: Stop
- **WHEN** the user runs `brindle-remote stop`
- **THEN** every session is hung up, every client receives its sessions' exits, and the server exits
