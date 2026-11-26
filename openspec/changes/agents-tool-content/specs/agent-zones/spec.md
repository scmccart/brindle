## MODIFIED Requirements

### Requirement: Tool call rows
A tool call SHALL be shown as a row whose line holds an icon for its kind, its title cut to one line, a summary of its content, and its status (pending, in progress, completed or failed). The summary SHALL give the lines added and removed over the call's file changes, and the exit status or signal of its last command output that has one. An update to the call SHALL change that row in place. A row whose tool call has a detail SHALL show that it can be opened, and whether it is open.

#### Scenario: Updated in place
- **WHEN** the agent reports tool call `call_1` titled `Read main.rs` as pending, then as completed
- **THEN** the conversation has one line for `call_1`, which shows completed

#### Scenario: A diff's summary
- **WHEN** a tool call holds a file change with 12 added and 3 removed lines
- **THEN** its row's line shows `+12 −3`, and that the row can be opened

#### Scenario: A command's exit
- **WHEN** a tool call's command output ends with exit status 1
- **THEN** its row's line shows `exit 1`

#### Scenario: Nothing to open
- **WHEN** a tool call has no content, no locations and no raw input or result
- **THEN** its row is one line with no sign that it opens

### Requirement: Permission rows
A permission request SHALL be shown as a row with its title and one button per option the agent offers, labelled with the option's name, reject options before allow options, each group in the agent's order. While the request is pending, the detail of the tool call it names SHALL be shown with it, above its buttons, whatever the settings, and SHALL NOT be shown a second time in that tool call's own row. Choosing an option SHALL send it as the answer. Once answered, the row SHALL show the chosen option, or that the request was cancelled, and offer no buttons and no detail.

#### Scenario: Reject from the keyboard
- **WHEN** the agent asks `Run cargo test?` with options `Allow once` and `Reject`, the prompt input has focus, nothing is queued, and the user presses tab and then enter
- **THEN** the agent receives `Reject`, and the row says `Reject` was chosen

#### Scenario: Allow from the keyboard
- **WHEN** the agent asks `Run cargo test?` with options `Allow once` and `Reject`, the prompt input has focus, nothing is queued, and the user presses tab twice and then enter
- **THEN** the agent receives `Allow once`, and the row says `Allow once` was chosen

#### Scenario: Rejects come first
- **WHEN** the agent asks permission with options `Always allow`, `Allow once` and `Reject`, in that order
- **THEN** the row's buttons read `Reject`, `Always allow`, `Allow once`, in that order

#### Scenario: Reject with the pointer
- **WHEN** a permission row is pending and the user clicks `Reject`
- **THEN** the agent receives `Reject`, and the row says so

#### Scenario: Cancelled by stopping the turn
- **WHEN** a permission row is pending and the user stops the turn
- **THEN** the row says the request was cancelled and offers no buttons

#### Scenario: The edit is shown before it is allowed
- **WHEN** the agent asks permission for a tool call that holds a file change of `/p/main.rs`
- **THEN** the permission row shows that file change's lines above its buttons, and the tool call's own row shows its line alone

#### Scenario: Shown even when rows start closed
- **WHEN** the config sets `tool_details = "closed"` and the agent asks permission for a tool call that holds a file change
- **THEN** the permission row shows the file change above its buttons

#### Scenario: After the answer
- **WHEN** the user allows that request and the tool call goes on running
- **THEN** the permission row shows the chosen option alone, and the file change shows in the tool call's own row again

#### Scenario: A request naming no tool call
- **WHEN** the agent asks permission without naming a tool call
- **THEN** the row shows its title and buttons alone

#### Scenario: Inside the dangerous row
- **WHEN** the agent asks permission for a tool call that holds a file change, in any built-in theme
- **THEN** the file change shows inside the row's destructive border, its added and removed lines on their own colours

#### Scenario: A long turn before the request
- **WHEN** Brindle restores an agent zone whose session waits on a permission request for a tool call with a file change, and more than a page of entries came between the tool call and the request
- **THEN** the permission row shows the file change above its buttons, without the user scrolling up

### Requirement: Agent commands in the palette
While an agent zone has focus, the command palette SHALL list Stop Agent Turn, Remove Queued Prompt, Send Held Prompt Again, Copy Queued Prompt, Copy Last Agent Message, Copy Last Code Block, Toggle Agent Thoughts, Toggle Tool Details and Scroll Agent to Bottom, with the zone and tab commands, and SHALL NOT list commands that act on a terminal, such as Clear Scrollback or Find.

#### Scenario: Palette in an agent zone
- **WHEN** an agent zone has focus and the user opens the command palette
- **THEN** it lists "Stop Agent Turn", "Copy Last Code Block", "Toggle Tool Details" and "Split Zone Right", and not "Clear Scrollback"

## ADDED Requirements

### Requirement: Tool call details
An open tool call row SHALL show its detail below its line: the tool call's content items in the agent's order, then its locations as paths with their lines. Raw input and raw result SHALL be shown, as text, only when the tool call has no content items. A text item SHALL be rendered as agent messages are. Where content was left out, the detail SHALL say how much.

#### Scenario: Raw input as the fallback
- **WHEN** a tool call has no content and a raw input holding a patch as text
- **THEN** its open row shows that text

#### Scenario: Raw input hidden beside content
- **WHEN** a tool call has a file change and a raw input
- **THEN** its open row shows the file change and not the raw input

#### Scenario: A cut diff says so
- **WHEN** a file change kept its first lines and counts 400 more
- **THEN** its detail ends with a line saying 400 more lines aren't shown

### Requirement: Tool call rows open while they run
A tool call row that has a detail SHALL be open while its tool call is running and SHALL close when the tool call finishes, unless the `[agent]` settings `tool_details` and `tool_details_when_done` say otherwise. A tool call is running while its status is pending or in progress and its turn is running; it has finished once it is completed or failed, or its turn has ended. A row the user opened or closed by hand SHALL NOT be changed by this.

#### Scenario: Open, then closed
- **WHEN** the agent reports a tool call with a file change as in progress, and later as completed
- **THEN** its row is open while in progress and closed once completed

#### Scenario: A call that arrives finished
- **WHEN** the agent reports a tool call with a file change that is already completed
- **THEN** its row is closed

#### Scenario: The turn ends first
- **WHEN** a tool call is still in progress when its turn is stopped
- **THEN** its row closes

#### Scenario: Opened by hand stays open
- **WHEN** the user closes a running tool call's row and opens it again, and the tool call then completes
- **THEN** the row is still open

#### Scenario: A restored conversation
- **WHEN** Brindle is launched and restores an agent zone whose earlier turns hold finished tool calls
- **THEN** those rows are closed

#### Scenario: Earlier messages fetched
- **WHEN** the user scrolls an agent zone up and earlier entries holding finished tool calls with file changes are fetched
- **THEN** those rows are closed, and no notification appears

### Requirement: Opening a tool call row
A pointer press on the line of a tool call row that has a detail SHALL open or close it. A row SHALL stay as the user left it while its tool call changes. A detail SHALL add no keyboard stop.

#### Scenario: Opened with the pointer
- **WHEN** a closed tool call row has a file change and the user presses its line
- **THEN** the file change shows below the line, and a second press hides it

#### Scenario: Open rows survive updates
- **WHEN** a tool call row is open and the agent appends to the call's command output
- **THEN** the row stays open and shows the new output

#### Scenario: Tab passes over a detail
- **WHEN** a tool call row is open, the prompt input has focus and the user presses tab
- **THEN** focus goes to the zone's next stop, never into the detail

### Requirement: Diffs in the conversation
A file change SHALL be shown with its operation and path, the path it came from for a move or copy, and its hunks' lines in the terminal's font family, each line with a sign (none for context, `+` for added, `−` for removed) and its line number, added and removed lines on their own background colours. A file change with no hunks SHALL show its operation and path alone.

#### Scenario: A modified file
- **WHEN** an open tool call row holds a `modify` of `/p/main.rs` with one removed and one added line among context
- **THEN** the row shows `/p/main.rs`, the removed line with `−`, the added line with `+`, and the context lines with no sign, each with its line number

#### Scenario: A change with no lines
- **WHEN** a tool call holds a `modify` of `/p/a.rs` with no hunks
- **THEN** its detail shows that `/p/a.rs` was modified, and no lines

### Requirement: Command output in the conversation
Command output SHALL be shown as a block in the terminal's font family, labelled with its command line when there is one, with its exit status or signal once known. While its tool call's row is open and the conversation is at its end, new output SHALL stay in view as it arrives.

#### Scenario: Output as it arrives
- **WHEN** a tool call row is open, the conversation is at its end, and its command prints more lines than the zone holds
- **THEN** the latest line stays in view

#### Scenario: A failed command
- **WHEN** a command's output ends with exit status 2
- **THEN** its block shows the exit status 2

### Requirement: Copying tool call content
Each file change and each command output in a detail SHALL have a copy button, operated with the pointer. A file change SHALL copy as unified diff text, and command output as its text.

#### Scenario: Copy a diff
- **WHEN** the user clicks the copy button of a file change that removes `old();` and adds `new();` in `/p/main.rs`
- **THEN** the clipboard holds unified diff text naming `/p/main.rs` with a `-    old();` line and a `+    new();` line

#### Scenario: Copy output
- **WHEN** the user clicks the copy button of a command output whose text is `ok\n`
- **THEN** the clipboard holds `ok\n`

### Requirement: Toggling every tool detail
The `toggle_agent_tool_details` action SHALL open every tool call row of the zone that has a detail when any of them is closed, and close them all otherwise, as if the user had opened or closed each by hand. The command palette SHALL list it as Toggle Tool Details while an agent zone is active, and not in a terminal zone.

#### Scenario: Open all
- **WHEN** an agent zone has three tool call rows with details, one of them open, and the user runs Toggle Tool Details
- **THEN** all three are open, and running it again closes all three

#### Scenario: Not in a terminal
- **WHEN** a terminal zone is active and the user opens the command palette
- **THEN** Toggle Tool Details is not listed
