# Spec Delta

## ADDED Requirements

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
