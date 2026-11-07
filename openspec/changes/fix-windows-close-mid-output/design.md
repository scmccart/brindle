# Design

## Context

See proposal.md, "Why", for the cause. The constraints that shape the fix:

- **GPUI 0.2.2 on Windows** (`platform/windows/dispatcher.rs`, `platform.rs`). `dispatch_on_main_thread` sends the runnable on a flume channel and calls `PostMessageW(WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD)` every time. The handler drains the channel. Painting comes from the VSync thread's `RedrawWindow(RDW_INVALIDATE)`, which means `WM_PAINT`, and `cx.quit()` ends in `PostQuitMessage`. Win32 hands out both only when no posted message is waiting, so the window can paint and quit only while the main thread's queue has idle gaps.
- **Three event loops have the same shape**: `while let Some(batch) = rx.ready_chunks(N).next().await { this.update(…) }`.
  - `Terminal::spawn_events`: the alacritty and scanner events of local Windows tabs and Linux server sessions. `Wakeup` is sent after every parse.
  - `Terminal::spawn_remote_events`: agent and remote output, parsed on the UI thread in `feed`.
  - The tmux session reader in `src/tmux/mod.rs`: `%output` for every pane.

  None of them awaits anything that can be pending while its producer keeps up.
- **What was measured on this desktop** (Windows 11 26300, packaged ConPTY 1.25.260930003):

  | Build | Result |
  |---|---|
  | unchanged | hangs; main thread at 100 %; `Responding=False` |
  | yield after each batch | responds and dumps, but doesn't exit; main thread still at 100 % |
  | `timer(4 ms)` after each batch | case passes in about 7 s, but not reliably |
  | pacing alone (found during apply) | hangs: one 351-event update took 4.6 s |
  | pacing and coalescing | 7–8 s in every run |
  | coalescing alone | 8 s and 13 s, the second over the case's limit |

  Per-event timing during apply found that the handlers themselves take under 1 ms. The time went to the `TitleChanged` subscribers. cmd retitles the console for every command it runs (`C:\…\cmd.exe - echo flood N`). `terminal_automatic_title` ignores console titles and falls back to the foreground process name, which takes a Toolhelp `Snapshot` (zone, then tab, two or three per event). One title event per loop iteration came to seconds per batch.

## Goals / Non-Goals

**Goals:**
- Leave the main thread's message queue idle regularly during any output flood, so `WM_PAINT`, `WM_QUIT`, timers and input all get through.
- Add no latency to output that follows a quiet spell (typing, prompts).
- Use one helper for all three loops.

**Non-Goals:**
- Patching or forking GPUI's dispatcher, for example to coalesce posts or give the drain a time budget.
- Changing how the I/O threads read or parse. Parsing stays off the UI thread where it is today.
- The close path (`Pseudoconsole` drop on `conpty-close`). It was not involved.

## Decisions

### Pace updates in time, not by yielding
After an update, the loop notes when it finished. When the next batch is ready sooner than `PACE` after that, the loop first awaits a GPUI timer (`background_executor().timer`) for the rest of `PACE`. After the wait it also takes at most one more ready chunk into the same update. Draining until the channel is empty never ends under a flood: that was measured during apply. A batch that arrives after `PACE` has already passed is handled at once.

Pacing starts only after `BURST` (4) updates in a row have each come within `PACE` of the previous one. The first version paced from the second quick update, and Linux `latency-tmux` regressed by about 4 ms at every percentile: a keystroke's `send-keys` response updates the session, and the echo's `%output` follows within `PACE`, so every echo waited. A short burst like that now goes straight through, while a flood is paced from its fifth update on.

- A timer sends nothing to the main thread until it fires: it runs on the threadpool and dispatches once. That leaves the gap Windows needs.
- **Alternatives considered:**
  - A bare yield: measured, and not enough. Each wake is still a posted message, so the queue never empties.
  - Coalescing `Wakeup`s at the sender with a flag: it cuts the number of wakes, but the next one is posted as soon as the UI clears the flag, so the queue still never idles.
  - Pacing on frames (`on_next_frame`): frames come from `WM_PAINT`, which is the thing being starved, and `Terminal` is a model with no window.

### Count a batch's `Wakeup`s and title changes once
`Terminal::spawn_events` splits each batch with `Batch::coalesce`, a pure function with unit tests:
- Its `Wakeup`s count as one, handled after the other events.
- Only its last `Title` or `ResetTitle` is applied.
- Every other event keeps its order.

Within one update nothing observes the titles in between, so only the last one matters. `Wakeup` handling (latency marker, sync-flush scheduling) depends only on the latest generation.

- **Alternatives considered:**
  - Caching the process snapshot across refreshes: it would make each refresh cheaper, but the number of refreshes would still scale with the program's titles.
  - Sharing one snapshot between the zone and tab refreshes: worth doing, but it's a separate change and isn't needed here.

### `PACE` is 4 ms
This value was measured to work. It caps a flooding terminal at about 250 updates a second, above any display rate, so painting isn't visibly throttled. With a fixed `PACE`, the worst-case added latency during continuous output is one interval.

- If Windows rounds the threadpool timer up (to its 15.6 ms default tick, for example), floods update at about 64 Hz. That is still acceptable, and output after a quiet spell is still unaffected.
- One constant serves every platform. Linux's calloop-based GPUI loop doesn't starve this way, but the pacing does no harm there, and a single code path keeps behaviour the same.

### One helper, three callers
A small async helper owns the "last update, wait if too soon" state. Each loop calls it before its `update`, so the batching logic of each loop stays as it is. The helper lives in `src/terminal/` (for example `pace.rs`), and the tmux module uses it from there.

### Test the flood itself
`windows-close-mid-output` already failed before its close step, so it covers the fix. A new case, `windows-flood-responsive`, makes the cause explicit. It floods `cmd` with no close and checks two things:
- The dump arrives on time.
- The screen shows `flood` lines. They can only be there if output reached the grid and the run wasn't frozen.

Pixel checks of repainting aren't needed. A starved window never reaches the dump.

## Risks / Trade-offs

- [Pacing could delay an echo typed during continuous output by up to `PACE`; without the burst allowance it delayed every tmux echo, measured] → Measure with `--latency-bench` (local shell) on Windows and Linux before and after. A quiet-spell echo should be unchanged.
- [Throughput: one update covers more bytes] → Parsing is on the I/O thread for local tabs, so throughput isn't affected. For agent and remote terminals `feed` parses on the UI thread, and bigger batches mean fewer, longer updates; check that `throughput-brindle` doesn't regress on Linux.
- [A busy tmux session sees its control events grouped up to 4 ms later] → Ordering is unchanged. The response FIFO matching depends only on order, not timing.
- [The CI runner never showed the bug, so CI can't prove the fix] → The new case fails on unfixed builds on fast machines (this desktop). Record the local before and after results in the PR.

## Open Questions

- Why the `windows-latest` runner drains in time: a slower ConPTY flood, fewer cores, or a lower VSync rate. It doesn't change the fix.
