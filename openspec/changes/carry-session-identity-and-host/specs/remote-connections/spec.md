## MODIFIED Requirements

### Requirement: Sessions come back after a reconnect
On reconnecting, every session Brindle still owns on that endpoint SHALL come back with its replay, then live output. Input typed while disconnected SHALL be delivered after the reconnect. A session SHALL be shown as lost only when the reconnected endpoint no longer has it. The endpoint SHALL count as having it only when it lists the session's id with the session's key. An attach cut off by the drop SHALL NOT be answered with a different session.

#### Scenario: Typing while disconnected
- **WHEN** the user types `ls` and enter in a remote tab while its connection is down
- **THEN** after the reconnect the shell runs `ls` once

#### Scenario: Session ended while disconnected
- **WHEN** a remote session's program is ended with `brindle-remote kill` on its host while Brindle is disconnected from that host
- **THEN** after the reconnect the tab says the session was lost, and stays open

#### Scenario: Id given to another session while disconnected
- **WHEN** `A8` reboots while Brindle is disconnected from it, and before Brindle reconnects another Brindle opens a tab on `A8` whose session gets the same id as a tab's session here
- **THEN** after the reconnect the tab here says the session was lost and stays open, and the other Brindle's tab keeps running, unaffected

#### Scenario: Attach cut off, id given to another session
- **WHEN** the user attaches a detached session on `A8` from "Attach Session…", the connection drops before its screen arrives, and when Brindle reconnects `A8` lists that id for a different session with no client
- **THEN** the new tab says `A8` no longer has the session, and shows nothing of the other session, which stays detached
