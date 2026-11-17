# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: Debug steps are checked before the run
An `--action` naming no action, a `--key` that isn't a keystroke Brindle can parse, and a `--hover` or `--click` position that isn't `COL,ROW` with known modifiers SHALL be refused as usage errors naming the step, before any window opens, as `--step-interval` and `--modality` values are.

#### Scenario: Misspelled action
- **WHEN** Brindle runs with `--action split_zone_rihgt --dump-screen-after 3`
- **THEN** it prints usage naming `split_zone_rihgt` and exits with status 2, without opening a window

### Requirement: The dump shows the config line
When the window shows a config line (a config error, or values used in place of what the file says), `--dump-screen-after` SHALL print it as one `--- config TEXT` line, TEXT escaped onto one line, and SHALL print no `--- config` line when the window shows none.

#### Scenario: Unknown theme in the dump
- **WHEN** the config sets `theme = "tokyonight"` and Brindle runs with `--dump-screen-after 3`
- **THEN** stdout has a `--- config` line naming `tokyonight` and `brindle-dark`
