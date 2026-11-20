## ADDED Requirements

### Requirement: An endpoint's session sizes are bounded
Brindle SHALL use a session size that an endpoint lists only when it is within Brindle's grid limits, from 10 columns and 1 row up to 2048 columns and 1024 rows. A listed size outside them SHALL count as no size, so the session's tab starts at the default size and takes the size of its zone once laid out. No grid Brindle creates or resizes, for any session, SHALL exceed those limits.

#### Scenario: Endpoint lists a huge session
- **WHEN** a remote endpoint with `connect_at_launch` lists a detached session at 65535×65535 and Brindle launches
- **THEN** Brindle opens the session's tab at the default size and keeps running, with its local tabs

#### Scenario: Endpoint lists an ordinary size
- **WHEN** a remote endpoint lists a detached session at 120×40 and the user attaches it
- **THEN** the replay is fed at 120×40 before the tab takes its zone's size
