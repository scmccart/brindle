## MODIFIED Requirements

### Requirement: Permission rows
A permission request SHALL be shown as a row with its title and one button per option the agent offers, labelled with the option's name, reject options before allow options, each group in the agent's order. Choosing an option SHALL send it as the answer. Once answered, the row SHALL show the chosen option, or that the request was cancelled, and offer no buttons.

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

## ADDED Requirements

### Requirement: Pending permission rows look dangerous
While a permission request is pending, its row SHALL be drawn as a destructive alert: a warning icon before its title, a background tinted with the zone's destructive colour, a border in that colour, its allow options as destructive buttons and its reject options as outline buttons. Once answered or cancelled, the row SHALL be drawn in the conversation's plain style.

#### Scenario: A pending request
- **WHEN** the agent asks `Run cargo test?` with options `Allow once` and `Reject`
- **THEN** the row shows a warning icon, a destructive border and tinted background, `Allow once` as a destructive button and `Reject` as an outline button

#### Scenario: Answered
- **WHEN** the user chooses `Reject` on that row
- **THEN** the row shows `Reject` as the outcome, with no warning icon, tint or destructive border

### Requirement: The working row
While an agent zone's turn is working, its conversation SHALL end with a muted status row that isn't an entry and isn't copied: the title of the turn's newest tool call in progress, else `Working…`, and `Stopping…` while a cancel is under way. The row SHALL NOT show while the turn waits on a permission, and SHALL go when the turn ends or fails. Its text SHALL shimmer while it shows.

#### Scenario: Before the first chunk
- **WHEN** the user sends `hi` and the agent hasn't answered yet
- **THEN** the conversation ends with the row `Working…`

#### Scenario: A tool call in progress
- **WHEN** the agent reports tool call `Read main.rs` as in progress
- **THEN** the row reads `Read main.rs`, and reads `Working…` again once that call completes

#### Scenario: Waiting on a permission
- **WHEN** the agent asks permission during the turn
- **THEN** the conversation ends with the permission row and no working row, and the working row comes back once the request is answered and the turn goes on

#### Scenario: Stopping
- **WHEN** the user stops a working turn and the agent hasn't answered the cancel yet
- **THEN** the row reads `Stopping…`

#### Scenario: Gone at the end
- **WHEN** the agent ends the turn
- **THEN** the conversation has no working row

### Requirement: A spinner while an agent works
While an agent zone's status is `working`, its status dot SHALL be drawn as a small spinner: on its zone header when the tab has two or more zones, and on its tab's face when the tab's shown status is `working` and one of the tab's working zones is an agent zone. Other statuses keep their dots, and a terminal's `working` record keeps its dot.

#### Scenario: Scrolled away
- **WHEN** an agent zone in a tab of two zones is working
- **THEN** its zone header shows a spinner in place of its status dot

#### Scenario: A background tab
- **WHEN** an agent in a background tab of one zone is working
- **THEN** that tab's face shows a spinner, and the green dot once the turn ends normally

### Requirement: Motion only while an agent works
Spinners and shimmering text SHALL advance in steps, at most 10 a second, from one clock for the whole window. The clock SHALL run only while some agent zone of the window is working, and SHALL NOT wake or redraw the window when none is.

#### Scenario: Stops at the end of the turn
- **WHEN** the only working agent zone's turn ends
- **THEN** the window's motion clock stops, and nothing in the window moves until an agent works again

#### Scenario: One clock for many zones
- **WHEN** two agent zones in different tabs are working
- **THEN** the window redraws for motion at most 10 times a second, not once per zone
