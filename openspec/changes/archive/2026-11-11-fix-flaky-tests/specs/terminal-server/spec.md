## MODIFIED Requirements

### Requirement: Clients reconnect after a core restart
When the core restarts, its clients' connections end. A client that reconnects within the reconnect grace period and identifies itself SHALL get back every session it was attached to when its connection ended, without the close policy applying, and SHALL receive each one's replay before live output resumes. Sessions the client had explicitly detached, or that another client attached since, SHALL NOT be given back to it. This SHALL hold across core restarts and upgrades: the server keeps each session's owner and policy as they last changed. An attach's change of owner and policy SHALL be kept from the moment the attaching client receives the start of its replay: a core restart after that, however soon, SHALL give the session back to that client and not to its previous owner.

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

#### Scenario: Core killed right after an attach
- **WHEN** client B attaches a session that client A started, and the core is killed with SIGKILL as soon as B receives the start of its replay
- **THEN** after both reconnect to the next core, B gets the session back and A doesn't

