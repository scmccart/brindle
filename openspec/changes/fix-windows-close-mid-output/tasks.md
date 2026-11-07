# Tasks

## 1. Failing check first

- [x] 1.1 Add the e2e case `windows-flood-responsive` to `tests/e2e/cases/win.rs`. It runs `-p cmd --send 'for /L %i in (1,0,2) do @echo flood %i\r'` with no close and dumps at 6 s. It fails unless the dump arrives within `ctx.secs(6.0) + 5.0` s and the screen contains `flood`. Register it beside `windows-close-mid-output`. Verify: `BRINDLE_E2E=1 cargo test --test e2e -- windows-flood-responsive windows-close-mid-output` fails both cases on this desktop at the unchanged build (ask before the GUI run).

## 2. Pacing helper

- [x] 2.1 Add `src/terminal/pace.rs` with `PACE` (4 ms) and a pure `delay(last_update: Option<Instant>, now: Instant) -> Option<Duration>`, which returns the rest of `PACE`, or `None` when it has passed or there was no update. Verify: unit tests for no previous update, within `PACE`, exactly at `PACE` and after `PACE` pass with `cargo test -p brindle terminal::pace`.
- [x] 2.2 Add the async wrapper `Pace::wait(&mut self, cx: &AsyncApp)` and `Pace::updated(&mut self)`. The wrapper awaits `background_executor().timer(d)` only when `delay` returns `Some(d)`. Doc-comment why: posted main-thread wakes starve `WM_PAINT` and `WM_QUIT` on Windows (see design.md). Verify: `cargo build` and `cargo clippy -p brindle` are clean.

## 3. Use it in the event loops

- [x] 3.1 In `Terminal::spawn_events`, call `pace.wait` before each `this.update`. After the wait, take at most one more ready chunk into the same update (one `now_or_never` on `rx.next()`; draining until empty never ends under a flood), and then call `pace.updated()`. Verify: `windows-flood-responsive` and `windows-close-mid-output` pass on this desktop.
- [x] 3.4 Add `Batch::coalesce` in `src/terminal/mod.rs` so that each update handles its batch's `Wakeup`s once and applies only the last title change. Use it in `spawn_events`. Verify: the `terminal::tests::batch_*` unit tests pass, and the full close variant exits in under 9 s in three runs out of three.
- [x] 3.2 Do the same in `Terminal::spawn_remote_events`, keeping consecutive `Output` merged into one `feed` across the drained chunks. Verify: `cargo test -p brindle remote` passes. On the Linux machine, the `remote-*` e2e cases pass (step 5.2).
- [x] 3.3 Do the same in the tmux session reader (`src/tmux/mod.rs`, `event_rx.ready_chunks(4096)`), keeping the event order. Verify: `cargo test -p brindle tmux` passes. On Windows, `BRINDLE_E2E=1 cargo test --test e2e -- windows-wsl-tmux` passes or skips as before.

## 4. Latency and docs

- [x] 4.1 Measure `--latency-bench` for a local `cmd` tab on Windows at the unchanged build and with the fix. Record the medians and p99 in the PR description. Verify: the medians differ by less than `PACE`.
- [x] 4.2 Update CLAUDE.md. In the Architecture paragraph on how alacritty events reach GPUI, say that the loop is paced (`terminal/pace.rs`). Under Windows "Pitfalls found on the way", add one line: every GPUI main-thread wake is a posted message, so a task that wakes continuously starves `WM_PAINT` and `WM_QUIT`. Verify: the text names the helper as it exists in the code.

## 5. Integration

- [x] 5.1 On Windows, run `cargo test --workspace` and `BRINDLE_E2E=1 cargo test --test e2e` (all Windows cases; ask first). Verify: everything passes, apart from cases that skip as before.
- [ ] 5.2 Push the branch and ask the Linux machine's session to run `cargo test --locked`, `cargo build --release --locked` and `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e`. Also ask for a `BRINDLE_PERF=1 cargo test --test perf` comparison against a run of `main` (`BRINDLE_PERF_BASELINE`). Verify: the tests pass, and latency and throughput show no regression beyond noise.

## Workflow follow-up

- Open the PR with a `fix(terminal): …` title. Archive the change after merge (`docs(openspec): archive fix-windows-close-mid-output`).
