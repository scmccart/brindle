# Spec Delta

## ADDED Requirements

### Requirement: Sessions keep their client's layout
A client SHALL be able to give a session a layout, data the server doesn't interpret describing where the session sits: with `DETACH`, and, on protocol 3, at any time while it has the session attached. The server SHALL keep the latest layout across core restarts and upgrades and when the client's connection ends, and report it in the session list while the session is detached. It SHALL drop it on attach, and SHALL NOT keep one over 64 KiB.

#### Scenario: Layout listed after a detach
- **WHEN** a client detaches a session with a layout and another client lists sessions
- **THEN** that session's entry has the same layout

#### Scenario: Layout updated while attached
- **WHEN** a client on protocol 3 sets a session's layout twice while attached, and is then killed with SIGKILL
- **THEN** listing sessions reports the second layout for it

#### Scenario: Layout survives a core restart
- **WHEN** a session was detached with a layout, its core is killed with SIGKILL, and a new core adopts the holder
- **THEN** listing sessions from the new core reports the same layout

#### Scenario: Attach clears the layout
- **WHEN** a session detached with a layout is attached by a client, which then detaches it without a layout
- **THEN** listing sessions reports no layout for it

#### Scenario: Only the attached client sets it
- **WHEN** a client sets the layout of a session another client has attached
- **THEN** it gets an error naming the session, and the session's layout is unchanged

#### Scenario: Oversized layout
- **WHEN** a client detaches a session with a 100 KiB layout
- **THEN** the session is detached and listed with no layout

#### Scenario: Older client
- **WHEN** a client that predates layouts detaches a session
- **THEN** the session is detached as before and listed with no layout

## MODIFIED Requirements

### Requirement: Protocol versions
A client and the core SHALL agree on a protocol version when they connect, and so SHALL the core and each holder. A core SHALL speak every client protocol version and every holder protocol version from the first terminal-server release's up to its own. Client protocol 2 adds process info. Client protocol 3 adds layout updates for attached sessions. A client SHALL refuse a core that speaks none of its versions, with an error naming both versions. A holder speaking an older holder protocol that can't store a session's changed owner or policy SHALL keep working; only those changes are lost if the core restarts before the holder drains.

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
