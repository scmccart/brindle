# Spec Delta

## MODIFIED Requirements

### Requirement: Debug automation hooks
For automated testing, Brindle SHALL support repeatable `--send <TEXT>`, `--action <NAME>`, `--hover <POS>` and `--click <POS>` steps. Steps SHALL run in command-line order, one second apart, against the window's active tab:
- `--send` text is typed as user input, and `\r`, `\n`, `\t` and `\e` are unescaped;
- `--action` runs a named action;
- `--hover` moves the mouse over a cell of the active terminal, and `--click` presses and releases the left button there. `POS` is `COL,ROW` (zero-based cells), optionally prefixed with modifiers such as `ctrl-` or `ctrl-shift-`, which are held for the step.

`--dump-screen-after <SECONDS>` SHALL print every tab title, the active terminal's size and modes, the active terminal's grid geometry (the window-pixel origin of its first cell, its cell width and line height, and the window's scale factor), the link under the mouse while hover feedback is showing, and its visible screen text, then quit. These hooks SHALL NOT use synthetic OS-level input.

#### Scenario: Scripted check
- **WHEN** Brindle runs with `--send 'echo hi\r' --dump-screen-after 3`
- **THEN** stdout contains the tab list, a screen header, a grid geometry line, and a line `hi`, and the process exits

#### Scenario: Grid geometry for pixel checks
- **WHEN** Brindle runs with `--send 'printf "\e[41m \e[0m"\r' --dump-screen-after 3`, and the window is captured before it quits
- **THEN** the red cell in the capture sits at the dumped origin, offset by the cell size for its row and column and scaled by the dumped scale factor

#### Scenario: Unknown action
- **WHEN** an `--action` step names an action that does not exist
- **THEN** an error is logged and the remaining steps still run

#### Scenario: Hovering a link
- **WHEN** the screen shows `https://example.com` at row 0 and Brindle runs with `--hover ctrl-3,0 --dump-screen-after 3`
- **THEN** the dump has a line `--- link https://example.com`

#### Scenario: Clicking a link
- **WHEN** `BRINDLE_LINK_OPENER` names a script that records its argument and Brindle runs with `--click ctrl-3,0` over that URL
- **THEN** the script records `https://example.com`

#### Scenario: Malformed position
- **WHEN** a `--hover` or `--click` step's position cannot be parsed
- **THEN** an error is logged and the remaining steps still run
