# Spec Delta

## ADDED Requirements

### Requirement: Contexts survive attach
The server SHALL keep each session's OSC 3008 contexts with the same rules as the client, and an attaching or reconnecting client SHALL replace the session's contexts with those in its replay. A client attached to a server that predates context tracking SHALL show no contexts for that session until a program starts one.

#### Scenario: Attach inside run0
- **WHEN** a detached session's shell runs inside a `run0` elevation, and a client attaches
- **THEN** the session's tab shows the red `root` badge

#### Scenario: Ended while disconnected
- **WHEN** the client loses its connection while a zone is inside a container context, the context ends meanwhile, and the client reconnects
- **THEN** the zone shows no badge
