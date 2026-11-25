## ADDED Requirements

### Requirement: Attachments in the prompt input
An agent zone SHALL show the attachments waiting to go with the next prompt as a row of items above the prompt input's text, each with its kind, its name and, for text, its size. Sending the prompt SHALL send them with it and clear them once the server accepts it; a refused prompt SHALL keep its attachments with its text. Blank text SHALL NOT be sent, with attachments or without. The items SHALL NOT be keyboard stops.

#### Scenario: Sent with the prompt
- **WHEN** the prompt input holds one text attachment and the user types `why?` and presses enter
- **THEN** the agent receives the prompt with the attachment, and the input has neither text nor attachments

#### Scenario: Attachments need words
- **WHEN** the prompt input holds an attachment and no text, and the user presses enter
- **THEN** nothing is sent, and the attachment stays

#### Scenario: Refused
- **WHEN** the server refuses a prompt that has an attachment
- **THEN** the text and the attachment both stay, with the reason beside the input

### Requirement: Removing attachments
With the cursor at the very start of the prompt input's text and nothing selected, backspace SHALL remove the last attachment. The `agent_clear_attachments` action SHALL remove them all, and a pointer press on an item's remove mark SHALL remove that item. The command palette SHALL list Clear Attachments only while the active agent zone has attachments.

#### Scenario: Backspace at the start
- **WHEN** the prompt input holds two attachments and the text `hi` with the cursor before the `h`, and the user presses backspace
- **THEN** one attachment remains, and the text is still `hi`

#### Scenario: Backspace in the text
- **WHEN** the cursor is after the `h` and the user presses backspace
- **THEN** the `h` is removed, and both attachments remain

#### Scenario: Clear
- **WHEN** the user runs Clear Attachments with two attachments
- **THEN** the prompt input has none, and its text is unchanged

### Requirement: Attachment limits are said when attaching
Adding a ninth attachment SHALL be refused: the window's notice line SHALL say a prompt takes at most 8, and the attachments already there SHALL stay. Text longer than 256 KiB SHALL be refused outright, never shortened: the notice line SHALL say how long it is and what the limit is, and nothing SHALL be attached.

#### Scenario: A ninth attachment
- **WHEN** the prompt input holds 8 attachments and the user attaches a file
- **THEN** the notice line says a prompt takes at most 8 attachments, and 8 remain

#### Scenario: Output that is too long
- **WHEN** the user runs Send Last Command Output to Agent for a command that printed 1 MiB
- **THEN** nothing is attached, the notice line says the output is 1 MiB and an attachment takes at most 256 KiB, and the terminal keeps focus

#### Scenario: A dropped file that is too long
- **WHEN** the user drops a 300 KiB text file on the zone of an agent on an SSH host
- **THEN** it isn't attached, and the notice line names the file, its size and the limit

### Requirement: Attaching a file by path
The `agent_attach_file` action SHALL ask for a path and attach it as a link, as typed, without checking it; the agent reads the file on its own host, where a relative path is relative to the session's working directory. The command palette SHALL list it as Attach File… while an agent zone is active.

#### Scenario: A relative path
- **WHEN** the user runs Attach File…, types `src/main.rs` and presses enter
- **THEN** the prompt input shows a link attachment `main.rs`, and focus is back in the prompt input

#### Scenario: Cancelled
- **WHEN** the user runs Attach File… and presses escape
- **THEN** nothing is attached

### Requirement: Dropping files on an agent zone
Files dropped on an agent zone SHALL be attached. When the zone's agent runs on this machine, each SHALL be a link to its path. When it runs on another host or in a container, a file that is text SHALL be attached as text under its file name, and a file that isn't SHALL be refused. A directory, and a file that can't be read, SHALL be refused. Each refusal SHALL name the file and the reason on the window's notice line, and SHALL NOT stop the other files being attached.

#### Scenario: A local agent
- **WHEN** the user drops `/home/u/a.rs` on the zone of an agent that runs on this machine
- **THEN** the prompt input shows a link attachment to `/home/u/a.rs`

#### Scenario: A remote agent
- **WHEN** the user drops a 2 KiB text file `notes.md` on the zone of an agent on an SSH host
- **THEN** the prompt input shows a text attachment `notes.md` holding the file's text

#### Scenario: Not text, for a remote agent
- **WHEN** the user drops `logo.png` and `notes.md` on that zone
- **THEN** `notes.md` is attached, and the notice line says `logo.png` isn't text and can't be sent to an agent on that host

### Requirement: Sending a command's output to an agent
In a terminal zone, the `send_last_output_to_agent` action SHALL attach the output of the last command, named by its command line when the shell reported one, to an agent zone's prompt as a text attachment. It SHALL then show that agent zone with keyboard focus in its prompt input, and SHALL NOT send anything: the output reaches the agent only when the user sends the prompt.

#### Scenario: The last command's output
- **WHEN** the user ran `cargo test` in a terminal tab, one agent zone is open in another tab, and the user runs Send Last Command Output to Agent
- **THEN** the agent's tab is active, its prompt input has focus and a text attachment named `cargo test` holding that output, and the agent has received nothing

#### Scenario: No command line known
- **WHEN** the shell reported no command line for the last command, in a tab titled `zsh`
- **THEN** the attachment is named `output from zsh`

### Requirement: What output a terminal sends
The output sent from a terminal SHALL be the text Copy Last Command Output would copy, with lines joined by a line feed on every platform: what the terminal shows, without the prompt or the command line. When Copy Last Command Output would copy nothing, nothing SHALL be attached and the terminal SHALL keep focus.

#### Scenario: The same text as copying
- **WHEN** the user ran `printf 'one\ntwo\n'` and runs Send Last Command Output to Agent
- **THEN** the attachment's text is `one` and `two` on two lines, the same text Copy Last Command Output copies, without the prompt line

#### Scenario: Colours are not sent
- **WHEN** the command printed its output in red
- **THEN** the attachment holds the output's characters and no escape sequence

#### Scenario: A wrapped line
- **WHEN** the output holds one long line the terminal wrapped over two rows
- **THEN** the attachment holds it as one line

#### Scenario: Output that has scrolled away
- **WHEN** the last command's output has left the scrollback and the user runs Send Last Command Output to Agent
- **THEN** nothing is attached, and the terminal keeps focus

### Requirement: When Send Last Command Output to Agent is listed
The command palette SHALL list Send Last Command Output to Agent only once the terminal has received a prompt mark, as Copy Last Command Output is, and never in an agent zone.

#### Scenario: No shell integration
- **WHEN** a terminal has received no prompt mark
- **THEN** the command palette doesn't list Send Last Command Output to Agent

#### Scenario: In an agent zone
- **WHEN** an agent zone is active and the user opens the command palette
- **THEN** Send Last Command Output to Agent is not listed

### Requirement: Choosing the agent for a command's output
When the window has several agent zones, sending a command's output to an agent SHALL first list them, the most recently focused first, each by its tab's and zone's titles, and attach to the one the user chooses; escape SHALL attach nothing. With one agent zone it SHALL be used without asking. With none, the window's notice line SHALL say no agent zone is open.

#### Scenario: Two agents
- **WHEN** two agent zones are open and the user runs Send Last Command Output to Agent
- **THEN** a list shows both, the one focused last first, and choosing the second attaches the output there

#### Scenario: No agent
- **WHEN** no agent zone is open and the user runs Send Last Command Output to Agent
- **THEN** the notice line says no agent zone is open, and the terminal keeps focus

### Requirement: Attachments in the conversation
A user message SHALL list its attachments under its text: a link as its path, and a text attachment as a row with its name and size. Rows SHALL start closed and SHALL add no keyboard stop. A waiting or held prompt's row SHALL say how many attachments it has.

#### Scenario: Shown with the message
- **WHEN** the user sends `why?` with a text attachment `cargo test` and a link to `src/main.rs`
- **THEN** the user message shows `why?`, a closed row `cargo test` with its size, and `src/main.rs`

### Requirement: Opening a text attachment
A pointer press on a text attachment's row SHALL fetch its text from the server and show it under the row, in the terminal's font family; until it arrives the row SHALL show that it is loading. A second press SHALL close it. When the server no longer keeps the text, the row SHALL say so. An agent zone SHALL hold the text of open rows only.

#### Scenario: Opened
- **WHEN** the user presses the `cargo test` row
- **THEN** the row shows it is loading, then the attached output shows under it

#### Scenario: After a relaunch
- **WHEN** Brindle is quit and launched again, and the user opens the same row in the restored zone
- **THEN** the same output shows

#### Scenario: No longer kept
- **WHEN** the server can't read the attachment's text any more and the user presses its row
- **THEN** the row says the text is no longer kept

#### Scenario: A queued prompt
- **WHEN** a prompt with two attachments waits in the queue
- **THEN** its row above the prompt input says it has 2 attachments
