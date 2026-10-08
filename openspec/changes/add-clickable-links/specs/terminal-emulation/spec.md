# Spec Delta

## MODIFIED Requirements

### Requirement: Mouse reporting
When the program enables mouse tracking (modes 1000, 1002 or 1003), mouse presses, releases, drags, motion and wheel events SHALL be reported to the program.
- Coordinates SHALL be relative to the terminal's own grid.
- SGR encoding (1006) SHALL be used when enabled; otherwise UTF-8 (1005) or legacy X10 encoding.
- Holding shift SHALL bypass reporting and select text instead.
- A ctrl-click on a link SHALL bypass reporting and open the link instead, as defined by the `links` capability.

#### Scenario: SGR click
- **WHEN** SGR mouse mode is on and the user left-clicks column 5, row 10
- **THEN** the program receives `ESC [ < 0 ; 5 ; 10 M` and, on release, the same ending in `m`

#### Scenario: Shift overrides reporting
- **WHEN** mouse reporting is on and the user shift-drags
- **THEN** text is selected and nothing is sent to the program

#### Scenario: Ctrl-click on a link overrides reporting
- **WHEN** mouse reporting is on and the user ctrl-clicks a URL
- **THEN** the URL opens and nothing is sent to the program

### Requirement: Selection and copy
The user SHALL be able to select text with the mouse:
- drag for a simple selection;
- double-click for a word;
- triple-click for a line;
- ctrl- or alt-drag for a block. A ctrl-drag that starts on a link SHALL NOT select; alt-drag SHALL block-select anywhere.

Finishing a selection SHALL copy it to the primary selection, and to the clipboard as well when `copy_on_select` is enabled. A middle click SHALL paste the primary selection. Dragging past the top or bottom edge SHALL scroll.

#### Scenario: Copy with keybinding
- **WHEN** the user selects text and presses ctrl-shift-c
- **THEN** the clipboard contains the selected text

#### Scenario: Block selection from a link
- **WHEN** the user alt-drags starting on a URL
- **THEN** a block of text is selected and the URL is not opened
