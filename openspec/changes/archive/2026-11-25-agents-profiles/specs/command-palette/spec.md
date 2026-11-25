# Spec Delta

## ADDED Requirements

### Requirement: Agent sessions in the sessions palette
The sessions palette SHALL list detached agent sessions with the terminals of their endpoint, each row marked as an agent with its activity in place of a title, and its command and directory. Enter on such a row SHALL attach it as a new agent tab, with the profile "Launch reattaches detached agent sessions" in `session-restore` gives it, and `picker_kill` SHALL end it.

#### Scenario: Attach a detached agent
- **WHEN** an agent session waiting on a permission request is detached, and the user runs "Attach Session…"
- **THEN** its row is marked as an agent that is blocked, and enter on it opens an active agent tab showing the conversation and the pending request

#### Scenario: Kill a detached agent
- **WHEN** the user highlights a detached agent session's row and presses ctrl-shift-delete
- **THEN** the agent ends and its row disappears
