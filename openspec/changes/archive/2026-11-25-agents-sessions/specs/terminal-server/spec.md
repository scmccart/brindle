# Spec Delta

## MODIFIED Requirements

### Requirement: A core and its holders
The server SHALL run as two kinds of process from the same binary. Holders own the sessions' PTYs, and the plain pipes of agent sessions: they are the programs' parent, carry their I/O, and parse nothing. The core runs the emulators, the ACP client of agent sessions, the client protocol and the management commands, and drives the holders. Clients SHALL talk only to the core.

#### Scenario: Programs belong to a holder
- **WHEN** a client starts a session
- **THEN** the program's parent is a holder process, not the core and not the client

#### Scenario: Holders serve many sessions
- **WHEN** a client starts three sessions
- **THEN** all three run under the same holder

#### Scenario: Agents and terminals share a holder
- **WHEN** a client starts a terminal session and an agent session
- **THEN** both programs' parent is the same holder

### Requirement: Protocol versions
A client and the core SHALL agree on a protocol version when they connect, and so SHALL the core and each holder. A core SHALL speak every client protocol version and every holder protocol version from the first terminal-server release's up to its own. Client protocol 2 adds process info. Client protocol 3 adds layout updates for attached sessions. Client protocol 4 adds agent sessions. Holder protocol 3 adds sessions on plain pipes and their standard error. A client SHALL refuse a core that speaks none of its versions, with an error naming both versions. A holder speaking an older holder protocol that can't store a session's changed owner or policy SHALL keep working; only those changes are lost if the core restarts before the holder drains.

#### Scenario: Older client, newer core
- **WHEN** a Brindle from an older release connects to a core from a newer release
- **THEN** its tabs open and work through that core

#### Scenario: Newer core, older holder
- **WHEN** a newer core adopts a holder started by an older release
- **THEN** that holder's sessions keep working: input, output, resize, exit and kill

#### Scenario: Newer client, older core
- **WHEN** a client that speaks protocols 1 and 2 connects to a core that speaks only protocol 1
- **THEN** they agree on protocol 1, the client's sessions work, and it receives no process info

#### Scenario: Layout updates need protocol 3
- **WHEN** a client that speaks protocols 1 to 3 connects to a core that speaks protocols 1 and 2
- **THEN** they agree on protocol 2, the client sends no layout updates, and its sessions still get their layouts when it detaches them

#### Scenario: Agent sessions need protocol 4
- **WHEN** a client that speaks protocols 1 to 4 connects to a core that speaks protocols 1 to 3
- **THEN** they agree on protocol 3, its terminal sessions work, and it can start no agent session

#### Scenario: Pipe sessions need holder protocol 3
- **WHEN** a core that speaks holder protocols 1 to 3 starts an agent session
- **THEN** it starts it on a holder that negotiated holder protocol 3, starting a holder of its own build if it has none

### Requirement: Management commands
`brindle-remote` SHALL provide `list`, `kill <id>`, `kill --all`, `status` and `stop` for the server of the current server directory. `list` and `status` SHALL accept `--json`. When no server is running, `list` prints nothing and `status` says so, and both exit successfully. `kill` with an unknown id SHALL fail with an error naming the id. `list` SHALL show agent sessions as well as terminal sessions, telling them apart, and `kill` and `stop` SHALL end agent sessions as they end terminals.

#### Scenario: List
- **WHEN** a server has two sessions and the user runs `brindle-remote list`
- **THEN** one line per session shows its id, pid, attached or detached state, policy, age, title, command and working directory

#### Scenario: List an agent session
- **WHEN** a server has a terminal session and an agent session waiting on a permission request, and the user runs `brindle-remote list` and `brindle-remote list --json`
- **THEN** the agent session's line shows `agent: blocked` in the title's place, and its JSON entry has an `agent` object with activity `blocked`; the terminal session's line and entry are as before

#### Scenario: Kill one
- **WHEN** the user runs `brindle-remote kill <id>` for an attached session
- **THEN** that session's program is hung up and its client receives the session's exit

#### Scenario: Status
- **WHEN** the user runs `brindle-remote status --json`
- **THEN** it prints the core's version, protocol versions, pid, uptime and client count, and for each holder its pid, build, session count and whether it is draining

#### Scenario: Stop
- **WHEN** the user runs `brindle-remote stop`
- **THEN** every session is hung up, every client receives its sessions' exits, and the core and holders exit
