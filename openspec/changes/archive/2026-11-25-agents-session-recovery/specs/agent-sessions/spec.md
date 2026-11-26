# Spec Delta

## MODIFIED Requirements

### Requirement: The agent's exit is reported
When an agent exits, the client SHALL receive a transcript event with its exit status and the last 16 KiB of its standard error; after a version mismatch the standard error is added as a notice instead, and the failed state stays. When the conversation can be continued (see "A session whose agent ended is kept for its conversation") the session SHALL stay; otherwise the session's exit SHALL follow, after which the session SHALL no longer be listed, except as a detached session's exit kept for its owner as for terminals.

#### Scenario: Agent crashes
- **WHEN** an attached agent that never offered `session/load` or `session/resume` writes `fatal: no model` to its standard error and exits with status 1
- **THEN** the client receives the agent's exit with status 1 and standard error ending `fatal: no model`, then the session's exit

#### Scenario: Agent that can continue crashes
- **WHEN** an attached agent that offered `session/resume` and had a ready ACP session exits with status 1
- **THEN** the client receives the agent's exit with status 1, receives no session exit, and the session is still listed

### Requirement: Transcript event forms are fixed for each protocol version
On a connection that negotiated client protocol 4, the server SHALL send only the transcript event kinds and enumerated values of protocol 4; a new kind or value SHALL come with a new client protocol version. Within protocol 4 an event MAY gain optional fields, which a protocol 4 client SHALL be able to ignore.

#### Scenario: Every event decodes
- **WHEN** a protocol 4 client decodes every transcript event of an agent session's turn, queue, permission, cancel, trim, exit, lost holder and continued conversation
- **THEN** each decodes as one of protocol 4's event forms

#### Scenario: An unknown field is ignored
- **WHEN** a protocol 4 client decodes a protocol 4 event that carries an extra field it doesn't know
- **THEN** it decodes to the same event as without the field

## ADDED Requirements

### Requirement: The transcript names the agent's conversation
Once the agent has given an ACP session, the transcript SHALL hold that session's id, the absolute directory it works in, and whether the agent offered `session/load`, `session/resume`, both or neither, and every replay SHALL carry them.

#### Scenario: Conversation named after session/new
- **WHEN** the fake ACP agent offers `loadSession` and `sessionCapabilities.resume` in `initialize` and answers `session/new` with session id `s1`
- **THEN** the client receives the conversation `s1` with the session's directory, marked as offering both load and resume, before the agent's ready state

#### Scenario: Conversation in a replay
- **WHEN** a client attaches an agent session whose agent answered `session/new` with session id `s1`
- **THEN** the replay holds the conversation `s1`

### Requirement: A session whose agent ended is kept for its conversation
When an agent exits, or its holder ends, after it gave an ACP session and offered `session/load` or `session/resume`, the server SHALL keep the session with its log, queue slot, conversation and directory, listed with activity `error`, while its client has it attached or its owner may still return. A detach SHALL end such a session, and so SHALL its close, a `KILL` or a `STOP`.

#### Scenario: Kept while attached
- **WHEN** an attached agent that offered `session/resume` exits, and a minute passes
- **THEN** the session is still listed, with activity `error`, and its log holds the conversation and the exit

#### Scenario: Detached after the exit
- **WHEN** a client detaches an agent session whose agent has exited and that is kept for its conversation
- **THEN** the session is no longer listed

#### Scenario: Owner away when the agent exits
- **WHEN** an agent with a `kill` policy exits while its client's connection is gone and lingering, and the client reconnects with its token in time
- **THEN** the client gets the session back with its replay, which ends with the agent's exit, and no session exit

### Requirement: A lost holder leaves agent sessions that can continue
When the holder running an agent session ends while the core runs, a session whose conversation can be continued SHALL stay as an ended one does, its agent recorded as lost with its holder; waiting permission requests are recorded as cancelled and a turn in flight ends as failed. Other agent sessions SHALL be lost as before.

#### Scenario: Holder killed
- **WHEN** the holder of an attached agent session whose agent offered `session/load` is killed with SIGKILL
- **THEN** the client receives the agent's state as lost with its holder, receives no session exit, and the session is still listed

### Requirement: Continuing a conversation
The client that has an ended agent session attached SHALL be able to continue it, giving the agent's command, arguments and environment. The server SHALL start that command in the session's directory, keeping the session's id, log and slot, then send `initialize`, then `session/resume` for the conversation when offered, else `session/load`, with the directory and no MCP servers. The session SHALL be ready once that succeeds; new entries' ids follow the log's last.

#### Scenario: Resumed
- **WHEN** an attached agent that offered `session/resume` exits, the client continues the session, and the agent started again offers `session/resume`
- **THEN** the agent receives `initialize`, then `session/resume` with the conversation's id and the session's directory, the client receives the agent as starting then ready, and the log holds every entry from before

#### Scenario: Ids continue
- **WHEN** a continued session's log ended at entry 12 and the client then sends a prompt
- **THEN** the prompt's user entry is entry 13

#### Scenario: Continue refused
- **WHEN** a client asks to continue an agent session whose agent is still running, or one it hasn't attached
- **THEN** it gets an error naming the session, and nothing is started

#### Scenario: A changed command
- **WHEN** a client continues an ended agent session with arguments other than those the session was started with
- **THEN** the agent is started with the new arguments, and the session is listed with the new command

### Requirement: A conversation continued by loading keeps its log
When an ended session is continued with `session/load`, the server SHALL NOT add the conversation the agent replays before answering to the log, and SHALL add a notice saying the conversation continued and how many of the agent's replayed updates weren't shown.

#### Scenario: Copilot-like agent
- **WHEN** an ended session holding two turns is continued, and the agent started again offers only `session/load`, replays 7 updates and answers
- **THEN** the log holds the two turns once each, then a notice saying the conversation continued and that the agent's replay of 7 updates wasn't shown, and the agent is ready

### Requirement: A conversation the server no longer has can be continued
A client SHALL be able to start an agent session that continues a conversation by its id, in the directory it gave. The server SHALL send `session/load` and add the agent's replay to the new session's empty log as it comes. When the agent offers only `session/resume`, the server SHALL resume and add a notice saying the earlier messages aren't shown.

#### Scenario: Loaded into a new session
- **WHEN** a client starts an agent session continuing conversation `s1`, and the agent replays a user message `hi` and an agent message `hello`, then answers `session/load`
- **THEN** the new session's log holds a notice that it continues an earlier conversation, the user message `hi` and the agent message `hello`, and the agent is ready

#### Scenario: Resume only
- **WHEN** a client starts an agent session continuing conversation `s1`, and the agent offers `session/resume` but not `session/load`
- **THEN** the agent receives `session/resume` for `s1`, and the log holds a notice that the conversation continued without its earlier messages

### Requirement: An agent that can't continue the conversation
When the agent started to continue a conversation offers neither `session/load` nor `session/resume`, or answers either with an error other than an authentication error, the server SHALL record the agent as failed with a message saying so, send nothing more and hang it up, and the session SHALL then end as after a version mismatch. An authentication error SHALL leave the agent needing a login, as for `session/new`.

#### Scenario: Neither offered after an update
- **WHEN** a client continues an ended session and the agent started again offers neither `session/load` nor `session/resume`
- **THEN** the client receives the agent's state as failed, saying the agent can't continue conversations, then the session's exit, and the agent received no `session/new`

#### Scenario: Unknown conversation
- **WHEN** the agent answers `session/resume` with an error whose message is `no such session`
- **THEN** the agent's state is failed with a message saying the conversation couldn't be continued and showing `no such session`, then the session ends

### Requirement: Nothing is sent again on its own after an exit
A turn running when its agent exits or is lost SHALL end as failed saying the agent exited, stay in the log, and never be sent to the agent again. A waiting prompt SHALL be held because the agent stopped; once the continued agent is ready, the client SHALL be able to send it again, and the server SHALL NOT send it on its own.

#### Scenario: Turn in flight
- **WHEN** an agent exits during a turn for the prompt `fix it`, and the client continues the session
- **THEN** the turn shows as failed saying the agent exited, and the agent started again never receives `fix it`

#### Scenario: Held prompt sent again after continuing
- **WHEN** `next` was waiting when the agent exited, the client continues the session, the agent is ready, and the client sends the held prompt again
- **THEN** the agent receives `next` as a new turn, and it had received nothing before that

#### Scenario: Pending permission at the exit
- **WHEN** an agent exits while a permission request waits, and the session is continued
- **THEN** the request's entry stays recorded as cancelled, and the continued agent has received no answer to it

### Requirement: What survives an ended agent session
A core restart or upgrade SHALL keep, for each running agent session, its conversation, so a later exit can still be continued. An agent session whose agent has ended SHALL be held only by the core: a core restart or upgrade SHALL end it, and its client SHALL find it gone on reconnecting.

#### Scenario: Conversation across a core restart
- **WHEN** the core is killed with SIGKILL while an agent that offered `session/resume` is ready, a new core adopts its holder, and the agent then exits
- **THEN** the session is kept for its conversation and can be continued

#### Scenario: Ended session across a core restart
- **WHEN** an attached agent session's agent has exited and is kept for its conversation, and the core is killed with SIGKILL
- **THEN** after the client reconnects, the session isn't returned to it and isn't listed
