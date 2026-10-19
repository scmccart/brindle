# command-line Specification

## Purpose
Let users and scripts launch Brindle with a specific profile, command or directory, and give developers non-interactive hooks to drive and inspect a running instance for automated testing.

## Requirements

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
- `--wait-for` holds the steps after it until the active terminal's visible screen text, with its lines joined by line breaks, contains TEXT, unescaped as for `--send`. After 30 seconds without it, Brindle SHALL log a warning and run the remaining steps.

`--dump-screen-after <SECONDS>` SHALL print every tab title, the active terminal's size and modes, the active terminal's grid geometry (the window-pixel origin of its first cell, its cell width and line height, and the window's scale factor), the link under the mouse while hover feedback is showing, the active terminal's selected text when there is a selection, and its visible screen text, then quit. While a text input has focus, the dump SHALL also print that input's text, cursor offset and selection. When the active terminal's find bar is open, the dump SHALL also include a line with its query, options, match count and the current match's position on screen. These hooks SHALL NOT use synthetic OS-level input.

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

### Requirement: Latency log
`--latency-log <FILE>` SHALL time every keystroke a terminal tab receives, whether typed or from `--latency-bench`, and append one JSON line per keystroke to FILE: the key, and the milliseconds until the program's output arrived, until that output was painted, and until the next frame began. On quit, Brindle SHALL print a summary to stderr. Without the option, no timing SHALL be recorded.

#### Scenario: Typed keys are logged
- **WHEN** Brindle runs with `--latency-log /tmp/lat.jsonl` and the user types `ls` at a shell prompt
- **THEN** `/tmp/lat.jsonl` gains two lines, each with the key and non-decreasing `output_ms`, `paint_ms` and `frame_ms` values

#### Scenario: Keystroke without output
- **WHEN** a logged keystroke produces no output within one second (for example, a key the program ignores)
- **THEN** its line is written with `"timeout": true` and no stage times, and it counts as a timeout in the summary

#### Scenario: Disabled by default
- **WHEN** Brindle runs without `--latency-log` or `--latency-bench`
- **THEN** no latency file is written and nothing about latency is printed

### Requirement: Latency benchmark
`--latency-bench <N>` SHALL type N keystrokes into the active tab after all `--send`/`--action` steps, alternating `x` and Backspace. Each keystroke SHALL go through the same key handling as a real key press, without synthetic OS-level input, and SHALL wait until its output is painted or one second passes, plus a randomized gap, before the next. Brindle SHALL then print a summary to stdout and quit.

#### Scenario: Benchmark in a shell
- **WHEN** Brindle runs with `--latency-bench 200` in a shell tab
- **THEN** stdout contains a `--- latency samples=200 timeouts=T` line followed by one line each for `output`, `paint` and `frame`, giving min, p50, p95, p99 and max in milliseconds, and the process exits

#### Scenario: Benchmark in a tmux tab
- **WHEN** Brindle runs with `-p tmux --latency-bench 200` against a tmux server
- **THEN** the keystrokes go to the active pane through tmux, and the summary measures the tmux round trip

#### Scenario: Combined with the screen dump
- **WHEN** Brindle runs with both `--latency-bench` and `--dump-screen-after`
- **THEN** whichever finishes first prints its output and the process quits

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

### Requirement: Plain launch restores the session
A launch without `-p`, `-d`, `-e` or `--no-restore` is a plain launch. With `restore_session` on, a plain launch SHALL restore the recorded session as defined by the `session-restore` capability. Any other launch SHALL open its single window as requested and SHALL NOT restore. The debug options (`--send`, `--action`, `--dump-screen-after`) SHALL NOT affect whether a launch is plain, and they act on the first restored window.

#### Scenario: Profile launch skips restore
- **WHEN** restore is on and the user runs `brindle -p tmux`
- **THEN** one window opens with the tmux profile, and the recorded tabs are not reopened

#### Scenario: Skip once
- **WHEN** restore is on and the user runs `brindle --no-restore`
- **THEN** one window opens with a single default-profile tab

### Requirement: Notification log for tests
When the environment variable `BRINDLE_NOTIFY_LOG` names a file, Brindle SHALL append each notification it would show to that file instead of sending it to the desktop, one line per notification: the source (`bell`, `osc9` or `osc777`), the title and the body, separated by tabs. The line SHALL be written before the notification counts as sent, so it is in the file even if Brindle quits right after, for example from `--dump-screen-after`.

#### Scenario: Logged notification
- **WHEN** Brindle runs with `BRINDLE_NOTIFY_LOG=/tmp/n.log`, `notifications.when = "always"`, and `--send 'printf "\033]9;hi\a"\r'`
- **THEN** `/tmp/n.log` contains a line `osc9<TAB><tab title><TAB>hi` and no desktop notification appears

### Requirement: No console window on Windows
On Windows, starting Brindle from the Start Menu, Explorer or a shortcut SHALL NOT open a console window. Programs Brindle starts outside a tab, such as tmux through WSL, the link opener and tmux session checks, SHALL NOT open console windows either.

#### Scenario: Start Menu launch
- **WHEN** the user starts Brindle from the Start Menu
- **THEN** only Brindle's window opens, with no console window beside it

#### Scenario: WSL attach opens no console
- **WHEN** the user opens a control-mode profile whose command is `wsl`
- **THEN** no console window appears while the session is attached

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
