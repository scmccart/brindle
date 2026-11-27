## MODIFIED Requirements

### Requirement: The server answers the agent's requests
The server SHALL answer every request the agent sends. A request for a method the server did not offer (file system, terminal, elicitation, or any other) SHALL get JSON-RPC error `-32601`. A notification the server does not know SHALL be ignored. A permission request SHALL be answered only as "Permission requests wait for the client's choice" says. Each request SHALL be answered once, even when the core restarts or is upgraded between reading the request and the agent's next message.

#### Scenario: File read refused
- **WHEN** the agent sends `fs/read_text_file`
- **THEN** it receives error `-32601` for that request id, and the session carries on

#### Scenario: Unknown notification
- **WHEN** the agent sends a notification `_vendor/ping`
- **THEN** nothing is added to the transcript and the session carries on

#### Scenario: Refused request across a core restart
- **WHEN** the agent sends `fs/read_text_file` with id 7, and the core is killed with SIGKILL right after it answers, before anything else happens in the session
- **THEN** after a new core adopts the session, the agent has received exactly one answer for id 7, error `-32601`

### Requirement: Permission requests wait for the client's choice
A permission request from the agent SHALL be added to the log with its options and mark the turn blocked. The server SHALL answer it only with the option an attached client chose, or `cancelled` on a cancel, never on its own. With no client attached it SHALL wait, however long. An answer to a request already answered, or naming an option the request does not offer, SHALL be refused with an error. When the agent process that asked can no longer receive a choice, because it exited, was lost with its holder, was hung up to be started again, or its session's ACP handling failed, the request SHALL be recorded as cancelled in the log at that moment, and the agent SHALL receive `cancelled` for it if it is still running. The log's pending permission requests SHALL always be exactly the ones the agent is waiting on.

#### Scenario: Allowed by the client
- **WHEN** the agent asks permission with options `allow-once` and `reject-once`, and the attached client chooses `allow-once`
- **THEN** the agent receives outcome `selected` with option `allow-once`, the entry records it, and the turn is working again

#### Scenario: No client attached
- **WHEN** the agent asks permission in a detached session and no client attaches for a minute
- **THEN** the agent has received no answer, and the session is listed as blocked

#### Scenario: Answered after attaching
- **WHEN** a client attaches a session whose agent is waiting on a permission request, and chooses `reject-once`
- **THEN** the replay showed the pending request, and the agent receives outcome `selected` with option `reject-once`

#### Scenario: Answered twice
- **WHEN** a client answers a permission request that was already answered
- **THEN** it gets an error naming the request, and the agent receives nothing more

#### Scenario: Pending at a restart for a login
- **WHEN** a permission request waits as entry 7, the agent then answers the turn's prompt with `-32000`, and the client restarts the agent after a terminal login
- **THEN** entry 7 is recorded as cancelled before the agent is shown starting, the new agent receives no answer to it, and a client attaching afterwards gets a replay that starts at the newest page, not at entry 7

#### Scenario: Pending at a handling fault
- **WHEN** a permission request waits and the session's ACP handling fails
- **THEN** the request is recorded as cancelled, and the agent, which still runs, receives outcome `cancelled` for it

### Requirement: The agent's exit is reported
When an agent exits, the client SHALL receive a transcript event with its exit status and the last 16 KiB of its standard error; after a version mismatch the standard error is added as a notice instead, and the failed state stays. When the conversation can be continued (see "A session whose agent ended is kept for its conversation") the session SHALL stay; otherwise the session's exit SHALL follow, after which the session SHALL no longer be listed, except as a detached session's exit kept for its owner as for terminals. An owner that returns to such a kept exit SHALL first receive the session's replay, which ends with the agent's exit and its standard error, and then the session's exit.

#### Scenario: Agent crashes
- **WHEN** an attached agent that never offered `session/load` or `session/resume` writes `fatal: no model` to its standard error and exits with status 1
- **THEN** the client receives the agent's exit with status 1 and standard error ending `fatal: no model`, then the session's exit

#### Scenario: Agent that can continue crashes
- **WHEN** an attached agent that offered `session/resume` and had a ready ACP session exits with status 1
- **THEN** the client receives the agent's exit with status 1, receives no session exit, and the session is still listed

#### Scenario: Owner returns to an exit it missed
- **WHEN** an agent that never offered `session/load` or `session/resume` ends its turn and exits with status 1 while its owner's connection is gone and lingering, and the owner reconnects with its token in time
- **THEN** the owner receives a replay holding the turn's end and the agent's exit with status 1 and its standard error, then the session's exit, and the session is no longer listed

#### Scenario: Owner returns to an exit a new core adopted
- **WHEN** an agent that can't be continued exits while no core runs, a new core adopts its holder, and the owner reconnects
- **THEN** the owner's replay ends with the agent's exit and its standard error, then the session's exit follows

### Requirement: A faulty agent is confined to its session
Output from an agent SHALL NOT end the core, the holder or another session. A line that is not a JSON-RPC message SHALL be skipped and counted in a notice. A line over 8 MiB SHALL be dropped up to its end, with a notice. Control characters other than newline and tab SHALL be replaced in transcript text. A failure while handling one agent's output SHALL stop only that session's ACP handling, with its reason in the transcript. Nothing the failed handling asked to send SHALL reach the agent: the only messages the agent receives from then on are `cancelled` answers to its waiting permission requests. After such a failure, the attached client's log and the server's stored state SHALL hold the same entries as the server's log.

#### Scenario: Not JSON
- **WHEN** an agent writes `hello world` on its own line, then a valid agent message chunk
- **THEN** the transcript holds a notice that a line was not an ACP message and the chunk's text, and the session carries on

#### Scenario: Escape sequences in text
- **WHEN** an agent's message text holds `ESC [ 2 J`
- **THEN** the transcript entry holds U+FFFD in place of the escape character

#### Scenario: Handling fault
- **WHEN** handling one agent session's output fails inside the core
- **THEN** that session's agent state says ACP handling failed and why, its prompts are refused from then on, its agent runs until the session is closed, and other sessions are unaffected

#### Scenario: A fault after part of a step was applied
- **WHEN** handling a prompt fails after the prompt's user entry was added to the log
- **THEN** the attached client receives a fresh replay that holds the user entry and the failed state, the agent never receives the prompt, and a core that adopts the session after a SIGKILL has the same log

#### Scenario: Events after a fault
- **WHEN** a session's handling failed after part of a step was applied, and its agent then exits
- **THEN** the attached client shows the agent's exit after the failure, with every entry the server's log holds

### Requirement: Continuing a conversation
The client that has an ended agent session attached SHALL be able to continue it, giving the agent's command, arguments and environment. The server SHALL start that command in the session's directory, keeping the session's id, log and slot, then send `initialize`, then `session/resume` for the conversation when offered, else `session/load`, with the directory and no MCP servers. The session SHALL be ready once that succeeds; new entries' ids follow the log's last. Until the new agent has started, a continue SHALL follow the session's close policy as a starting session does: when its client is gone, a `kill` session's continue SHALL be dropped and the session SHALL end, and a `detach` session's continue SHALL go ahead unattached.

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

#### Scenario: Client gone before the continue starts
- **WHEN** a client continues a `kill` agent session, and its connection ends before the new agent has started
- **THEN** no agent is started for it, the session is no longer listed, and a `STOP` sent afterwards is answered at once

#### Scenario: Detached client gone before the continue starts
- **WHEN** a client continues a `detach` agent session, and its connection ends before the new agent has started
- **THEN** the agent is started, the session is listed as detached, and a client that attaches it later sees the agent ready with the conversation

### Requirement: An agent that needs a login can be restarted
A client's restart request while the agent needs a login SHALL hang up the running agent and start it again with the command, arguments and environment the request gives, keeping the log, the slot and any ACP session, then make the session ready as a recovery does: with `session/new` when no ACP session was made, else by taking up that session. The restart SHALL close what the old agent process left open: its waiting permission requests are recorded as cancelled, and a list request out to it is refused saying the agent is started again. Until the new agent has started, the restart SHALL follow the close policy as a continue does.

#### Scenario: Restarted after a terminal login
- **WHEN** a ready agent answered a prompt with `-32000`, and the client asks to restart it with its command, arguments and environment
- **THEN** the old agent is hung up, a new agent process is sent `initialize` with `auth.terminal`, the conversation and the held prompt are kept, and the session becomes ready in the same ACP session

#### Scenario: A permission waiting at the restart
- **WHEN** a permission request waits when a ready agent answers a prompt with `-32000`, and the client restarts the agent after a terminal login
- **THEN** the request is recorded as cancelled, and the client's answer to it afterwards gets an error naming the request, and the new agent receives nothing for it

### Requirement: Refusing a list
A request for an agent's conversations SHALL be refused with an error saying why when the agent doesn't offer a list, when the agent is neither ready nor choosing, when an earlier list for the session hasn't been answered, and when the agent answers with an error, whose message SHALL be given. An agent that needs a login to list SHALL be shown as needing one, as for any request that does, and SHALL return to what it was doing after the login. A list out when the agent process it was sent to stops (it exits, is lost with its holder, is hung up to be started again, or its session's ACP handling fails) SHALL be refused at that moment saying why, and a later list request SHALL NOT be refused as one in progress.

#### Scenario: An agent without a list
- **WHEN** a client asks for the conversations of an agent that doesn't offer `session/list`
- **THEN** it gets an error saying the agent can't list conversations, and the agent receives nothing

#### Scenario: One list at a time
- **WHEN** a client asks for a list, and asks again before the agent has answered
- **THEN** the second request gets an error saying a list is in progress

#### Scenario: The agent's error
- **WHEN** the agent answers `session/list` with an error whose message is `index unavailable`
- **THEN** the client gets an error holding `index unavailable`

#### Scenario: A login to list
- **WHEN** a choosing agent answers `session/list` with the error that a login is needed, and the user logs in
- **THEN** the session says a login is needed, and is choosing again once the login succeeds

#### Scenario: A list out at a restart
- **WHEN** a list request is out to an agent that needs a login, and the client restarts the agent after a terminal login
- **THEN** the list request gets an error saying the agent is started again, and a list asked for once the new agent is ready goes to it
