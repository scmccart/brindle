# text-input Specification

## Purpose
Give every place Brindle asks for typed text (palettes, prompts, the settings dialog and the find bar) the same single-line text entry, with a cursor, selection, mouse positioning, clipboard and input-method support.

## Requirements

### Requirement: Cursor and caret
A focused text input SHALL show a caret at its cursor. Typed text SHALL be inserted at the cursor. The cursor SHALL always sit between characters, never inside a multi-byte character. An input without focus SHALL show no caret.

#### Scenario: Insert in the middle
- **WHEN** an input holds `tmx`, the cursor is after `t`, and the user types `u`
- **THEN** the input holds `tumx` with the cursor after `u`

#### Scenario: Non-ASCII text
- **WHEN** an input holds `café` and the user presses left once and then backspace
- **THEN** the input holds `caé`

### Requirement: Cursor movement
Left and right SHALL move the cursor by one character. Home SHALL move it to the start, and end and ctrl-e to the end. ctrl-left and ctrl-right SHALL move it to the previous and next word boundary, where a word is a run of letters, digits or underscores.

#### Scenario: Word movement
- **WHEN** an input holds `tmux -L work` with the cursor at the end, and the user presses ctrl-left twice
- **THEN** the cursor is before `L`

#### Scenario: Readline ends
- **WHEN** an input holds `abc` with the cursor at the start, and the user presses ctrl-e and then types `x`
- **THEN** the input holds `abcx`

### Requirement: Deleting text
Backspace SHALL delete the character before the cursor, and delete SHALL delete the character after it. ctrl-backspace and ctrl-w SHALL delete back to the previous word boundary. When there is a selection, each of these SHALL delete the selection instead.

#### Scenario: Delete a word
- **WHEN** an input holds `~/src/brindle` with the cursor at the end, and the user presses ctrl-w
- **THEN** the input holds `~/src/`

### Requirement: Selection
Shift with any movement key SHALL extend a selection from the cursor. Typing or pasting SHALL replace the selection. Moving without shift SHALL clear the selection; left and right SHALL then leave the cursor at the selection's start or end. The selection SHALL be drawn highlighted.

#### Scenario: Replace a selection
- **WHEN** an input holds `main` with the cursor at the end, and the user presses shift-home and types `work`
- **THEN** the input holds `work`

### Requirement: Mouse positioning
Clicking in a text input SHALL focus it and place the cursor at the nearest character boundary to the click. Dragging SHALL select from the press to the pointer. Double-clicking SHALL select the word under the pointer.

#### Scenario: Click to place the cursor
- **WHEN** an input holds `/bin/zsh` and the user clicks between `/` and `z`
- **THEN** the input has focus and typing inserts text before `z`

### Requirement: Clipboard
ctrl-c and ctrl-shift-c SHALL copy the selection. ctrl-x SHALL cut it. ctrl-v and ctrl-shift-v SHALL paste at the cursor, replacing any selection. Copy and cut with no selection SHALL do nothing.

#### Scenario: Paste replaces the selection
- **WHEN** the clipboard holds `zsh`, the input holds `/bin/bash` with `bash` selected, and the user presses ctrl-v
- **THEN** the input holds `/bin/zsh`

### Requirement: Single line
A text input SHALL hold a single line. Line breaks in pasted or inserted text SHALL become spaces.

#### Scenario: Multi-line paste
- **WHEN** the user pastes `one\ntwo` into an input
- **THEN** the input holds `one two`

### Requirement: Input methods
A text input SHALL accept text from an input method. Composed but uncommitted text SHALL be shown at the cursor, marked as in progress, and SHALL replace the selection when it is committed. The candidate window SHALL be placed at the cursor.

#### Scenario: Compose a character
- **WHEN** the user composes `é` with an input method in a focused input
- **THEN** the in-progress text is shown marked at the cursor, and committing it inserts `é`

### Requirement: Keys left to the surrounding UI
A text input SHALL NOT consume enter, escape, tab, shift-tab, up, down, ctrl-p or ctrl-n. These keys SHALL reach the palette or dialog that contains the input, so its navigation and confirm and cancel keys work while the input has focus.

#### Scenario: Navigate while typing
- **WHEN** the command palette's filter has focus and the user presses down and then enter
- **THEN** the selection moves down one command and that command runs

### Requirement: Select all
ctrl-a and ctrl-shift-a SHALL select all of the input's text, with the cursor at the end. In a terminal, ctrl-a SHALL still reach the program.

#### Scenario: Retype the whole text
- **WHEN** an input holds `main`, and the user presses ctrl-a and types `work`
- **THEN** the input holds `work`

#### Scenario: Terminals keep ctrl-a
- **WHEN** a terminal has focus and the user presses ctrl-a
- **THEN** the program receives `^A`

### Requirement: Text areas
A text area SHALL edit text of several lines with the cursor, movement, deletion, selection, mouse, clipboard, select-all and input-method behaviour of a text input, except that line breaks SHALL be kept: pasted `\r\n` and `\r` SHALL become `\n`, and other control characters except tab SHALL be dropped. Long lines SHALL wrap at the area's width.

#### Scenario: Multi-line paste
- **WHEN** the user pastes `one\r\ntwo` into a text area
- **THEN** the area holds `one\ntwo`, on two lines

#### Scenario: Wrapping
- **WHEN** a text area is 20 characters wide and the user types 30 characters with no break
- **THEN** the text is shown on two lines and holds no line break

### Requirement: Text area line keys
In a text area, up and down SHALL move the cursor to the previous and next shown line, keeping its horizontal position across a run of such moves, and home and end SHALL move it to the start and end of the shown line. ctrl-home and ctrl-end SHALL move it to the start and end of the text. With shift, each SHALL extend the selection. Shift-enter SHALL insert a line break.

#### Scenario: Down keeps the column
- **WHEN** a text area holds `abcdef\nxy\nabcdef` with the cursor after `abcd` on the first line, and the user presses down twice
- **THEN** the cursor is after `abcd` on the third line

#### Scenario: Shift-enter
- **WHEN** a text area holds `ab` with the cursor after `a`, and the user presses shift-enter
- **THEN** the area holds `a\nb`

### Requirement: Keys a text area leaves to the surrounding UI
A text area SHALL NOT consume enter, escape, tab or shift-tab, nor up on its first shown line or down on its last, so its owner can bind them. Enter SHALL NOT reach the owner while an input method is composing.

#### Scenario: Enter goes to the owner
- **WHEN** a text area inside an owner that binds enter has focus and the user presses enter
- **THEN** the owner's action runs, and no line break is inserted

#### Scenario: Composing
- **WHEN** an input method is composing in a text area and the user presses enter to commit
- **THEN** the composed text is inserted, and the owner's enter action does not run

### Requirement: Text area height
A text area SHALL grow with its text from one line to the most lines its owner allows, then scroll so the cursor's line stays in view.

#### Scenario: Grows then scrolls
- **WHEN** a text area allows 8 lines and the user enters 12 lines
- **THEN** it is 8 lines tall and shows the line holding the cursor
