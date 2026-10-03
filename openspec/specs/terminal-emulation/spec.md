# terminal-emulation Specification

## Purpose
Run programs on a pseudo-terminal and carry keyboard, mouse, paste, focus and clipboard traffic between the user and the program using xterm-compatible encodings, so shells, editors and tmux behave as they do in mainstream terminals.

## Requirements

### Requirement: Program launch environment
The terminal SHALL start each program on its own PTY with `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=Brindle` and `TERM_PROGRAM_VERSION` set, plus any environment the profile defines. Profile entries SHALL override the defaults.

#### Scenario: Default environment
- **WHEN** a tab starts the user's shell
- **THEN** `echo $TERM $COLORTERM $TERM_PROGRAM` prints `xterm-256color truecolor Brindle`

#### Scenario: Profile environment override
- **WHEN** a profile sets `env = { EDITOR = "nvim" }`
- **THEN** programs in tabs opened with that profile see `EDITOR=nvim`

### Requirement: Launch failure is reported in the tab
If the program cannot be started, the terminal SHALL show an error naming the program and the reason, instead of failing silently.

#### Scenario: Missing program
- **WHEN** a tab is opened with a command that does not exist
- **THEN** the tab shows text saying Brindle could not start that program, along with the OS error

### Requirement: Keyboard encoding
Special keys and modifier combinations SHALL be sent as xterm byte sequences. Plain text, with no modifiers or shift only, SHALL arrive through the platform text-input path so IME composition and dead keys work.
- Cursor and Home/End keys SHALL follow application-cursor mode when unmodified.
- Modified keys SHALL use the `CSI 1;m X` and `CSI n;m ~` forms.
- Ctrl+letter SHALL send the C0 control code.
- Alt SHALL act as Meta by prefixing ESC.

#### Scenario: Control codes reach the program
- **WHEN** the user presses ctrl-c or ctrl-b
- **THEN** the program receives byte 0x03 or 0x02 respectively

#### Scenario: Application cursor mode
- **WHEN** the program has enabled application cursor mode and the user presses Up
- **THEN** the program receives `ESC O A` (otherwise `ESC [ A`)

#### Scenario: Modified arrow
- **WHEN** the user presses ctrl-left
- **THEN** the program receives `ESC [ 1 ; 5 D`

#### Scenario: Alt as meta
- **WHEN** the user presses alt-f
- **THEN** the program receives `ESC f`

#### Scenario: Text is not double-sent
- **WHEN** the user types a plain letter
- **THEN** the program receives that character exactly once

### Requirement: Mouse reporting
When the program enables mouse tracking (modes 1000, 1002 or 1003), mouse presses, releases, drags, motion and wheel events SHALL be reported to the program.
- Coordinates SHALL be relative to the terminal's own grid.
- SGR encoding (1006) SHALL be used when enabled; otherwise UTF-8 (1005) or legacy X10 encoding.
- Holding shift SHALL bypass reporting and select text instead.

#### Scenario: SGR click
- **WHEN** SGR mouse mode is on and the user left-clicks column 5, row 10
- **THEN** the program receives `ESC [ < 0 ; 5 ; 10 M` and, on release, the same ending in `m`

#### Scenario: Shift overrides reporting
- **WHEN** mouse reporting is on and the user shift-drags
- **THEN** text is selected and nothing is sent to the program

### Requirement: Wheel scrolling
The mouse wheel SHALL:
- send wheel reports when the program has enabled mouse reporting;
- otherwise, in the alternate screen with alternate-scroll mode (1007) on, send arrow keys;
- otherwise scroll the scrollback.

#### Scenario: Scrolling a pager
- **WHEN** `less` is running (alternate screen, no mouse reporting) and the user scrolls up
- **THEN** `less` receives Up arrow sequences and scrolls its content

#### Scenario: Scrolling history
- **WHEN** a shell prompt is showing and the user scrolls up
- **THEN** earlier output from the scrollback becomes visible

### Requirement: Paste
Pasted text SHALL be wrapped in `ESC [200~` … `ESC [201~` when the program has enabled bracketed paste, with any ESC bytes removed from the pasted text. Otherwise newlines SHALL be converted to carriage returns. Pasting or typing SHALL scroll the view back to the bottom and clear any selection.

#### Scenario: Bracketed paste cannot end early
- **WHEN** bracketed paste is on and the clipboard contains `x ESC[201~ y`
- **THEN** the program receives `ESC[200~x[201~yESC[201~`

### Requirement: Focus reporting
When the program enables focus events (mode 1004), the terminal SHALL send `ESC [ I` when it gains focus and `ESC [ O` when it loses focus. Focus counts as lost when the window is deactivated.

#### Scenario: tmux focus-events
- **WHEN** tmux runs with `focus-events on` and the user switches to another window
- **THEN** tmux receives `ESC [ O`

### Requirement: OSC 52 clipboard
Programs SHALL be able to set the clipboard and the primary selection through OSC 52. Reading the clipboard through OSC 52 SHALL be refused unless the configuration enables it.

#### Scenario: Copy from tmux
- **WHEN** tmux, with `set-clipboard on`, copies a selection
- **THEN** the system clipboard contains that text

#### Scenario: Clipboard read refused by default
- **WHEN** a program requests the clipboard contents through OSC 52 and `osc52_paste` is false
- **THEN** no clipboard contents are sent to the program

### Requirement: Selection and copy
The user SHALL be able to select text with the mouse:
- drag for a simple selection;
- double-click for a word;
- triple-click for a line;
- ctrl- or alt-drag for a block.

Finishing a selection SHALL copy it to the primary selection, and to the clipboard as well when `copy_on_select` is enabled. A middle click SHALL paste the primary selection. Dragging past the top or bottom edge SHALL scroll.

#### Scenario: Copy with keybinding
- **WHEN** the user selects text and presses ctrl-shift-c
- **THEN** the clipboard contains the selected text

### Requirement: Scrollback
Each terminal SHALL keep `scrollback_lines` lines of history. The user SHALL be able to scroll by line, by page, and to the top or bottom, and to clear the scrollback.

#### Scenario: Page up
- **WHEN** the user presses shift-pageup
- **THEN** the view moves up one page of history

### Requirement: Grid follows the view size
A terminal's grid SHALL be resized to the number of whole cells that fit in its area after padding, and the PTY SHALL be told the new size.

#### Scenario: Window resize
- **WHEN** the user makes the window wider
- **THEN** `tput cols` reports the larger column count
