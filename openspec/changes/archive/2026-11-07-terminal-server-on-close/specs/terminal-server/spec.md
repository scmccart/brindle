## MODIFIED Requirements

### Requirement: Close policies
Each session SHALL have a close policy, chosen when it is started. With `kill`, the session SHALL end, hanging up its program and the processes under it, when its client closes it, or when its client is gone: its connection ended and it didn't reconnect within the reconnect grace period after a core restart. With `detach`, the session SHALL keep running unattached when its client is gone. A client that explicitly detaches a session SHALL leave it running unattached whatever its policy, and a `kill` session detached that way SHALL become a `detach` session.

#### Scenario: Client crashes with kill
- **WHEN** a client that started a `kill` session is killed with SIGKILL
- **THEN** the session's program and the processes under it are gone within a few seconds

#### Scenario: Client leaves with detach
- **WHEN** a client that started a `detach` session disconnects
- **THEN** the session is listed as detached and its program keeps running

#### Scenario: Explicit detach of a kill session
- **WHEN** a client detaches a `kill` session and then disconnects
- **THEN** the session is listed as detached with policy `detach`, and its program keeps running

### Requirement: Clients reconnect after a core restart
When the core restarts, its clients' connections end. A client that reconnects within the reconnect grace period and identifies itself SHALL get back every session it was attached to when its connection ended, without the close policy applying, and SHALL receive each one's replay before live output resumes. Sessions the client had explicitly detached, or that another client attached since, SHALL NOT be given back to it. This SHALL hold across core restarts and upgrades: the server keeps each session's owner and policy as they last changed.

#### Scenario: Reconnect keeps kill sessions
- **WHEN** the core restarts while a client has a `kill` session running `htop`, and the client reconnects within the grace period
- **THEN** the session is still running with the same pid, and the client receives its replay and then live output

#### Scenario: No reconnect ends kill sessions
- **WHEN** the core restarts and a client with `kill` sessions doesn't reconnect within the grace period
- **THEN** those sessions end

#### Scenario: Detached sessions stay detached
- **WHEN** a client detaches a session, then the core restarts and the client reconnects
- **THEN** the session is still listed as detached, and the client receives no replay of it

#### Scenario: Attached by another client
- **WHEN** client B attaches a session that client A started, and the core restarts
- **THEN** after both reconnect, B gets the session back and A doesn't

### Requirement: Protocol versions
A client and the core SHALL agree on a protocol version when they connect, and so SHALL the core and each holder. A core SHALL speak every client protocol version and every holder protocol version from this change's first one up to its own. A client SHALL refuse a core that speaks none of its versions, with an error naming both versions. A holder speaking an older holder protocol that can't store a session's changed owner or policy SHALL keep working; only those changes are lost if the core restarts before the holder drains.

#### Scenario: Older client, newer core
- **WHEN** a Brindle from an older release connects to a core from a newer release
- **THEN** its tabs open and work through that core

#### Scenario: Newer core, older holder
- **WHEN** a newer core adopts a holder started by an older release
- **THEN** that holder's sessions keep working: input, output, resize, exit and kill

### Requirement: Management commands
`brindle-remote` SHALL provide `list`, `kill <id>`, `kill --all`, `status` and `stop` for the server of the current server directory. `list` and `status` SHALL accept `--json`. When no server is running, `list` prints nothing and `status` says so, and both exit successfully. `kill` with an unknown id SHALL fail with an error naming the id.

#### Scenario: List
- **WHEN** a server has two sessions and the user runs `brindle-remote list`
- **THEN** one line per session shows its id, pid, attached or detached state, policy, age, title, command and working directory

#### Scenario: Kill one
- **WHEN** the user runs `brindle-remote kill <id>` for an attached session
- **THEN** that session's program is hung up and its client receives the session's exit

#### Scenario: Status
- **WHEN** the user runs `brindle-remote status --json`
- **THEN** it prints the core's version, protocol versions, pid, uptime and client count, and for each holder its pid, build, session count and whether it is draining

#### Scenario: Stop
- **WHEN** the user runs `brindle-remote stop`
- **THEN** every session is hung up, every client receives its sessions' exits, and the core and holders exit

## ADDED Requirements

### Requirement: Sessions are identified beyond their id
Each session SHALL have a random key, reported when it starts and in the session list, that no other session of any server lifetime shares. The session list SHALL also report each session's title, its start time and its policy. A client SHALL be able to tell a session it recorded from a different session that later got the same id.

#### Scenario: Same id, different key
- **WHEN** the server stops, a new server starts, and its first session gets id 1 as a session of the old server did
- **THEN** the two sessions' keys differ

#### Scenario: Title listed
- **WHEN** a session's program sets its title to `build` and a client lists sessions
- **THEN** that session's entry has title `build`

### Requirement: Attaching takes ownership
A client that attaches a session SHALL become its owner: it gets the session back on reconnect, as defined by "Clients reconnect after a core restart", and the session's policy applies when that client is gone.

#### Scenario: Attached kill session
- **WHEN** client B attaches a detached `kill` session and B is killed with SIGKILL
- **THEN** the session's program ends within a few seconds

### Requirement: Server outlives the desktop's app scope
On Linux with a systemd user manager, the core and the holders SHALL run outside the cgroup of the client that started them, so stopping that client's desktop app scope, or an out-of-memory kill of it, doesn't end the sessions. Without a user manager they SHALL start as before. Sessions SHALL NOT be required to survive the end of the user's login session.

#### Scenario: App scope stopped
- **WHEN** Brindle was launched from GNOME, has a `detach` session running `sleep 1000`, and its `app-gnome-brindle-*.scope` is stopped with `systemctl --user stop`
- **THEN** the `sleep` keeps running and is listed as detached

#### Scenario: No user manager
- **WHEN** no systemd user manager is reachable
- **THEN** the server starts and serves sessions as it did before this change
