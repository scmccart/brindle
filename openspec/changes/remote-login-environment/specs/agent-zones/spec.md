## ADDED Requirements

### Requirement: An agent started again gets a new start's environment
An agent started again in its session, after a terminal login or by Continue conversation, SHALL run with the environment that a new zone's start of the same profile on the same endpoint gets, as `remote-connections` and this capability define it.

#### Scenario: Remote restart after a terminal login
- **WHEN** an agent profile on SSH host `A8` offers a `terminal` login, the user runs it in the login zone, and it exits with status 0
- **THEN** the agent started again has the same `HOME`, `PATH` and login variables as the first agent had, and the zone takes prompts

#### Scenario: Local continue
- **WHEN** a local agent profile sets `env = { MODEL = "small" }`, its agent exited, and the user presses Continue conversation
- **THEN** the agent started again runs with `MODEL=small` and Brindle's own environment without the variables Brindle sets only for terminals, as a new zone's agent does
