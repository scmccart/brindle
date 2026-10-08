# Spec Delta

## MODIFIED Requirements

### Requirement: Debug automation hooks
For automated testing, Brindle SHALL support repeatable `--send <TEXT>`, `--action <NAME>` and `--find <TEXT>` steps. Steps SHALL run in command-line order, one second apart, against the window's active tab:
- `--send` text is typed as user input, and `\r`, `\n`, `\t` and `\e` are unescaped;
- `--action` runs a named action;
- `--find` opens the find bar on the active terminal with TEXT as its query, as if the user had typed it.

`--dump-screen-after <SECONDS>` SHALL print every tab title, the active terminal's size and modes, the active terminal's grid geometry (the window-pixel origin of its first cell, its cell width and line height, and the window's scale factor), and its visible screen text, then quit. When the active terminal's find bar is open, the dump SHALL also include a line with its query, options, match count and the current match's position on screen. These hooks SHALL NOT use synthetic OS-level input.

#### Scenario: Scripted check
- **WHEN** Brindle runs with `--send 'echo hi\r' --dump-screen-after 3`
- **THEN** stdout contains the tab list, a screen header, a grid geometry line, and a line `hi`, and the process exits

#### Scenario: Grid geometry for pixel checks
- **WHEN** Brindle runs with `--send 'printf "\e[41m \e[0m"\r' --dump-screen-after 3`, and the window is captured before it quits
- **THEN** the red cell in the capture sits at the dumped origin, offset by the cell size for its row and column and scaled by the dumped scale factor

#### Scenario: Unknown action
- **WHEN** an `--action` step names an action that does not exist
- **THEN** an error is logged and the remaining steps still run

#### Scenario: Scripted search
- **WHEN** Brindle runs with `--send 'echo needle\r' --find needle --dump-screen-after 4`
- **THEN** stdout contains a find line with query `needle`, a match count of at least one, and the current match's row and column
