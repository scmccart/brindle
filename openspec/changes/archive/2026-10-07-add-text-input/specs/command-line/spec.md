## MODIFIED Requirements

### Requirement: Debug automation hooks
For automated testing, Brindle SHALL support repeatable `--send <TEXT>` and `--action <NAME>` steps. Steps SHALL run in command-line order, one second apart, against the window's active tab:
- `--send` text is typed as user input, and `\r`, `\n`, `\t` and `\e` are unescaped. While a text input has keyboard focus (in a palette or the settings dialog), the text SHALL be inserted into that input at its cursor instead of being sent to the terminal, with control characters dropped;
- `--action` runs a named action.

`--dump-screen-after <SECONDS>` SHALL print every tab title, the active terminal's size and modes, the active terminal's grid geometry (the window-pixel origin of its first cell, its cell width and line height, and the window's scale factor), and its visible screen text, then quit. While a text input has focus, the dump SHALL also print that input's text, cursor offset and selection. These hooks SHALL NOT use synthetic OS-level input.

#### Scenario: Scripted check
- **WHEN** Brindle runs with `--send 'echo hi\r' --dump-screen-after 3`
- **THEN** stdout contains the tab list, a screen header, a grid geometry line, and a line `hi`, and the process exits

#### Scenario: Grid geometry for pixel checks
- **WHEN** Brindle runs with `--send 'printf "\e[41m \e[0m"\r' --dump-screen-after 3`, and the window is captured before it quits
- **THEN** the red cell in the capture sits at the dumped origin, offset by the cell size for its row and column and scaled by the dumped scale factor

#### Scenario: Unknown action
- **WHEN** an `--action` step names an action that does not exist
- **THEN** an error is logged and the remaining steps still run

#### Scenario: Typing into the palette
- **WHEN** Brindle runs with `--action open_command_palette --send 'splt' --action text_input_left --send 'i' --dump-screen-after 5`
- **THEN** the dump shows the focused input holding `split` with the cursor after `i`, and nothing was typed into the terminal
