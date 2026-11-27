# command-line Specification

## Purpose
Let users and scripts launch Brindle with a specific profile, command or directory, and give developers non-interactive hooks to drive and inspect a running instance for automated testing.

## Requirements

### Requirement: Launch options
Brindle SHALL accept:
- `terminal [PROFILE]`, the subcommand that asks for a terminal tab, with PROFILE matched case-insensitively against the config's profile names, and under it:
  - `-d/--working-directory <DIR>`;
  - `-e/--command <CMD> [ARGS]...`, which runs CMD instead of the profile's program and takes every argument after it, so it comes last;
- `agent [PROFILE]`, the subcommand that asks for an agent tab, with PROFILE matched as under `terminal`, and under it:
  - `-d/--working-directory <DIR>`; without it the agent starts in the profile's `cwd`, else in the working directory of the `brindle agent` command;
  - `-p/--prompt <PROMPT>`, a first prompt, sent once the agent is ready (see `agent-zones`' "A launch's first prompt"); `-p -` reads it from standard input (see "A first prompt from standard input");
  - `-c/--continue`, which continues the agent's last conversation in that directory, and `-r/--resume`, which opens the list of its conversations there (see "Continuing a conversation from the command line");
- `--config <FILE>`;
- `--no-restore`, which skips session restore for this launch;
- `--attach <ID>` (Linux and Windows), which opens a tab attached to the detached terminal server session `ID`, an agent tab for an agent session. When the terminal server's sessions can't be listed, that tab's message SHALL say so, and SHALL NOT say that session `ID` doesn't exist. It SHALL NOT be combined with a subcommand.

PROFILE, `-d`, `-p`, `-c` and `-r` MAY come in any order after their subcommand, and before `-e`. An unknown argument, an option given without the value it takes (`-e` without a command included), a value an option can't use, a blank PROMPT, a PROMPT over 256 KiB of UTF-8, `-p`, `-c` or `-r` under `terminal`, `-e` under `agent`, a second PROFILE, `-p`, `-c` or `-r`, `-c` with `-r`, and `--attach` given with a subcommand SHALL print usage, naming the option or argument, and exit with status 2.

A PROFILE that names no profile SHALL be refused, by the Brindle that would open the tab and against the config it runs with, with an error naming PROFILE and listing the profile names, and the launch SHALL exit with status 2 without opening a window or a tab, whether it would become the running Brindle or hand its request to one. When that config has an error and Brindle runs on the default config, the names listed SHALL be the default config's, and the error SHALL say the config has an error. No launch SHALL fall back to another profile for an unknown name.

#### Scenario: Run a command
- **WHEN** the user runs `brindle terminal -e htop`
- **THEN** a tab running htop opens and becomes active, and when htop exits its tab closes

#### Scenario: Profile, directory and command
- **WHEN** the user runs `brindle terminal -d /srv Work -e htop`
- **THEN** an active tab of the `Work` profile runs htop in `/srv`

#### Scenario: A profile named like a subcommand
- **WHEN** the config has a profile named `terminal` and the user runs `brindle terminal terminal`
- **THEN** an active tab of the `terminal` profile opens

#### Scenario: Agent with a directory and a prompt
- **WHEN** `Agent` is an agent profile and the user runs `brindle agent Agent -d ~/src -p 'run the tests'`
- **THEN** an active agent tab opens whose agent session starts in `~/src`, and `run the tests` is sent once the agent is ready

#### Scenario: Agent in the command's directory
- **WHEN** `Agent` sets no `cwd` and the user runs `brindle agent Agent` in a shell in `~/src/brindle`
- **THEN** the agent session starts in `~/src/brindle`

#### Scenario: A command for an agent
- **WHEN** the user runs `brindle agent Agent -e other-acp`
- **THEN** Brindle prints usage naming `-e`, saying an agent runs its profile's command, and exits with status 2 without opening a window

#### Scenario: Continue under the terminal subcommand
- **WHEN** the user runs `brindle terminal Work -c`
- **THEN** Brindle prints usage naming `-c` and exits with status 2, without opening a window

#### Scenario: Attach a session
- **WHEN** `brindle-remote list` shows a detached session 7 running `htop`, and the user runs `brindle --attach 7`
- **THEN** an active tab shows that `htop`, and the session is no longer detached

#### Scenario: Attach an agent session
- **WHEN** `brindle-remote list` shows a detached agent session 8, and the user runs `brindle --attach 8`
- **THEN** an active agent tab shows its conversation

#### Scenario: Attach a missing session
- **WHEN** the user runs `brindle --attach 99` and there is no session 99, or it is attached elsewhere
- **THEN** an active tab's message names session 99 and the reason

#### Scenario: Attach when the server can't be listed
- **WHEN** the user runs `brindle --attach 7` and Brindle can't list the terminal server's sessions
- **THEN** an active tab's message says the terminal server's sessions could not be listed

#### Scenario: Attach with a subcommand
- **WHEN** the user runs `brindle terminal Work --attach 7`
- **THEN** Brindle prints usage naming `--attach` and exits with status 2, without opening a window

#### Scenario: Unknown profile
- **WHEN** the config has profiles `Shell` and `Work`, and the user runs `brindle terminal wrk`
- **THEN** Brindle prints that no profile is named `wrk` and that the profiles are `Shell` and `Work`, and exits with status 2 without opening a window

#### Scenario: Unknown agent profile
- **WHEN** the config has profiles `Shell` and `Agent`, and the user runs `brindle agent agnt`
- **THEN** Brindle prints that no profile is named `agnt` and that the profiles are `Shell` and `Agent`, and exits with status 2 without opening a window

#### Scenario: Unknown profile while Brindle runs
- **WHEN** Brindle runs with two tabs and profiles `Shell` and `Work`, and the user runs `brindle terminal wrk` in a shell
- **THEN** the launch prints that no profile is named `wrk`, listing `Shell` and `Work`, and exits with status 2, and the window still has two tabs

#### Scenario: Profile added while Brindle runs
- **WHEN** Brindle runs, the user adds a profile `Logs` to the config, Brindle reloads it, and the user runs `brindle terminal Logs`
- **THEN** the window gains an active tab of the `Logs` profile

#### Scenario: Unknown profile with a broken config
- **WHEN** the config file has a syntax error, so Brindle runs on the default config, and the user runs `brindle terminal Work`
- **THEN** Brindle prints that no profile is named `Work`, that the config has an error, and the default config's profile names, and exits with status 2 without opening a window

#### Scenario: Matched without regard to case
- **WHEN** the config has a profile `Work` and the user runs `brindle terminal wORK`
- **THEN** an active tab of the `Work` profile opens

#### Scenario: Two profiles
- **WHEN** the user runs `brindle terminal Work Shell`
- **THEN** Brindle prints usage naming `Shell` and exits with status 2, without opening a window

#### Scenario: Option without its value
- **WHEN** the user runs `brindle terminal -d`, `brindle agent -p`, or `brindle --dump-screen-after soon`
- **THEN** Brindle prints usage naming the option and exits with status 2, without opening a window

### Requirement: Informational options
`--help`, `--version`, `--list-actions` (every bindable action name), `--list-themes` (the built-in themes), `--export-ui-tokens` (the built-in themes' UI tokens as CSS) and `--shell-integration <bash|zsh|fish|pwsh>` (that shell's integration script) SHALL print their output and exit without opening a window. `--shell-integration` with a missing or unsupported shell SHALL print an error naming the supported shells and exit with status 2.

`--export-ui-tokens` SHALL print, for each built-in theme, a CSS rule setting shadcn/ui's custom properties (`--background`, `--foreground`, `--card`, `--card-foreground`, `--popover`, `--popover-foreground`, `--primary`, `--primary-foreground`, `--secondary`, `--secondary-foreground`, `--muted`, `--muted-foreground`, `--accent`, `--accent-foreground`, `--destructive`, `--destructive-foreground`, `--border`, `--input`, `--ring`, `--radius`) to the tokens Brindle's chrome draws with in that theme, and the theme's terminal colours (`--terminal-background`, `--terminal-foreground`, `--terminal-cursor`, `--terminal-selection`, `--terminal-ansi-0` to `--terminal-ansi-15`). Each rule SHALL match `[data-theme="NAME"]`; the `brindle-light` rule SHALL also match `:root` and the `brindle-dark` rule `.dark`, and the `.dark` rule SHALL come after the `:root` rule.

#### Scenario: List actions
- **WHEN** the user runs `brindle --list-actions`
- **THEN** each bindable action name is printed, including `activate_tab_<N>` and `new_tab_with_profile_<N>`

#### Scenario: Print a shell script
- **WHEN** the user runs `brindle --shell-integration zsh`
- **THEN** the zsh integration script is printed and the process exits 0 without opening a window

#### Scenario: Print the PowerShell script
- **WHEN** the user runs `brindle --shell-integration pwsh`
- **THEN** the PowerShell integration script is printed and the process exits 0 without opening a window

#### Scenario: Export UI tokens
- **WHEN** the user runs `brindle --export-ui-tokens`
- **THEN** a CSS rule per built-in theme is printed, `brindle-dark`'s setting `--primary` to its accent `#e6a756`, and the process exits 0 without opening a window

### Requirement: Debug automation hooks
For automated testing, Brindle SHALL support repeatable `--send <TEXT>`, `--action <NAME>`, `--hover <POS>`, `--click <POS>`, `--find <TEXT>` and `--wait-for <TEXT>` steps. Steps SHALL run in command-line order, one second apart, against the window's active tab. `--step-interval <SECONDS>` SHALL space them that many seconds apart instead, with half that interval before the first step, and SHALL be rejected with a usage error unless it is a positive number:
- `--send` text is typed as user input, and `\r`, `\n`, `\t` and `\e` are unescaped. While a text input has keyboard focus (in a palette, the settings dialog or the find bar), the text SHALL be inserted into that input at its cursor instead of being sent to the terminal, with control characters dropped;
- `--action` runs a named action;
- `--hover` moves the mouse over a cell of the active terminal, and `--click` presses and releases the left button there. `--press` presses the left button on a cell and holds it, so a later `--hover` drags, and `--release` releases the held button over a cell. `POS` is `COL,ROW` (zero-based cells), optionally prefixed with modifiers such as `ctrl-` or `ctrl-shift-`, which are held for the step;
- `--find` opens the find bar on the active terminal with TEXT as its query, as if the user had typed it;
- `--wait-for` holds the steps after it until the active terminal's visible screen text, with its lines joined by line breaks, contains TEXT, unescaped as for `--send`. After 30 step intervals without it (30 seconds at the default interval), Brindle SHALL log a warning, record the wait as timed out, and run the remaining steps.

`--dump-screen-after <SECONDS>` SHALL print every tab title, the active terminal's size and modes, the active terminal's grid geometry (the window-pixel origin of its first cell, its cell width and line height, and the window's scale factor), the link under the mouse while hover feedback is showing, the active terminal's selected text when there is a selection, one `--- status` line per program status record of the active terminal (root first, then by id: `state=` always, then `id=`, `kind=`, `progress=`, `app=`, `title=` and `msg=` when set, strings quoted), and its visible screen text, then quit. When the active tab is a local tab with two or more zones, the dump SHALL also print a `--- zones count=N active=ID` line followed by one `--- zone id=ID rect=X,Y,W,H kind=KIND profile="NAME"[ host="…"][ active]` line per zone in tree order (with normalized `[0, 1]` coordinates, and the zone's kind, profile and remote host as "Zone kinds and profiles in the dump" defines them) before the active terminal's screen text. While a text input has focus, the dump SHALL also print that input's text, cursor offset and selection. When the active terminal's find bar is open, the dump SHALL also include a line with its query, options, match count and the current match's position on screen. For each `--wait-for` step that timed out, the dump SHALL print a `--- wait-for timeout "TEXT"` line (TEXT escaped onto one line), and for a `--wait-for` step still waiting when the dump is taken, a `--- wait-for pending "TEXT"` line, so a dump that shows an unexpected screen says which wait failed. These hooks SHALL NOT use synthetic OS-level input.

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

#### Scenario: The wait scales with the interval
- **WHEN** Brindle runs with `--step-interval 2 --wait-for 'NEVER' --send 'x'`, and `NEVER` never appears
- **THEN** `x` is typed about 61 seconds after launch (one second before the first step, then 60 seconds of waiting), not about 31

#### Scenario: A timed-out wait in the dump
- **WHEN** Brindle runs with `--step-interval 0.1 --wait-for 'NEVER' --dump-screen-after 6`, and `NEVER` never appears
- **THEN** the dump contains the line `--- wait-for timeout "NEVER"`

#### Scenario: A pending wait in the dump
- **WHEN** Brindle runs with `--wait-for 'NEVER' --dump-screen-after 3`, and `NEVER` never appears
- **THEN** the dump contains the line `--- wait-for pending "NEVER"`, and a dump with no `--wait-for` step prints neither line

#### Scenario: Invalid interval
- **WHEN** Brindle runs with `--step-interval 0`
- **THEN** it prints a usage error and exits without opening a window

#### Scenario: Status records in the dump
- **WHEN** Brindle runs with `--send 'printf "\e]7501;state=blocked:kind=auth:app=brew\e\\"\r' --dump-screen-after 3`
- **THEN** stdout contains the line `--- status state=blocked kind=auth app="brew"`

#### Scenario: Zones in the dump
- **WHEN** Brindle runs in a local tab of profile `Shell` with `--action split_terminal_right --dump-screen-after 3`
- **THEN** stdout contains `--- zones count=2 active=2`, `--- zone id=1 rect=0,0,0.5,1 kind=terminal profile="Shell"` and `--- zone id=2 rect=0.5,0,0.5,1 kind=terminal profile="Shell" active`, and a single-zone local tab prints no `--- zones` line

### Requirement: Latency log
`--latency-log <FILE>` SHALL time every keystroke a terminal tab receives, whether typed or from `--latency-bench`, and append one JSON line per keystroke to FILE: the key, and the milliseconds until the program's output arrived, until that output was painted, and until the next frame began. On quit, Brindle SHALL print a summary to stderr. Without the option, no timing SHALL be recorded. If writing FILE fails, Brindle SHALL stop writing it and keep timing, and the summary SHALL say that the log stopped early, naming the file and the error.

#### Scenario: Typed keys are logged
- **WHEN** Brindle runs with `--latency-log /tmp/lat.jsonl` and the user types `ls` at a shell prompt
- **THEN** `/tmp/lat.jsonl` gains two lines, each with the key and non-decreasing `output_ms`, `paint_ms` and `frame_ms` values

#### Scenario: Keystroke without output
- **WHEN** a logged keystroke produces no output within one second (for example, a key the program ignores)
- **THEN** its line is written with `"timeout": true` and no stage times, and it counts as a timeout in the summary

#### Scenario: Disabled by default
- **WHEN** Brindle runs without `--latency-log` or `--latency-bench`
- **THEN** no latency file is written and nothing about latency is printed

#### Scenario: Log can't be written
- **WHEN** Brindle runs with `--latency-log` naming a file on a full disk, and keys are typed
- **THEN** on quit the summary still counts every key, and says the log stopped early, naming the file and the write error

### Requirement: Desktop install options
Brindle SHALL accept `--install-desktop` and `--uninstall-desktop`, with an optional `--prefix <DIR>`, as defined by the `desktop-integration` capability. Like the informational options, they SHALL do their work and exit without opening a window or connecting to a display. They SHALL exit with status 0 on success. A file that cannot be written or removed SHALL produce an error naming the path and a non-zero exit. `--prefix` without one of these options SHALL be rejected as a usage error.

#### Scenario: Headless install
- **WHEN** `brindle --install-desktop` runs over SSH with no display
- **THEN** the files are installed and the process exits 0 without trying to open a window

#### Scenario: Unwritable target
- **WHEN** `brindle --install-desktop --prefix /usr` runs without write access
- **THEN** an error naming the path that could not be written is printed, and the exit status is non-zero

#### Scenario: Stray prefix
- **WHEN** the user runs `brindle --prefix /usr/local`
- **THEN** usage is printed and the exit status is 2

### Requirement: Notification log for tests
When the environment variable `BRINDLE_NOTIFY_LOG` names a file, Brindle SHALL append each notification it would show to that file instead of sending it to the desktop, one line per notification: the source (`bell`, `osc9`, `osc777`, `osc7501` or `agent`), the title and the body, separated by tabs. Lines SHALL be appended in the order the notifications were sent. Writing them SHALL NOT hold up the window, and every line SHALL be in the file by the time Brindle has exited, even if it quits right after the notification, for example from `--dump-screen-after`.

#### Scenario: Logged notification
- **WHEN** Brindle runs with `BRINDLE_NOTIFY_LOG=/tmp/n.log`, `notifications.when = "always"`, and `--send 'printf "\033]9;hi\a"\r'`
- **THEN** `/tmp/n.log` contains a line `osc9<TAB><tab title><TAB>hi` and no desktop notification appears

#### Scenario: Quit right after notifications
- **WHEN** Brindle runs as `brindle terminal -e sh -c 'printf "\033]9;one\a\033]777;notify;t;two\a"'` with `BRINDLE_NOTIFY_LOG=/tmp/n.log` and `notifications.when = "always"`, so its only tab's program sends two notifications and exits, which quits Brindle
- **THEN** once the process has exited, `/tmp/n.log` holds the `osc9` line for `one` followed by the `osc777` line for `two`

#### Scenario: Logged agent notification
- **WHEN** Brindle runs with `BRINDLE_NOTIFY_LOG=/tmp/n.log` and `notifications.when = "always"`, and an agent named `Fake Agent` ends a turn normally
- **THEN** `/tmp/n.log` contains a line `agent<TAB>Fake Agent — <tab title><TAB>Done`

### Requirement: Console output on Windows
When Brindle is started from a console on Windows, its informational output, usage errors and `--dump-screen-after` output SHALL appear in that console, or in the file or pipe its output is redirected to. Closing that console, or pressing ctrl-c in it, SHALL NOT close Brindle's windows.

#### Scenario: Version from PowerShell
- **WHEN** the user runs `brindle --version` in PowerShell
- **THEN** the version is printed in that PowerShell window

#### Scenario: Launching console closes
- **WHEN** the user runs `brindle` from PowerShell, then presses ctrl-c in that PowerShell and closes its window
- **THEN** Brindle's window stays open and keeps working

#### Scenario: Redirected output
- **WHEN** a test runs `brindle --dump-screen-after 2 > dump.txt` on Windows
- **THEN** `dump.txt` contains the screen dump

### Requirement: Terminal server directory for tests
When the environment variable `BRINDLE_SERVER_DIR` is set and not empty, Brindle and `brindle-remote` SHALL use it as the terminal server's directory, as defined by the `terminal-server` capability, on Linux and on Windows. Programs in tabs inherit it, so `brindle-remote` commands run in a tab manage that same server.

#### Scenario: Isolated server
- **WHEN** a test runs Brindle with `BRINDLE_SERVER_DIR=/tmp/case-1`
- **THEN** its local tabs run in a server whose socket is in `/tmp/case-1`, and the user's own server is not touched

#### Scenario: Isolated server on Windows
- **WHEN** a test runs Brindle on Windows with `BRINDLE_SERVER_DIR=C:\Temp\case-1`
- **THEN** its local tabs run in a server whose lock and log files are in `C:\Temp\case-1`, and the user's own server is not touched

#### Scenario: Managing from a tab
- **WHEN** the user runs `brindle-remote list` in a local tab
- **THEN** it lists the sessions of the server that tab runs in, that tab's own session included

### Requirement: A launch restores, then opens what it asks for
A launch that becomes the instance (see `session-restore`'s "One process owns the state") SHALL restore the recorded session and reattach detached sessions, as defined by `session-restore`, unless it has `--no-restore`. It SHALL then open what it asks for as the active tab: `brindle terminal` a new terminal tab with its profile, directory and command, `brindle agent` a new agent tab with its profile, directory and first prompt, `--attach` a tab attached to that session. A launch with none of them SHALL open no further tab, or one tab of the default profile, whatever its kind, when nothing was restored. The debug options (`--send`, `--action`, `--dump-screen-after`) act on the active tab after this.

#### Scenario: Profile launch
- **WHEN** Brindle quit with two tabs, and the user runs `brindle terminal Work`
- **THEN** the window shows the two restored tabs and an active tab with the `Work` profile

#### Scenario: Terminal without a profile
- **WHEN** Brindle quit with two tabs, and the user runs `brindle terminal`
- **THEN** the window shows the two restored tabs and an active tab with the default profile

#### Scenario: Agent launch
- **WHEN** Brindle quit with two tabs, and the user runs `brindle agent Agent`
- **THEN** the window shows the two restored tabs and an active agent tab of `Agent`

#### Scenario: Plain launch
- **WHEN** Brindle quit with two tabs, and the user runs `brindle`
- **THEN** the window shows the two restored tabs and no other tab

#### Scenario: Skip once
- **WHEN** the user runs `brindle --no-restore`
- **THEN** the window opens with a single default-profile tab

#### Scenario: Agent as the default profile
- **WHEN** the default profile is the agent profile `Agent` and the user runs `brindle --no-restore`
- **THEN** the window opens with a single agent tab of `Agent`

### Requirement: A launch goes to the running Brindle
A launch without debug options while an instance runs (one with the same state directory) SHALL send its request to the instance and exit with status 0 once the instance has accepted it. The instance SHALL raise its window and open what the launch asked for, as "A launch restores, then opens what it asks for" says, without restoring again; a `brindle terminal -e` command SHALL start in the launching process's working directory, as SHALL a `brindle agent` launch's agent when neither `-d` nor its profile sets a directory, and a `brindle agent -p` prompt, read by the launching process, SHALL be sent by the instance once that agent is ready. When the instance doesn't accept the request within 5 seconds, the launch SHALL print why and exit with status 1. The request channel SHALL be reachable only by the same user. Before sending anything, the launch SHALL check that the instance runs as the same user and, on Windows, at the same integrity level, and SHALL give it no right to act as the launching user; otherwise it SHALL send nothing, print why and exit with status 1. On Windows, a launch with no console that gives up (refused, or not accepted within 5 seconds) SHALL also show why in a message box. On Windows an elevated and an unelevated Brindle have different default state directories, so each is its own instance: a launch at either level, while a Brindle at the other runs, opens its own window, and the integrity check is reached only when `BRINDLE_STATE_DIR` names one directory for both.

#### Scenario: Second plain launch
- **WHEN** Brindle runs with two tabs and the user launches `brindle` from the desktop
- **THEN** the window is raised, it still has two tabs, and no second window opens

#### Scenario: New tab from a shell
- **WHEN** Brindle runs with two tabs and the user runs `brindle terminal` in a shell
- **THEN** the `brindle` command exits, and Brindle's window gains a third, active tab with the default profile

#### Scenario: Command from a shell
- **WHEN** Brindle runs and the user runs `brindle terminal -e htop` in a shell in `/srv`
- **THEN** the `brindle` command exits, and Brindle's window gains an active tab running htop in `/srv`

#### Scenario: Agent with a prompt from a shell
- **WHEN** Brindle runs and the user runs `brindle agent Agent -p 'run the tests'` in a shell in `~/src/brindle`, and `Agent` sets no `cwd`
- **THEN** the `brindle` command exits without waiting for the agent, and Brindle's window gains an active agent tab in `~/src/brindle` that sends `run the tests` once the agent is ready

#### Scenario: Attach from a shell
- **WHEN** Brindle runs, session 7 is detached, and the user runs `brindle --attach 7`
- **THEN** Brindle's window gains an active tab showing session 7

#### Scenario: Pipe served by someone else on Windows
- **WHEN** another account's process has created the instance pipe's name, and the user runs `brindle terminal -e pwsh`
- **THEN** the launch sends nothing to that process, prints that the pipe's server is not this user's Brindle, and exits with status 1

#### Scenario: Elevated launch beside an unelevated Brindle on Windows
- **WHEN** an unelevated Brindle runs, and the user starts `brindle` elevated without `BRINDLE_STATE_DIR`
- **THEN** a second, elevated window opens with the elevated Brindle's own session, and the unelevated window is unchanged

#### Scenario: Unelevated launch beside an elevated Brindle on Windows
- **WHEN** an elevated Brindle runs, and the user starts `brindle` unelevated from the Start Menu without `BRINDLE_STATE_DIR`
- **THEN** an unelevated window opens with the unelevated Brindle's own session, the elevated window is unchanged, and no message box shows

#### Scenario: Shared state directory across levels on Windows
- **WHEN** an unelevated Brindle runs with `BRINDLE_STATE_DIR=C:\s`, and the user starts `brindle` elevated with the same `BRINDLE_STATE_DIR`
- **THEN** the elevated launch sends nothing, says that the running Brindle is at another integrity level, and exits with status 1

#### Scenario: Refused launch from the Start Menu
- **WHEN** a launch with no console is refused by the check above
- **THEN** a message box says why before the launch exits with status 1

### Requirement: Latency benchmark samples
`--latency-bench <N>` SHALL type N keystrokes into the active tab after all `--send`/`--action` steps, alternating `x` and Backspace. Each keystroke SHALL go through the same key handling as a real key press, without synthetic OS-level input, and SHALL wait until its output is painted or one second passes, plus a randomized gap, before the next. Brindle SHALL then print a summary to stdout and quit.

#### Scenario: Benchmark in a shell
- **WHEN** Brindle runs with `--latency-bench 200` in a shell tab
- **THEN** stdout contains a `--- latency samples=200 timeouts=T` line followed by one line each for `output`, `paint` and `frame`, giving min, p50, p95, p99 and max in milliseconds, and the process exits

#### Scenario: Combined with the screen dump
- **WHEN** Brindle runs with both `--latency-bench` and `--dump-screen-after`
- **THEN** whichever finishes first prints its output and the process quits

### Requirement: No console window for Windows launches
On Windows, starting Brindle from the Start Menu, Explorer or a shortcut SHALL NOT open a console window. Programs Brindle starts outside a tab, such as the link opener, SHALL NOT open console windows either.

#### Scenario: Start Menu launch
- **WHEN** the user starts Brindle from the Start Menu
- **THEN** only Brindle's window opens, with no console window beside it

### Requirement: Terminal server helper override
When `BRINDLE_SERVER_HELPER` is set and not empty, Brindle SHALL use the file it names as the local terminal server's `brindle-remote`, on Linux and on Windows. Without it, Brindle SHALL look beside its own executable, and on Linux SHALL first honour `BRINDLE_REMOTE_AGENT`. On Windows, `BRINDLE_REMOTE_AGENT` SHALL keep naming only the Linux helper that remote profiles deploy.

#### Scenario: Override on Windows
- **WHEN** Brindle runs on Windows with `BRINDLE_SERVER_HELPER=C:\build\brindle-remote.exe` and `BRINDLE_REMOTE_AGENT` naming a Linux helper
- **THEN** local tabs run in a server started from a copy of `C:\build\brindle-remote.exe`, and remote profiles deploy the Linux helper

#### Scenario: Override names nothing
- **WHEN** `BRINDLE_SERVER_HELPER` names a file that doesn't exist and the user opens a local tab
- **THEN** the tab shows an error naming `BRINDLE_SERVER_HELPER` and the path

### Requirement: Debug steps are checked before the run
An `--action` naming no action, a `--key` that isn't a keystroke Brindle can parse, and a `--hover`, `--click` or `--right-click` position that isn't `COL,ROW` with known modifiers SHALL be refused as usage errors naming the step, before any window opens, as `--step-interval` and `--modality` values are.

#### Scenario: Misspelled action
- **WHEN** Brindle runs with `--action split_zone_rihgt --dump-screen-after 3`
- **THEN** it prints usage naming `split_zone_rihgt` and exits with status 2, without opening a window

#### Scenario: Malformed right-click position
- **WHEN** Brindle runs with `--right-click 3 --dump-screen-after 3`
- **THEN** it prints usage naming `--right-click` and `3` and exits with status 2, without opening a window

### Requirement: The dump shows the config line
When the window shows a config line (a config error, or values used in place of what the file says), `--dump-screen-after` SHALL print it as one `--- config TEXT` line, TEXT escaped onto one line, and SHALL print no `--- config` line when the window shows none.

#### Scenario: Unknown theme in the dump
- **WHEN** the config sets `theme = "tokyonight"` and Brindle runs with `--dump-screen-after 3`
- **THEN** stdout has a `--- config` line naming `tokyonight` and `brindle-dark`

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
- **THEN** it prints `BRINDLE_TEST_HOLDER_PANIC`, `BRINDLE_TEST_FEED_PANIC` and `BRINDLE_TEST_ACP_PANIC`, each on its own line, and exits 0

#### Scenario: Release build
- **WHEN** a release build runs `brindle --list-fault-switches` or `brindle-remote --list-fault-switches`
- **THEN** it prints nothing and exits 0

#### Scenario: Extra arguments to the helper
- **WHEN** `brindle-remote --list-fault-switches extra` runs
- **THEN** it prints an error saying the option takes no arguments and exits with a failure status

### Requirement: The flat launch options are removed
`-p/--profile`, and `-d/--working-directory` and `-e/--command` given without a subcommand, SHALL be refused as usage errors (status 2, before any window opens) that name the option and say what to type instead: `brindle terminal PROFILE`, `brindle terminal -d DIR` or `brindle terminal -e COMMAND`. `-p` and `--profile` after `terminal` SHALL be refused the same way, saying to name the profile directly.

#### Scenario: Old profile option
- **WHEN** the user runs `brindle -p work`
- **THEN** Brindle prints that `-p` was removed and to run `brindle terminal work`, and exits with status 2 without opening a window

#### Scenario: Old command option
- **WHEN** the user runs `brindle -e htop`
- **THEN** Brindle prints that `-e` now belongs to `brindle terminal` and to run `brindle terminal -e htop`, and exits with status 2 without opening a window

#### Scenario: Profile option under the subcommand
- **WHEN** the user runs `brindle terminal -p work`
- **THEN** Brindle prints that the profile is named directly, as `brindle terminal work`, and exits with status 2 without opening a window

#### Scenario: Old options while Brindle runs
- **WHEN** Brindle runs and the user runs `brindle -d /srv` in a shell
- **THEN** the launch prints the usage error and exits with status 2, and the running window gains no tab

### Requirement: Usage errors without a console on Windows
On Windows, a launch refused with a usage error that has no console to print to, such as one from a shortcut or the Run dialog, SHALL also show the error in a message box before it exits with status 2.

#### Scenario: Old option in a shortcut
- **WHEN** a Windows shortcut runs `brindle.exe -p work`
- **THEN** a message box says `-p` was removed and to use `brindle terminal work`, and no window opens

### Requirement: Options around the subcommand
The first argument that is neither an option nor an option's value SHALL be the subcommand, and any other such word in its place SHALL be refused as a usage error that names it and points at `brindle terminal`. `--config`, `--no-restore`, the debug, informational and desktop install options SHALL work before or after the subcommand, and with none.

#### Scenario: Debug options after the subcommand
- **WHEN** the user runs `brindle terminal Shell --no-restore --send 'echo hi\r' --dump-screen-after 3`
- **THEN** the dump shows a `Shell` tab with `hi` on its screen

#### Scenario: Options before the subcommand
- **WHEN** the user runs `brindle --no-restore --send 'echo hi\r' --dump-screen-after 3 terminal Shell`
- **THEN** the dump shows the same as with the options after the subcommand

#### Scenario: Debug options inside a command
- **WHEN** the user runs `brindle terminal -e printf '%s\n' --send`
- **THEN** the tab's program prints `--send`, and no step runs

#### Scenario: A profile name without the subcommand
- **WHEN** the user runs `brindle work`
- **THEN** Brindle prints usage naming `work` and suggesting `brindle terminal work`, and exits with status 2 without opening a window

#### Scenario: Informational option after the subcommand
- **WHEN** the user runs `brindle terminal --version`
- **THEN** the version is printed and the process exits 0 without opening a window

### Requirement: A launch's profile has its subcommand's kind
A profile named under a subcommand SHALL be of that subcommand's kind: a terminal profile under `terminal`, an agent profile under `agent`. Otherwise the launch SHALL be refused as a usage error naming the profile, its kind and the subcommand that opens it, with status 2 and nothing opened, checked as an unknown name is (see "Launch options"). A subcommand that names no profile SHALL use the default profile when it is of the subcommand's kind, else the first profile of that kind in the config; when the config has none, the launch SHALL be refused the same way, saying no profile of that kind exists.

#### Scenario: Terminal profile under the terminal subcommand
- **WHEN** the config has profiles `Shell` and `Work` and the user runs `brindle terminal Work`
- **THEN** an active terminal tab of the `Work` profile opens

#### Scenario: Unknown name under the terminal subcommand
- **WHEN** no profile is named `wrk` and the user runs `brindle terminal wrk`
- **THEN** the launch is refused for the unknown name, with status 2, and no tab opens

#### Scenario: Agent profile under the terminal subcommand
- **WHEN** `Agent` is an agent profile and the user runs `brindle terminal Agent`
- **THEN** Brindle prints that `Agent` is an agent profile, opened with `brindle agent Agent`, and exits with status 2 without opening a window

#### Scenario: Terminal profile under the agent subcommand
- **WHEN** `Shell` is a terminal profile and the user runs `brindle agent Shell`
- **THEN** Brindle prints that `Shell` is a terminal profile, opened with `brindle terminal Shell`, and exits with status 2 without opening a window

#### Scenario: Wrong kind while Brindle runs
- **WHEN** Brindle runs and the user runs `brindle agent Shell` in a shell, and `Shell` is a terminal profile
- **THEN** the launch prints that `Shell` is a terminal profile and exits with status 2, and the window gains no tab

#### Scenario: Agent without a profile, default is a terminal
- **WHEN** the default profile is `Shell`, the config's first agent profile is `Agent`, and the user runs `brindle agent`
- **THEN** an active agent tab of `Agent` opens

#### Scenario: Terminal without a profile, default is an agent
- **WHEN** the default profile is the agent profile `Agent`, the config's first terminal profile is `Shell`, and the user runs `brindle terminal`
- **THEN** an active terminal tab of `Shell` opens

#### Scenario: No agent profile
- **WHEN** the config has no agent profile and the user runs `brindle agent`
- **THEN** Brindle prints that the config has no agent profile and exits with status 2 without opening a window

### Requirement: The hand-off across versions
A running Brindle SHALL take launches from older and newer versions: a request naming a profile, directory or command, with no kind or the terminal kind, SHALL open a terminal tab; a request of the agent kind SHALL open an agent tab with the profile, directory and prompt it carries in its own agent fields; and a request naming a kind of tab it can't open SHALL be refused with nothing opened, the launch printing why and exiting with status 1. An agent request SHALL carry nothing in the fields a terminal request uses. A refusal for an unknown or wrong-kind profile, or for a missing profile of the kind, SHALL make a launch of this version exit with status 2. A plain launch's request SHALL stay as older versions read it.

#### Scenario: Older launch, newer Brindle
- **WHEN** a Brindle of this version runs and an older `brindle -p Work` launch hands it its request
- **THEN** the window gains an active tab with the `Work` profile

#### Scenario: Newer launch, older Brindle
- **WHEN** a Brindle from before `agents-launch-commands` runs and the user runs `brindle terminal Work -e htop` with this version
- **THEN** the older window gains an active `Work` tab running htop

#### Scenario: A kind the running Brindle can't open
- **WHEN** a Brindle of this version runs and a launch hands it a request for a kind of tab it doesn't know
- **THEN** it opens nothing and answers why, and the launch prints that answer and exits with status 1

#### Scenario: Unknown name, older Brindle
- **WHEN** a Brindle from before `agents-launch-commands` runs and the user runs `brindle terminal wrk` with this version, and no profile is named `wrk`
- **THEN** the older window opens its default profile with its dim line naming `wrk`, as it always has, and the launch exits with status 0

#### Scenario: Agent launch, Brindle without agent tabs
- **WHEN** a Brindle of `agents-launch-commands` but not of this change runs, and the user runs `brindle agent Agent` with this version
- **THEN** the running Brindle opens nothing and answers that it can't open agent tabs, and the launch prints that and exits with status 1

#### Scenario: Agent launch, Brindle before the subcommands
- **WHEN** a Brindle from before `agents-launch-commands` runs, and the user runs `brindle agent Agent -p hi` with this version
- **THEN** the older window is raised and opens no tab

### Requirement: The hand-off request's size
A running Brindle SHALL read a launch's request up to 1 MiB, its line ending included. A launch whose request would be larger SHALL send nothing, print that the launch is too large to hand to the running Brindle, naming the limit, and exit with status 1. A running Brindle that reads 1 MiB without the line's end SHALL open nothing and answer that the request is over the limit.

#### Scenario: Oversized launch
- **WHEN** Brindle runs and a launch's command and arguments encode to more than 1 MiB
- **THEN** the launch sends nothing, prints that it is too large for the running Brindle's 1 MiB limit, and exits with status 1, and the window gains no tab

#### Scenario: Oversized request from another program
- **WHEN** a process of the same user writes 2 MiB without a line end to the running Brindle's launch socket
- **THEN** Brindle opens nothing and answers that the request is over its 1 MiB limit

### Requirement: A first prompt from standard input
`brindle agent … -p -` SHALL read the first prompt from the launching process's standard input, to its end, before any hand-off or window. Standard input that is a terminal or absent, empty or blank, not UTF-8, or over 256 KiB SHALL be refused as a usage error (status 2) saying which, with nothing opened. A launch handed to a running Brindle whose request would then exceed that Brindle's 1 MiB request limit SHALL send nothing and exit with status 1, as any oversized launch does.

#### Scenario: Prompt from a file
- **WHEN** `task.md` holds 200 KiB of text and the user runs `brindle agent Agent -p - < task.md`
- **THEN** an agent tab opens and sends the text of `task.md` once the agent is ready

#### Scenario: Standard input is a terminal
- **WHEN** the user runs `brindle agent Agent -p -` in an interactive shell with nothing redirected
- **THEN** Brindle prints that `-p -` reads standard input, which is a terminal, and exits with status 2 without waiting for input or opening a window

#### Scenario: Too long
- **WHEN** the user pipes 300 KiB into `brindle agent Agent -p -`
- **THEN** Brindle prints that the prompt is over the 256 KiB limit and exits with status 2 without opening a window

#### Scenario: Not UTF-8
- **WHEN** the user redirects a file holding bytes that aren't UTF-8 into `brindle agent Agent -p -`
- **THEN** Brindle prints that the prompt isn't UTF-8 and exits with status 2 without opening a window

#### Scenario: Piped on Windows
- **WHEN** on Windows the user runs `Get-Content -Raw task.md | brindle agent Agent -p -` in PowerShell, or `brindle agent Agent -p - < task.md` in cmd
- **THEN** an agent tab opens and sends the text of `task.md` once the agent is ready

### Requirement: Debug hooks in an agent zone
When the active zone is an agent zone, `--dump-screen-after` SHALL print, in place of the terminal's lines, a `--- agent` line (zone, state, name, turn, activity, status), a `--- entry` line per entry the zone holds, in order (id, kind, fields, text on one line), a `--- working` line while the working row shows (its text), a `--- queued` line while the slot holds a prompt (waiting, or held and why), a `--- conversation` line (the count of entries the zone holds, at the end or away, the earlier entries not yet loaded as a count, `loading` or `none`, and a failed fetch's reason) and a `--- prompt` line (status, refusal). A permission's `--- entry` line SHALL list its options in the agent's order. `--wait-for` SHALL match those lines.

#### Scenario: A turn in the dump
- **WHEN** Brindle runs `brindle agent Fake -p hi --wait-for 'Hello!' --dump-screen-after 2` against a fake ACP agent that answers `Hello!`
- **THEN** the dump has `--- entry 1 user "hi"` and `--- entry 2 agent "Hello!"`, a `--- agent` line with `turn=ended:end_turn activity=done`, and no `--- working` line

#### Scenario: A pending permission in the dump
- **WHEN** the fake ACP agent asks permission with options `allow-once` and `reject-once`, and the dump is taken
- **THEN** the permission's `--- entry` line names both options and says it is pending

#### Scenario: A held prompt in the dump
- **WHEN** `next` was queued, the user stopped the turn, and the dump is taken
- **THEN** the dump has `--- queued held reason=cancelled "next"`

#### Scenario: Working in the dump
- **WHEN** the fake ACP agent has the prompt `hi`, reports tool call `Read main.rs` as in progress and waits, and the dump is taken
- **THEN** the dump has `--- working "Read main.rs"` after the entry lines

#### Scenario: Earlier entries in the dump
- **WHEN** an agent zone shows the newest page of a conversation with 300 entries before it, and the dump is taken
- **THEN** the dump has `--- entry` lines only for the entries the zone holds, and its `--- conversation` line has `earlier=300`

#### Scenario: A short conversation in the dump
- **WHEN** the dump is taken of an agent zone whose whole log fits in one page, at its end
- **THEN** its `--- conversation` line reads `entries=N at=end earlier=none`, so waits and checks on `conversation entries=N` and `entries=N at=end` still match

#### Scenario: Waiting for earlier entries
- **WHEN** Brindle scrolls an agent zone to its top with `--key shift-pageup` steps, then runs `--wait-for 'earlier=none'`
- **THEN** the later steps run once the zone holds the session's whole log

### Requirement: Driving an agent zone
`--send` SHALL type into the agent zone's prompt input, as into any focused text input. `--key` SHALL press keys there (enter sends, shift-enter adds a line, escape stops a turn, tab moves among the zone's stops), and `--action` SHALL run the agent actions. The `--- focus` line SHALL report `agent stop=prompt`, `agent stop=queued:remove`, `agent stop=queued:send-again`, `agent stop=queued:discard`, `agent stop=permission:<entry>:<option>` or `agent stop=stop`.

#### Scenario: Focus on an option
- **WHEN** permission request entry 3 with options `allow-once` and `reject-once` is pending, nothing is queued, the prompt input has focus, and Brindle runs `--key tab --dump-screen-after 2`
- **THEN** the dump's focus line reads `--- focus agent stop=permission:3:reject-once modality=keyboard`

#### Scenario: Answer from the keyboard
- **WHEN** the same request is pending and Brindle runs `--key tab --key enter --dump-screen-after 2`
- **THEN** the dump's permission entry says `reject-once` was chosen

#### Scenario: Allow from the keyboard
- **WHEN** the same request is pending and Brindle runs `--key tab --key tab --key enter --dump-screen-after 2`
- **THEN** the dump's permission entry says `allow-once` was chosen

#### Scenario: Send a held prompt from the keyboard
- **WHEN** `next` is held, no turn runs, the prompt input has focus, and Brindle runs `--key tab --key enter --dump-screen-after 2`
- **THEN** the dump has no `--- queued` line and an entry `user "next"`

### Requirement: Driving the close confirmation
While the close confirmation is open, `--dump-screen-after` SHALL print a `--- confirm close` line naming the zones it would close and their agents' statuses, and the `--- focus` line SHALL report `confirm stop=cancel` or `confirm stop=close`. `--key` SHALL drive it as real keys do.

#### Scenario: Confirmation in the dump
- **WHEN** zone 1's agent is running a turn and Brindle runs `--action close_zone --dump-screen-after 2`
- **THEN** the dump has a `--- confirm close` line naming zone 1 as `working`, and `--- focus confirm stop=cancel modality=keyboard`

### Requirement: Rendered blocks in the dump
In an agent zone's dump, each agent message's and thought's `--- entry` line SHALL keep its Markdown source and SHALL be followed by one `--- block entry=N part=K kind=KIND "TEXT"` line per top-level block, in order: KIND is `paragraph`, `heading:L`, `code` or `code:LANG`, `list`, `quote`, `table` or `rule`, and TEXT the block's shown text escaped onto one line. `--wait-for` SHALL match them.

#### Scenario: A fence in the dump
- **WHEN** entry 2, an agent message, is `Try:` then a blank line and a fence opened with three backticks and `rust` holding `fn main() {}`, and the dump is taken
- **THEN** the dump has `--- block entry=2 part=0 kind=paragraph "Try:"` and `--- block entry=2 part=1 kind=code:rust "fn main() {}\n"` right after entry 2's line

#### Scenario: User messages have no blocks
- **WHEN** entry 1 is the user's message `**hi**` and the dump is taken
- **THEN** the dump has `--- entry 1 user "**hi**"` and no `--- block entry=1` line

### Requirement: Links in the dump
Each link in a block SHALL add, after that block's line and in order, a `--- link entry=N part=K target="URI" text="TEXT" action=ACTION` line, ACTION being what a ctrl-click would do on this platform: `open`, `confirm`, or `refuse:no-scheme`, `refuse:scheme`, `refuse:other-host` or `refuse:no-drive`. `--wait-for` SHALL match them.

#### Scenario: Three links
- **WHEN** entry 2's text is `[docs](https://example.com) <mailto:a@example.com> [x](javascript:void)` and the dump is taken
- **THEN** the dump has, after its paragraph's block line, link lines for `https://example.com` with `action=open`, `mailto:a@example.com` with `action=confirm`, and `javascript:void` with `action=refuse:scheme`

### Requirement: Driving the link confirmation
While the link confirmation is open, `--dump-screen-after` SHALL print a `--- confirm link "URI"` line, and the `--- focus` line SHALL report `confirm stop=cancel` or `confirm stop=open`. `--key` SHALL drive it as real keys do.

#### Scenario: Confirmation in the dump
- **WHEN** the user has ctrl-clicked a `mailto:` link in an agent message and the dump is taken
- **THEN** the dump has `--- confirm link "mailto:dev@example.com"` and `--- focus confirm stop=cancel modality=pointer`

### Requirement: Spinners and motion in the dump
`--dump-screen-after` SHALL print `--- spinner tab=N` for each tab whose face shows a spinner, numbered as the tab lines, then `--- spinner zone=ID` for each zone of the active tab whose header shows one when it has two or more zones, and a `--- motion running` or `--- motion stopped` line saying whether the window's motion clock runs.

#### Scenario: A working agent
- **WHEN** the only tab is an agent zone whose turn is working, and the dump is taken
- **THEN** the dump has `--- spinner tab=1` and `--- motion running`

#### Scenario: Nothing working
- **WHEN** the agent's turn has ended, and the dump is taken
- **THEN** the dump has no `--- spinner` line, and has `--- motion stopped`

### Requirement: Tool call rows in the dump
In an agent zone's dump, a tool call's `--- entry` line SHALL say, after its status and before its title, whether its row is open (`open=yes`, `open=no`, `open=permission` while a pending permission request shows its detail, or `open=none` when it has nothing to open) and how many content items it holds (`items=N`).

#### Scenario: A running tool call
- **WHEN** the fake ACP agent reports a tool call with one diff as in progress, and the dump is taken
- **THEN** the tool call's `--- entry` line has `open=yes items=1`

#### Scenario: A finished tool call
- **WHEN** the fake ACP agent then reports that tool call as completed, and the dump is taken
- **THEN** the tool call's `--- entry` line has `open=no items=1`

### Requirement: Tool call counts and summaries in the dump
A tool call's `--- entry` line SHALL give, after `items=N`, the number of items it left out past its bounds (`more=M`), then its row's summary (`summary="…"`), each only when there is one.

#### Scenario: A row's summary
- **WHEN** the fake ACP agent reports a tool call whose one diff replaces one line, and the dump is taken
- **THEN** the tool call's `--- entry` line has `items=1 summary="+1 −1"` before its title

#### Scenario: Items left out
- **WHEN** the fake ACP agent reports a tool call with 40 text items, and the dump is taken
- **THEN** the tool call's `--- entry` line has `items=32 more=8`

#### Scenario: A row with no summary
- **WHEN** the fake ACP agent reports a tool call with no content, and the dump is taken
- **THEN** the tool call's `--- entry` line has `open=none items=0` followed by its title, with no `more=` or `summary=` field

### Requirement: Permission details in the dump
A permission request's `--- entry` line SHALL say, after its outcome, how many parts of its tool call's detail it shows (`detail=N`), and SHALL print nothing more when it shows none, so its line reads as before tool call content existed.

#### Scenario: A pending permission's detail
- **WHEN** the fake ACP agent asks permission for a tool call that holds one file change, and the dump is taken
- **THEN** the permission's `--- entry` line has `detail=` with a count above zero after `outcome=pending`, and the tool call's line has `open=permission`

#### Scenario: A permission with nothing to show
- **WHEN** the fake ACP agent asks permission `Write a file` for a tool call with no content, and the dump is taken
- **THEN** the permission's `--- entry` line ends with `outcome=pending "Write a file"`, and the tool call's line has `open=none items=0`

### Requirement: Tool call content in the dump
After a tool call's `--- entry` line, whether its row is open or closed, the dump SHALL print a `--- item` line per content item (the entry, the item's index, its kind and that kind's fields), a `--- hunk` line per hunk after its diff's line, a `--- location` line per location, and a `--- raw` line for each raw value the detail shows. Every text SHALL be escaped onto one line, and `--wait-for` SHALL match these lines.

#### Scenario: A diff in the dump
- **WHEN** the fake ACP agent reports a tool call whose diff of `/p/main.rs` replaces `old();` with `new();`, and the dump is taken
- **THEN** the dump has a line `--- item entry=N index=0 kind=diff op=modify path="/p/main.rs" added=1 removed=1 omitted=0` and a `--- hunk` line whose text holds `-    old();` and `+    new();`

#### Scenario: Command output in the dump
- **WHEN** the fake ACP agent reports a command `cargo test` whose output is `ok\n` with exit status 0
- **THEN** the dump has `--- item entry=N index=0 kind=output command="cargo test" exit=0 omitted=0 "ok\n"`

#### Scenario: Output with no command line
- **WHEN** the fake ACP agent reports a command's output `/p\n` with exit status 0 and no command line, as Codex does
- **THEN** the dump has `--- item entry=N index=0 kind=output exit=0 omitted=0 "/p\n"`, with no `command=` field

#### Scenario: Waiting for content
- **WHEN** Brindle runs with `--wait-for 'kind=diff'` against a fake ACP agent that sends a diff after a delay
- **THEN** the later steps run once the diff has arrived

#### Scenario: Raw input only as the fallback
- **WHEN** a tool call has a file change and a raw input
- **THEN** the dump has no `--- raw` line for it

### Requirement: Zone kinds and profiles in the dump
Each `--- zone` line of `--dump-screen-after` SHALL say, after the zone's rectangle, its kind (`kind=terminal` or `kind=agent`) and its profile's name (`profile="NAME"`, escaped onto the line), and `host="…"` when the zone runs on a remote host, before `active`. The palette that asks which profile a split runs SHALL print as any palette list does and be driven by the same list actions.

#### Scenario: A mixed tab's zone lines
- **WHEN** a tab holds a shell of profile `Shell` and a focused agent zone of profile `Claude`, and the dump is taken
- **THEN** it has a line `--- zone id=1 rect=… kind=terminal profile="Shell"` and a line `--- zone id=2 rect=… kind=agent profile="Claude" active`

#### Scenario: A remote zone
- **WHEN** one zone of the active tab runs on the SSH host `A8`
- **THEN** its zone line holds `host=` naming `A8`, and the local zones' lines hold no `host=`

#### Scenario: Driving the profile question
- **WHEN** a run uses `--action split_agent_right` with two agent profiles, then `--action list_down` and `--action list_confirm`
- **THEN** the dump shows a new agent zone of the second agent profile

### Requirement: Continuing a conversation from the command line
Under `agent`, Brindle SHALL also accept `-c/--continue`, which opens the agent tab on the last conversation the profile's agent had in the launch's directory, and `-r/--resume`, which opens it on the list of that agent's conversations there. Either MAY be given with `-d` and with `-p`, whose prompt SHALL be sent once the chosen conversation's agent is ready. `-c` and `-r` together, either under `terminal`, and either given twice SHALL print usage and exit with status 2.

#### Scenario: Continue the last
- **WHEN** the agent has conversations in `~/src/brindle`, the newest titled `Add paging`, and the user runs `brindle agent Claude -c` in a shell there
- **THEN** an agent tab opens showing `Add paging`, with no list shown

#### Scenario: Choose from the list
- **WHEN** the user runs `brindle agent Claude -r` in `~/src/brindle`
- **THEN** an agent tab opens with the list of `Claude`'s conversations in `~/src/brindle`, and the one chosen is shown in that tab

#### Scenario: The list dismissed
- **WHEN** that list is closed with escape
- **THEN** the tab stays, choosing, and offers Conversations… and New conversation

#### Scenario: With a directory and a prompt
- **WHEN** the user runs `brindle agent Claude -c -d ~/src/api -p 'run the tests'`
- **THEN** the last conversation in `~/src/api` is continued, and receives `run the tests` once its agent is ready

#### Scenario: Both flags
- **WHEN** the user runs `brindle agent Claude -c -r`
- **THEN** Brindle prints usage naming the options and exits with status 2, opening nothing

### Requirement: Continue with nothing to continue
`brindle agent … -c` in a directory where the agent has no earlier conversation SHALL open the agent tab choosing, saying that there is no earlier conversation in that directory, with New conversation offered. It SHALL NOT start a new conversation on its own. The same SHALL hold, with its reason, when the agent can't list its conversations.

#### Scenario: Nothing to continue
- **WHEN** the agent has no conversation in `~/src/new` and the user runs `brindle agent Claude -c` there
- **THEN** the tab opens saying there is no earlier conversation in `~/src/new`, no conversation is started, and New conversation starts one

#### Scenario: An agent that can't list
- **WHEN** the profile's agent doesn't offer a list and the user runs `brindle agent Fake -c`
- **THEN** the tab opens saying the agent can't list conversations, with New conversation offered

### Requirement: A continuing launch handed to an older Brindle
A `brindle agent` launch with `-c` or `-r` that is handed to a running Brindle from before these options SHALL NOT open a new conversation in their place: the launch SHALL print that the running Brindle is older and can't continue a conversation, and exit with status 1.

#### Scenario: An older Brindle runs
- **WHEN** a Brindle from before this change runs, and the user runs `brindle agent Claude -c` with this version
- **THEN** no tab opens, the launch says the running Brindle can't continue a conversation, and exits with status 1

### Requirement: Conversations in the dump
The dump SHALL give a choosing agent's state as `choosing`. An open conversations palette SHALL print `--- palette conversations profile="…" dir="…" state=starting|listing|listed|empty|failed more=yes|no`, then its rows as palette rows print, a row of a conversation a zone shows ending in `open`; the question of where to open SHALL print as a palette list of its rows.

#### Scenario: A listed palette
- **WHEN** the conversations palette shows two conversations, one of them open in a zone, and the dump is taken
- **THEN** it holds the `--- palette conversations` line with `state=listed`, then the two rows, the second ending in `open`

#### Scenario: A choosing zone
- **WHEN** `brindle agent Fake -c --dump-screen-after 3` runs against a fake agent with no conversations
- **THEN** the dump's agent line has `state=choosing`, and its prompt line's reason says there is no earlier conversation in the directory

### Requirement: Driving the conversations palettes
The choosing screen's buttons SHALL be focus stops named `conversations` and `new` in the dump's focus line. `--wait-for` SHALL match the conversations palette's lines, and the list actions SHALL drive the conversations palette and the question of where to open.

#### Scenario: Driving a choice
- **WHEN** a run uses `--action agent_continue_conversation`, waits for `state=listed`, then `--action list_confirm` twice
- **THEN** the first conversation is opened where the question's first row says
