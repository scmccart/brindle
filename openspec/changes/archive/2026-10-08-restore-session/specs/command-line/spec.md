# Spec Delta

## MODIFIED Requirements

### Requirement: Launch options
Brindle SHALL accept:
- `-p/--profile <NAME>` (matched case-insensitively; an unknown name falls back to the default profile with a warning);
- `-d/--working-directory <DIR>`;
- `--config <FILE>`;
- `-e/--command <CMD> [ARGS]...`, which runs CMD instead of the profile's program and must come last;
- `--no-restore`, which skips session restore for this launch.

An unknown argument SHALL print usage and exit with status 2.

#### Scenario: Run a command
- **WHEN** the user runs `brindle -e htop`
- **THEN** a window opens running htop, and closing htop closes the window

## ADDED Requirements

### Requirement: Plain launch restores the session
A launch without `-p`, `-d`, `-e` or `--no-restore` is a plain launch. With `restore_session` on, a plain launch SHALL restore the recorded session as defined by the `session-restore` capability. Any other launch SHALL open its single window as requested and SHALL NOT restore. The debug options (`--send`, `--action`, `--dump-screen-after`) SHALL NOT affect whether a launch is plain, and they act on the first restored window.

#### Scenario: Profile launch skips restore
- **WHEN** restore is on and the user runs `brindle -p tmux`
- **THEN** one window opens with the tmux profile, and the recorded tabs are not reopened

#### Scenario: Skip once
- **WHEN** restore is on and the user runs `brindle --no-restore`
- **THEN** one window opens with a single default-profile tab
