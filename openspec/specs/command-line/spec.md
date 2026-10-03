# command-line Specification

## Purpose
Let users and scripts launch Brindle with a specific profile, command or directory, and give developers non-interactive hooks to drive and inspect a running instance for automated testing.

## Requirements

### Requirement: Launch options
Brindle SHALL accept:
- `-p/--profile <NAME>` (matched case-insensitively; an unknown name falls back to the default profile with a warning);
- `-d/--working-directory <DIR>`;
- `--config <FILE>`;
- `-e/--command <CMD> [ARGS]...`, which runs CMD instead of the profile's program and must come last.

An unknown argument SHALL print usage and exit with status 2.

#### Scenario: Run a command
- **WHEN** the user runs `brindle -e htop`
- **THEN** a window opens running htop, and closing htop closes the window

### Requirement: Informational options
`--help`, `--version`, `--list-actions` (every bindable action name) and `--list-themes` (the built-in themes) SHALL print their output and exit without opening a window.

#### Scenario: List actions
- **WHEN** the user runs `brindle --list-actions`
- **THEN** each bindable action name is printed, including `activate_tab_<N>` and `new_tab_with_profile_<N>`

### Requirement: Debug automation hooks
For automated testing, Brindle SHALL support repeatable `--send <TEXT>` and `--action <NAME>` steps. Steps SHALL run in command-line order, one second apart, against the window's active tab:
- `--send` text is typed as user input, and `\r`, `\n`, `\t` and `\e` are unescaped;
- `--action` runs a named action.

`--dump-screen-after <SECONDS>` SHALL print every tab title, the active terminal's size and modes, and its visible screen text, then quit. These hooks SHALL NOT use synthetic OS-level input.

#### Scenario: Scripted check
- **WHEN** Brindle runs with `--send 'echo hi\r' --dump-screen-after 3`
- **THEN** stdout contains the tab list, a screen header, and a line `hi`, and the process exits

#### Scenario: Unknown action
- **WHEN** an `--action` step names an action that does not exist
- **THEN** an error is logged and the remaining steps still run
