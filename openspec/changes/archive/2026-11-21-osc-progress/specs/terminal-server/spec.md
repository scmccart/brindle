# Spec Delta

## ADDED Requirements

### Requirement: Progress survives attach
The server SHALL keep each session's progress (`OSC 9 ; 4`) with the same lifetime rules as the client, and an attaching or reconnecting client SHALL receive it with the session's replay. A client attached to a server that predates progress tracking SHALL show no progress for that session until the program reports again.

#### Scenario: Progress after attach
- **WHEN** a program in a detached session reported normal at 60%, and a client attaches
- **THEN** the session's tab shows a bar 60% of its width long

#### Scenario: Core restart
- **WHEN** a session's program reported paused at 20% and the core is restarted
- **THEN** after the client reconnects, the tab shows a yellow bar 20% of its width long
