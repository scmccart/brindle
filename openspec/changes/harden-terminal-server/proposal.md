# Proposal

**Status: outline only.** This collects the follow-ups that `fix-flaky-tests` found but left out of scope. Each item still needs its own decision before specs, design or tasks are written. Items may split out into their own changes, and #45 most likely will. It builds on `fix-flaky-tests`, which needs to merge first: `pass_fd_as`, `TestClient::hung_up` and the holder's read before losing a core come from that change.

## Why

Root-causing the flaky tests (#32, #36) turned up a few defects and hazards that weren't flakes:
- Session programs can inherit an ignored SIGHUP, so closing a tab doesn't hang them up.
- The unit tests leak a process-wide `XDG_RUNTIME_DIR` into tests that start real systemd scopes.
- There are three narrow races in the hang-up and server-start paths.

None fails today in normal use. They are cheap to fix while the analysis is fresh.

## What Changes

1. **Session programs start with default signal handling (#44, bug).**
   - Today `remote-agent/src/pty.rs` execs the program with Brindle's or the holder's dispositions and signal mask. Under `nohup`, an ignored SIGHUP reaches every session, and a hang-up then relies on the holder's SIGKILL 1 s later.
   - Reset the job-control and termination signals to `SIG_DFL` and clear the mask in the child before exec. This applies to the holder and to the stdio `--server` agent, which share `pty.rs`.
   - Add a holder test that runs with SIGHUP ignored.
2. **Unit tests don't change the process environment (#43, bug).**
   - `src/instance.rs`'s test sets `XDG_RUNTIME_DIR` with `set_var`. The `remote::local` tests read it through `scope::current()`, so depending on test order they create real `brindle-serve-*.scope` units on a desktop.
   - Pass the runtime directory in, or have the `remote::local` tests set `BRINDLE_NO_SCOPE=1`.
   - Audit the other `set_var`/`remove_var` calls in tests for the same leak.
3. **Hang-up signals before it closes (#46, item 1).**
   - The holder closes the PTY master before it sends SIGHUP, so a program blocked reading the terminal exits 1 on EIO about once in 4,000 hang-ups, instead of dying of SIGHUP.
   - Signalling the process group first gave a SIGHUP death in 24,000 of 24,000 tries.
   - Open question: does a consistent exit status matter enough (`EXIT` reporting, notifications) to change the order?
4. **A holder drains a dead core fully (#46, item 2).**
   - On a failed write to its core, the holder reads once, up to 1 MiB, before dropping the core.
   - Read until `WouldBlock`, bounded, so a frame like `SET_META` can't be lost behind a larger socket buffer.
5. **The ready pipe can't be mistaken for an exec error (#46, item 3).**
   - With `--ready-fd 3`, the ready pipe's `dup2` can replace `Command`'s exec-error pipe, and a failed exec's error bytes would read as "ready".
   - Check the ready byte's value, or choose the fd number after the spawn's own pipes.
6. **GPUI window fixes (#45, dependencies).**
   - zed-industries/zed#62081: a new window can stay blank until something else wakes the event loop.
   - zed-industries/zed#63089: a panic on a window-manager close, which `src/main.rs` currently avoids by deferring its quit.
   - Both arrive only with a gpui upgrade past crates.io 0.2.2. That upgrade is its own change, given the `Cargo.lock` constraints. It is listed here so the upgrade's checklist covers both fixes. It is not work for this change.

## Open questions

- Should items 3–5 go in at all, given that no failure is known? Item 1 and item 2 are real bugs.
- Should "programs start with default signal handling" and "a hang-up ends programs by SIGHUP" become spec requirements, or stay implementation detail?
- Which signals to reset: SIGHUP, SIGINT, SIGQUIT, SIGPIPE, SIGTERM, SIGCHLD, SIGTSTP, SIGTTIN, SIGTTOU, or every signal, as `posix_spawn` with `POSIX_SPAWN_SETSIGDEF` would?
- Whether item 6 gets a placeholder change (`upgrade-gpui`) now, or waits for a gpui release worth taking.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `terminal-server`: likely "Sessions" or "Close policies". Session programs start with default signal dispositions, and a hang-up ends them with SIGHUP even when Brindle or the server was started with it ignored. To be confirmed by the open question above.
- `remote-connections`: possibly the same guarantee for the stdio helper's sessions.

Items 2, 4 and 5 are test or internal robustness work with no requirement change.

## Impact

- `remote-agent/src/pty.rs`: the child's signal reset (holder and stdio agent).
- `remote-agent/src/server/holder.rs`: the hang-up order, and draining before `lose_core`.
- `remote-agent/src/server/daemon.rs`, `src/remote/local.rs` (`start_core`, `pass_fd_as`), `remote-agent/tests/support/mod.rs`: the ready-fd handling.
- `src/instance.rs` tests, and the `remote::local` tests.
- Tests: `remote-agent/tests/holder.rs` gains an ignored-SIGHUP case. Existing tests that accept either ending of a hung-up `cat` (`TestClient::hung_up`) could tighten if item 3 lands.
- Issues: closes #43 and #44; #46 when its items are decided; #45 stays open for the gpui upgrade.
