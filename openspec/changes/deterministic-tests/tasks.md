# Tasks

Prerequisites: changes 4 (`isolate-test-runs`), 5 (`enforce-platform-seams`) and 7 (`keep-pure-modules-pure`) have landed on `main`, as the plan's order has them. Sites are cited as on `ab6462b`, so re-find each one by its code after those changes' edits. "Scaled limit" means `limit(…)` from the crate's wait module (design §1).

## 1. Pure code and the wait helpers (shared by every later group)

- [ ] 1.1 `src/terminal/pace.rs` (b28 #13, design §4):
  - add `pub fn drain_with<S: Stream + Unpin>(rx: &mut S, first: S::Item, handle: impl FnMut(S::Item), now: impl FnMut() -> Instant)`, and make `drain` call it with `Instant::now`;
  - rewrite `drain_stops_at_the_budget` on a test clock that advances `BUDGET / 2 + MS` per handled item, with no `std::thread::sleep`, keeping its order-of-events assertions.

  Verify: `cargo test --locked -p brindle terminal::pace` passes, and `rg -n 'thread::sleep' src/terminal/pace.rs` finds nothing.
- [ ] 1.2 `brindle`'s test support (design §1):
  - turn `src/test_support.rs` into `src/test_support/mod.rs`, declared `#[cfg(test)] mod test_support;` in `src/main.rs` on every platform;
  - move today's `write_executable` and its lease code into `exec.rs` under `#[cfg(unix)]`;
  - add `wait.rs`: `limit`, `eventually`, `until` and `Quiet`, with the factor `BRINDLE_TEST_SLOW` read once and a panic naming the variable on a bad value;
  - add `dir.rs`: `TempDir::new(test_name)`, a fresh `temp_dir()/brindle-<name>-<pid>-<n>` removed on `Drop`.
  - Give `Quiet` and `limit` unit tests: a value unchanged for the window, a change resetting it, `None` resetting it, and factor parsing as a pure `parse_factor(Option<&str>) -> Result<f64, String>`.

  Verify: `cargo test --locked -p brindle test_support` passes on Linux; the `write_executable` callers still build (`cargo test --locked -p brindle --no-run`).
- [ ] 1.3 Add `remote-agent/src/test_support.rs` (`#[cfg(test)] mod test_support;` in `remote-agent/src/main.rs`) with the same wait items and `TempDir`, and `remote-protocol/src/test_support.rs` (`#[cfg(test)]` in `lib.rs`) with the wait items, each with the 1.2 unit tests. Verify: `cargo test --locked -p brindle-remote-agent test_support` and `cargo test --locked -p brindle-remote-protocol test_support` pass; `cargo test --locked -p brindle --test architecture` passes, so no dependency was added.
- [ ] 1.4 Add `remote-agent/tests/support/wait.rs`, re-exported by `support/mod.rs`:
  - `limit`, `eventually` (moved from `support/mod.rs:470`, same signature, scaled inside), `until` and `Quiet`;
  - `throttle(pause)`, documented as a stimulus and not scaled;
  - `retry_busy_exec(spawn)`, the bounded `ETXTBSY` retry with its cause written beside it (design §1);
  - `skip(Capability, reason)` with `enum Capability { Scopes, PackagedConpty }` (design §5), appending to `env!("CARGO_TARGET_TMPDIR")/skipped.log`, writing stderr, and panicking when `BRINDLE_TEST_REQUIRE` names the capability.
  - Move `support/unix.rs:142` and `support/relay.rs:67` onto `retry_busy_exec`.
  - Rebuild `stays_blocked` (`support/unix.rs:305-325`) on `Quiet` over `context_switches`, polled by `eventually`.
  - Add a Windows `stays_blocked(pid)` to `support/windows.rs`: `Quiet` over `GetProcessIoCounters`' `WriteTransferCount`.
  - Move `attach_until` (`support/mod.rs:977`) and `Relay::exits_within` (`support/relay.rs:133`) onto `until`.
  - Verify: `cargo test --locked -p brindle-remote-agent --test holder --test core --test relay --test manage --test upgrade` passes on Linux, and `rg -n 'thread::sleep' remote-agent/tests` lists only `support/wait.rs`.
- [ ] 1.5 Add `tests/e2e/wait.rs` (`mod wait;` in `tests/e2e/main.rs`):
  - `Scale` (`Copy`): `secs`, `poll` (moved from `remote.rs:1366`), `eventually` and `observe_for`;
  - `Ctx::scale()`; remove `Ctx::sleep`.
  - Add `Run::dump_after_last_step(secs)` to `tests/e2e/brindle.rs`, with a doc saying the offset is unscaled and must name the app limit it is measured against.
  - Add self-test entries for `Scale::secs` and `dump_after_last_step`'s computed dump time.
  - Verify: `BRINDLE_E2E=1 cargo test --locked -p brindle --test e2e -- --self-test` passes (no window opens).

## 2. Unit tests in the shared crates and the server (`remote-protocol`, `remote-agent/src`)

- [ ] 2.1 `remote-protocol/src/win.rs:955`: the prune poll uses `test_support::eventually` with `limit(20 s)`. Verify: Windows CI's `cargo test` (its first run, `testing R17`).
- [ ] 2.2 `remote-agent/src/server/conn.rs:444` (b10 #15): delete the sleep. Verify: `cargo test --locked -p brindle-remote-agent server::conn` passes.
- [ ] 2.3 `remote-agent/src/conpty/api.rs:215-223` (b09 #10): `scratch()` returns a `TempDir`. Verify: `cargo test --locked -p brindle-remote-agent conpty::api`, and that no `brindle-remote-conpty-*` directory is left in the temp directory.
- [ ] 2.4 `remote-agent/src/server/platform/windows/conn.rs` (b13 #7, #8, design §4):
  - `:428`'s pacing becomes a per-chunk gate the reader opens;
  - `:508` and `:609`'s delayed writers wait on a channel the reader sends just before it blocks;
  - `:496` and `:602` assert the read's result instead of `elapsed`;
  - `:618`'s `< 5 s` goes.

  Verify: Windows CI's `cargo test` (first run).
- [ ] 2.5 `…/windows/event_loop.rs` (b13 #7, #8): `:675` hands over through a channel before the wake; `:536` and `:680` assert the returned events instead of `elapsed`. Verify: Windows CI.
- [ ] 2.6 `…/windows/daemon.rs` (b13 #8):
  - `:966`, `:1073` and `:1088` assert survival after the starter's exit, without a window;
  - `:988` asserts the timeout error;
  - `eventually` (`:702`) is replaced by `crate::test_support`'s.

  Verify: Windows CI.
- [ ] 2.7 `…/windows/listen.rs` (b14 #3): `:856` waits, through `eventually`, until the extra connects are refused with `ERROR_PIPE_BUSY`; `:955` and `:986` use a reader gate. Verify: Windows CI.
- [ ] 2.8 `…/windows/lock.rs` (b14 #4): `TempDir`, and a child guard that kills the 120 s child on drop. Verify: Windows CI, and a deliberately failed assertion in a scratch run leaves no `brindle-lock-*` directory (don't commit it).
- [ ] 2.9 `…/windows/pipe.rs:1475` (b14 #5): drop the `< 20 s` assertion. Verify: Windows CI.

## 3. The app's unit tests (`src/`)

- [ ] 3.1 `src/remote/local/connection.rs`, production side: add `last_look: Option<Instant>` to `State`, set by `end_when_unused` on each look, documented as an observation for tests that decides nothing. Verify: `cargo build --locked` has no warnings.
- [ ] 3.2 `connection.rs`'s tests (b25 #20–#23, design §4):
  - replace the local `eventually` with `crate::test_support::wait`;
  - add `settled(&far)` (a `LIST` round trip, then an empty `try_recv`) and convert `quiet` and every `recv_timeout(300 ms).is_err()` site (`:2448, 2506, 2520, 2533, 2587-2588, 2647, 2651, 2687-2689, 2705, 2729, 2737, 2817, 2840, 2886, 2916, 2943, 3033, 3038, 3199, 3237-3238, 3273, 3311, 3351`), then delete `quiet`;
  - `:2463`: drop the sleep;
  - `:2867, 2938, 2980, 3339`: wait until `last_look >= t0 + idle`, then assert alive;
  - `:3431`: assert the gate's `DETACH` time precedes `flush`'s return time;
  - `:2381`: `>= 1` heartbeat, with `timings.dead >= timings.heartbeat * 2` asserted;
  - `:3445`: a far deadline on a background thread, `eventually(is_finished)` and `false`.

  Verify: `cargo test --locked -p brindle remote::local::connection` passes, and `rg -n 'thread::sleep|from_millis\(300\)' src/remote/local/connection.rs` finds nothing in its tests.
- [ ] 3.3 `src/remote/local/session.rs` (b26 #9–#13):
  - `temp_dir` becomes `TempDir`;
  - `:716-718` and `:750-752` use `eventually`;
  - `:852` holds the fake core's gate closed past `start_with` and drops the `elapsed` bound;
  - `:961` waits for the request in the connection;
  - the 300 ms negatives (`:491, 589, 687, 727, 766, 830, 971, 972, 1031`) end on a later in-order message;
  - the local `eventually` (`:923`) goes.

  Verify: `cargo test --locked -p brindle remote::local::session` passes, with no `thread::sleep` or 300 ms window left in its tests.
- [ ] 3.4 `src/remote/local/relay.rs:701` (b26 #13): drop the 300 ms "still running" window; the ordered `Ok(0)`/`BrokenPipe` after `wire.end()` is the check. Verify: `cargo test --locked -p brindle remote::local::relay`.
- [ ] 3.5 Temporary directories behind `TempDir`:
  - `src/terminal/links.rs:293-329` (b28 #12), both twins;
  - `src/config_edit.rs:749-814` (b00 #12);
  - `src/remote/mod.rs:272-366` and `src/session.rs:737-741, 790, 817, 891, 1179, 1205` (b27 #25), each created fresh.
  - Rebase over change 1's new `session.rs` test if it landed.
  - Verify: `cargo test --locked -p brindle` passes, and after it `ls $(dirname $(mktemp -u))` shows no `brindle-opener-test-*`, `brindle-config-edit-*` or `brindle-session-*` directory from this run.
- [ ] 3.6 `lock_is_exclusive` (b27 #24, design §7):
  - build the tests (`cargo test -p brindle --locked --no-run`);
  - run the unit-test binary 200 times, 8 at a time (`seq 200 | xargs -P8 -I{} <binary> -q`), with the retry removed in a scratch edit, then 200 times with it;
  - write the counts into the comment beside the retry (or, at 0 without it, that the loop didn't reproduce it and why the retry stays).

  Verify: the comment states both counts, and the loop's commands are in the PR body.

## 4. Real-process tests (`remote-agent/tests/`)

- [ ] 4.1 `core_logged(dir, session, Decision)` in `support/mod.rs`: `enum Decision { Detached, Lingers(u32), OwnerGone, LingerOver }`, matched without `_ =>`, waiting through `eventually` for the core's own line. Also `left(dir, count)` for `client N left (` lines. Verify: compiled by 4.2.
- [ ] 4.2 `core.rs`, the policy and grace tests (design §3): `:607`, `:638`, `:1153` (log markers); `:665` and `:1322` (the `never` sentinel); `:1424` (drop the sleep); `:899-905` (`until`). Verify: `cargo test --locked -p brindle-remote-agent --test core` passes; the two grace tests finish once the sentinel's line appears.
- [ ] 4.3 `core.rs`, the linger and listing tests:
  - `:1446` and `:1453` with a 10 s linger and a lingering sentinel;
  - `:1473-1483` with the `Lingers(4)` marker, the kept lower bound and the line order;
  - `:1506-1512` with `until(detached_secs >= 2)` and the `since_drop` bracket;
  - `:1128` with the `before ≤ started ≤ after` bracket.

  Verify: `cargo test --locked -p brindle-remote-agent --test core linger detached keys` passes.
- [ ] 4.4 `core.rs`, the flood and process tests:
  - `:433` becomes `throttle`;
  - `:519-527` becomes `stays_blocked`;
  - `:537-541` drops the round bound;
  - `:1558`'s draining thread signals after 64 KiB;
  - `:1640` drops the 3 s bound;
  - `:1669-1673` gets the `go` file, the sentinel session and the `LIST` round trip.

  Verify: `cargo test --locked -p brindle-remote-agent --test core flood stalled foreground idle` passes.
- [ ] 4.5 `core.rs`'s skips:
  - `:1739-1744` calls `skip(Capability::Scopes, reason)`, keeping change 4's `BRINDLE_SCOPE_TEST` opt-in, whose own skip line also goes through `skip`;
  - `:2221-2224` calls `skip(Capability::PackagedConpty, …)`.

  Verify: `cargo test --locked -p brindle-remote-agent --test core holders_and_restarted` passes and leaves one `scopes` line in `target/tmp/skipped.log`; with `BRINDLE_TEST_REQUIRE=scopes` and no opt-in, it fails naming the reason.
- [ ] 4.6 `core.rs`'s Windows module: `:1932` uses Windows `stays_blocked`, `:1939` drops the round bound, `:2080-2086` uses `until`, and `:2260` uses `until` with `pump`. Verify: Windows CI (first run, `testing R17`).
- [ ] 4.7 `holder.rs`:
  - `:400` becomes `throttle`;
  - `:669` and `:1343` drop their elapsed bounds;
  - `:1449` uses Windows `stays_blocked(pid)`;
  - `:1492` calls `skip(PackagedConpty)`;
  - `:1516` uses `until` with `pump`.

  Verify: `cargo test --locked -p brindle-remote-agent --test holder` on Linux, and Windows CI for the Windows module.
- [ ] 4.8 `manage.rs`: `duration_column` with a small `#[test]` of its own (`3s`, `2m5s`, `1h`, `-`, `x` rejected); `:125` and `:129` assert through it; `:184` uses `until`. Verify: `cargo test --locked -p brindle-remote-agent --test manage` passes.
- [ ] 4.9 `relay.rs`: `:166` drops the upper bound; `:221-237` becomes one `eventually` sending heartbeats and feeding `Quiet`. Verify: `cargo test --locked -p brindle-remote-agent --test relay` passes.
- [ ] 4.10 `upgrade.rs:366-381` (b18 #4, #5): `eventually(TIMEOUT, …)` replaces the literal 20 s, the elapsed bound goes, and the test asserts the holder's `the core it started is listening` line. Verify: `cargo test --locked -p brindle-remote-agent --test upgrade` passes on Linux; Windows CI for the Windows twin.
- [ ] 4.11 In `.github/workflows/ci.yml` (`test` job, `:99`) and `build-windows.yml` (`:60`), remove `target/tmp/skipped.log` before `cargo test --locked --workspace` and print it after, under `if: always()`. Verify: the PR's job logs show the skip lines (`scopes` on Linux unless the runner can make scopes; none on Windows once the DLL is found).
- [ ] 4.12 Read 4.11's Windows log. If the `REPARENT` tests ran (no `packaged-conpty` line), set `BRINDLE_TEST_REQUIRE: packaged-conpty` on that step. If they skipped, find why the DLL isn't beside the test helper, and record it in the PR as open instead of requiring it. Verify: the next Windows run is green with the variable set, or the PR says why it isn't.

## 5. e2e cases and the perf runner (`tests/`)

- [ ] 5.1 Shell-side delays: `notify.rs:37` and `status.rs:93` wait for `<case dir>/go`, created by a `--send 'touch …/go\r'` step after `new_tab`; `status.rs:38` takes `-t` from `ctx.secs(3.0)`. Verify: `scripts/with-xvfb env BRINDLE_E2E=1 BRINDLE_E2E_SLOW=4 cargo test --locked -p brindle --test e2e -- notify status` passes; at factor 4 it failed before (b33 #1, b35 #2).
- [ ] 5.2 `local_server.rs` (b33 #2–#8):
  - `:577`, `:817`, `:1045`, `:1098`, `:1142` and `:1145` wait on the conditions in design §6;
  - `:1172` is deleted;
  - the loops at `:96, 1491, 1517, 1620, 1711` and the 5 s at `:1618` go through `Scale`;
  - `slow: f64` becomes `Scale`.

  Verify: `scripts/with-xvfb env BRINDLE_E2E=1 BRINDLE_E2E_SLOW=2 cargo test --locked -p brindle --test e2e -- local-server` passes.
- [ ] 5.3 `remote.rs`:
  - `poll` moves to `Scale::poll`;
  - the loops at `:821` (`observe_for`), `:1388`, `:1643` and `:1913` go through `Scale`;
  - `palette_slow_host` (b34 #4) pauses on the relay condition and drops the second `call_at`, asserts `ctx.secs(INTERVAL) < 25.0` or returns the skip, and dumps through `dump_after_last_step(0.7)` with `LIST_LIMIT` named beside it.

  Verify: with Docker and the musl helper, `scripts/with-xvfb env BRINDLE_E2E=1 BRINDLE_REMOTE_E2E_REQUIRED=1 BRINDLE_E2E_SLOW=3 cargo test --locked -p brindle --test e2e -- remote-palette-slow-host` passes (b34 #4's failing factor), as does the `remote` filter at the default factor.
- [ ] 5.4 `multi_window.rs:14` and `restore.rs:196`: in place of `ctx.sleep`, `Scale::poll` until the case's server lists the first run's sessions. `instance.rs:60` goes through `Scale`. Verify: `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --locked -p brindle --test e2e -- second restore-second instance` passes.
- [ ] 5.5 `win.rs`:
  - `:495-502` loops on `KeyAvailable` within a scaled limit;
  - `:539-541` derives the ping count from `ctx.secs(4.0)`;
  - `:765` and `:777` drop their bounds;
  - `:115`, `:360`, `:712` and the loops at `:122, 368, 376, 733` go through `Scale`.

  Verify: CI's advisory `windows-e2e` job (first run, `testing R17`); report it as CI's.
- [ ] 5.6 `tests/e2e/brindle.rs`: the `+ 30.0` at `:546` and the 10 s at `:227` go through `ctx.secs`. Verify: `BRINDLE_E2E=1 cargo test --locked -p brindle --test e2e -- --self-test` passes.
- [ ] 5.7 `tabs-strip-colors`' `face_top` (b35 #7, design §7):
  - loop the case 100 times under `scripts/with-xvfb` with the band on the edge (scratch edit), recording failures, each dump's `--- grid` line and the edge's device row;
  - if it fails, fix the cause in `Strip`, put the band back on the edge, rerun 100 times, and write "N in 100 before, 0 in 100 after" in the comment and the PR;
  - if not, file a GitHub issue (`gh issue create`) with the numbers and the hypothesis, cite `#NN` in `face_top`'s comment, and keep the 1 px band.

  Verify: the comment carries either the numbers or the issue number.
- [ ] 5.8 Perf `--shard` (b36 #2):
  - move `parse_shard`/`take_shard` and their self-test entries from `tests/e2e/main.rs` to `tests/common/mod.rs`;
  - give `tests/perf/main.rs` `--shard K/N`, round-robin after filters, with its usage line and module doc;
  - list the moved self-test in both runners.

  Verify: `BRINDLE_PERF=1 cargo test --locked -p brindle --test perf -- --self-test` and the e2e self-test pass; `BRINDLE_PERF=1 cargo test --locked -p brindle --test perf -- --list --shard 1/2` lists every other case.

## 6. Architecture checks, with their docs in the same commit

Write each planted test first and watch it fail (`testing R13`). Verify each task with `cargo test --locked -p brindle --test architecture`.

- [ ] 6.1 `test_waits_go_through_the_wait_helper`, `every_allowed_sleeper_exists` and `a_sleep_in_a_test_is_found` (design §8). They are written on change 7's matcher, over `remote-agent/tests/**` and `tests/e2e/cases/**`, with `remote-agent/tests/support/wait.rs` the one allowed file. In the same commit:
  - `testing-architecture.md`: R7 names `BRINDLE_TEST_SLOW` and `BRINDLE_E2E_SLOW`; R5 gains the unscaled-offset sentence; Enforcement gains the sentence in design §8, and the "No lint keeps `std::thread::sleep` …" bullet is narrowed to `#[cfg(test)]` modules in `src/`;
  - `.claude/rules/tests.md` names both factors and the check under "Enforced by".

  Verify also: re-adding `thread::sleep(Duration::from_millis(1));` to any case makes the check fail and name that line (don't commit it).
- [ ] 6.2 `tests_set_no_process_environment` and `a_set_var_in_a_test_is_found` (design §8). They scan `tests/**`, `remote-agent/tests/**` and the test lines of every crate's `src/`, using change 5's test-item scanner. In the same commit:
  - `ports-and-adapters.md`'s Enforcement names it for R10, and "R4, R6–R11 and R13 rely on review" drops R10;
  - `testing-architecture.md`'s Enforcement adds it under R7.

  Verify also: it passes on the tree as change 4 left it, which has no test `set_var`.
- [ ] 6.3 Change 7's `runner_gaps` requires `"--shard"`, and `a_runner_without_a_self_test_is_found` gains a runner without `--shard`. In the same commit, `testing-architecture.md`'s Enforcement says the runner check covers `--shard`. Verify also: removing `--shard` from perf in a scratch edit makes it fail.
- [ ] 6.4 `CLAUDE.md`:
  - the terminal-server "Tests" bullet names `BRINDLE_TEST_SLOW`, `BRINDLE_TEST_REQUIRE` and `target/tmp/skipped.log`;
  - "End-to-end suite" says waits go through `tests/e2e/wait.rs`'s `Scale`;
  - "Performance suite" lists `--shard`;
  - "The rules broken most often" says `tests/architecture.rs` now rejects a sleep in those trees.

  Verify by reading it against groups 1–5.

## 7. Integration checks

- [ ] 7.1 Run, with no new warnings:
  - `cargo test --workspace --locked` with default test threads;
  - `BRINDLE_TEST_SLOW=3 cargo test --workspace --locked`, which passes and takes no longer than about 1.5 times the first run, since only limits stretch;
  - `cargo build --release --locked`;
  - both harness self-tests.
- [ ] 7.2 Loop whole test binaries in parallel, as CI runs them (`testing R10`): build with `--no-run`, then run each of `brindle`'s unit binary and `remote-agent`'s `core`, `holder`, `relay`, `manage` and `upgrade` binaries 50 times, 4 at a time, on `main` and on this branch. Record failures per binary as "before → after" for the PR, 0 included.
- [ ] 7.3 Run `scripts/with-xvfb env BRINDLE_E2E=1 BRINDLE_E2E_SLOW=2 cargo test --locked -p brindle --test e2e` (headless, no desktop window), all cases. Every case passes or skips for a stated reason, and no `/tmp/brindle-e2e-*` directory is left.
- [ ] 7.4 On the PR, check CI:
  - `commits`, `test` and the title check are green;
  - `windows` is green, the first run of every Windows-only edit;
  - `skipped.log` is printed in both test jobs;
  - the e2e shards and `windows-e2e` are advisory: read any failure, and fix it if it comes from this change.
- [ ] 7.5 Manual checks: none, since nothing here needs real keys or a pointer. Write "Manual checks: none" in the PR body, with 3.6's, 5.7's and 7.2's numbers and every Windows result labelled as CI's (`testing R11`, `R16`).

## Workflow follow-up

- Land as one PR from a branch off `main`, titled `test: wait for conditions, not the clock, and report skips` (at most 93 characters). Put the archive commit (`docs(openspec): archive deterministic-tests`) on the same PR. There is no spec delta to sync (`skip_specs: true`).
- If 5.7 filed an issue, the PR body links it as the open flake.
