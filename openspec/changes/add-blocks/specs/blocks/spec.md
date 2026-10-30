# Spec Delta

## Purpose

Group each shell command and its output into a visual block that can be collapsed and can scroll on its own, so long output stays manageable and a command stays visible beside its output.

## ADDED Requirements

### Requirement: Blocks from commands
Each command with an output mark SHALL be shown as a block. Its header SHALL be the rows from the prompt start up to the output start, and its body SHALL be the rows from the output start up to the command's end, or else the next prompt or the cursor. A prompt with no output mark, and rows outside any command, SHALL be drawn as plain rows.

#### Scenario: Command becomes a block
- **WHEN** a shell with integration loaded runs `ls`
- **THEN** the `ls` prompt row is the block's header and the lines `ls` printed are its body

#### Scenario: Empty command line
- **WHEN** the user presses enter on an empty prompt
- **THEN** no block is created for that prompt

#### Scenario: No shell integration
- **WHEN** the shell emits no OSC 133 marks
- **THEN** the terminal looks and scrolls as it would with `blocks = false`

### Requirement: Blocks setting
The `blocks` setting SHALL turn blocks on and off. When it is off, a terminal SHALL draw, scroll and respond to input as if blocks did not exist. Changing it SHALL apply to open terminals on config reload.

#### Scenario: Turned off while running
- **WHEN** blocks are showing and `blocks = false` is saved to the config
- **THEN** every terminal shows its plain grid, with collapsed output visible again

### Requirement: Grid size unchanged
Blocks SHALL NOT change the terminal's rows or columns as programs see them. All block decoration SHALL be painted inside existing rows and padding.

#### Scenario: First command runs
- **WHEN** a terminal shows its first block
- **THEN** `stty size` reports the same size as before

### Requirement: Block height
An expanded block SHALL be as tall as its rows, up to the height cap. The cap SHALL be the viewport's rows minus the rows of the latest prompt, or the full viewport's rows while the latest command is still running. A block taller than the cap SHALL show its header and a window of its body.

#### Scenario: Short output
- **WHEN** a command prints 5 lines in a 40-row terminal
- **THEN** the block is 6 rows tall and has no scroll indicator

#### Scenario: Output reaches the cap
- **WHEN** a command prints 500 lines in a 40-row terminal and finishes, leaving a one-row prompt
- **THEN** the block is 39 rows tall, its command is visible and the prompt is visible below it

#### Scenario: Window made shorter
- **WHEN** the window is resized so the cap falls below a block's height
- **THEN** the block shrinks to the new cap and its command stays visible

### Requirement: Tall headers
A header taller than half the cap SHALL keep only its first rows, up to half the cap, fixed. Its remaining rows SHALL scroll with the body.

#### Scenario: Pasted script
- **WHEN** the user runs a 60-line pasted command in a 40-row terminal
- **THEN** the first 20 rows stay fixed and the rest of the command scrolls with the output

### Requirement: Following the tail
A block's body SHALL show its newest rows until the user scrolls inside it. Typing into the terminal SHALL make the running block follow its tail again.

#### Scenario: Long build
- **WHEN** a running command prints more lines than fit
- **THEN** its command stays at the top of the block and new output appears at the bottom

#### Scenario: Typing returns to the tail
- **WHEN** the user has scrolled up inside a running block and then types
- **THEN** the block shows its newest rows

### Requirement: Inner scroll position
A block scrolled away from its tail SHALL keep showing the same text as output arrives and as the window is resized.

#### Scenario: Reading while it runs
- **WHEN** the user scrolls up inside a running block and the command prints 100 more lines
- **THEN** the rows on screen do not change

#### Scenario: Reflow
- **WHEN** the user has scrolled inside a block and makes the window narrower
- **THEN** the same line of output is at the top of the block's window

### Requirement: Scroll indicator
A block whose body does not fit SHALL show a thumb at the right edge of its body window, sized and placed to show which part of the body is visible.

#### Scenario: Thumb shown
- **WHEN** a block's body is taller than its window
- **THEN** a thumb is drawn, and it reaches the bottom of the window while the block follows its tail

### Requirement: Collapsing
The user SHALL be able to collapse and expand a finished block. A block is finished once its command has ended or a later prompt has started. A collapsed block SHALL show only its header. New blocks SHALL start expanded. A running block, or one whose body is empty, SHALL NOT collapse.

#### Scenario: Collapse a finished block
- **WHEN** the user collapses the block of a finished `cargo build`
- **THEN** its output is hidden and the rows below move up beneath its header

#### Scenario: Running block
- **WHEN** a command is still running
- **THEN** its block has no chevron and the toggle action leaves it expanded

#### Scenario: Empty body
- **WHEN** the user runs and quits `vim`, which leaves no output on the main screen
- **THEN** its block shows only its header, with no chevron

### Requirement: Collapse controls
A collapsible block SHALL show a chevron at the right end of its header's first row, and clicking it SHALL toggle the block. `toggle_block` SHALL toggle the block under the pointer, or else the latest finished block. `collapse_all_blocks` and `expand_all_blocks` SHALL act on every finished block. All three SHALL be in the command palette.

#### Scenario: Chevron click
- **WHEN** the user clicks an expanded block's chevron
- **THEN** the block collapses and no selection starts

#### Scenario: Collapse all
- **WHEN** the user runs "Collapse All Blocks" from the command palette
- **THEN** every finished block in the terminal collapses and a running block stays expanded

### Requirement: Collapsed summary
A collapsed block's header SHALL show its body's line count, the command's duration when known, and its exit status when known, right-aligned on the first row beside the chevron. The summary SHALL be left out when it would touch text on that row.

#### Scenario: Summary
- **WHEN** a build printing 1204 lines finishes successfully in 12 seconds and is collapsed
- **THEN** its header shows `1204 lines`, `12s` and `exit 0`

#### Scenario: Long command line
- **WHEN** the command text reaches the summary's position
- **THEN** the command text stays readable and no summary is shown

### Requirement: Block appearance
Block headers SHALL be drawn on a tint derived from the theme's background, and cells that set their own background SHALL keep it. Body rows SHALL keep the terminal's background. Each block SHALL have a rail at the grid's left edge: red (ANSI 1) for a failed command when `mark_failed_commands` is on, blue (ANSI 4) while running, and a muted color otherwise.

#### Scenario: Failed command
- **WHEN** a command exits with status 1
- **THEN** its rail is red for the height of the block

#### Scenario: Colored output
- **WHEN** a command prints text with its own background color
- **THEN** that output looks the same as it does with `blocks = false`

#### Scenario: Rail in a tmux pane
- **WHEN** a block is shown in a control-mode pane, which has no padding
- **THEN** the rail is drawn over the edge of the first column and the pane's position is unchanged

### Requirement: Wheel latching
The wheel SHALL scroll the block under the pointer if its body can scroll that way, and otherwise the terminal's view. A wheel run SHALL stay on the scroller it started on, without passing into or out of a block, until the wheel has been idle for about 300 ms.

#### Scenario: Scrolling past a huge block
- **WHEN** the user keeps wheeling through history and a 5000-line block moves under the pointer
- **THEN** the view keeps scrolling past it and the block's own position does not change

#### Scenario: Pause, then scroll inside
- **WHEN** the user stops wheeling with the pointer over a tall block and then wheels again
- **THEN** that block's body scrolls

#### Scenario: Reaching a block's end
- **WHEN** a wheel run inside a block reaches the top of its body
- **THEN** the block stops there and the view does not move until the wheel has paused

#### Scenario: After the pause
- **WHEN** the wheel pauses at the top of a block's body and the user wheels up again
- **THEN** the terminal's view scrolls up

### Requirement: Keyboard scrolling
Line, page, top and bottom scrolling and the prompt jumps SHALL move the terminal's view, not a block's body. Prompt jumps SHALL treat a block as one stop and put its header at the top of the view.

#### Scenario: Skipping a tall block
- **WHEN** the view is at the bottom below a 5000-line block and the user presses ctrl-shift-up twice
- **THEN** the first press shows that block's header at the top and the second shows the previous prompt

### Requirement: Full-screen programs
While the alternate screen is active, a terminal SHALL show its grid exactly, with no headers, rails, chevrons or indicators. When the alternate screen ends, the view and every block's state SHALL be as they were.

#### Scenario: Running vim
- **WHEN** the user opens `vim` from a shell with blocks showing
- **THEN** vim fills the whole terminal and none of it is covered

#### Scenario: Leaving vim
- **WHEN** the user had collapsed blocks, then opened and quit `vim`
- **THEN** those blocks are still collapsed

### Requirement: Inline programs
A running block taller than the cap SHALL stop covering the live screen, showing it exactly, while the cursor is on a row its header would cover or the program has mouse reporting on.

#### Scenario: Cursor reaches a covered row
- **WHEN** a program on the main screen moves the cursor to the top row of a full running block
- **THEN** the header steps aside and that row is visible

#### Scenario: Mouse reporting on the main screen
- **WHEN** a program on the main screen enables mouse reporting while its block is at the cap
- **THEN** a click on a row reports the same row the program drew there

### Requirement: Header outlives its text
A block whose header rows have left the scrollback SHALL keep showing its command, as it read when the command started.

#### Scenario: Output longer than the scrollback
- **WHEN** a running command prints more lines than `scrollback_lines`
- **THEN** its block still shows its command at the top

### Requirement: Selection in blocks
Selection SHALL work on the text shown at each row. A selection that spans a collapsed block SHALL include its hidden lines when copied. Dragging past the top or bottom of a body window SHALL scroll that body, then the view once the body has reached its end.

#### Scenario: Copy across a collapsed block
- **WHEN** the user selects from the row above a collapsed block to the row below it and copies
- **THEN** the clipboard holds the block's header, its hidden output and both neighbouring rows

#### Scenario: Drag inside a body
- **WHEN** the user drags past the bottom of a tall block's window
- **THEN** the body scrolls and the selection extends with it

### Requirement: Pointer targets in blocks
Links, double- and triple-click selection, and mouse reports SHALL act on the text shown under the pointer, however the rows above it are collapsed or scrolled.

#### Scenario: Link below a collapsed block
- **WHEN** the user ctrl-clicks a URL in a row below a collapsed block
- **THEN** that URL opens

### Requirement: Blocks in tmux panes
Blocks SHALL work in control-mode panes as they do in local tabs. Each pane SHALL have its own blocks. Text restored on attach SHALL have no blocks until the shell marks new commands, and classic tmux SHALL show none.

#### Scenario: Control-mode pane
- **WHEN** a shell with integration loaded runs a command in a control-mode pane
- **THEN** the pane shows a block for it, and scrolling it leaves other panes unchanged

#### Scenario: Reattach
- **WHEN** Brindle reattaches to a session with earlier output in a pane
- **THEN** that output is drawn as plain rows
