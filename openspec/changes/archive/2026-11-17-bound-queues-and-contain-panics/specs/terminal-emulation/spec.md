## MODIFIED Requirements

### Requirement: A session lost for good is reported in the tab
When Brindle can't get a session back, because its holder ended, the core no longer has it, or no core is reachable within a bounded time, the tab SHALL keep its last screen and show a line saying the session was lost and why, instead of closing. A tab whose session was lost, exited or let go after an emulator failure SHALL NOT act on a session that later got the same id: its input, resizes and close SHALL NOT reach that session.

#### Scenario: Holder killed
- **WHEN** the holder of a local tab's session is killed with SIGKILL
- **THEN** the tab stays open with its last screen and a line saying the session was lost

#### Scenario: Closing a lost tab after its id was reused
- **WHEN** the holder and then the core of a local tab's session are killed with SIGKILL, the tab says its session was lost, the user opens a new tab whose session gets the same id, and then types into the lost tab and closes it
- **THEN** the new tab's program receives none of that input and keeps running

## ADDED Requirements

### Requirement: An emulator failure is confined to its tab
When Brindle's emulator fails on a session's output, Brindle SHALL let go of that session without ending it, leaving it detached and running, and the tab SHALL stay open, showing in place of its screen that Brindle can't show the session and that it can be attached again. Closing that tab SHALL leave the session running. The other tabs, on the same endpoint too, SHALL keep working.

#### Scenario: Emulator fails on one zone's output
- **WHEN** a window has two local zones and Brindle's emulator fails on the output of one zone's session
- **THEN** that zone stays open and says Brindle can't show the session any more and that it can be attached again, the session is listed as detached with its program still running, and the other zone keeps showing its program's output and taking input

#### Scenario: Closing a tab whose emulator failed
- **WHEN** Brindle's emulator failed on a tab's output and the user closes that tab
- **THEN** the session stays listed as detached and its program keeps running
