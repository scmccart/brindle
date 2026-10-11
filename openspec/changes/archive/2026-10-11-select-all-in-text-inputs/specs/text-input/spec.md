## MODIFIED Requirements

### Requirement: Cursor movement
Left and right SHALL move the cursor by one character. Home SHALL move it to the start, and end and ctrl-e to the end. ctrl-left and ctrl-right SHALL move it to the previous and next word boundary, where a word is a run of letters, digits or underscores.

#### Scenario: Word movement
- **WHEN** an input holds `tmux -L work` with the cursor at the end, and the user presses ctrl-left twice
- **THEN** the cursor is before `L`

#### Scenario: Readline ends
- **WHEN** an input holds `abc` with the cursor at the start, and the user presses ctrl-e and then types `x`
- **THEN** the input holds `abcx`

## ADDED Requirements

### Requirement: Select all
ctrl-a and ctrl-shift-a SHALL select all of the input's text, with the cursor at the end. In a terminal, ctrl-a SHALL still reach the program.

#### Scenario: Retype the whole text
- **WHEN** an input holds `main`, and the user presses ctrl-a and types `work`
- **THEN** the input holds `work`

#### Scenario: Terminals keep ctrl-a
- **WHEN** a terminal has focus and the user presses ctrl-a
- **THEN** the program receives `^A`
