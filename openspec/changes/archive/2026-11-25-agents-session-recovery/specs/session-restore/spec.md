# Spec Delta

## ADDED Requirements

### Requirement: Agent zones whose agent ended are not restored
An agent zone whose agent has exited or was lost SHALL NOT be recorded for session restore, even while its session is kept for continuing. Quitting SHALL leave no such session on the terminal server, and the next launch SHALL NOT bring the zone back; its tab's other zones are recorded as usual.

#### Scenario: Quit with an exited agent
- **WHEN** an agent zone's agent has exited with a conversation that could be continued, and the user quits and relaunches
- **THEN** the zone isn't restored, and `brindle-remote list` doesn't show its session

#### Scenario: Split tab with an exited agent
- **WHEN** a tab holds a running agent zone beside one whose agent exited, and the user quits and relaunches
- **THEN** the tab comes back with the running agent's zone alone
