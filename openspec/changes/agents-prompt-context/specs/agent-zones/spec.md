## ADDED Requirements

### Requirement: Attachments in the prompt input
An agent zone SHALL show the attachments waiting to go with the next prompt as a row of items above the prompt input's text, each with its kind, its name and, for text, its size and whether it was cut. Sending the prompt SHALL send them with it and clear them once the server accepts it; a refused prompt SHALL keep its attachments with its text. Blank text SHALL NOT be sent, with attachments or without. The items SHALL NOT be keyboard stops.

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
Adding an attachment that would make more than 8, or more than 128 KiB of attached text together, SHALL be refused: the window's notice line SHALL say which limit, and the attachments already there SHALL stay. Text longer than 64 KiB SHALL be attached cut to 64 KiB, the output of a command keeping its end and any other text its start, and its item SHALL say it was cut.

#### Scenario: A ninth attachment
- **WHEN** the prompt input holds 8 attachments and the user attaches a file
- **THEN** the notice line says a prompt takes at most 8 attachments, and 8 remain

#### Scenario: Long output keeps its end
- **WHEN** the user sends a command's output of 200 KiB to an agent
- **THEN** the attachment holds the last 64 KiB of it, and its item says it was cut

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

### Requirement: Sending terminal text to an agent
In a terminal zone, the `send_selection_to_agent` action SHALL attach the selected text, and the `send_last_output_to_agent` action the output of the last command, named by its command line when the shell reported one, to an agent zone's prompt as a text attachment. It SHALL then show that agent zone with keyboard focus in its prompt input, and SHALL NOT send anything.

#### Scenario: The last command's output
- **WHEN** the user ran `cargo test` in a terminal tab, one agent zone is open in another tab, and the user runs Send Last Command Output to Agent
- **THEN** the agent's tab is active, its prompt input has focus and a text attachment named `cargo test` holding that output, and the agent has received nothing

#### Scenario: A selection
- **WHEN** the user selects two lines in a terminal and runs Send Selection to Agent
- **THEN** the agent zone's prompt input has a text attachment holding those two lines

### Requirement: When the terminal's agent commands are listed
The command palette SHALL list Send Selection to Agent only while the terminal has a selection, and Send Last Command Output to Agent only once the terminal has received a prompt mark, as Copy Last Command Output is. Neither SHALL be listed in an agent zone.

#### Scenario: Nothing selected
- **WHEN** a terminal has no selection and the user opens the command palette
- **THEN** Send Selection to Agent is not listed

#### Scenario: No shell integration
- **WHEN** a terminal has received no prompt mark
- **THEN** the command palette doesn't list Send Last Command Output to Agent

### Requirement: Choosing the agent for terminal text
When the window has several agent zones, sending terminal text to an agent SHALL first list them, the most recently focused first, each by its tab's and zone's titles, and attach to the one the user chooses; escape SHALL attach nothing. With one agent zone it SHALL be used without asking. With none, the window's notice line SHALL say no agent zone is open.

#### Scenario: Two agents
- **WHEN** two agent zones are open and the user runs Send Selection to Agent
- **THEN** a list shows both, the one focused last first, and choosing the second attaches the selection there

#### Scenario: No agent
- **WHEN** no agent zone is open and the user runs Send Selection to Agent
- **THEN** the notice line says no agent zone is open, and the terminal keeps focus

### Requirement: Attachments in the conversation
A user message SHALL list its attachments under its text: a link as its path, and a text attachment as a row with its name and size that opens, on a pointer press, to show the text that was sent, in the terminal's font family, with which end was cut when it was. Rows SHALL start closed and SHALL add no keyboard stop. A waiting or held prompt's row SHALL say how many attachments it has.

#### Scenario: Shown with the message
- **WHEN** the user sends `why?` with a text attachment `cargo test` and a link to `src/main.rs`
- **THEN** the user message shows `why?`, a closed row `cargo test` with its size, and `src/main.rs`

#### Scenario: Opened
- **WHEN** the user presses the `cargo test` row
- **THEN** the attached output shows under it

#### Scenario: A queued prompt
- **WHEN** a prompt with two attachments waits in the queue
- **THEN** its row above the prompt input says it has 2 attachments
