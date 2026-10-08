# Proposal

## Why

Brindle claims to be fast and GPU accelerated, but nothing measures it. Every change to the paint path, event batching or tmux handling could make it slower without anyone noticing. In particular, GPUI 0.2.2 re-renders the whole window on any `cx.notify()`. Two numbers would catch most regressions and make the claim checkable:
- **throughput:** how fast Brindle drains the PTY, measured with vtebench, the tool Alacritty and other terminals use, so the method matches published numbers;
- **input latency:** how long it takes from Brindle receiving a keystroke to painting the echo.

## What Changes

- **Latency probes.** Brindle can time each keystroke it receives, in three stages:
  - when the program's output arrives;
  - when the terminal that shows it finishes painting;
  - when the next frame starts.

  Probes are off unless a debug option enables them, and cost nothing when off.
- **New debug options:**
  - `--latency-log <FILE>` records each probe as a JSON line, including probes from real typing, and prints a summary on quit.
  - `--latency-bench <N>`, run after the other steps, types N keystrokes into the active tab through GPUI's in-process keystroke dispatch. That is the same path as real key presses, and it uses no OS-level synthetic input. It waits for each echo, then prints a percentile summary and quits.
- **Performance suite** (`tests/perf/`): an opt-in `harness = false` test target, enabled with `BRINDLE_PERF=1`.
  - It runs vtebench inside Brindle, in a local tab and in a tmux tab. vtebench is fetched at a pinned commit and built on first use, so it needs no manual setup.
  - It runs `--latency-bench` in a local shell tab and in a tmux control-mode tab on a throwaway tmux server.
  - It writes raw data and a summary table to `target/tmp/perf/`, and can print the change from a previous run.

  It reports numbers and never fails on them. Comparisons are between Brindle runs, such as before and after a change.
- README and CLAUDE.md document how to run the suite and how vtebench is fetched.

Out of scope:
- End-to-end key-to-photon latency (Typometer, or hardware with a camera or photodiode).
- Running other terminals (Alacritty and so on) side by side. The vtebench method matches theirs, so published figures give rough context.
- Running the suite in CI.
- Pass/fail regression thresholds.

## Capabilities

### New Capabilities

None. The vtebench runner is test tooling.

### Modified Capabilities

- `command-line`: the "Debug automation hooks" requirement gains `--latency-log` and `--latency-bench`.

## Impact

- **Code:**
  - `src/main.rs`: argument parsing, the benchmark driver and the summary output.
  - `src/terminal_view.rs`: the key path and the IME text path start probes.
  - `src/terminal/mod.rs`: output arrival marks a content generation.
  - `src/terminal_element.rs`: painting completes probes.
  - A new small `src/latency.rs` holds the probe state and statistics.
- **Tests:** a new `tests/perf/` target that reuses the e2e helpers (`tests/e2e/brindle.rs` and `tmux.rs`), plus a `[[test]]` entry in `Cargo.toml`.
- **External tools:** vtebench, needed only by the suite. It isn't published on crates.io, so the suite clones a pinned commit into `target/perf-tools/vtebench` and builds it with its own lockfile. That needs network access on first run, and leaves Brindle's `Cargo.lock` untouched.
- **Dependencies:** none are new. The JSON lines are written by hand.
