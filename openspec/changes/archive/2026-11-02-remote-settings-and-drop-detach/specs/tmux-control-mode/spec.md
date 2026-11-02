## ADDED Requirements

### Requirement: Dropped remote connection detaches the session
A remote control-mode session SHALL detect that its connection is dead, whether the transport closed or went silent without closing, within a bounded time. It SHALL then end the session as a detach with a reason that says the connection was lost, close its transport, and make a bounded best-effort attempt to detach its own tmux client on the remote server over a new connection. The attempt SHALL NOT delay or block the window, and its failure SHALL NOT be reported as an error. The remote tmux session and its windows SHALL be left running.

#### Scenario: Connection cut silently
- **WHEN** a remote control session's connection stops delivering anything without closing, and does not recover
- **THEN** within the detection limit the session is detached with a connection-lost reason and its panes are marked exited

#### Scenario: Remote client released
- **WHEN** a dropped session's host is reachable again by a new connection
- **THEN** the session's old tmux client is detached on the server and `list-clients` no longer shows it

#### Scenario: Host unreachable
- **WHEN** the connection is dropped and the host stays unreachable
- **THEN** the detach attempt gives up after its time limit, the window stays responsive, and no error is shown beyond the connection-lost reason

#### Scenario: Slow but live link
- **WHEN** the connection is slow but answers within the limit
- **THEN** the session stays attached

#### Scenario: Local sessions unaffected
- **WHEN** a local control session is attached
- **THEN** no heartbeat is sent and no connection-lost detection applies
