## ADDED Requirements

### Requirement: Relay to the server
`brindle-remote relay` SHALL connect its standard input and output to the core of the server directory it resolves, as a client of that core, starting the core when none runs, so that a client on another machine can reach the server over any byte stream (SSH, a container's `exec`). Before relaying, it SHALL report its version, build and environment, the user's login shell and home directory. It SHALL relay protocol frames unchanged in both directions, and end when either side closes.

#### Scenario: Relay starts the server
- **WHEN** no server runs for the user, and `brindle-remote relay` is started with its standard input and output connected to a client
- **THEN** a core runs, its parent is not the relay, and the client can start a session through the relay

#### Scenario: Relay ends with its client
- **WHEN** a relay's client closes the relay's standard input
- **THEN** the relay exits, and the core treats that client as gone

### Requirement: Relay liveness
The relay SHALL exchange heartbeats with its client on its standard input and output, separately from the protocol frames it relays: it sends one when it has written nothing for 10 s, and drops the client's without relaying them. It SHALL end when it has read nothing from its client for 60 s. The client's silence SHALL be measured from what the relay reads, independently of whether its writes to the client are blocked.

#### Scenario: Silent client
- **WHEN** a relay's client stays connected but sends nothing, not even heartbeats, for 60 s
- **THEN** the relay exits within a few seconds after that

#### Scenario: Heartbeats aren't relayed
- **WHEN** a client sends heartbeats through a relay to a core of an older release
- **THEN** the core receives none of them, and the connection keeps working

### Requirement: Relays offer upgrades
Before relaying, a relay whose binary is newer than the running core (by the rules a local client uses) SHALL check that it speaks the running holders' protocol versions, ask the core to step down, and start the next core from its own binary. A failed check or a refused step-down SHALL leave the running core in place, and the relay SHALL relay to it.

#### Scenario: Newer Brindle connects
- **WHEN** a host's core is version N with a session running `htop`, and a Brindle of version N+1 connects through its relay
- **THEN** the host's core runs version N+1, and `htop` keeps running with the same pid

#### Scenario: Older Brindle connects
- **WHEN** a host's core is version N+1, and a Brindle of version N connects through its relay
- **THEN** the core keeps running, and the version N Brindle's tabs work through it

### Requirement: A server started by a relay outlives the connection
A core or holder started by a relay SHALL keep running after the SSH login or `exec` that started it ends, on hosts where the user's processes are allowed to outlive their logins. It SHALL NOT be placed where the end of the user's last login would stop it. A server that ends with the container it runs in is expected.

#### Scenario: Last SSH login ends
- **WHEN** a relay started over SSH on a systemd host without user lingering started the server, a `detach` session runs `sleep 1000`, and the SSH connection ends while the user has no other login
- **THEN** the `sleep` keeps running and is listed as detached at the next connect

### Requirement: A reconnecting client replaces its old connection
When a client connects with the token of a client that is still connected, the core SHALL close the older connection and give its sessions to the new one, replaying each, without their close policies applying.

#### Scenario: Half-dead connection
- **WHEN** a client's connection stops delivering data without closing, and the client connects again with the same token while the core still holds the old connection
- **THEN** the old connection is closed, and the new one receives a replay of every session the old one was attached to, with the same pids

### Requirement: Clients may ask for a reconnect grace
A client SHALL be able to ask, when it connects, that its `kill` sessions wait for it for a stated time, at most 10 minutes, after its connection ends. Such a session SHALL end when that time passes without the client reconnecting with the same token. If the client reconnects in time, the session SHALL be given back to it as after a core restart. Without the request, `kill` sessions SHALL end as soon as the connection ends.

#### Scenario: Back within the grace
- **WHEN** a client that asked for a 300 s grace loses its connection while its `kill` session runs `htop`, and reconnects after 60 s
- **THEN** it receives `htop`'s replay, with the same pid

#### Scenario: Not back
- **WHEN** a client that asked for a 300 s grace loses its connection and doesn't reconnect
- **THEN** its `kill` sessions end about 300 s later

### Requirement: Process info for clients that ask
A client that negotiated client protocol 2 and asks for it SHALL receive, for each session it is attached to, the name and working directory of the session's foreground process: once after it attaches or starts the session, and again whenever either changes. The core SHALL look at most once a second per session, and only at sessions that have had output or input since its last look. A core on Windows MAY report no process info.

#### Scenario: Foreground process changes
- **WHEN** a client that asked for process info runs `htop` in its session's shell
- **THEN** within about a second it receives process info naming `htop`

#### Scenario: Idle session
- **WHEN** a session has had neither output nor input for a minute
- **THEN** its client receives no process info in that minute

### Requirement: The session list tells whether an owner may return
For each detached session, the session list SHALL report whether it still has an owner (its last client left without detaching it) and how long it has been detached.

#### Scenario: Owner dropped
- **WHEN** a client's connection to a `detach` session ends without a `DETACH`, and another client lists sessions 30 s later
- **THEN** the session is listed as detached, owned, and detached for about 30 s

#### Scenario: Explicit detach
- **WHEN** a client detaches a session and another client lists sessions
- **THEN** the session is listed as detached and not owned

### Requirement: Programs start with default signal handling
A holder SHALL start each session's program with every signal's default disposition and an empty signal mask, whatever the holder inherited. Hanging up a session SHALL end a program that hasn't changed its own signal handling with SIGHUP, even when the server was started with SIGHUP ignored.

#### Scenario: Server started under nohup
- **WHEN** the server was started with SIGHUP ignored, a session runs `cat`, and the session is closed
- **THEN** `cat` is ended by SIGHUP, and is gone well before the holder's SIGKILL would be sent

## MODIFIED Requirements

### Requirement: Protocol versions
A client and the core SHALL agree on a protocol version when they connect, and so SHALL the core and each holder. A core SHALL speak every client protocol version and every holder protocol version from the first terminal-server release's up to its own. Client protocol 2 adds process info. A client SHALL refuse a core that speaks none of its versions, with an error naming both versions. A holder speaking an older holder protocol that can't store a session's changed owner or policy SHALL keep working; only those changes are lost if the core restarts before the holder drains.

#### Scenario: Older client, newer core
- **WHEN** a Brindle from an older release connects to a core from a newer release
- **THEN** its tabs open and work through that core

#### Scenario: Newer core, older holder
- **WHEN** a newer core adopts a holder started by an older release
- **THEN** that holder's sessions keep working: input, output, resize, exit and kill

#### Scenario: Newer client, older core
- **WHEN** a client that speaks protocols 1 and 2 connects to a core that speaks only protocol 1
- **THEN** they agree on protocol 1, the client's sessions work, and it receives no process info
