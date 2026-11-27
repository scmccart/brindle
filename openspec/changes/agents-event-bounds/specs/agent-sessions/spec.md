## ADDED Requirements

### Requirement: Every transcript event fits one message
Every transcript event the server sends, live, replayed or fetched, SHALL fit in one client protocol message, whatever the agent sends. The server SHALL bound every field an agent fills. An event still too large SHALL NOT be kept or sent: the session's handling SHALL fail with a message naming the event's kind, entry, size and the limit, shown in the zone and logged. An identifier the server sends back unchanged SHALL never be cut: one too long SHALL be left out or refused, saying why.

#### Scenario: A large file change reaches the client
- **WHEN** the agent reports a tool call that creates a file of 300,000 empty lines while a client is attached
- **THEN** the client receives the tool call with its file change, bounded and with the rest counted as not kept, and every later update of the turn

#### Scenario: An attach after a large tool call
- **WHEN** a client attaches a session whose newest entry is a tool call holding 32 command outputs, each with a 64 KiB command line
- **THEN** the replay holds that entry, its content and the session's state, and the client's copy equals the server's log from the page's first entry on

#### Scenario: A session id too long to keep
- **WHEN** the agent answers `session/new` with a session id of 2 KiB
- **THEN** the agent's start fails with a message saying its session id is longer than 1 KiB, and nothing is sent to the agent in that session's name

#### Scenario: A stop reason the server doesn't know
- **WHEN** the agent ends a turn with a stop reason of its own named with 100 KiB of text
- **THEN** the turn ends with that stop reason's name cut as a title is, and an attached client receives it

## MODIFIED Requirements

### Requirement: The log is bounded
An agent session's log SHALL stay within 16 MiB in its stored form after every update. When it would pass that, the server SHALL drop the oldest entries whole until the log is within 12 MiB, record how many were dropped, and the session SHALL carry on. A message, thought or user entry SHALL hold at most 1 MiB of text: streamed text that would take it past that SHALL continue in a new entry of the same kind, so no text is cut and no single entry can hold the log over its bound. Entries that were dropped SHALL NOT be fetched again.

#### Scenario: Long conversation
- **WHEN** an agent session's log passes 16 MiB
- **THEN** its oldest entries are gone, the log is within 12 MiB, a replay starts with a notice saying how many were dropped, and the latest entries are intact

#### Scenario: Dropped entries can't be fetched
- **WHEN** a client fetches entries older than the oldest one the log still holds
- **THEN** it receives no entries and is told nothing earlier remains

#### Scenario: A reply past 1 MiB continues in a new entry
- **WHEN** the agent streams one 2.5 MiB reply in `agent_message_chunk`s while a client is attached
- **THEN** the log holds the reply as three consecutive agent message entries of at most 1 MiB each, whose texts joined are the reply, and the client's copy holds the same three

#### Scenario: One huge reply keeps the log bounded
- **WHEN** the agent streams one 20 MiB thought after a short conversation
- **THEN** the log is within 16 MiB after every chunk, its oldest entries are dropped whole, the newest 12 MiB or less of the thought is kept, and the session carries on and can be checkpointed

#### Scenario: A shown conversation load past 1 MiB
- **WHEN** the agent replays a loaded conversation, shown, whose first user message is 1.5 MiB
- **THEN** the log holds that message as two consecutive user entries

### Requirement: Permission requests wait for the client's choice
A permission request from the agent SHALL be added to the log with its options and mark the turn blocked. The server SHALL answer it only with the option an attached client chose, or `cancelled` on a cancel, never on its own. With no client attached it SHALL wait, however long. An answer to a request already answered, or naming an option the request does not offer, SHALL be refused with an error. A request SHALL keep at most its first 16 options; an option whose id is empty, longer than 1 KiB or holds a control character SHALL be left out, since its id must be sent back unchanged; an option's name SHALL be cut at 4 KiB. When options were left out the log SHALL say how many, and a request left with no option is answered `cancelled`, as one offering none is.

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

#### Scenario: Too many options
- **WHEN** the agent asks permission with 20 options, each named with 64 KiB of text
- **THEN** the attached client is shown the request with its first 16 options, each name cut at 4 KiB, the log says 4 options were left out, and choosing one sends the agent that option's id

#### Scenario: An option id too long to send back
- **WHEN** the agent asks permission with options `allow-once` and a second option whose id is 2 KiB
- **THEN** the request is shown with `allow-once` only, and the log says one option was left out

### Requirement: Tool call locations, raw input and raw result are kept
The server SHALL keep the file locations a tool call names, each a path and an optional line, and the tool call's raw input and raw result as text: a JSON string as its text, any other value as indented JSON. The server SHALL format a raw value only as far as its bound, so a raw value whose indented form is far larger than the message that carried it costs no more than its bound to keep.

#### Scenario: A location
- **WHEN** the agent reports a tool call with location `/p/main.rs` line 12
- **THEN** the entry holds that path and line

#### Scenario: Raw input that is text
- **WHEN** the agent reports a tool call whose raw input is the JSON string `*** Begin Patch\n+hi\n`
- **THEN** the entry's raw input is that text, with its line break

#### Scenario: Raw input that is an object
- **WHEN** the agent reports a tool call whose raw input is `{"command":"pwd"}`
- **THEN** the entry's raw input is that object as indented JSON

#### Scenario: Deeply nested raw input
- **WHEN** the agent reports a tool call whose raw input is 120 nested arrays around four million zeros, in one 8 MiB line
- **THEN** the entry's raw input is the start of its indented form, 16 KiB at most and ending with `…`, and the session and every other session carry on

### Requirement: Tool call content is bounded
One content item SHALL hold at most 64 KiB: a text item keeps its start, a file change its first lines, and command output its end, dropping whole lines from its start. A file change's size SHALL count each kept line with 4 bytes more for its sign and separators and each hunk with 64 bytes more, so a change of many short or empty lines is bounded like any other. A tool call SHALL hold at most 32 items and 256 KiB of items, where an item counts its text, its paths and, for command output, its command line, directory and exit signal, and command output counts as the full 64 KiB it may grow to, whatever it holds yet, so a call keeps at most three outputs and counts the rest as left out; 32 locations; and 16 KiB each of raw input and raw result. A path (a location's, a file change's or its source's, or a command's directory) SHALL be cut at 4 KiB. A terminal id longer than 1 KiB SHALL be ignored, its output not kept. What is left out SHALL be counted in the entry.

#### Scenario: Output past the bound
- **WHEN** a command prints 200 KiB of lines
- **THEN** the entry's output item holds the last lines, at most 64 KiB of them, and counts the bytes dropped before them

#### Scenario: A long text item
- **WHEN** the agent reports a text content block of 100 KiB
- **THEN** the entry holds its first 64 KiB, marked as cut

#### Scenario: Too many items
- **WHEN** the agent reports a tool call with 40 content items
- **THEN** the entry holds the first 32 and counts 8 more

#### Scenario: More command outputs than fit
- **WHEN** the agent reports a tool call with five terminals, each with a short command line
- **THEN** the entry holds the first three outputs, each able to grow to 64 KiB, and counts two more as left out

#### Scenario: The log's bound still holds
- **WHEN** an agent's tool calls fill the log past 16 MiB
- **THEN** the oldest entries are dropped whole, as before, and the log stays within its bound

#### Scenario: A file of empty lines
- **WHEN** the agent reports a diff that adds a file of 300,000 empty lines
- **THEN** the entry's file change holds its first 16,384 lines or fewer, within 64 KiB counted with each line's 4 bytes, and counts the rest as not kept

#### Scenario: Long location paths
- **WHEN** the agent reports a tool call with 40 locations, each a 64 KiB path
- **THEN** the entry holds 32 locations, each path cut at 4 KiB

### Requirement: Clients hold the same tool content as the server
A client that applies the server's transcript events SHALL hold the same tool call content as the server, bounds included. A client SHALL apply the tool content bounds itself to every event it applies: content past them SHALL be dropped and counted as the server counts it, so a client never holds more than the bounds whatever it is sent, and content within them SHALL be kept unchanged. Every kept text of a tool call SHALL have its control characters other than line breaks and tabs replaced, as message text has.

#### Scenario: Bounded output on both sides
- **WHEN** a command prints 200 KiB of lines while a client is attached
- **THEN** the client's copy of the output item equals the server's, text and dropped byte count alike

#### Scenario: Control characters in a path
- **WHEN** the agent reports a file change whose path holds an escape character
- **THEN** the kept path has that character replaced

#### Scenario: Content past the bounds from a server
- **WHEN** a client applies a tool call update carrying 40 content items and a 100 KiB raw input
- **THEN** its copy of the entry holds 32 items and counts at least 8 more, and its raw input is cut at 16 KiB ending with `…`

#### Scenario: A hunk numbered past the largest line number
- **WHEN** a client receives a file change whose hunk starts at line 4,294,967,295 and holds two lines
- **THEN** the event is refused as malformed, that zone shows its session broken with the reason, and Brindle and its other zones carry on
