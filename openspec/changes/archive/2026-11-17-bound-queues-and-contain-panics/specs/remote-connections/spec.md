## MODIFIED Requirements

### Requirement: Sessions come back after a reconnect
On reconnecting, every session Brindle still owns on that endpoint SHALL come back with its replay, then live output. Input typed while disconnected SHALL be delivered after the reconnect, unless refused under the input bound. A session SHALL be shown as lost only when the reconnected endpoint no longer has it. The endpoint SHALL count as having it only when it lists the session's id with the session's key. An attach cut off by the drop SHALL NOT be answered with a different session.

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

## ADDED Requirements

### Requirement: Input waiting to reach a session is bounded
Input waits in Brindle while its session starts and while its connection is down or slow. Input typed into a tab while 4 MiB or more of that tab's earlier input is waiting SHALL NOT be sent, then or later, until less than 1 MiB is waiting. The first input refused after input was last taken SHALL make the tab say that input was not sent and why. Input taken before the bound was reached SHALL still be delivered, in order.

#### Scenario: Input past the bound while disconnected
- **WHEN** a remote tab's connection is down, more than 4 MiB of input pasted into it is waiting to be sent, and the user then types `pwd` and enter
- **THEN** the tab says, once, that input was not sent, and after the reconnect the pasted input reaches the shell and `pwd` never does

#### Scenario: Input taken again once the backlog drains
- **WHEN** input was refused in a tab because 4 MiB was waiting, the connection comes back, and less than 1 MiB of that tab's input is still waiting when the user types `pwd` and enter
- **THEN** the shell runs `pwd` once

#### Scenario: A large paste while connected
- **WHEN** a tab's connection is up, less than 4 MiB of its input is waiting, and the user pastes 6 MiB
- **THEN** the whole paste reaches the program, and the tab says nothing about refused input
