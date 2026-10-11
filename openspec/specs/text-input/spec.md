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
Left and right SHALL move the cursor by one character. Home and ctrl-a SHALL move it to the start, and end and ctrl-e to the end. ctrl-left and ctrl-right SHALL move it to the previous and next word boundary, where a word is a run of letters, digits or underscores.

#### Scenario: Word movement
- **WHEN** an input holds `tmux -L work` with the cursor at the end, and the user presses ctrl-left twice
- **THEN** the cursor is before `L`

#### Scenario: Readline ends
- **WHEN** the user presses ctrl-a and then types `x`
- **THEN** `x` is inserted at the start of the text

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
