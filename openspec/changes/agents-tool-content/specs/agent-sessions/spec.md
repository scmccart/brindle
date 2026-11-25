## ADDED Requirements

### Requirement: Tool calls keep their content
The server SHALL keep each tool call's content in its log entry, in the agent's order: text, file changes and command output. When the agent replaces a tool call's content the entry's content SHALL be replaced, and when it appends to it the entry's content SHALL grow. Content SHALL be in every replay and SHALL survive detaches, core restarts and upgrades.

#### Scenario: Text content
- **WHEN** the agent reports tool call `call_1` with a text content block `done`
- **THEN** the log's entry for `call_1` holds one text item `done`

#### Scenario: Content replaced
- **WHEN** the agent reports `call_1` with a text item `a`, then updates `call_1` with a content list holding only a text item `b`
- **THEN** the entry holds one text item, `b`

#### Scenario: Replayed after a restart
- **WHEN** a tool call's entry holds a file change, the core restarts, and a client attaches the session
- **THEN** the replay holds the tool call with the same file change

#### Scenario: Unchanged content sends nothing
- **WHEN** the agent updates a tool call with the content list it already has
- **THEN** attached clients receive no event for it

### Requirement: Tool content Brindle doesn't show is named
A text block in a tool call's content SHALL be kept as its text. A content block that isn't text, and content of a kind the server doesn't know, SHALL be kept as a short text saying what was left out, with none of its data.

#### Scenario: An image in tool content
- **WHEN** the agent reports a tool call whose content is an image block
- **THEN** the entry holds a text item naming an image, and no image data

#### Scenario: An unknown kind of content
- **WHEN** the agent reports a tool call whose content has the type `chart`
- **THEN** the entry holds a text item saying content of type `chart` isn't shown

### Requirement: File changes are kept as hunks
A file change SHALL be kept as its operation (add, delete, modify, move or copy, or the agent's own name for another), its path, the path it came from for a move or copy, and its changed lines as hunks: each hunk's starting line in the old and the new file, and its lines, each context, added or removed. A change with no changed lines to show SHALL be kept with no hunks.

#### Scenario: A change with no changed lines
- **WHEN** the agent reports a diff of `/p/a.rs` whose old and new text are the same
- **THEN** the entry holds one file change, `modify` `/p/a.rs`, with no hunks

### Requirement: Hunks are computed from file texts
When the agent gives a file's old and new text, the server SHALL compute the file change's hunks, with three lines of context, and SHALL NOT keep the texts. A file with no old text SHALL be an add. The work SHALL be bounded whatever the texts, and the hunks SHALL always turn the old text into the new one as far as they are kept.

#### Scenario: One line changed
- **WHEN** the agent reports a diff of `/p/main.rs` whose old text has 100 lines and whose new text differs in line 50 only
- **THEN** the entry holds one file change, `modify` `/p/main.rs`, with one hunk starting at line 47 that holds three context lines, the removed line, the added line and three context lines

#### Scenario: A new file
- **WHEN** the agent reports a diff of `/p/new.txt` with no old text and new text `hi\n`
- **THEN** the entry holds one file change, `add` `/p/new.txt`, with one hunk holding the added line `hi`

#### Scenario: A very large rewrite
- **WHEN** the agent reports a diff whose old and new text each hold 50,000 lines with nothing in common
- **THEN** the entry's file change removes old lines and adds new ones up to the item bound, with the rest counted as not kept, and the session carries on

### Requirement: Command output is kept as plain text
A command's output SHALL be kept as one item: its command line and directory when the agent gives them, its text, and its exit status or signal once the agent reports one. Output the agent sends in pieces SHALL be appended to the item's text as it arrives, and attached clients SHALL be sent each piece. Escape sequences SHALL be removed from the text, including one split across two pieces; of a line holding carriage returns, only what follows the last one SHALL be kept.

#### Scenario: Streamed output
- **WHEN** the agent reports a command's output as `com`, then `piling\n`, then an exit status of 0
- **THEN** the entry holds one output item with text `compiling\n` and exit status 0, and an attached client received the two pieces in order

#### Scenario: Colour sequences
- **WHEN** a command's output is `\x1b[31merror\x1b[0m: bad\n`
- **THEN** the kept text is `error: bad\n`

#### Scenario: A sequence split across pieces
- **WHEN** a command's output arrives as `\x1b[3` and then `1mred\n`
- **THEN** the kept text is `red\n`

#### Scenario: A progress line
- **WHEN** a command's output is `10%\r50%\r100%\n`
- **THEN** the kept text is `100%\n`

### Requirement: Tool call locations, raw input and raw result are kept
The server SHALL keep the file locations a tool call names, each a path and an optional line, and the tool call's raw input and raw result as text: a JSON string as its text, any other value as indented JSON.

#### Scenario: A location
- **WHEN** the agent reports a tool call with location `/p/main.rs` line 12
- **THEN** the entry holds that path and line

#### Scenario: Raw input that is text
- **WHEN** the agent reports a tool call whose raw input is the JSON string `*** Begin Patch\n+hi\n`
- **THEN** the entry's raw input is that text, with its line break

#### Scenario: Raw input that is an object
- **WHEN** the agent reports a tool call whose raw input is `{"command":"pwd"}`
- **THEN** the entry's raw input is that object as indented JSON

### Requirement: Tool call content is bounded
One content item SHALL hold at most 64 KiB: a text item keeps its start, a file change its first lines, and command output its end, dropping whole lines from its start. A tool call SHALL hold at most 32 items and 256 KiB of items, 32 locations, and 16 KiB each of raw input and raw result. What is left out SHALL be counted in the entry.

#### Scenario: Output past the bound
- **WHEN** a command prints 200 KiB of lines
- **THEN** the entry's output item holds the last lines, at most 64 KiB of them, and counts the bytes dropped before them

#### Scenario: A long text item
- **WHEN** the agent reports a text content block of 100 KiB
- **THEN** the entry holds its first 64 KiB, marked as cut

#### Scenario: Too many items
- **WHEN** the agent reports a tool call with 40 content items
- **THEN** the entry holds the first 32 and counts 8 more

#### Scenario: The log's bound still holds
- **WHEN** an agent's tool calls fill the log past 16 MiB
- **THEN** the oldest entries are dropped whole, as before, and the log stays within its bound

### Requirement: Clients hold the same tool content as the server
A client that applies the server's transcript events SHALL hold the same tool call content as the server, bounds included. Every kept text of a tool call SHALL have its control characters other than line breaks and tabs replaced, as message text has.

#### Scenario: Bounded output on both sides
- **WHEN** a command prints 200 KiB of lines while a client is attached
- **THEN** the client's copy of the output item equals the server's, text and dropped byte count alike

#### Scenario: Control characters in a path
- **WHEN** the agent reports a file change whose path holds an escape character
- **THEN** the kept path has that character replaced

### Requirement: An unreadable content item loses only itself
A content item the server can't read SHALL be left out, and the rest of the tool call's update SHALL be applied. The log SHALL get one notice for the tool call saying how many items couldn't be read.

#### Scenario: One bad item
- **WHEN** the agent reports a tool call titled `Edit a.rs` with status `in_progress` and two content items, the first a diff with no path
- **THEN** the entry shows `in_progress` and holds the second item, and a notice says one content item of `Edit a.rs` couldn't be read

### Requirement: A permission request's tool call is kept with the call
When a permission request describes the tool call it asks about, the server SHALL apply that description's content, locations and raw input to the tool call's entry before adding the permission request, adding the entry when the log has none for that call. The description's title SHALL stay the permission request's own and SHALL NOT change the tool call's title, kind or status.

#### Scenario: Content only in the request
- **WHEN** the agent reports tool call `call_1` with no content, then asks permission for `call_1` with a diff of `/p/a.txt` in the request
- **THEN** the entry for `call_1` holds that file change, and the permission request follows it in the log

#### Scenario: A request for a call the log doesn't have
- **WHEN** the agent asks permission for `call_9`, which it never reported, with a title and a raw input
- **THEN** the log gains a tool call entry for `call_9` with that raw input, then the permission request

#### Scenario: The request's title differs
- **WHEN** tool call `call_1` is titled `Write a.txt` and the agent asks permission for it with the title `Allow writing a.txt?`
- **THEN** the tool call's title is still `Write a.txt`, and the permission request's is `Allow writing a.txt?`
