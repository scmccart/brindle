## ADDED Requirements

### Requirement: Remote zone split
Splitting a zone whose profile has a remote target SHALL create the new PTY on the same SSH host or container and SHALL inherit the active remote working directory. A local directory SHALL NOT be passed to a remote profile, and a directory from one remote target SHALL NOT be passed to another target.

#### Scenario: Split on the same SSH host
- **WHEN** an SSH-backed zone on host `A8` is in `/srv/app` and the user selects Split Zone Right
- **THEN** the new zone connects to `A8` and starts in `/srv/app`

#### Scenario: Open remote profile from a local tab
- **WHEN** the active local tab is in `C:\src` and the user opens an SSH profile
- **THEN** Brindle does not send `C:\src` as the remote working directory
