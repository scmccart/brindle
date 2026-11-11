## ADDED Requirements

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

## MODIFIED Requirements

### Requirement: A launch restores, then opens what it asks for
A launch that becomes the instance (see `session-restore`'s "One process owns the state") SHALL restore the recorded session and reattach detached sessions, as defined by `session-restore`, unless it has `--no-restore`. It SHALL then open what it asks for as the active tab: `-p` and `-d` a new tab with that profile or directory, `-e` a tab running the command, `--attach` a tab attached to that session. A launch with none of them SHALL open no further tab, or one default-profile tab when nothing was restored. The debug options (`--send`, `--action`, `--dump-screen-after`) act on the active tab after this.

#### Scenario: Profile launch
- **WHEN** Brindle quit with two tabs, and the user runs `brindle -p Work`
- **THEN** the window shows the two restored tabs and an active tab with the `Work` profile

#### Scenario: Skip once
- **WHEN** the user runs `brindle --no-restore`
- **THEN** the window opens with a single default-profile tab

## REMOVED Requirements

### Requirement: Latency benchmark
**Reason**: Rewritten without its tmux scenario; the benchmark behaves as before. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Latency benchmark samples".

### Requirement: No console window on Windows
**Reason**: Rewritten without the tmux-through-WSL wording and scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "No console window for Windows launches".
