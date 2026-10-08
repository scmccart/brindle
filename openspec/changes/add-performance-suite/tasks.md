# Tasks

## 1. Probe state and statistics

- [ ] 1.1 Add `src/latency.rs`, holding:
  - the `Latency` global: enabled flag, optional log writer, pending probes per terminal entity, completed samples;
  - nearest-rank percentile and summary formatting (design decision 5);
  - JSON-line formatting.

  Verify with unit tests for the percentiles (1 sample, an even count, all equal), the summary text (including `timeouts=`), and JSON lines for a completed probe and a timeout.
- [ ] 1.2 Add the probe life cycle as pure functions over the state, with injected `Instant`s:
  - `start(terminal, label, generation)`;
  - `output(terminal, generation, at)`, which assigns the generation to every waiting probe with no output time;
  - `painted(terminal, generation, at)`;
  - `frame(probe, at)`;
  - `expire(now)`, which times out probes after 1 s.

  Verify with unit tests for batched echoes of two keys, a notify-only paint not completing a probe, and a timeout.

## 2. Hooks in the terminal and views

- [ ] 2.1 Add an output generation to `Terminal`. Verify with a unit test that `feed()` advances the generation and `input()` doesn't.
  - an `Arc<AtomicU64>` counter, plus an atomic "last output" timestamp;
  - incremented by `Listener::send_event` on `Wakeup` (`Pty` backend) and by `feed()` (`Remote` backend);
  - accessor methods.
- [ ] 2.2 Start probes in `TerminalView::key_down`, when a key is encoded, and in `replace_text_in_range`, for committed text only, not pre-edit. Each start records the current generation. Verify with `cargo build`, and check that the disabled path is one `bool` check by reading the code.
- [ ] 2.3 Mark output on the terminal's event drain, using the listener's timestamp for `Pty`. In `TerminalElement`, read the generation under the prepaint term lock, call `painted` at the end of `paint`, and schedule `window.on_next_frame` for the frame stage. Verify with `cargo test` and `cargo build --release`.

## 3. Command-line options

- [ ] 3.1 Parse `--latency-log <FILE>` and `--latency-bench <N>` and add them to `HELP` under DEBUGGING.
  - Either option installs the enabled `Latency` global.
  - On quit, flush the log and print the summary to stderr when `--latency-log` is set.

  Verify that `brindle --help` lists both options, and with a unit test of `parse_args`-style parsing if it is factored out; otherwise by running it manually.
- [ ] 3.2 Add the benchmark driver to the steps task, as in design decision 4:
  - ten warm-up keystrokes;
  - `dispatch_keystroke` alternating `x` and `backspace`;
  - wait for the frame stage or a timeout;
  - a 10–40 ms xorshift jitter;
  - print the summary to stdout, then quit, with whichever of this and `--dump-screen-after` finishes first winning.

  Verify (this opens a window, so ask first): `./target/release/brindle -e bash --norc --noprofile --latency-bench 50` prints `--- latency samples=50` with three stage lines and exits.
- [ ] 3.3 Verify the tmux path (opens a window, so ask first) against a throwaway server: `--latency-bench 50` with a profile using `tmux_args = ["-L", "brindle-perf-manual", "-f", "/dev/null"]` prints a summary with no timeouts. Kill the server afterwards.
- [ ] 3.4 Document `--latency-log` and `--latency-bench` in README.md's debugging section. Say what the `output`, `paint` and `frame` stages measure, and that this is not key-to-photon latency. Verify that the README commands match `--help`.

## 4. Performance suite

- [ ] 4.1 Make the e2e helpers usable from a second target:
  - move anything `brindle.rs` and `tmux.rs` need from `tests/e2e/main.rs` (`Result`, `ensure!`, `Ctx` pieces) into a shared module included with `#[path]`;
  - let `brindle::Run` accept extra arguments and a `-e` command.

  Verify that `BRINDLE_E2E=1 cargo test --test e2e -- --self-test` and `--list` still pass.
- [ ] 4.2 Add the `[[test]] name = "perf"` target (`tests/perf/main.rs`, `harness = false`):
  - opt-in through `BRINDLE_PERF=1`;
  - name filters and `--list`;
  - binary resolution (design decision 7);
  - `BRINDLE_PERF_DISPLAY`;
  - the per-run directory and `latest` symlink;
  - environment notes in the header: CPU governor, display type, `brindle --version`, git HEAD.

  Verify that plain `cargo test` prints a skip line for perf, and `BRINDLE_PERF=1 cargo test --test perf -- --list` lists the cases.
- [ ] 4.3 Add `latency-shell` and `latency-tmux`, which run `--latency-bench` with 500 samples (`BRINDLE_PERF_SAMPLES` overrides it), save the log, and parse the summary into `summary.json`. Verify (ask first) that both produce summaries and that the throwaway tmux server is gone afterwards.
- [ ] 4.4 Add vtebench support:
  - use `BRINDLE_PERF_VTEBENCH` if set;
  - otherwise clone the pinned commit into `target/perf-tools/vtebench` and build it with `cargo build --release --locked`, reusing the build on later runs;
  - record the commit hash;
  - skip with the error if fetching or building fails.

  Parse `.dat` files into a mean per benchmark, after confirming the column layout against vtebench's `gnuplot/` scripts. Verify that a first run fetches and builds into `target/perf-tools/`, that a second run reuses it, and that Brindle's `Cargo.lock` is unchanged. Also verify with a self-test that parses a checked-in sample `.dat` fixture. Parse `.dat` files into a mean per benchmark, after confirming the column layout against vtebench's `gnuplot/` scripts. Verify with a self-test that parses a checked-in sample `.dat` fixture.
- [ ] 4.5 Add `throughput-brindle` and `throughput-tmux`, recording the grid size from `stty size`. Verify (ask first) that both write `.dat` files and appear in the table with the grid size.
- [ ] 4.6 Print the summary table: rows are cases or benchmarks, columns are local and tmux. Add the `BRINDLE_PERF_BASELINE` delta column. Verify with a self-test that formats the table from two fixture `summary.json` files.
- [ ] 4.7 Document the suite:
  - in README.md's testing section: vtebench is fetched automatically (`BRINDLE_PERF_VTEBENCH` overrides it), the environment variables (including `BRINDLE_PERF_SAMPLES`), how to read the results, and how to run on a quiet machine;
  - in CLAUDE.md: a short "Performance suite" subsection next to "End-to-end suite".

  Verify that the documented commands match the runner's `--list` and its variables.

## 5. Integration check

- [ ] 5.1 Full run (ask first): `cargo build --release && BRINDLE_PERF=1 cargo test --test perf`. Then run again with `BRINDLE_PERF_BASELINE=target/tmp/perf/latest`. Confirm that every case either reports or skips with a reason, that the deltas print, and that `cargo test` and the e2e suite are unaffected.
