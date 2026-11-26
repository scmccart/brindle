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
A client attached to an agent session SHALL be able to cancel the running turn. The server SHALL first hold a waiting prompt, then send `session/cancel`, answer each of the session's pending permission requests with the `cancelled` outcome and record that outcome, and end the turn when the agent answers the prompt. A cancel with no turn running SHALL change nothing.

#### Scenario: Cancel while waiting for permission
- **WHEN** a turn is waiting on a permission request and the client cancels
- **THEN** the agent receives `session/cancel` and the `cancelled` outcome for that request, and the turn ends with stop reason `cancelled` once the agent answers

#### Scenario: Cancel holds the waiting prompt
- **WHEN** a turn is running with a prompt waiting, and the client cancels
- **THEN** the client is told the prompt is held because the turn was cancelled, with its text, before the turn ends, and the agent never receives it

### Requirement: Transcript event forms are fixed for each protocol version
On a connection that negotiated client protocol 4, the server SHALL send only the transcript event kinds and enumerated values of protocol 4; a new kind or value SHALL come with a new client protocol version. Within protocol 4 an event MAY gain optional fields, which a protocol 4 client SHALL be able to ignore.

#### Scenario: Every event decodes
- **WHEN** a protocol 4 client decodes every transcript event of an agent session's turn, queue, permission, cancel, trim and exit
- **THEN** each decodes as one of protocol 4's event forms

#### Scenario: An unknown field is ignored
- **WHEN** a protocol 4 client decodes a protocol 4 event that carries an extra field it doesn't know
- **THEN** it decodes to the same event as without the field

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
When an agent exits, the client SHALL receive a transcript event with its exit status and the last 16 KiB of its standard error, then the session's exit; after a version mismatch the standard error is added as a notice instead, and the failed state stays. The session SHALL then no longer be listed, except as a detached session's exit kept for its owner as for terminals.

#### Scenario: Agent crashes
- **WHEN** an attached agent writes `fatal: no model` to its standard error and exits with status 1
- **THEN** the client receives the agent's exit with status 1 and standard error ending `fatal: no model`, then the session's exit

### Requirement: Login is left to the agent's own tools
When the agent answers `session/new` or a prompt with "authentication required" (`-32000`), the session SHALL stay running and its transcript SHALL say the agent needs a login through its own command line, with the agent's message. A prompt that got the error SHALL end its turn as failed and not be sent again, and a waiting prompt SHALL be held. Prompts and resends SHALL be refused from then on, the activity SHALL be `error`, and the server SHALL NOT call `authenticate`.

#### Scenario: Authentication required
- **WHEN** the agent answers `session/new` with error `-32000` and message `run agent login`
- **THEN** the transcript's agent state is "login required" with `run agent login`, prompts are refused with an error saying so, and the session's activity is `error`

#### Scenario: Login demanded by a prompt
- **WHEN** a ready agent answers a prompt with error `-32000` and message `session expired`, while another prompt is waiting
- **THEN** the turn ends as failed, the waiting prompt is held because a login is needed, the agent state is "login required" with `session expired`, the agent receives neither prompt again, and a resend of the held prompt is refused saying a login is needed

### Requirement: The session list shows agent sessions
On protocol 4 the session list SHALL mark each agent session with its activity (`idle`, `working`, `blocked`, `done` or `error`, the names of program status) and the name the agent gave in `initialize`. `working` covers starting and a running turn, `blocked` a pending permission request, `done` a turn that ended with `end_turn`, `idle` a cancelled turn or a new session, and `error` everything that stopped the agent or a turn.

#### Scenario: Blocked and done
- **WHEN** one agent session waits on a permission request and another has finished a turn with `end_turn`
- **THEN** the list shows the first as `blocked` and the second as `done`

#### Scenario: Turn refused by the model
- **WHEN** an agent ends a turn with stop reason `refusal`
- **THEN** the session's activity is `error`

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
