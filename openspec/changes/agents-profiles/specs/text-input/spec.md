# Spec Delta

## ADDED Requirements

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
