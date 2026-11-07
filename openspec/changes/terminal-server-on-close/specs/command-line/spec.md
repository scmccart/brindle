## MODIFIED Requirements

### Requirement: Launch options
Brindle SHALL accept:
- `-p/--profile <NAME>` (matched case-insensitively; an unknown name falls back to the default profile with a warning);
- `-d/--working-directory <DIR>`;
- `--config <FILE>`;
- `-e/--command <CMD> [ARGS]...`, which runs CMD instead of the profile's program and must come last;
- `--no-restore`, which skips session restore for this launch;
- `--attach <ID>` (Linux), which opens the window with a tab attached to the detached terminal server session `ID` instead of a new tab.

An unknown argument SHALL print usage and exit with status 2.

#### Scenario: Run a command
- **WHEN** the user runs `brindle -e htop`
- **THEN** a window opens running htop, and closing htop closes the window

#### Scenario: Attach a session
- **WHEN** `brindle-remote list` shows a detached session 7 running `htop`, and the user runs `brindle --attach 7`
- **THEN** a window opens with one tab showing that `htop`, and the session is no longer detached

#### Scenario: Attach a missing session
- **WHEN** the user runs `brindle --attach 99` and there is no session 99, or it is attached elsewhere
- **THEN** a window opens with a tab whose message names session 99 and the reason

### Requirement: Plain launch restores the session
A launch without `-p`, `-d`, `-e`, `--attach` or `--no-restore` is a plain launch. A plain launch SHALL restore the recorded session as defined by the `session-restore` capability. Any other launch SHALL open its single window as requested and SHALL NOT restore. The debug options (`--send`, `--action`, `--dump-screen-after`) SHALL NOT affect whether a launch is plain, and they act on the first restored window.

#### Scenario: Profile launch skips restore
- **WHEN** the user runs `brindle -p tmux`
- **THEN** one window opens with the tmux profile, and the recorded tabs are not reopened

#### Scenario: Skip once
- **WHEN** the user runs `brindle --no-restore`
- **THEN** one window opens with a single default-profile tab
