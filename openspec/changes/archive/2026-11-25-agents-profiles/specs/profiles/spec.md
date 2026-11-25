# Spec Delta

## ADDED Requirements

### Requirement: Agent profiles
A profile SHALL be able to set `type`: `"terminal"` (the default) or `"agent"`. An agent profile runs its command as an ACP agent in an agent zone (see `agent-zones`) instead of a terminal. It SHALL name a command; one that doesn't is a config error naming the profile. Its arguments, directory, environment, theme, font size and remote target SHALL keep their meaning. Any other `type` value is a config error.

#### Scenario: Agent profile
- **WHEN** a profile sets `type = "agent"`, `command = "agent-acp"` and `args = ["--stdio"]`
- **THEN** opening it opens an agent zone whose agent runs `agent-acp --stdio`

#### Scenario: No command
- **WHEN** a profile named `Agent` sets `type = "agent"` and no command
- **THEN** the config line reports that the agent profile `Agent` needs a command, and Brindle starts with the default settings

#### Scenario: Unknown type
- **WHEN** a profile sets `type = "chat"`
- **THEN** the config line reports the profile's `type` as invalid, naming `terminal` and `agent`

#### Scenario: Terminal by default
- **WHEN** a profile sets no `type`
- **THEN** its tabs are terminals, as before

### Requirement: Agent profile description
The description the picker and the profile menu show for an agent profile SHALL read `agent · <command>`, after its remote target when it has one.

#### Scenario: Local agent
- **WHEN** an agent profile runs `agent-acp`
- **THEN** its picker row and menu item read `agent · agent-acp`

#### Scenario: Remote agent
- **WHEN** an agent profile on SSH host `A8` runs `agent-acp`
- **THEN** its description reads `ssh:A8 · agent · agent-acp`
