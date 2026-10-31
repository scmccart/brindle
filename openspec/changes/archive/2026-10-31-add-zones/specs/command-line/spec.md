# Spec Delta

## MODIFIED Requirements

### Requirement: Debug automation hooks
For automated testing, Brindle SHALL support repeatable `--send <TEXT>`, `--action <NAME>`, `--hover <POS>`, `--click <POS>`, `--find <TEXT>` and `--wait-for <TEXT>` steps. Steps SHALL run in command-line order, one second apart, against the window's active tab. `--step-interval <SECONDS>` SHALL space them that many seconds apart instead, with half that interval before the first step, and SHALL be rejected with a usage error unless it is a positive number:
- `--send` text is typed as user input, and `\r`, `\n`, `\t` and `\e` are unescaped. While a text input has keyboard focus (in a palette, the settings dialog or the find bar), the text SHALL be inserted into that input at its cursor instead of being sent to the terminal, with control characters dropped;
- `--action` runs a named action;
- `--hover` moves the mouse over a cell of the active terminal, and `--click` presses and releases the left button there. `POS` is `COL,ROW` (zero-based cells), optionally prefixed with modifiers such as `ctrl-` or `ctrl-shift-`, which are held for the step;
- `--find` opens the find bar on the active terminal with TEXT as its query, as if the user had typed it;
- `--wait-for` holds the steps after it until the active terminal's visible screen text, with its lines joined by line breaks, contains TEXT, unescaped as for `--send`. After 30 seconds without it, Brindle SHALL log a warning and run the remaining steps.

`--dump-screen-after <SECONDS>` SHALL print every tab title, the active terminal's size and modes, the active terminal's grid geometry (the window-pixel origin of its first cell, its cell width and line height, and the window's scale factor), the link under the mouse while hover feedback is showing, the active terminal's selected text when there is a selection, one `--- status` line per program status record of the active terminal (root first, then by id: `state=` always, then `id=`, `kind=`, `progress=`, `app=`, `title=` and `msg=` when set, strings quoted), and its visible screen text, then quit. When the active tab is a local tab with two or more zones, the dump SHALL also print a `--- zones count=N active=ID` line followed by one `--- zone id=ID rect=X,Y,W,H[ active]` line per zone in tree order (with normalized `[0, 1]` coordinates) before the active terminal's screen text. While a text input has focus, the dump SHALL also print that input's text, cursor offset and selection. When the active terminal's find bar is open, the dump SHALL also include a line with its query, options, match count and the current match's position on screen. These hooks SHALL NOT use synthetic OS-level input.

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

#### Scenario: Hovering a link
- **WHEN** the screen shows `https://example.com` at row 0 and Brindle runs with `--hover ctrl-3,0 --dump-screen-after 3`
- **THEN** the dump has a line `--- link https://example.com`

#### Scenario: Clicking a link
- **WHEN** `BRINDLE_LINK_OPENER` names a script that records its argument and Brindle runs with `--click ctrl-3,0` over that URL
- **THEN** the script records `https://example.com`

#### Scenario: Malformed position
- **WHEN** a `--hover` or `--click` step's position cannot be parsed
- **THEN** an error is logged and the remaining steps still run

#### Scenario: Selection in the dump
- **WHEN** Brindle runs with `--action select_all --dump-screen-after 3`
- **THEN** the dump contains a selection line with the selected text, escaped onto one line

#### Scenario: Scripted search
- **WHEN** Brindle runs with `--send 'echo needle\r' --find needle --dump-screen-after 4`
- **THEN** stdout contains a find line with query `needle`, a match count of at least one, and the current match's row and column

#### Scenario: Stretched steps
- **WHEN** Brindle runs with `--step-interval 2 --send 'a' --send 'b' --dump-screen-after 4`
- **THEN** the dump shows `a` typed but not `b`, because the steps are due about 3 and 5 seconds after launch

#### Scenario: Waiting for the screen
- **WHEN** a slow shell shows its first prompt `MYPROMPT>` four seconds after launch, and Brindle runs with `--wait-for 'MYPROMPT>' --send 'cd /tmp\r' --dump-screen-after 8`
- **THEN** `cd /tmp` is typed at that prompt, about a second after it appears, rather than before the shell is ready

#### Scenario: Invalid interval
- **WHEN** Brindle runs with `--step-interval 0`
- **THEN** it prints a usage error and exits without opening a window

#### Scenario: Status records in the dump
- **WHEN** Brindle runs with `--send 'printf "\e]7501;state=blocked:kind=auth:app=brew\e\\"\r' --dump-screen-after 3`
- **THEN** stdout contains the line `--- status state=blocked kind=auth app="brew"`

#### Scenario: Zones in the dump
- **WHEN** Brindle runs in a local tab with `--action split_zone_right --dump-screen-after 3`
- **THEN** stdout contains `--- zones count=2 active=2`, `--- zone id=1 rect=0,0,0.5,1` and `--- zone id=2 rect=0.5,0,0.5,1 active`, and a single-zone local tab prints no `--- zones` line
