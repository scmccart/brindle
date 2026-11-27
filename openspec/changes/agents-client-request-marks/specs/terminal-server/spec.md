## MODIFIED Requirements

### Requirement: Session I/O
For an attached session, the server SHALL carry the client's input and resizes to the PTY, stream its output to the client in order, and report the program's exit with its exit status. One session's slow or stalled client SHALL NOT delay other sessions. While more than its queue's high mark waits to be sent to one client, the server SHALL take no further request from that client, and SHALL take them again, in order and none lost, once that client has read its queue down to the low mark; a client whose connection failed SHALL still have the requests it sent before handled.

#### Scenario: Exit status
- **WHEN** the program in an attached session runs `exit 3`
- **THEN** the client receives the session's exit with status 3, and the session is no longer listed

#### Scenario: A stalled client
- **WHEN** one client stops reading while its program floods output
- **THEN** sessions of other clients keep streaming output without delay

#### Scenario: A client that asks without reading
- **WHEN** one client sends page requests for an agent session's log, one after another, and reads none of the answers
- **THEN** the server stops taking that client's requests once a megabyte of answers waits for it, its memory stays bounded, and other clients' requests are answered without delay

#### Scenario: Requests resume once the client reads
- **WHEN** that client then reads what waits for it
- **THEN** the server answers its remaining requests in the order they were sent
