## MODIFIED Requirements

### Requirement: Debug steps are checked before the run
An `--action` naming no action, a `--key` that isn't a keystroke Brindle can parse, and a `--hover`, `--click` or `--right-click` position that isn't `COL,ROW` with known modifiers SHALL be refused as usage errors naming the step, before any window opens, as `--step-interval` and `--modality` values are.

#### Scenario: Misspelled action
- **WHEN** Brindle runs with `--action split_zone_rihgt --dump-screen-after 3`
- **THEN** it prints usage naming `split_zone_rihgt` and exits with status 2, without opening a window

#### Scenario: Malformed right-click position
- **WHEN** Brindle runs with `--right-click 3 --dump-screen-after 3`
- **THEN** it prints usage naming `--right-click` and `3` and exits with status 2, without opening a window

## ADDED Requirements

### Requirement: Right-click debug step
Brindle SHALL support a repeatable `--right-click <POS>` debug step that presses and releases the right mouse button on a cell of the active terminal, through the terminal's own mouse handling, as `--click` does for the left button. `POS` SHALL take the same form as `--click`'s, modifiers included, and the step SHALL run in order with the other steps. It SHALL NOT use synthetic OS-level input.

#### Scenario: Paste with a right-click step
- **WHEN** the config sets `right_click = "paste"`, the clipboard holds `echo hi`, and Brindle runs with `--right-click 0,0 --send '\r' --dump-screen-after 4` in a shell tab with no text selected
- **THEN** the dump shows a line `hi`

#### Scenario: Right-click step over a selection
- **WHEN** the config sets `right_click = "paste"`, the clipboard holds `echo hi`, and Brindle runs with `--action select_all --right-click 0,0 --dump-screen-after 3` in a shell tab
- **THEN** the dump has a `--- selection` line, and no `echo hi` was typed at the prompt

#### Scenario: Shift held for the step
- **WHEN** a program has turned on SGR mouse reporting and Brindle runs with `--right-click shift-5,1`
- **THEN** the program receives no mouse report for the step

### Requirement: Fault switch listing
`brindle --list-fault-switches` and, where the helper has server roles (Linux and Windows), `brindle-remote --list-fault-switches` SHALL print the `BRINDLE_TEST_*` fault switches that build honours, one name per line, and exit with status 0 without opening a window or starting a server role. A build without fault switches, such as a release build, SHALL print nothing and exit with status 0. `--version` output SHALL NOT change.

#### Scenario: Debug build of Brindle
- **WHEN** a debug build runs `brindle --list-fault-switches`
- **THEN** it prints `BRINDLE_TEST_SINK_PANIC` and `BRINDLE_TEST_LAUNCH_LISTING`, each on its own line, and exits 0

#### Scenario: Debug build of the helper
- **WHEN** a debug build runs `brindle-remote --list-fault-switches`
- **THEN** it prints `BRINDLE_TEST_HOLDER_PANIC` and `BRINDLE_TEST_FEED_PANIC`, each on its own line, and exits 0

#### Scenario: Release build
- **WHEN** a release build runs `brindle --list-fault-switches` or `brindle-remote --list-fault-switches`
- **THEN** it prints nothing and exits 0

#### Scenario: Extra arguments to the helper
- **WHEN** `brindle-remote --list-fault-switches extra` runs
- **THEN** it prints an error saying the option takes no arguments and exits with a failure status
