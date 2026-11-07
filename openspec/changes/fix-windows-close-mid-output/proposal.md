# Proposal

## Why

On Windows, a tab whose program prints continuously freezes Brindle's window. It stops repainting, Windows marks it "not responding", and it ignores quitting. The e2e case `windows-close-mid-output` fails on this desktop every time for that reason ("Brindle didn't exit"). The close step isn't to blame: flooding alone, with no close, hangs the same way, with the packaged ConPTY or `BRINDLE_CONPTY=inbox`, and in debug or release builds.

The cause, found on 2026-11-07 at `1b4bd93`:
- Each terminal's event task (`Terminal::spawn_events`) loops on `ready_chunks` and never awaits anything that is pending while output keeps arriving. Under a flood it stays inside one poll and holds the main thread.
- GPUI 0.2.2 on Windows wakes the main thread with one posted `WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD` message per task wake. Win32 delivers `WM_PAINT` and `WM_QUIT` only when no posted message is waiting. A steady stream of wakes therefore starves painting and quitting, even when the task does yield.

Two experiments in a scratch worktree confirmed it:
- A bare yield after each batch made the window respond and the dump print, but Brindle still didn't exit, and the main thread stayed at 100 %.
- Waiting 4 ms between batches made the case pass. It exited in about 7 s, with one tab left.

The CI runner passes. That's probably because its program produces output more slowly, which lets the channel drain. Users hit the freeze on fast machines.

## What Changes

- A terminal's event loop paces its UI updates while output keeps arriving. Batches that come within a short interval of the previous update wait out the rest of that interval, so the main thread's message queue goes idle between them. An event after a quiet spell is still handled at once.
- A batch's `Wakeup` events and title changes are handled once per update, not once per event. Found during implementation: cmd retitles the console for every command, and each title change made Brindle take process snapshots, which cost seconds per batch under a flood.
- The same pacing applies to the other per-terminal and per-session event loops of the same shape: remote and agent terminals, and tmux sessions.
- The `windows-close-mid-output` case keeps checking close during output. A flood-only check confirms that the window keeps painting and quits on time while a program floods.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `terminal-emulation`: adds a requirement that the window stays responsive (it paints, takes input and quits) while a terminal receives continuous output.

## Impact

- `src/terminal/mod.rs` (`spawn_events`, `spawn_remote_events`) and `src/tmux/mod.rs` (the session event loop), plus a small shared pacing helper.
- `tests/e2e/cases/win.rs`: a flood-only case beside `windows-close-mid-output`.
- Output latency: an update can be delayed by at most the pacing interval, and only during continuous output. Keystroke echo after a quiet spell is unaffected. The `--latency-bench` numbers are checked before and after.
- No dependency changes. GPUI isn't patched.
