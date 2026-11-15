# Spec Delta

## ADDED Requirements

### Requirement: Detached sessions keep their client's layout
A client that detaches a session SHALL be able to give a layout with it: data the server doesn't interpret, describing where the session sat. The server SHALL keep it while the session stays detached, across core restarts and upgrades, and SHALL report it in the session list. It SHALL drop the layout when a client attaches the session, and SHALL refuse to keep one over 64 KiB.

#### Scenario: Layout listed after a detach
- **WHEN** a client detaches a session with a layout and another client lists sessions
- **THEN** that session's entry has the same layout

#### Scenario: Layout survives a core restart
- **WHEN** a session was detached with a layout, its core is killed with SIGKILL, and a new core adopts the holder
- **THEN** listing sessions from the new core reports the same layout

#### Scenario: Attach clears the layout
- **WHEN** a session detached with a layout is attached by a client, which then detaches it without a layout
- **THEN** listing sessions reports no layout for it

#### Scenario: Oversized layout
- **WHEN** a client detaches a session with a 100 KiB layout
- **THEN** the session is detached and listed with no layout

#### Scenario: Older client
- **WHEN** a client that predates layouts detaches a session
- **THEN** the session is detached as before and listed with no layout
