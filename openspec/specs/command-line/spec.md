# command-line Specification

## Purpose
Let users and scripts launch Brindle with a specific profile, command or directory, and give developers non-interactive hooks to drive and inspect a running instance for automated testing.

## Requirements

### Requirement: Launch options
Brindle SHALL accept:
- `-p/--profile <NAME>` (matched case-insensitively; an unknown name falls back to the default profile, and the tab it opens SHALL say, in one dim line, that no profile has that name and which profile it uses);
- `-d/--working-directory <DIR>`;
- `--config <FILE>`;
- `-e/--command <CMD> [ARGS]...`, which runs CMD instead of the profile's program and must come last;
- `--no-restore`, which skips session restore for this launch;
- `--attach <ID>` (Linux and Windows), which opens a tab attached to the detached terminal server session `ID`. When the terminal server's sessions can't be listed, that tab's message SHALL say so, and SHALL NOT say that session `ID` doesn't exist.

An unknown argument, an option given without the value it takes (`-e` without a command included), and a value an option can't use SHALL print usage, naming the option, and exit with status 2.

#### Scenario: Run a command
- **WHEN** the user runs `brindle -e htop`
- **THEN** a tab running htop opens and becomes active, and when htop exits its tab closes

#### Scenario: Attach a session
- **WHEN** `brindle-remote list` shows a detached session 7 running `htop`, and the user runs `brindle --attach 7`
- **THEN** an active tab shows that `htop`, and the session is no longer detached

#### Scenario: Attach a missing session
- **WHEN** the user runs `brindle --attach 99` and there is no session 99, or it is attached elsewhere
- **THEN** an active tab's message names session 99 and the reason

#### Scenario: Attach when the server can't be listed
- **WHEN** the user runs `brindle --attach 7` and Brindle can't list the terminal server's sessions
- **THEN** an active tab's message says the terminal server's sessions could not be listed

#### Scenario: Unknown profile
- **WHEN** no profile is named `wrk` and the user runs `brindle -p wrk`
- **THEN** a tab of the default profile opens and becomes active, and it says no profile is named `wrk` and which profile it uses

#### Scenario: Option without its value
- **WHEN** the user runs `brindle -p`, or `brindle --dump-screen-after soon`
- **THEN** Brindle prints usage naming the option and exits with status 2, without opening a window

### Requirement: Informational options
`--help`, `--version`, `--list-actions` (every bindable action name), `--list-themes` (the built-in themes) and `--shell-integration <bash|zsh|fish|pwsh>` (that shell's integration script) SHALL print their output and exit without opening a window. `--shell-integration` with a missing or unsupported shell SHALL print an error naming the supported shells and exit with status 2.

#### Scenario: List actions
- **WHEN** the user runs `brindle --list-actions`
- **THEN** each bindable action name is printed, including `activate_tab_<N>` and `new_tab_with_profile_<N>`

#### Scenario: Print a shell script
- **WHEN** the user runs `brindle --shell-integration zsh`
- **THEN** the zsh integration script is printed and the process exits 0 without opening a window

#### Scenario: Print the PowerShell script
- **WHEN** the user runs `brindle --shell-integration pwsh`
- **THEN** the PowerShell integration script is printed and the process exits 0 without opening a window

### Requirement: Debug automation hooks
For automated testing, Brindle SHALL support repeatable `--send <TEXT>`, `--action <NAME>`, `--hover <POS>`, `--click <POS>`, `--find <TEXT>` and `--wait-for <TEXT>` steps. Steps SHALL run in command-line order, one second apart, against the window's active tab. `--step-interval <SECONDS>` SHALL space them that many seconds apart instead, with half that interval before the first step, and SHALL be rejected with a usage error unless it is a positive number:
- `--send` text is typed as user input, and `\r`, `\n`, `\t` and `\e` are unescaped. While a text input has keyboard focus (in a palette, the settings dialog or the find bar), the text SHALL be inserted into that input at its cursor instead of being sent to the terminal, with control characters dropped;
- `--action` runs a named action;
- `--hover` moves the mouse over a cell of the active terminal, and `--click` presses and releases the left button there. `POS` is `COL,ROW` (zero-based cells), optionally prefixed with modifiers such as `ctrl-` or `ctrl-shift-`, which are held for the step;
- `--find` opens the find bar on the active terminal with TEXT as its query, as if the user had typed it;
- `--wait-for` holds the steps after it until the active terminal's visible screen text, with its lines joined by line breaks, contains TEXT, unescaped as for `--send`. After 30 step intervals without it (30 seconds at the default interval), Brindle SHALL log a warning, record the wait as timed out, and run the remaining steps.

`--dump-screen-after <SECONDS>` SHALL print every tab title, the active terminal's size and modes, the active terminal's grid geometry (the window-pixel origin of its first cell, its cell width and line height, and the window's scale factor), the link under the mouse while hover feedback is showing, the active terminal's selected text when there is a selection, one `--- status` line per program status record of the active terminal (root first, then by id: `state=` always, then `id=`, `kind=`, `progress=`, `app=`, `title=` and `msg=` when set, strings quoted), and its visible screen text, then quit. When the active tab is a local tab with two or more zones, the dump SHALL also print a `--- zones count=N active=ID` line followed by one `--- zone id=ID rect=X,Y,W,H[ active]` line per zone in tree order (with normalized `[0, 1]` coordinates) before the active terminal's screen text. While a text input has focus, the dump SHALL also print that input's text, cursor offset and selection. When the active terminal's find bar is open, the dump SHALL also include a line with its query, options, match count and the current match's position on screen. For each `--wait-for` step that timed out, the dump SHALL print a `--- wait-for timeout "TEXT"` line (TEXT escaped onto one line), and for a `--wait-for` step still waiting when the dump is taken, a `--- wait-for pending "TEXT"` line, so a dump that shows an unexpected screen says which wait failed. These hooks SHALL NOT use synthetic OS-level input.

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
- **WHEN** Brindle runs in a local tab with `--action split_zone_right --dump-screen-after 3`
- **THEN** stdout contains `--- zones count=2 active=2`, `--- zone id=1 rect=0,0,0.5,1` and `--- zone id=2 rect=0.5,0,0.5,1 active`, and a single-zone local tab prints no `--- zones` line

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
When the environment variable `BRINDLE_NOTIFY_LOG` names a file, Brindle SHALL append each notification it would show to that file instead of sending it to the desktop, one line per notification: the source (`bell`, `osc9`, `osc777` or `osc7501`), the title and the body, separated by tabs. Lines SHALL be appended in the order the notifications were sent. Writing them SHALL NOT hold up the window, and every line SHALL be in the file by the time Brindle has exited, even if it quits right after the notification, for example from `--dump-screen-after`.

#### Scenario: Logged notification
- **WHEN** Brindle runs with `BRINDLE_NOTIFY_LOG=/tmp/n.log`, `notifications.when = "always"`, and `--send 'printf "\033]9;hi\a"\r'`
- **THEN** `/tmp/n.log` contains a line `osc9<TAB><tab title><TAB>hi` and no desktop notification appears

#### Scenario: Quit right after notifications
- **WHEN** Brindle runs with `BRINDLE_NOTIFY_LOG=/tmp/n.log`, `notifications.when = "always"` and `-e sh -c 'printf "\033]9;one\a\033]777;notify;t;two\a"'`, so its only tab's program sends two notifications and exits, which quits Brindle
- **THEN** once the process has exited, `/tmp/n.log` holds the `osc9` line for `one` followed by the `osc777` line for `two`

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
A launch that becomes the instance (see `session-restore`'s "One process owns the state") SHALL restore the recorded session and reattach detached sessions, as defined by `session-restore`, unless it has `--no-restore`. It SHALL then open what it asks for as the active tab: `-p` and `-d` a new tab with that profile or directory, `-e` a tab running the command, `--attach` a tab attached to that session. A launch with none of them SHALL open no further tab, or one default-profile tab when nothing was restored. The debug options (`--send`, `--action`, `--dump-screen-after`) act on the active tab after this.

#### Scenario: Profile launch
- **WHEN** Brindle quit with two tabs, and the user runs `brindle -p Work`
- **THEN** the window shows the two restored tabs and an active tab with the `Work` profile

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
- **THEN** it prints `BRINDLE_TEST_HOLDER_PANIC` and `BRINDLE_TEST_FEED_PANIC`, each on its own line, and exits 0

#### Scenario: Release build
- **WHEN** a release build runs `brindle --list-fault-switches` or `brindle-remote --list-fault-switches`
- **THEN** it prints nothing and exits 0

#### Scenario: Extra arguments to the helper
- **WHEN** `brindle-remote --list-fault-switches extra` runs
- **THEN** it prints an error saying the option takes no arguments and exits with a failure status
