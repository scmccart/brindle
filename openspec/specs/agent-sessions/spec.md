# agent-sessions Specification

## Purpose
The terminal server hosts ACP agents (Agent Client Protocol) as agent sessions: it runs the agent, acts as its ACP client, keeps its conversation as a transcript log across clients, core restarts and upgrades, and serves that log to clients on client protocol 4.

## Requirements

### Requirement: Agent sessions run on plain pipes
A client on protocol 4 SHALL be able to start an agent session with a command, arguments, working directory, environment, close policy and profile name. The server SHALL run the command under a holder, connected by plain pipes for its standard input, output and error and with no terminal, with exactly that environment. It SHALL report the session's id, the agent's pid and the session's key, or the reason the agent could not be started.

#### Scenario: No terminal
- **WHEN** a client starts an agent session on Linux
- **THEN** the agent's process has no controlling terminal, and its parent is a holder

#### Scenario: Bytes pass unchanged
- **WHEN** an agent writes a message whose text holds a tab, `\r\n` inside a JSON string and multibyte UTF-8
- **THEN** the transcript entry made from it holds exactly that text

#### Scenario: No pseudoconsole on Windows
- **WHEN** a client starts an agent session on Windows
- **THEN** the agent runs as a child of the holder with no pseudoconsole and no console window

#### Scenario: Agent cannot start
- **WHEN** a client starts an agent session whose command does not exist
- **THEN** the server replies with an error naming the program and the OS error, and no session is listed

#### Scenario: Batch file named bare on Windows
- **WHEN** a client starts an agent session on Windows with command `npx`, and `npx.cmd` is on `PATH`
- **THEN** the server replies with an error saying `npx` is a batch file and showing the `cmd /c npx …` form to use, and no session is listed

#### Scenario: Empty command
- **WHEN** a client starts an agent session with no command
- **THEN** the server refuses it with an error saying an agent session needs a command, and starts nothing

### Requirement: The server is the agent's ACP client
On starting an agent session the server SHALL send `initialize` asking for ACP protocol version 1, with client capabilities that offer no file system and no terminal methods, and its name and version, then `session/new` with the session's working directory as an absolute path and no MCP servers. The session SHALL be ready for prompts once `session/new` succeeds.

#### Scenario: Initialize
- **WHEN** a client starts an agent session running the fake ACP agent
- **THEN** the agent receives `initialize` with `protocolVersion` 1, no `fs` and no `terminal` capability, and then `session/new` with the absolute working directory and an empty `mcpServers`

#### Scenario: Home-relative directory
- **WHEN** a client starts an agent session with working directory `~/src` and `HOME=/home/u` in its environment
- **THEN** the agent receives `session/new` with `cwd` `/home/u/src`

### Requirement: An agent that doesn't speak ACP version 1 is closed with the reason
When the agent rejects `initialize` (`-32022` or another error) or answers with a protocol version other than 1, the server SHALL record the agent as failed with a message naming the agent's versions and the server's 1 (or the agent's error), send nothing more, and hang it up. That failed state SHALL be the last agent state the client receives: the agent's exit SHALL NOT replace it, and the session's exit follows.

#### Scenario: Agent refuses version 1
- **WHEN** the agent answers `initialize` with error `-32022` and data `{"requested":1,"supported":[2]}`
- **THEN** the client receives the agent's state as failed with a message saying the agent supports ACP version 2 and the server speaks 1, then the session's exit, and no exit status replaces that state

#### Scenario: Agent answers another version
- **WHEN** the agent answers `initialize` successfully with protocol version 2
- **THEN** the client receives the agent's state as failed with a message saying the agent speaks ACP version 2 and the server speaks 1, the agent receives no `session/new`, and the session ends once the agent has exited

### Requirement: The server answers the agent's requests
The server SHALL answer every request the agent sends. A request for a method the server did not offer (file system, terminal, elicitation, or any other) SHALL get JSON-RPC error `-32601`. A notification the server does not know SHALL be ignored. A permission request SHALL be answered only as "Permission requests wait for the client's choice" says.

#### Scenario: File read refused
- **WHEN** the agent sends `fs/read_text_file`
- **THEN** it receives error `-32601` for that request id, and the session carries on

#### Scenario: Unknown notification
- **WHEN** the agent sends a notification `_vendor/ping`
- **THEN** nothing is added to the transcript and the session carries on

### Requirement: The transcript log
The server SHALL keep each agent session's conversation as a log of entries, whether or not a client is attached: user messages, agent messages, thoughts, tool calls, permission requests with their outcome, and notices, each with an id that stays the same for the session's life, plus the turn state and the agent's state. Streamed chunks SHALL merge into one entry, and a tool call's updates SHALL change its entry.

#### Scenario: Chunks merge
- **WHEN** the agent sends three `agent_message_chunk` updates `Hel`, `lo` and `!` in one turn
- **THEN** the log holds one agent message entry with text `Hello!`

#### Scenario: Tool call updated by id
- **WHEN** the agent sends `tool_call` with id `call_1` and status `pending`, then `tool_call_update` for `call_1` with status `completed`
- **THEN** the log holds one tool call entry for `call_1`, with its title and status `completed`

#### Scenario: Unknown update kind
- **WHEN** the agent sends a `session/update` with an update kind the server does not know, then an agent message chunk
- **THEN** the unknown update changes nothing, and the chunk is added as usual

### Requirement: Prompts
A client attached to a ready agent session SHALL be able to send a prompt of plain text. The server SHALL add it to the log as a user message, send it to the agent as `session/prompt`, and mark the turn working. When the agent answers the prompt, the turn SHALL end with the agent's stop reason, or as failed with the agent's error message.

#### Scenario: A turn
- **WHEN** a client sends the prompt `hi` and the agent streams `hello` and answers with stop reason `end_turn`
- **THEN** the client receives a user message `hi`, an agent message `hello`, and the turn's end with `end_turn`, in that order

#### Scenario: Prompt error
- **WHEN** the agent answers a prompt with a JSON-RPC error whose message is `rate limited`
- **THEN** the turn ends as failed, and the transcript shows `rate limited`

### Requirement: A queue of one prompt
A prompt sent during a turn SHALL wait in the session's queue slot, and attached clients SHALL be told. Only when the turn ends with stop reason `end_turn` and the agent ready SHALL the server send it as the next turn. While the slot holds a prompt and a turn runs, a further prompt SHALL be refused with an error saying a prompt is already queued. A prompt before the session is ready, or over 256 KiB of UTF-8, SHALL be refused with an error saying why.

#### Scenario: Prompt during a turn
- **WHEN** a client sends a second prompt while the first turn is running, and the first turn ends with `end_turn`
- **THEN** the client is told the second prompt is waiting, the agent receives it only after answering the first, and the client is then told the slot is empty and sees it as a user message

#### Scenario: Third prompt
- **WHEN** a client sends a third prompt while a turn is running and a prompt is waiting
- **THEN** it gets an error saying a prompt is already queued, and the waiting prompt and the agent are unchanged

#### Scenario: Prompt before ready
- **WHEN** a client sends a prompt before the agent has answered `session/new`
- **THEN** it gets an error saying the agent isn't ready, and nothing is queued

#### Scenario: Oversized prompt
- **WHEN** a client sends a prompt of 300 KiB
- **THEN** it gets an error naming the 256 KiB limit, and nothing is added to the log or queued

### Requirement: A prompt that isn't sent is held
When the turn a prompt waits for ends any other way than `end_turn` (cancelled, failed, or another stop reason), when the agent stops being ready, or when a restart loses the turn's end, the server SHALL keep the prompt in the slot as held, with the reason, and tell attached clients. It SHALL NOT send a held prompt on its own. A held prompt SHALL be in every replay and SHALL survive detaches, core restarts and upgrades.

#### Scenario: A failed turn holds it
- **WHEN** a prompt is waiting and the agent answers the running prompt with a JSON-RPC error
- **THEN** the client is told the prompt is held because the turn failed, with its text, and the agent never receives it

#### Scenario: Another stop reason holds it
- **WHEN** a prompt is waiting and the turn ends with stop reason `max_tokens`
- **THEN** the prompt is held with that stop reason, and the agent receives nothing more

#### Scenario: Held while detached
- **WHEN** a client detaches mid-turn with a prompt waiting, the turn fails, and a client attaches the session later
- **THEN** the replay shows the prompt held with its text, and the agent never received it

#### Scenario: Agent stops with a prompt waiting
- **WHEN** a prompt is waiting and the agent exits before its turn ends
- **THEN** the client is told the prompt is held because the agent stopped, then the agent's exit

### Requirement: A held prompt is sent again or discarded by the client
A client attached to an agent session SHALL be able to send a held prompt again: with no turn running and the agent ready it is sent as a new turn, while a turn runs it waits again, and while the agent isn't ready the request is refused. A client SHALL be able to discard a waiting or held prompt, which empties the slot. Either request with no fitting prompt in the slot SHALL be refused with an error. A new prompt with no turn running SHALL be sent and leave a held prompt held.

#### Scenario: Sent again when idle
- **WHEN** a prompt is held, no turn is running, and the client sends it again
- **THEN** the slot is empty, the agent receives the prompt, and the client sees it as a user message

#### Scenario: Sent again during a turn
- **WHEN** a prompt is held, the client starts a new turn with another prompt, then sends the held one again
- **THEN** the held prompt waits, and the agent receives it after the new turn ends with `end_turn`

#### Scenario: Discarded
- **WHEN** a client discards a waiting prompt, and the turn then ends with `end_turn`
- **THEN** the client is told the slot is empty, and the agent never receives the prompt

#### Scenario: Nothing to discard or send again
- **WHEN** a client asks to discard or send again with an empty slot
- **THEN** it gets an error saying nothing is queued, or that no prompt is held

### Requirement: Cancelling a turn
A client attached to an agent session SHALL be able to cancel the running turn. The server SHALL first hold a waiting prompt, then send `session/cancel`, answer each of the session's pending permission requests with the `cancelled` outcome and record that outcome, and end the turn when the agent answers the prompt. While the agent is logging in, a cancel SHALL cancel the login instead (see "A login can be cancelled"). A cancel with no turn running and no login running SHALL change nothing.

#### Scenario: Cancel while waiting for permission
- **WHEN** a turn is waiting on a permission request and the client cancels
- **THEN** the agent receives `session/cancel` and the `cancelled` outcome for that request, and the turn ends with stop reason `cancelled` once the agent answers

#### Scenario: Cancel holds the waiting prompt
- **WHEN** a turn is running with a prompt waiting, and the client cancels
- **THEN** the client is told the prompt is held because the turn was cancelled, with its text, before the turn ends, and the agent never receives it

#### Scenario: Cancel while logging in
- **WHEN** the agent is logging in and the client cancels
- **THEN** the agent receives `$/cancel_request` and no `session/cancel`, and the agent state is "login required"

### Requirement: Transcript event forms are fixed for each protocol version
On a connection that negotiated client protocol 4, the server SHALL send only the transcript event kinds and enumerated values of protocol 4 as first released; once a release has shipped protocol 4, a new kind or value SHALL come with a new client protocol version. Within protocol 4 an event MAY gain optional fields, which a protocol 4 client SHALL be able to ignore.

#### Scenario: Every event decodes
- **WHEN** a protocol 4 client decodes every transcript event of an agent session's turn, queue, permission, cancel, login, trim and exit
- **THEN** each decodes as one of protocol 4's event forms

#### Scenario: An unknown field is ignored
- **WHEN** a protocol 4 client decodes a protocol 4 event that carries an extra field it doesn't know
- **THEN** it decodes to the same event as without the field

#### Scenario: A login demand without methods
- **WHEN** a protocol 4 client decodes the "login required" agent state with a message and no methods
- **THEN** its encoding is the same as before login methods existed

### Requirement: Permission requests wait for the client's choice
A permission request from the agent SHALL be added to the log with its options and mark the turn blocked. The server SHALL answer it only with the option an attached client chose, or `cancelled` on a cancel, never on its own. With no client attached it SHALL wait, however long. An answer to a request already answered, or naming an option the request does not offer, SHALL be refused with an error.

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

### Requirement: Attach replays the transcript
A client on protocol 4 SHALL be able to attach a detached agent session. Before any live event the server SHALL send a delimited replay of the whole log, the turn state and the agent's state as transcript events, after which the client's copy SHALL equal the server's. Attaching follows the ownership and close policy rules of terminal sessions.

#### Scenario: Replay after a turn
- **WHEN** a session finished a turn while detached, and a client attaches it
- **THEN** the replay holds the user message, the agent message and the turn's end, and folding it gives the server's log

#### Scenario: Live after replay
- **WHEN** a client attaches a session in the middle of a turn
- **THEN** it receives the replay, then the turn's later chunks as live events, and none twice

#### Scenario: Attaching again replays from the start
- **WHEN** a client detaches an agent session after receiving part of a turn, and attaches it again
- **THEN** the new replay holds the whole log from its first kept entry, whatever the client had received before

### Requirement: Agent sessions keep their client's layout
An agent session SHALL keep a layout as a terminal session does: given with `DETACH`, or with `SET_LAYOUT` while its client has it attached, kept across core restarts and upgrades and when the client's connection ends, listed while it is detached, dropped on attach, and never over 64 KiB.

#### Scenario: Split agent tab detached
- **WHEN** a client detaches two agent sessions, each with a layout naming both sessions' keys
- **THEN** listing sessions reports each agent session with that layout

#### Scenario: Layout set while attached survives a core restart
- **WHEN** a client on protocol 4 sets an attached agent session's layout, is killed with SIGKILL, and the core is killed and replaced
- **THEN** listing sessions from the new core reports that layout for the agent session

### Requirement: How agent sessions are listed
The session list SHALL report an agent session's working directory as the absolute directory its `session/new` was given, and its size as 0 columns by 0 rows. The server SHALL ignore the size in an `ATTACH` of an agent session, and SHALL refuse a resize of one with an error naming the session, changing nothing.

#### Scenario: Directory listed absolute
- **WHEN** a client starts an agent session with working directory `~/src` and `HOME=/home/u`, and lists sessions
- **THEN** the agent session's directory is `/home/u/src`

#### Scenario: No size
- **WHEN** a client lists sessions, then attaches the agent session with the 0×0 size it was listed with
- **THEN** the agent session was listed as 0×0, and the attach replays its transcript

#### Scenario: Resize refused
- **WHEN** a client sends a resize to 80×24 for an agent session it has attached
- **THEN** it gets an error naming the session, the agent receives nothing, and the session is still listed as 0×0

### Requirement: Agent sessions follow the close policies
Agent sessions SHALL have the close policies of terminal sessions: with `kill` the agent is hung up when its client closes it or is gone, with `detach` it keeps running unattached, and an explicit detach keeps it running. Hanging up an agent SHALL close its standard input, then end it and the processes it started if they are still running after a short grace.

#### Scenario: Detached mid-turn
- **WHEN** a client detaches an agent session mid-turn and the agent finishes the turn
- **THEN** the session stays listed, and a client that attaches it later sees the whole turn

#### Scenario: Closed
- **WHEN** a client closes an agent session whose agent ignores the end of its input
- **THEN** within a few seconds the agent and the processes it started are gone, and the session is no longer listed

### Requirement: The agent's exit is reported
When an agent exits, the client SHALL receive a transcript event with its exit status and the last 16 KiB of its standard error; after a version mismatch the standard error is added as a notice instead, and the failed state stays. When the conversation can be continued (see "A session whose agent ended is kept for its conversation") the session SHALL stay; otherwise the session's exit SHALL follow, after which the session SHALL no longer be listed, except as a detached session's exit kept for its owner as for terminals.

#### Scenario: Agent crashes
- **WHEN** an attached agent that never offered `session/load` or `session/resume` writes `fatal: no model` to its standard error and exits with status 1
- **THEN** the client receives the agent's exit with status 1 and standard error ending `fatal: no model`, then the session's exit

#### Scenario: Agent that can continue crashes
- **WHEN** an attached agent that offered `session/resume` and had a ready ACP session exits with status 1
- **THEN** the client receives the agent's exit with status 1, receives no session exit, and the session is still listed

### Requirement: The session list shows agent sessions
On protocol 4 the session list SHALL mark each agent session with its activity (`idle`, `working`, `blocked`, `done` or `error`, the names of program status) and the name the agent gave in `initialize`. `working` covers starting, logging in and a running turn, `blocked` a pending permission request, `done` a turn that ended with `end_turn`, `idle` a cancelled turn or a new session, and `error` everything that stopped the agent or a turn, a login demand included.

#### Scenario: Blocked and done
- **WHEN** one agent session waits on a permission request and another has finished a turn with `end_turn`
- **THEN** the list shows the first as `blocked` and the second as `done`

#### Scenario: Turn refused by the model
- **WHEN** an agent ends a turn with stop reason `refusal`
- **THEN** the session's activity is `error`

#### Scenario: Logging in
- **WHEN** an agent session's agent is logging in
- **THEN** the list shows it as `working`

### Requirement: Clients before protocol 4 never see agent sessions
A client on protocols 1–3 SHALL NOT find agent sessions in the session list, SHALL get an error naming the session when it asks to attach one, and SHALL NOT receive agent sessions on reconnect. A core that speaks holder protocols below 3 SHALL NOT be shown a holder's agent sessions; they keep running and come back when a core that speaks holder protocol 3 adopts the holder.

#### Scenario: Older client lists
- **WHEN** a client on protocol 3 lists the sessions of a server running one terminal and one agent session
- **THEN** it sees only the terminal session

#### Scenario: Older client attaches by id
- **WHEN** a client on protocol 3 asks to attach an agent session by its id
- **THEN** it gets an error saying the session needs protocol 4, and the session is unchanged

### Requirement: Agent sessions survive core restarts and upgrades
A core restart or an upgrade SHALL keep every agent session with its agent's pid, its log and its entry ids, its turn state, its queue slot (a waiting or held prompt), and its pending permission requests, including a turn in flight. Output the agent wrote while no core was running SHALL be applied when a core adopts the session.

#### Scenario: Core killed mid-turn
- **WHEN** the core is killed with SIGKILL while an agent session streams a turn, and the agent finishes the turn before a new core adopts the holder
- **THEN** after the client reconnects, its replay holds the whole turn with its end, and the agent has the same pid

#### Scenario: Pending permission across an upgrade
- **WHEN** the core is upgraded while an agent waits on a permission request, and the client then answers it
- **THEN** the agent receives the client's choice for the request id it sent

#### Scenario: Queued prompt across a core restart
- **WHEN** the core is killed with SIGKILL while a turn runs with a prompt waiting, a new core adopts the holder, and the turn ends with `end_turn`
- **THEN** the agent receives the waiting prompt once, as the next turn

#### Scenario: Held prompt across an upgrade
- **WHEN** a prompt is held and the core is upgraded
- **THEN** a client attaching afterwards is shown the same held prompt, and can send it again

### Requirement: A faulty agent is confined to its session
Output from an agent SHALL NOT end the core, the holder or another session. A line that is not a JSON-RPC message SHALL be skipped and counted in a notice. A line over 8 MiB SHALL be dropped up to its end, with a notice. Control characters other than newline and tab SHALL be replaced in transcript text. A failure while handling one agent's output SHALL stop only that session's ACP handling, with its reason in the transcript.

#### Scenario: Not JSON
- **WHEN** an agent writes `hello world` on its own line, then a valid agent message chunk
- **THEN** the transcript holds a notice that a line was not an ACP message and the chunk's text, and the session carries on

#### Scenario: Escape sequences in text
- **WHEN** an agent's message text holds `ESC [ 2 J`
- **THEN** the transcript entry holds U+FFFD in place of the escape character

#### Scenario: Handling fault
- **WHEN** handling one agent session's output fails inside the core
- **THEN** that session's agent state says ACP handling failed and why, its prompts are refused from then on, its agent runs until the session is closed, and other sessions are unaffected

### Requirement: The fake ACP agent is a test binary that is never shipped
The build SHALL produce `brindle-fake-acp-agent`, a program that plays an ACP agent from a script, in debug and release builds alike. No release archive, installer or package SHALL contain it.

#### Scenario: Built beside the helper
- **WHEN** the workspace is built with `cargo build --release`
- **THEN** `brindle-fake-acp-agent` is in the same directory as `brindle-remote`, and it plays a script's steps when run with it

#### Scenario: Not in the archives
- **WHEN** the Linux archive and the Windows zip are built
- **THEN** neither contains `brindle-fake-acp-agent`

### Requirement: The log is bounded
An agent session's log SHALL stay within 16 MiB in its stored form. When it would pass that, the server SHALL drop the oldest entries whole and record how many were dropped, and the session SHALL carry on.

#### Scenario: Long conversation
- **WHEN** an agent session's log passes 16 MiB
- **THEN** its oldest entries are gone, a replay starts with a notice saying how many were dropped, and the latest entries are intact

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

### Requirement: The agent's login methods are offered with a login demand
The server SHALL keep the login methods the agent advertised in `initialize` (`authMethods`), in the agent's order, and SHALL send them with every "login required" agent state, each with its id, name, optional description and kind. A method with no `type`, or type `agent`, SHALL be of kind `agent`. A method of any other type the server doesn't run SHALL be left out. Text from the agent SHALL be cleaned and bounded as other transcript text.

#### Scenario: Methods sent with the demand
- **WHEN** the agent advertises methods `oauth-personal` ("Log in with Google") and `api-key` ("API Key", described "Use an API key") and answers `session/new` with error `-32000` and message `log in first`
- **THEN** the client receives the agent state "login required" with `log in first` and both methods in that order, each of kind `agent`, with `api-key`'s description

#### Scenario: No methods advertised
- **WHEN** the agent advertises no methods and answers `session/new` with `-32000`
- **THEN** the agent state is "login required" with the agent's message and no methods, and nothing else changes

#### Scenario: A method of an unknown type
- **WHEN** the agent advertises a method of type `env_var` and one with no type, and needs a login
- **THEN** the login-required state carries only the method with no type

### Requirement: A client logs the agent in through the server
A client attached to an agent session whose agent needs a login SHALL be able to choose one of its `agent` methods (`AGENT_LOGIN`). The server SHALL send `authenticate` with that method's id, make the agent state "logging in" with the method's id and name, and wait for the answer however long it takes. A login request for a method the agent doesn't offer, while the agent doesn't need a login, or while a login runs SHALL be refused with an error saying why, and change nothing.

#### Scenario: Logging in
- **WHEN** the agent needs a login and the client chooses `oauth-personal`
- **THEN** the agent receives `authenticate` with `methodId` `oauth-personal`, the client receives the agent state "logging in" with that method, and the session's activity is `working`

#### Scenario: No answer for an hour
- **WHEN** the agent takes an hour to answer `authenticate`
- **THEN** the session stays "logging in" for that hour, and nothing is sent to the agent meanwhile

#### Scenario: Method not offered
- **WHEN** the client asks to log in with `no-such-method`
- **THEN** it gets an error naming the method, the agent receives nothing, and the agent still needs a login

#### Scenario: Not needed
- **WHEN** the client asks to log in while the agent is ready
- **THEN** it gets an error saying the agent isn't waiting for a login, and the agent receives nothing

### Requirement: A successful login carries the session on
When the agent answers `authenticate` successfully, the agent state SHALL become "starting". If the login was demanded by `session/new`, the server SHALL send `session/new` again, and the session SHALL become ready as at a start. If it was demanded by a prompt, the session SHALL become ready again with the same ACP session, the agent's name and its version, and nothing SHALL be sent to the agent on its own.

#### Scenario: Login at the start
- **WHEN** the agent answered `session/new` with `-32000`, the client logs in, and the agent answers `authenticate` with `{}` and the second `session/new` with a session id
- **THEN** the client receives the agent state "starting", then "ready" with the agent's name, and prompts are taken

#### Scenario: Login demanded by a prompt
- **WHEN** a ready agent answered a prompt with `-32000`, the client logs in, and the agent answers `authenticate` with `{}`
- **THEN** the agent state becomes "starting", then "ready", the agent receives no `session/new` and no prompt, and the next prompt goes to the same ACP session

#### Scenario: Still not logged in
- **WHEN** the agent answers the `session/new` sent after a successful login with `-32000` again
- **THEN** the agent state is "login required" with the agent's new message and its methods

### Requirement: A failed login offers the methods again
When the agent answers `authenticate` with an error, the agent state SHALL become "login required" again, with the agent's error message and its methods, and the client MAY choose a method again.

#### Scenario: Login refused
- **WHEN** the client logs in with `chat-gpt` and the agent answers `authenticate` with error message `browser login timed out`
- **THEN** the agent state is "login required" with `browser login timed out` and the same methods, and a second login request is accepted

### Requirement: A login can be cancelled
While the agent is logging in, a client's cancel SHALL send the agent `$/cancel_request` naming the `authenticate` request, and make the agent state "login required" again with the message it had before the login and its methods. The server SHALL ignore a later answer to the cancelled request.

#### Scenario: Cancelled login
- **WHEN** the client logs in with `oauth-personal` and cancels before the agent answers
- **THEN** the agent receives `$/cancel_request` with the `authenticate` request's id, and the agent state is "login required" with its earlier message and methods

#### Scenario: Late answer
- **WHEN** a login was cancelled, and the agent then answers the cancelled `authenticate` with `{}`
- **THEN** the agent state stays "login required", and the agent receives no `session/new`

### Requirement: Prompts while a login is needed
While the agent needs a login or is logging in, prompts and resends SHALL be refused with an error saying so. A prompt whose turn failed with `-32000` SHALL end its turn as failed and SHALL never be sent again by the server. A prompt waiting at the demand SHALL be held because a login is needed, and once the agent is ready again the client SHALL be able to send it again or discard it.

#### Scenario: Held prompt after a login
- **WHEN** the agent answered a prompt with `-32000` while another prompt was waiting, and the client logs in successfully
- **THEN** the held prompt stays held, the agent has received neither prompt again, and a resend of the held prompt sends it as a new turn

#### Scenario: Prompt while a login is needed
- **WHEN** the agent answered `session/new` with `-32000` and message `run agent login`, and the client sends a prompt
- **THEN** it gets an error saying the agent needs a login, nothing is queued, and the session's activity is `error`

#### Scenario: Prompt while logging in
- **WHEN** the client sends a prompt while the agent is logging in
- **THEN** it gets an error saying the agent is logging in, and nothing is queued

### Requirement: A login survives core restarts and upgrades
A core restart or an upgrade SHALL keep an agent session's login state: its methods, its login-required message, the ACP session a prompt's demand left, and a running login with its `authenticate` request id, so that the agent's answer is applied by the next core.

#### Scenario: Core killed while logging in
- **WHEN** the core is killed with SIGKILL while the agent is logging in, a new core adopts the holder, and the agent then answers `authenticate` with `{}`
- **THEN** the client, after reconnecting, sees the agent state go from "logging in" to "starting" and "ready"

### Requirement: Terminal login methods
The server SHALL advertise `clientCapabilities.auth.terminal: true` in `initialize`, and SHALL offer a method of type `terminal` with kind `terminal`, its arguments and its environment. It SHALL refuse a login request for a `terminal` method, saying the client runs it.

#### Scenario: Advertised
- **WHEN** a client starts an agent session running the fake ACP agent
- **THEN** the agent receives `initialize` with `clientCapabilities.auth.terminal` `true`, and no `fs` and no `terminal` capability

#### Scenario: Terminal method offered
- **WHEN** the agent advertises a method `cli-login` of type `terminal` with args `["login"]` and env `{"LOGIN_MODE":"device"}`, and needs a login
- **THEN** the login-required state carries `cli-login` of kind `terminal` with those args and env

#### Scenario: Not run by the server
- **WHEN** the client asks the server to log in with the `terminal` method `cli-login`
- **THEN** it gets an error saying the client runs that method, and the agent receives no `authenticate`

### Requirement: An agent that needs a login can be restarted
A client's restart request while the agent needs a login SHALL hang up the running agent and start it again with the command, arguments and environment the request gives, keeping the log, the slot and any ACP session, then make the session ready as a recovery does: with `session/new` when no ACP session was made, else by taking up that session.

#### Scenario: Restarted after a terminal login
- **WHEN** a ready agent answered a prompt with `-32000`, and the client asks to restart it with its command, arguments and environment
- **THEN** the old agent is hung up, a new agent process is sent `initialize` with `auth.terminal`, the conversation and the held prompt are kept, and the session becomes ready in the same ACP session
