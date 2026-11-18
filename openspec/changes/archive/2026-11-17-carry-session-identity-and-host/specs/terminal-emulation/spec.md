## MODIFIED Requirements

### Requirement: A session lost for good is reported in the tab
When Brindle can't get a session back, because its holder ended, the core no longer has it, or no core is reachable within a bounded time, the tab SHALL keep its last screen and show a line saying the session was lost and why, instead of closing. A tab whose session was lost or exited SHALL NOT act on a session that later got the same id: its input, resizes and close SHALL NOT reach that session.

#### Scenario: Holder killed
- **WHEN** the holder of a local tab's session is killed with SIGKILL
- **THEN** the tab stays open with its last screen and a line saying the session was lost

#### Scenario: Closing a lost tab after its id was reused
- **WHEN** the holder and then the core of a local tab's session are killed with SIGKILL, the tab says its session was lost, the user opens a new tab whose session gets the same id, and then types into the lost tab and closes it
- **THEN** the new tab's program receives none of that input and keeps running
