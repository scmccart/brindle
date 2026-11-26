## MODIFIED Requirements

### Requirement: Current working directory tracking
Remote sessions SHALL track the remote working directory through `/proc/<pid>/cwd`, reported by the host when it changes, and through OSC 7 reports. When splitting a zone or launching a new tab from an active remote zone, a new session on the same host SHALL inherit the remote directory.

#### Scenario: Remote zone split inherits remote CWD
- **WHEN** the user is in `/var/log` on a remote host and triggers `Split Terminal Right`
- **THEN** the newly opened split zone launches in `/var/log` on the same remote host
