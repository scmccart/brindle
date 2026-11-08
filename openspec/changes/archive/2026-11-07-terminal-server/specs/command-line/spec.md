# Spec Delta

## ADDED Requirements

### Requirement: Terminal server directory for tests
When the environment variable `BRINDLE_SERVER_DIR` is set and not empty, Brindle and `brindle-remote` SHALL use it as the terminal server's directory, as defined by the `terminal-server` capability. Programs in tabs inherit it, so `brindle-remote` commands run in a tab manage that same server.

#### Scenario: Isolated server
- **WHEN** a test runs Brindle with `BRINDLE_SERVER_DIR=/tmp/case-1`
- **THEN** its local tabs run in a server whose socket is in `/tmp/case-1`, and the user's own server is not touched

#### Scenario: Managing from a tab
- **WHEN** the user runs `brindle-remote list` in a local tab
- **THEN** it lists the sessions of the server that tab runs in, that tab's own session included
