# Design

## Context

- **Debug steps don't use the key path.** `--send` calls `Workspace::send_to_active` → `Terminal::input` → `write`, which skips `TerminalView::key_down` and the IME path. So timing has to start where real keys arrive, and the benchmark needs a way to drive that path.
- **Keys arrive two ways:**
  - special keys and modifier combos through `key_down`, which encodes them with `keys::to_esc_str`;
  - plain text through `EntityInputHandler::replace_text_in_range`.
- **GPUI 0.2.2's frame loop.** Inside one `on_request_frame` callback, GPUI runs the `on_next_frame` callbacks, then `draw` (prepaint, then paint), then `present`, then `complete_frame`.
  - `present` is private, and when the GPU or compositor shows the frame can't be observed.
  - Two points can be observed: the end of `TerminalElement::paint`, and an `on_next_frame` callback, which runs at the start of the next frame request.
- **`Window::dispatch_keystroke(Keystroke)`** sends a `KeyDownEvent` through keybinding resolution and `key_down`. If the key isn't handled and has a `key_char`, it is passed to the window's input handler. That is in-process GPUI dispatch, not OS-level synthetic input, so it follows CLAUDE.md's rule.
- **Output reaches GPUI in two ways:**
  - for `Pty` terminals, through the event listener (`AlacEvent::Wakeup`, sent from alacritty's I/O thread and drained by a GPUI task);
  - for `Remote` terminals (tmux panes), through `feed()` on the main thread.
- **`Terminal::input` notifies by itself** when it scrolls back to the bottom or clears a selection. So "the first paint after the key" can happen before the echo has arrived.
- **Fixed window size.** The window opens at 1000×660 logical pixels (`src/main.rs`), and there is no columns or lines setting.
- **vtebench** isn't on crates.io; it is built from a git checkout (`alacritty/vtebench`). It runs inside the terminal under test, writes the benchmark output to its own stdout, times how long the terminal takes to read it, and writes results with `--dat <file>`. It isn't installed on the development machine.

## Goals / Non-Goals

**Goals:**
- Latency timing that reports each stage separately (output, paint, frame), so a regression points at the program, Brindle's model, or rendering.
- No measurable cost when it is off: one check per key and per paint.
- Throughput measured the way other terminals measure it (vtebench's stock benchmarks at a fixed grid size), so the numbers can be compared across Brindle runs and roughly with published figures.
- Each run leaves its raw data behind, and one run can be compared with another.

**Non-Goals:**
- Measuring GPU execution, compositor or scanout time. The "frame" stage stops at the next frame request.
- Statistical regression detection or thresholds.
- Profiling (Tracy, flame graphs). This change measures; it doesn't diagnose.

## Decisions

1. **Probes start at both key entry points.** `key_down` starts a probe when it encodes a key. `replace_text_in_range` starts one when it writes committed text, but not for IME pre-edit updates.
   - Each probe stores its start `Instant`, a label (the key, or the committed text), and the terminal's entity id.
   - Rejected alternative: starting in `Terminal::input`. That would also time pastes and `--send`, which aren't keystrokes, and it would miss the time spent in keybinding resolution.

2. **An output generation decides which paint completes a probe.** Each `Terminal` gets an `Arc<AtomicU64>` that counts output:
   - for `Pty`, the event listener increments it when it sends `Wakeup`, on the I/O thread;
   - for `Remote`, `feed()` increments it.

   Probe stages:
   - **Output:** the first time the probe is checked after the generation passes the probe's starting generation, its output time is recorded and the target generation saved. Output events are checked when they're handled on the main thread. For `Pty` terminals, the listener also stores the time it sent `Wakeup` in an atomic, and that time is used instead, so the latency of the channel drain isn't counted.
   - **Paint:** `TerminalElement::prepaint` reads the generation while it holds the term lock to snapshot the grid, so the snapshot and the generation agree. At the end of `paint`, any probe whose target generation is at or below the painted generation gets its paint time and schedules `window.on_next_frame` to record its frame time.
   - Notifies from `input()` don't change the generation, so they can't complete a probe early.
   - Rejected alternative: "the next paint after the input". It completes too early because of the notifies above.
   - Rejected alternative: matching the echoed character in the grid. That's fragile, and it doesn't work for Backspace.

3. **Probe state is a GPUI global** in a new `src/latency.rs`, holding:
   - whether probes are enabled;
   - an optional `BufWriter<File>` for the log;
   - the pending probes, a `VecDeque` per terminal;
   - the completed samples, kept for the summary.

   When probes are disabled, every hook is a single check of a `bool` and returns.
   - Several keys can be waiting at once when typing fast. Each output event assigns the current generation to every waiting probe that has no output time yet, because the echoes are batched.
   - A probe with no output after one second becomes a timeout. Timeouts are checked lazily on the next hook call, and once more at quit.

4. **Benchmark driver.** `--latency-bench N` runs on the existing steps task, after the steps finish. Waiting for each probe and jittering the gap keeps keystrokes from overlapping or lining up with vsync.
   - Ten warm-up keystrokes are typed and dropped first.
   - Each keystroke is `window.dispatch_keystroke(Keystroke::parse("x"))` or `…("backspace")`, alternating, so a shell's line doesn't grow.
   - Before the next keystroke, the driver waits until the probe's frame stage is recorded or it times out, polling every 1 ms on the background timer.
   - The gap after that is drawn uniformly from 10 to 40 ms, using a small xorshift generator seeded from the time. No `rand` dependency is added.
   - Then it prints the summary and quits, the same way `--dump-screen-after` does. If both options are given, whichever finishes first quits.

5. **Summary format**, one line per stage, using nearest-rank percentiles over the samples that didn't time out:
   ```
   --- latency samples=200 timeouts=0
   output min=… p50=… p95=… p99=… max=…
   paint  min=… p50=… p95=… p99=… max=…
   frame  min=… p50=… p95=… p99=… max=…
   ```
   - All values are milliseconds from when the key was received, with two decimals.
   - The JSON log line is `{"key":"x","output_ms":…,"paint_ms":…,"frame_ms":…}`, or `{"key":"x","timeout":true}` for a timeout. It is written by hand, so no serde is needed.

6. **The suite is its own target, `tests/perf/`**: `[[test]] name = "perf"`, `harness = false`. It is opted into with `BRINDLE_PERF=1` and accepts the same name filters and `--list` as e2e.
   - It shares `Result`, `ensure!` and `TmuxServer` with e2e through `tests/common/`, included with `#[path]`. It has its own small runner for Brindle: e2e's `brindle::Run` is built around `--dump-screen-after`, window captures and the e2e theme, and none of that applies here.
   - It isn't a set of e2e cases because e2e cases pass or fail and run as one batch. Perf runs take minutes, need a quiet machine, and produce numbers rather than verdicts. CLAUDE.md's rule that GUI checks belong in e2e still holds for checks, and CLAUDE.md gains a line pointing at the perf target.

7. **It times a release binary.** The suite uses `BRINDLE_PERF_BIN`, or else `target/release/brindle`, and stops with "run `cargo build --release`" if neither exists.
   - The dev build compiles Brindle's own crate at opt-level 0, which would distort both measurements.

8. **Throughput cases:**
   - `throughput-brindle`: `brindle --config <perf config> -e sh -c 'stty size > grid; exec <vtebench> -b <benchmarks> --dat brindle.dat'`. When vtebench exits, the last tab closes and Brindle quits.
   - `throughput-tmux`: Brindle with a tmux profile on a throwaway server, running vtebench in the pane. This mostly measures tmux and the control-mode path, and is reported separately.
   - The grid size from `grid` goes into the report, because vtebench results depend on it.
   - **Getting vtebench.** By default, the suite clones `https://github.com/alacritty/vtebench` at a pinned commit (`ead80032e57dee2e75f0b51f2ea67528647d9944`, the current HEAD) into `target/perf-tools/vtebench`. It builds it with `cargo build --release --locked` in that directory, using vtebench's own lockfile, so Brindle's `Cargo.lock` isn't touched. Later runs reuse the build.
     - `BRINDLE_PERF_VTEBENCH=<checkout>` uses an existing checkout instead.
     - Each run records the commit hash in the report.
     - If cloning or building fails (no network, for example), the throughput cases are skipped with the error.
   - Rejected alternative: running Alacritty side by side. It needs another install, and grid and font matching only makes it approximate. Comparisons are between Brindle runs instead (decision 10).

9. **Latency cases:**
   - `latency-shell`: `bash --norc --noprofile` with a fixed prompt, `--latency-bench 500`.
   - `latency-tmux`: the same against a throwaway tmux server (`-L brindle-perf-<pid>-<n>`, `-f /dev/null`), which times the `send-keys` → `%output` round trip.
   - Both cases take 500 samples, overridable with `BRINDLE_PERF_SAMPLES`. A sample costs the round trip plus a 25 ms average gap, so about 20 s locally and about 30 s through tmux. That is cheap. With 500 samples, p99 rests on the 5 slowest, which is stable enough to compare runs; at 200 it would rest on 2. Using the same count in both cases keeps their percentiles comparable.

10. **Report.** Each run writes `target/tmp/perf/<UTC timestamp>/`, holding the `.dat` files, the latency logs, `summary.toml` and a printed table, and points a `latest` symlink at it.
    - `BRINDLE_PERF_BASELINE=<dir>` adds a delta column against that run's `summary.toml`. It is TOML so the existing `toml` and `serde` dependencies can read it back; adding `serde_json` would have been a new dependency.
    - Throughput per vtebench benchmark is the mean milliseconds parsed from the `.dat` files. The column layout is confirmed against vtebench's `gnuplot/` scripts during implementation.

## Risks / Trade-offs

- **Could the generation and the grid disagree?** No. Alacritty's `pty_read` sends `Wakeup` while it still holds the `Term` lock, and `feed()` advances inside its own lock, so the generation `prepaint` reads under the lock always matches the grid it snapshots.
- **Desktop noise** (other apps, compositor effects, CPU frequency scaling) moves the numbers. → The suite prints the CPU governor and display type with each run, and the README recommends repeating runs on an idle machine. Percentiles rather than means keep outliers from dominating the latency numbers.
- **Opening windows takes focus.** → As with e2e: ask before running, use the same Xwayland display default (`BRINDLE_PERF_DISPLAY`), and never send OS-level input.
- **The frame stage isn't key-to-photon.** → It is named `frame`, not "presented", and the spec and README say what it covers.
- **vtebench's output format may change** upstream. → The commit is pinned, so it only changes when the pin is bumped deliberately, and parsing fails loudly with the file path.
- **A per-key `on_next_frame` closure while benchmarking** adds a little main-thread work. → It is one closure per completed probe, and none when probes are disabled.
