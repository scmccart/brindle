# Proposal

## Why

The rest of the audit's finding 11 (2026-11-16, `docs/architecture/audits/2026-11-16.md`): tests that wait by the clock instead of for a condition, assert numbers that depend on machine speed, and pass when they never ran.
- **Fixed sleeps** (`testing R7`):
  - the real-process tests: `remote-agent/tests/core.rs` (b16, 14 entries), `holder.rs:1449` and `manage.rs:184` (b17), `upgrade.rs` (b18 #5);
  - the app's unit tests: `src/remote/local/{connection,session,relay}.rs` (b25 #20–#21, b26 #10, #11, #13), `src/terminal/pace.rs` (b28 #13);
  - the server's unit tests: `remote-agent/src/server/conn.rs` (b10 #15) and the Windows platform modules (b13 #7, b14 #3);
  - the e2e cases: `tests/e2e/cases/local_server.rs` (b33 #2–#8).
- **Timing asserts** (`testing R8`): b16 (6), b17 (`holder.rs:669`, `:1343`, `manage.rs:125`, `:129`, `relay.rs:166`), b18 #4, b25 #22–#23, b26 #12, b13 #8, b14 #5, b34 #4 and b35 #6.
- **No scaled limit:** `TIMEOUT` and every literal limit in the real-process and unit tests are fixed. Several e2e delays don't stretch with `BRINDLE_E2E_SLOW` (`testing R5`): `notify.rs:37`, `status.rs:38`, `:93`, `win.rs:495-502`, `:539-541` (b33 #1, b35 #2–#5), and `Run`'s `+ 30.0` deadline (b34 #3).
- **Temporary directories without a cleanup guard** (`testing R7`): b00 #12, b09 #10, b14 #4, b26 #9, b27 #25 and b28 #12.
- **Tests that skip and report a pass:**
  - the ConPTY `REPARENT` tests in `core.rs` and `holder.rs` (b16, b17);
  - the scopes test in `core.rs` (b16), which change 4 makes opt-in.
- **`testing R3`:** the perf runner has no `--shard` (b36 #2).
- **`testing R9` and `R10`:**
  - `tabs.rs`'s `face_top` was moved 1 px after a flake, with no numbers and no issue (b35 #7);
  - `session.rs`'s `Lock::acquire` retry names its cause but shows no numbers (b27 #24).

Nothing stops new sleeps either: `testing`'s Enforcement says no lint keeps `thread::sleep` out of tests. Each of these is a test that can fail on a loaded CI runner while the code is right, or pass while it is wrong.

## What Changes

- **One wait helper per test crate, with a scaled limit:**
  - `remote-agent/tests/support/wait.rs`;
  - `tests/e2e/wait.rs` (a `Scale` value taken from `Ctx`);
  - a `#[cfg(test)]` `test_support` module in `brindle`, `brindle-remote-agent` and `brindle-remote-protocol`.

  Each one polls a condition within a limit. The real-process and unit tests stretch that limit by a new `BRINDLE_TEST_SLOW` factor, and e2e keeps `BRINDLE_E2E_SLOW`. Throttles that are the test's stimulus, quiet windows and the documented `ETXTBSY` retry live in the same module under their own names.
- **Every fixed sleep and timing assert cited above is replaced.** Each one waits for the condition the test needs, or asserts the rule instead of a number:
  - a poll for an event;
  - a positive end marker for a "nothing happens" check: the core's own log line for its decision, a sentinel session whose end proves a deadline passed, or a round trip that orders everything before it;
  - a recorded order of events instead of an elapsed time;
  - an injected clock (`pace::drain`).
- **e2e delays scale:**
  - the shell-side `sleep 3`s wait for a file the case creates from the next tab;
  - every deadline goes through `Scale`;
  - `Ctx::sleep` goes;
  - the wall-clock bounds in `win.rs` go;
  - `palette-slow-host` takes its dump at a fixed, unscaled offset after the palette's step, since `LIST_LIMIT` doesn't stretch.
- **Temporary directories get guards** (`TempDir` with `Drop`) in every file cited above.
- **Skips are reported:**
  - `skip(Capability, reason)` writes the test's name to `$CARGO_TARGET_TMPDIR/skipped.log` and stderr;
  - `BRINDLE_TEST_REQUIRE` turns a named capability's skip into a failure;
  - CI prints the log and requires the packaged ConPTY on Windows.
- **Perf gets `--shard K/N`.** The parser moves to `tests/common/` and is shared with e2e, and change 7's runner check requires it.
- **The two `testing R9` items get numbers:**
  - `tabs-strip-colors`' `face_top` is looped headless with the band back on the edge, then fixed at its cause, or filed as an issue with the numbers;
  - `lock_is_exclusive`'s retry gets its before and after counts.
- **Two new checks in `tests/architecture.rs`, each with its docs in the same commit:**
  - no `std::thread::sleep` in `remote-agent/tests/**` outside `support/wait.rs`, and none in `tests/e2e/cases/**`;
  - no `std::env::set_var` or `remove_var` in test code, which change 4 left to this change.

  `testing`'s R7, R5 and Enforcement, and `ports`' Enforcement, say so.
- No change to Brindle's, the server's or the relay's behaviour. The production code that changes:
  - `pace::drain` gains a clock-taking twin;
  - `Connection`'s idle loop records when it last looked, which decides nothing.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Only tests, test harnesses, CI and docs change, plus two production seams that take a clock or record a time for tests. `.openspec.yaml` sets `skip_specs: true`.

## Impact

- **Real-process tests:**
  - `remote-agent/tests/{core,holder,manage,relay,upgrade}.rs`;
  - `remote-agent/tests/support/{mod,unix,windows,relay}.rs` and the new `support/wait.rs`.
- **Unit tests and their helpers:**
  - `src/test_support.rs` becomes `src/test_support/` (all platforms), plus new `remote-agent/src/test_support.rs` and `remote-protocol/src/test_support.rs`;
  - the tests in `src/remote/local/{connection,session,relay}.rs`, `src/remote/mod.rs`, `src/session.rs`, `src/config_edit.rs`, `src/terminal/{links,pace}.rs`;
  - the tests in `remote-agent/src/server/conn.rs`, `remote-agent/src/conpty/api.rs`, `remote-agent/src/server/platform/windows/{conn,event_loop,daemon,listen,lock,pipe}.rs`, and `remote-protocol/src/win.rs`.
- **Production code, no behaviour change:**
  - `src/terminal/pace.rs` gets `drain_with(…, now)`;
  - `src/remote/local/connection.rs` adds `State.last_look`.
- **Harnesses:**
  - `tests/e2e/{main,brindle}.rs`, the new `tests/e2e/wait.rs`;
  - `tests/e2e/cases/{local_server,remote,notify,status,win,tabs,multi_window,restore,instance}.rs`;
  - `tests/perf/main.rs` and `tests/common/mod.rs`.
- **Checks, CI and docs:**
  - `tests/architecture.rs`;
  - `.github/workflows/{ci,build-windows}.yml`;
  - `docs/architecture/testing-architecture.md`, `docs/architecture/ports-and-adapters.md`, `.claude/rules/tests.md` and `CLAUDE.md`.
- **Order and overlap with the remediation plan's changes:**
  - **Change 4 (`isolate-test-runs`)** must land first. It moves the stop step and `winproc` to `tests/common/` and removes `run_brindle`, which takes b34 #3's `remote.rs:1978` with it; its equivalent here is `Run`'s and `win.rs`'s `+ 30.0`. It makes the scopes test opt-in, and this change turns that test's skip into a reported one. It closes b27 #23, so this change keeps only b27 #25.
  - **Changes 5 (`enforce-platform-seams`) and 7 (`keep-pure-modules-pure`)** must land first too. The new checks use change 5's test-item scanner and change 7's import-following matcher. Change 7's `runner_gaps` gains `--shard` here, as its design says.
  - **Change 10 (`test-platform-logic-everywhere`)** moves decisions out of the Windows `pipe.rs`, `listen.rs` and `daemon.rs`. The tests this change edits stay in those seams, so whichever lands second rebases.
  - **Changes 2, 6 and 9:**
    - 2 (`carry-session-identity-and-host`) and 9 (`bound-queues-and-contain-panics`) add tests to `connection.rs` and `remote-agent/tests/holder.rs`, which must use the new helper;
    - 6 (`add-clippy-to-ci`) edits `tests/e2e/cases/{remote,tabs}.rs` and `holder.rs` for lints.

    Whichever lands second rebases.
  - **Change 1 (`keep-session-file-on-serialize-error`)** adds a test to `src/session.rs`, whose temporary directories this change puts behind guards.
  - **Change 13 (`extract-decisions-into-models`, not yet proposed)** plans a pure `idle_step`. Once that lands, the idle-loop tests here can become pure tests of it.
- **Changes in flight:**
  - `add-blocks` doesn't touch these tests;
  - `port-to-macos` will inherit the helpers;
  - `add-pocket-client` and `update-dependencies` don't overlap.
