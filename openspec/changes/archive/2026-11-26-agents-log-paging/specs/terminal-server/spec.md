## MODIFIED Requirements

### Requirement: Protocol versions
A client and the core SHALL agree on a protocol version when they connect, and so SHALL the core and each holder. A core SHALL speak every client protocol version and every holder protocol version from the first terminal-server release's up to its own. Client protocol 2 adds process info. Client protocol 3 adds layout updates for attached sessions. Client protocol 4 adds agent sessions, whose conversations are sent a page at a time with older entries fetched on demand. Holder protocol 3 adds sessions on plain pipes, their standard error, and checkpoints appended to. A client SHALL refuse a core that speaks none of its versions, with an error naming both versions. A holder speaking an older holder protocol that can't store a session's changed owner or policy SHALL keep working; only those changes are lost if the core restarts before the holder drains.

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

#### Scenario: Appended checkpoints go only to a holder of the core's build
- **WHEN** a core checkpoints an agent session that a draining holder of another build holds
- **THEN** it writes the whole checkpoint, never a segment to append
