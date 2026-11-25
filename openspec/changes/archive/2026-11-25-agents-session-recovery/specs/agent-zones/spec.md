# Spec Delta

## MODIFIED Requirements

### Requirement: An agent that exits
When an agent exits, its zone SHALL stay open and say that the agent exited, with its exit status and the end of its standard error, and the prompt input SHALL be unavailable, unless the agent had already failed for its ACP version (see "An agent that doesn't speak ACP version 1"). When the conversation can be continued the zone SHALL offer to continue it (see "Continuing a conversation"). The zone SHALL close only when the user closes it, and SHALL NOT be recorded for session restore.

#### Scenario: Agent crashes
- **WHEN** an agent writes `fatal: no model` to its standard error and exits with status 1
- **THEN** its zone stays open, saying the agent exited with status 1 and showing `fatal: no model`

#### Scenario: Agent crashes with a conversation to continue
- **WHEN** an agent that offered `session/resume` and had a ready ACP session exits with status 1
- **THEN** its zone says the agent exited with status 1, and shows a Continue conversation button where the prompt input's reason is

#### Scenario: Agent lost with its holder
- **WHEN** the holder running an agent that offered `session/load` is killed
- **THEN** the zone says the agent was lost with its terminal server's holder, and offers to continue the conversation

### Requirement: A disconnected endpoint
While the connection to an agent zone's endpoint is down, the zone SHALL say it is reconnecting and SHALL NOT send prompts, keeping the prompt input's text. When the session can't be got back, the zone SHALL say the session was lost, and stay open until the user closes it; when the zone knows the session's conversation, it SHALL offer to continue it in a new session.

#### Scenario: Reconnected
- **WHEN** the link to an SSH endpoint drops while its agent zone is idle, and comes back
- **THEN** the zone said it was reconnecting, then shows the conversation as before, and a prompt sent afterwards reaches the agent

#### Scenario: Lost after the agent exited
- **WHEN** an agent zone's agent has exited, and the terminal server's core is killed and replaced before the user continues it
- **THEN** the zone says the session was lost, and still offers to continue the conversation

### Requirement: Sending a held prompt again, or discarding it
The held row's send-again button and the `agent_resend` action SHALL send the held prompt: at once when the agent is ready and no turn runs, else queued again behind the running turn. Its discard button and `agent_unqueue` SHALL discard it. A new prompt sent while one is held and no turn runs SHALL be sent, leaving the held one held. When the agent isn't ready, send-again SHALL be unavailable; once the session has ended, discard SHALL only clear the row.

#### Scenario: Send again when idle
- **WHEN** `next` is held, no turn runs, and the user presses the row's send-again button
- **THEN** the row is gone, and the conversation shows the user message `next`

#### Scenario: Send again during a turn
- **WHEN** `next` is held, the user sends `other`, and while that turn runs presses send-again
- **THEN** the row shows `next` as queued, and it is sent when the `other` turn ends normally

#### Scenario: A new prompt beside a held one
- **WHEN** `next` is held, no turn runs, and the user types `other` and presses enter
- **THEN** `other` is sent, and `next` stays held

#### Scenario: Held when the agent exits
- **WHEN** `next` is queued and the agent exits
- **THEN** the zone says the agent exited, the row shows `next` as not sent because the agent stopped, its send-again button is unavailable, and discard clears the row

#### Scenario: Send again after continuing
- **WHEN** `next` was held because the agent stopped, and the user continues the conversation and the agent is ready
- **THEN** the row's send-again button is available, `next` has not been sent, and pressing the button sends it

## ADDED Requirements

### Requirement: Continuing a conversation
A zone whose agent exited or was lost with a conversation it can continue SHALL show a Continue conversation button, and the `agent_continue` action (Continue Agent Conversation) SHALL do the same. Either SHALL start the zone's agent profile's command again with its current arguments and environment on the same session, or in a new session when the server lost it, and the zone SHALL say the agent is starting until it is ready. Nothing SHALL be continued without the user asking.

#### Scenario: Continue after a crash
- **WHEN** an agent zone's agent exited after one turn, and the user presses Continue conversation
- **THEN** the zone says the agent is starting, then shows the earlier turn, and the prompt input takes a prompt that reaches the agent

#### Scenario: From the palette
- **WHEN** an agent zone's agent exited and the user runs Continue Agent Conversation from the palette
- **THEN** the conversation is continued as from the button

#### Scenario: Never on its own
- **WHEN** an agent zone's agent exits and a minute passes
- **THEN** the agent hasn't been started again

#### Scenario: Replay not shown
- **WHEN** a zone is continued with an agent that offers only `session/load`
- **THEN** the conversation shows the earlier turns once, then a notice that the conversation continued and the agent's replay wasn't shown

#### Scenario: Continued in a new session
- **WHEN** a zone says its session was lost, and the user continues the conversation
- **THEN** the zone shows a new session's conversation, as the agent replayed it, after a notice that it continues an earlier conversation

### Requirement: A conversation that can't be continued
A zone whose agent ended SHALL NOT offer to continue when the agent offered neither `session/load` nor `session/resume`, when it never gave an ACP session, or when the zone's agent profile no longer exists; the prompt input's reason SHALL say which. A continue that the agent refuses SHALL leave the zone showing that the agent can't be used, with the agent's message.

#### Scenario: Agent without load or resume
- **WHEN** an agent that offered neither `session/load` nor `session/resume` exits
- **THEN** the zone has no Continue conversation button, and the reason says the agent can't continue conversations

#### Scenario: Profile deleted
- **WHEN** an agent zone's agent exited and its profile has since been removed from the config
- **THEN** the zone has no Continue conversation button, and the reason names the missing profile

#### Scenario: Agent refuses
- **WHEN** the user continues a zone and the agent answers `session/resume` with an error `no such session`
- **THEN** the zone says the agent can't be used, shows `no such session`, and keeps the earlier conversation
