# Design

## Context

See proposal.md for the flakes and their causes. Each was traced by an investigation whose evidence shapes the fixes below:

- **Reproduced locally with the real test binaries**, using loops, parallel binaries and timestamping or exec-logging shims: the ready-fd handoff (`dup2(3, 3)`), `ETXTBSY` on the fake helper, the truncated tail, the lost owner, the lock retake, and the shared `config_edit` temp directory.
- **Traced from CI artifacts plus source, not reproduced:**
  - the dropped `WM_DELETE_WINDOW`: X server source shows a hung-up client is closed before its buffered requests are read (os/ospoll.c, os/connection.c `ClientReady` in the X11Libre mirror; Xorg 21.1 assumed the same);
  - `dump-grid-geometry`'s window wait;
  - the two Windows cases.

Three properties of the code constrain the fixes:

- Every test binary runs its tests on parallel threads that fork children (the terminal server's roles, `update-desktop-database`, fake helpers). Any fd open in one thread at the moment another forks is shared by that child until its `exec`, even when it is close-on-exec. `ETXTBSY` and the lock retake both come from this.
- The core is one thread on a poll loop. `flush_all` (`remote-agent/src/server/core.rs:382`) flushes clients before holders. A holder socket can also hold back many MiB of checkpoint chunks, so even holders-first flushing isn't a guarantee.
- The holder handles every frame it has read before acting on a closed core connection (`holder.rs` `read_core`: "What was read before a failure still counts"). Bytes the core wrote into the socket before dying still reach the holder.

## Goals / Non-Goals

**Goals:**
- Fix each flake at its cause. Each test must still check what it checked before, and no case may be skipped, retried as a whole, or made to wait longer on passing runs, except the bounded waits noted below.
- Make the remaining timing-sensitive e2e failures explain themselves in their artifacts.

**Non-Goals:**
- A general "retry flaky tests" mechanism in the harness or CI. Retries would hide the next real race.
- Upgrading gpui, or fixing GPUI's blank-new-window bug (zed-industries/zed#62081).
- The load-only holder failures and the test-hygiene issues listed as follow-ups in proposal.md.

## Decisions

### `request_close` waits for the server to process its request

Use `send_event(...)?.check()` (x11rb's sync round trip) instead of `send_event` + `flush`, then drop the connection.
- Alternative: keep one long-lived harness connection, as a window manager does. That is more state for one call.
- Alternative: sleep before dropping. That is a timing guess.
- After the round trip, the server has delivered the event to Brindle's queue, which is exactly the property a window manager's close button has.

### Window waits scale with `BRINDLE_E2E_SLOW`, and early returns reap Brindle

`capture`'s 3 s viewable-window wait (`tests/e2e/capture.rs:248`) becomes `ctx`-scaled with a 10 s base. The function takes the scale it needs as a parameter; it has no `ctx` today. The longer wait costs nothing on passing runs, since it ends when the window appears.

Every early `?` return in `tests/e2e/brindle.rs` after Brindle is spawned goes through a guard that kills and waits the child. Today one path (`brindle.rs:494`) leaves Brindle running, and it later panics on a broken stdout.

### The ready fd: clear close-on-exec when `dup2` is a no-op

In `support::start` (`remote-agent/tests/support/mod.rs:141`) and Brindle's `start_core` (`src/remote/local.rs:228`): when the pipe's fd already equals the target fd, clear `FD_CLOEXEC` with `fcntl` instead of calling `dup2`. This is the pattern `daemon.rs:63-69` and `support/mod.rs:928-937` already use. Factor it into one helper. Put it in `brindle-remote-protocol`, which both crates use, if that crate already depends on `libc`; otherwise use one helper per crate rather than add the dependency.

### `ETXTBSY`: write executables so that no forked writer can exist when they run

`script()` in `src/remote/local.rs` tests, and the equivalent in `terminal/links.rs` tests, writes the file. It then waits, bounded at 5 s, until a read lease (`fcntl(F_SETLEASE, F_RDLCK)`) can be taken, which the kernel grants only once no writer is open anywhere, and releases it.
- Alternative: retry the exec on `ExecutableFileBusy` in `start_core`. This is a product change for a test-only race, and it doesn't cover desktops, where these tests run the script through `systemd-run`.
- Alternative: write the file from a child process. Its fd is then never in this process to leak. This also works, but needs a subprocess per script.
- The lease was validated in a scratch miniature: 0 failures in 3,000 runs, against 99 without it.

### The lost owner: gate the attach's replay on its meta reaching the holder socket

When an attach changes a session's meta, the core sends `SET_META` to the holder. That client's `REPLAY_START`/replay frames must not be written to the client socket until the `SET_META` frame has been fully written to the holder socket.
- Implementation: give each holder connection a running count of bytes written. `store_meta` records the count at which the frame ends, and the attaching client's replay waits in the session's pending-replay path (where replays already wait while a new core rebuilds) until the holder's count passes it.
- Also flush holders before clients in `flush_all`, so the gate usually opens in the same loop turn and adds no latency.
- If the holder connection is lost before the frame is written, the replay proceeds. Nothing better is possible, and the next core falls back to the old owner, as today.
- Alternative: flush holders first, with no gate. It fixes the common case but not a backed-up holder socket, so the spec scenario wouldn't hold.
- Alternative: a holder acknowledgement for `SET_META`. That needs H1 version 3 for a property the socket buffer already gives.
- The test (`core.rs:939-956`) keeps its kill-right-after-attach shape. It becomes the regression check for the new spec scenario, and stays as aggressive as it is now.

### The truncated tail: the test waits for its own output to be processed

`a_session_with_more_output_than_a_tail_holds_rebuilds_exactly` floods past the tail cap and kills the core on `all printed`. The core is behaving as designed: with output still waiting it defers checkpoints past 4 MiB. The script gains `read x; printf 'ack\r\n'`, and the test sends a line and waits for `ack` before the kill. By then the core has read and handled all of the flood's output, including the final checkpoint it queued for the holder. The assertion at `core.rs:773` (no truncation) is unchanged. The fix was validated with 0 truncations in 1,598 harness runs.

### `lock_is_exclusive`: retry the retake, bounded

After `drop`, the test retries `Lock::acquire` for up to 5 s. A child forked by another test holds the lock's open file description only until its own exec, which takes milliseconds. A retry-free fix would need every test that forks to avoid running while the lock is open, which is not achievable in one test binary. The product's locking is unchanged and correct. The fix was validated with 0 failures in 20,000 miniature runs, against 847 without it.

### `config_edit` and `cmdline` tests get unique, cleaned temp directories

The two `config_edit` tests stop sharing `temp_dir("malformed")`, each taking a name of its own. The `cmdline` tests remove the `/tmp/brindle-default-*` directories they create, with a drop guard so a failing assertion still cleans up.

### Windows: probes poll, and PowerShell 5.1 is warmed up

`windows-conpty-host`, `conpty_fallback` and `caption` run their `during` probe as a poll, every 100 ms, until the condition holds, Brindle exits, or `ctx.secs(dump_at)` passes. They then assert on the last snapshot, so a failure still reports what was seen.

`windows-pwsh-integration` runs a timed headless 5.1 warm-up before its GUI iteration and writes the duration to `powershell-warmup.txt` in the case directory.

The fixed dump time stays. With the debug-hook changes (next section), a failure's dump now shows `--- wait-for pending` or `--- wait-for timeout`, and the warm-up time shows whether the runner was cold. This is mitigation and diagnosis: the cause is not proven, and the design doesn't claim a cure.

### Debug hooks: the wait limit scales and the dump reports waits

`WAIT_FOR_LIMIT` becomes 30 × the step interval. The step runner records each `--wait-for` step's outcome in state the dump can read. The dump prints `--- wait-for timeout "TEXT"` and `--- wait-for pending "TEXT"` lines, as specified in `specs/command-line/spec.md`. Text is escaped as the dump already escapes selection text. The e2e harness's dump parser must accept the new lines. Add a self-test for the parser and an e2e case for the timeout and pending lines (`tests/e2e/cases/`, no shell dependency: `--wait-for` text that never appears).

## Risks / Trade-offs

- **The X server root cause is about 75–80% certain, not observed.**
  - Mitigation: a headless reproduction without Brindle comes first (many short-lived connections each sending one client message to a counting window). It must show losses without the round trip and none with it.
  - Then loop the case under load before and after the fix.
  - If the reproduction shows no loss, stop and revisit before changing the case.
- **Gating replays could delay attaches behind a large checkpoint on the same holder.** The gate waits only for bytes queued before the `SET_META`, which is usually nothing, and flushing holders first keeps it in one loop turn. If timing tests show attach latency regress, the alternative is to write `SET_META` ahead of queued chunks at the next frame boundary.
- **Read leases need the file on a local filesystem the kernel supports leases on.** `/tmp` is tmpfs or ext4 on CI and desktops; if the lease call fails with `EINVAL`, fall back to the bounded wait without a lease, and log it.
- **The PowerShell case may still fail.** It will now say why, and that evidence decides the next step. Raising the dump time would slow every passing run without proof it helps.
- **New flakes may surface once these are gone.** The verification loops in tasks.md run whole binaries in parallel, as CI does, not just the fixed tests, so other races in the same binaries show up too.
