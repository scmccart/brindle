## MODIFIED Requirements

### Requirement: Launch options
Brindle SHALL accept:
- `-p/--profile <NAME>` (matched case-insensitively; an unknown name falls back to the default profile with a warning);
- `-d/--working-directory <DIR>`;
- `--config <FILE>`;
- `-e/--command <CMD> [ARGS]...`, which runs CMD instead of the profile's program and must come last;
- `--no-restore`, which skips session restore for this launch;
- `--attach <ID>` (Linux), which opens a tab attached to the detached terminal server session `ID`.

An unknown argument SHALL print usage and exit with status 2.

#### Scenario: Run a command
- **WHEN** the user runs `brindle -e htop`
- **THEN** a tab running htop opens and becomes active, and when htop exits its tab closes

#### Scenario: Attach a session
- **WHEN** `brindle-remote list` shows a detached session 7 running `htop`, and the user runs `brindle --attach 7`
- **THEN** an active tab shows that `htop`, and the session is no longer detached

#### Scenario: Attach a missing session
- **WHEN** the user runs `brindle --attach 99` and there is no session 99, or it is attached elsewhere
- **THEN** an active tab's message names session 99 and the reason

## REMOVED Requirements

### Requirement: Plain launch restores the session
**Reason**: Brindle has one window, so a launch's options can't open a separate window instead of the session. Replaced by "A launch restores, then opens what it asks for".
**Migration**: `brindle -p NAME`, `-d DIR` and `-e CMD` now open their tab beside the restored ones; use `--no-restore` for a launch without them.

## ADDED Requirements

### Requirement: A launch restores, then opens what it asks for
A launch that becomes the instance (see `session-restore`'s "One process owns the state") SHALL restore the recorded session and reattach detached sessions, as defined by `session-restore`, unless it has `--no-restore`. It SHALL then open what it asks for as the active tab: `-p` and `-d` a new tab with that profile or directory, `-e` a tab running the command, `--attach` a tab attached to that session. A launch with none of them SHALL open no further tab, or one default-profile tab when nothing was restored. The debug options (`--send`, `--action`, `--dump-screen-after`) act on the active tab after this.

#### Scenario: Profile launch
- **WHEN** Brindle quit with two tabs, and the user runs `brindle -p tmux`
- **THEN** the window shows the two restored tabs and an active tab with the tmux profile

#### Scenario: Skip once
- **WHEN** the user runs `brindle --no-restore`
- **THEN** the window opens with a single default-profile tab

### Requirement: A launch goes to the running Brindle
A launch without debug options while an instance runs (one with the same state directory) SHALL send its request to the instance and exit with status 0 once the instance has accepted it. The instance SHALL raise its window and open what the launch asked for, as "A launch restores, then opens what it asks for" says, without restoring again; a `-e` command SHALL start in the launching process's working directory. When the instance doesn't accept the request within 5 seconds, the launch SHALL print why and exit with status 1. The request channel SHALL be reachable only by the same user.

#### Scenario: Second plain launch
- **WHEN** Brindle runs with two tabs and the user launches `brindle` from the desktop
- **THEN** the window is raised, it still has two tabs, and no second window opens

#### Scenario: Command from a shell
- **WHEN** Brindle runs and the user runs `brindle -e htop` in a shell in `/srv`
- **THEN** the `brindle` command exits, and Brindle's window gains an active tab running htop in `/srv`

#### Scenario: Attach from a shell
- **WHEN** Brindle runs, session 7 is detached, and the user runs `brindle --attach 7`
- **THEN** Brindle's window gains an active tab showing session 7
