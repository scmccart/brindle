# Spec Delta

## MODIFIED Requirements

### Requirement: A session lost for good is reported in the tab
When Brindle can't get a session back, because its holder ended, the core no longer has it, or no core is reachable within a bounded time, the tab SHALL keep its last screen and show a line saying the session was lost and why, instead of closing. When no core was reachable, the reason SHALL include the last error Brindle met while trying. A tab whose session was lost, exited or let go after an emulator failure SHALL NOT act on a session that later got the same id: its input, resizes and close SHALL NOT reach that session.

#### Scenario: Holder killed
- **WHEN** the holder of a local tab's session is killed with SIGKILL
- **THEN** the tab stays open with its last screen and a line saying the session was lost

#### Scenario: Closing a lost tab after its id was reused
- **WHEN** the holder and then the core of a local tab's session are killed with SIGKILL, the tab says its session was lost, the user opens a new tab whose session gets the same id, and then types into the lost tab and closes it
- **THEN** the new tab's program receives none of that input and keeps running

#### Scenario: No core comes back
- **WHEN** a local tab's core is killed and every attempt to start another fails, for longer than Brindle waits
- **THEN** the tab's lost line says no terminal server came back in that time, followed by the error from the last attempt

## ADDED Requirements

### Requirement: Failures about a running session are shown in its tab
When the terminal server reports an error about a session a tab shows, or Brindle can't send a message to that session, the tab SHALL show one dim line saying what failed and why. The text SHALL be shown without any control characters it carries. A line that would repeat the previous one, with no output from the session in between, SHALL NOT be added again. A line due while the tab's screen is still being replayed SHALL be shown after the replay.

#### Scenario: Holder fails on a session's input
- **WHEN** a local tab's holder fails while handling that session's input, and the session keeps running
- **THEN** the tab shows a dim line naming the terminal server's error, and keeps taking input

#### Scenario: Repeated error
- **WHEN** the terminal server reports the same error about a session three times with no output between
- **THEN** the tab shows that line once
