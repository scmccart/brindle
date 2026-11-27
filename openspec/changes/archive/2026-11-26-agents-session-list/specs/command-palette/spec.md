## ADDED Requirements

### Requirement: Conversation commands in the palette
The command palette SHALL list `Agent Conversations…` while an agent zone has focus, and `Continue Agent Conversation…` in every pane while the config has an agent profile. Neither SHALL have a default key. The command that continues the conversation of a zone whose agent stopped SHALL be named `Continue This Conversation`.

#### Scenario: In a shell
- **WHEN** a shell has focus, the config has an agent profile, and the user opens the command palette
- **THEN** it lists "Continue Agent Conversation…" and not "Agent Conversations…"

#### Scenario: In an agent zone
- **WHEN** an agent zone has focus and the user opens the command palette
- **THEN** it lists "Agent Conversations…" and "Continue Agent Conversation…"

#### Scenario: No agent profile
- **WHEN** the config has no agent profile
- **THEN** the palette lists neither

#### Scenario: A stopped agent's own conversation
- **WHEN** an agent zone's agent exited with a conversation it can continue, and the user opens the command palette
- **THEN** it lists "Continue This Conversation", which continues that zone's conversation
