# Design

## Context

See proposal.md, "Why". This change closes the rest of the audit's finding 11 (F11), the part change 4 doesn't. The citations are `docs/architecture/audits/2026-11-16.md`, "Findings" 11 and "Design smells". "bNN #K" is item K under Violations in `docs/architecture/audits/2026-11-16/bNN.md`. b16 and b17 list their entries by `file:line` and are cited that way. The test code is unchanged since the audit (`ab6462b`). Every site below was opened again on this branch.

How things stand:
- **Real-process tests** (`remote-agent/tests/`):
  - `support/mod.rs:470` has `eventually(timeout, condition) -> bool`, polling every 20 ms, and `TIMEOUT` is a fixed 30 s (`:47`).
  - Other polls are written by hand with `thread::sleep`:
    - `attach_until` (`:977`);
    - `stays_blocked` (`support/unix.rs:322`, a quiet window over context switches);
    - `Relay::exits_within` (`support/relay.rs:133`);
    - the `ETXTBSY` spawn retries (`support/unix.rs:142`, `support/relay.rs:67`);
    - the `Throttled` reader (`holder.rs:400`).
- **The core logs each decision it takes about a session** (`remote-agent/src/server/core/sessions.rs`), and `TempDir::logs()` reads those lines:
  - `client N left (…) with K session(s)`, followed in the same handler by `client_gone`;
  - `session N: detached`;
  - `session N: its client left; it waits Ns for it`;
  - `session N: its owner didn't come back; hanging up`;
  - `session N: its client didn't come back in time; hanging up`;
  - `client N is back for K session(s)`.

  The holder logs `the core it started is listening`. These lines are the positive markers this design uses for "nothing happens" checks.
- **Unit tests** have their own polls:
  - `connection.rs:2281` and `session.rs:923` each define an `eventually`, with fixed 10 s limits;
  - `connection.rs:2240` `quiet` is a 300 ms window;
  - `remote-agent/src/server/platform/windows/daemon.rs:702` has a third `eventually`.

  `src/test_support.rs` (`#[cfg(all(test, unix))]`) holds only `write_executable`. Temporary directories are named by pid and test and removed by a trailing `remove_dir_all(...).ok()`, except where `RemoveOnDrop` (`src/terminal/cmdline.rs`) or `TempDir` (agent tests) guard them.
- **e2e:**
  - `Ctx::secs` scales by `BRINDLE_E2E_SLOW`, and `Ctx::sleep` sleeps a scaled time (`tests/e2e/main.rs:63-77`). `Run`'s timeline (`call_at`, steps, dump) is scaled.
  - The cases' waits are written in four ways: `remote.rs`'s `poll(slow, secs, what, probe)`, `local_server.rs`'s `Server::wait`, `instance.rs`'s `Launcher` loop, and literal loops in `win.rs` and `local_server.rs`. Some of these have unscaled limits (`local_server.rs:1618`, `win.rs:115`, `win.rs:712`, `brindle.rs:227`, `brindle.rs:546`, `win.rs:360`).
  - The harness already reports skips (`Outcome::Skip`).
- **Perf:** the runner (`tests/perf/main.rs:104-218`) has a variable, filters, `--list` and `--self-test`, but no `--shard`. e2e parses `--shard` in `tests/e2e/main.rs:102-120` (`parse_shard`, `take_shard`).
- **`tests/architecture.rs`:**
  - After change 5 it has a test-item scanner (`#[cfg(test)]` items and test-only module files).
  - After change 7 it has `production_lines(text)`, an import-following matcher (`use_paths`, `resolved_paths`, needles that match a resolved path) and `runner_gaps(suite, text)`, which checks the variable, `--list` and `--self-test`.
  - Change 4 left the `set_var`/`remove_var` search here (its design's Non-Goals), and change 7 left `--shard` here (its design §9).

## Goals / Non-Goals

**Goals:**
- **No fixed sleep stands in for a condition** in the cited tests (`testing R7`). Each wait polls a condition within a scaled limit. Each "nothing happens" check ends on a positive marker, such as a log line, a sentinel or a round trip, not on a window.
- **No assertion on a number that depends on timing** (`testing R8`). Bounds the code itself guarantees stay, such as "not before the linger ended".
- **Every e2e delay scales** (`testing R5`), except offsets measured against the app's own unscaled limits, which are named as such.
- **A skipped test is visible** in the run and in CI, and CI fails it where the capability must be present (`testing R11`, `R17`).
- **Perf meets `testing R3`'s runner shape.**
- **The two `testing R9` items have numbers** (`testing R10`).
- **The sleep and environment rules hold for code written later:** a check in `tests/architecture.rs`, its docs changed in the same commit.

**Non-Goals:**
- **The absence windows the audit lists only as design smells:**
  - `holder.rs:1477` `quiet_for(1 s)`;
  - `relay.rs:197` and `:288-290`, which expect `Recv::TimedOut`;
  - `relay.rs:201` (`heartbeats >= 3`) and `:241` (RSS);
  - `holder.rs:1051` (tail size);
  - `core.rs:1702`'s 3 s `process_within`.

  Each shows that nothing arrives on a link with no other traffic to order it against, and none was cited as a violation. They stay, listed in `design-smells.md`.
- **Smaller items the plan doesn't cite:**
  - `core.rs:2300-2309`'s unguarded `running` process (b16, `testing R4`);
  - `core.rs:762`'s debug-only fault injection under `--release` (b16 smell);
  - b17's `testing R17` gaps (`holder.rs:809`, `:908`).
- **Pure models for the idle loop and the core's tick** (`idle_step`, `end_grace`, `end_lingering` as functions of `now`). Change 13 plans them. The tests here use markers instead, and can become pure tests once the models exist.
- **The sleep check over `#[cfg(test)]` modules in `src/`.** Production code in the same files sleeps legitimately (`pace`, backoff), and the plan scopes the check to the two test trees. The unit-test sites fixed here are held by review, and the Enforcement sentence says so.
- **The shared stop step's 100 ms polls** in `tests/common/server.rs` (change 4). They are bounded, live outside both checked trees and serve perf too, which has no scale factor.
- **b34 #3 at `remote.rs:1978`.** Change 4 deletes that code (`run_brindle`). Its equivalents, `Run`'s and `win.rs`' `+ 30.0`, are in scope.

## Decisions

### 1. One wait module per test crate, scaled by one factor per suite family

Every crate's tests get the same three items, so a reader knows the shape anywhere.
- **`limit(base: Duration) -> Duration`:** `base` times the factor. The factor is read once into a `OnceLock<f64>`. A value that isn't a positive number panics, naming the variable.
- **`eventually(limit, condition) -> bool`:** polls every 20 ms within `limit(limit)`.
  - Its signature stays as it is today, so the existing `assert!(eventually(TIMEOUT, …), "…")` calls get the scaling without an edit.
  - The value-returning form is `until(what, limit, probe: FnMut() -> Option<T>) -> T`, which panics with `what` and the scaled limit.
- **`Quiet`:** a non-blocking observer (`Quiet::new(window)`, `observe(Option<u64>) -> bool`). It reports true once a sampled value has stayed the same for `window`, so a quiet window is polled through `eventually` and never sleeps on its own.

Where each lives:

| Tests | Module | Factor |
| --- | --- | --- |
| `remote-agent/tests/**` | `support/wait.rs`, re-exported by `support/mod.rs` | `BRINDLE_TEST_SLOW` |
| `brindle` unit tests | `src/test_support/wait.rs`; `src/test_support.rs` becomes the directory `src/test_support/` (`#[cfg(test)]` on every platform), with today's `write_executable` in `exec.rs` under `#[cfg(unix)]` and a `TempDir` guard in `dir.rs` | `BRINDLE_TEST_SLOW` |
| `brindle-remote-agent` unit tests | new `remote-agent/src/test_support.rs` (`#[cfg(test)] mod test_support;` in `main.rs`): the wait items and `TempDir` | `BRINDLE_TEST_SLOW` |
| `brindle-remote-protocol` unit tests | new `remote-protocol/src/test_support.rs` (`#[cfg(test)]` in `lib.rs`): the wait items | `BRINDLE_TEST_SLOW` |
| `tests/e2e/cases/**` | `tests/e2e/wait.rs`: `Scale` (below) | `BRINDLE_E2E_SLOW` |

**`remote-agent/tests/support/wait.rs` also holds the sleeps that are not waits,** each under its own name, so the check has one file to allow:
- **`throttle(pause)`:** the stimulus of a slow reader (`core.rs:433`, the `Throttled` reader in `holder.rs:400`). It is not scaled, because the test's subject is the throttle itself, not a wait.
- **`retry_busy_exec(spawn)`:** the bounded `ETXTBSY` retry from `support/unix.rs:142` and `support/relay.rs:67`. It keeps its cause beside it (another thread's fork holds the writer's fd until its exec), which is the proven OS transient `testing R9` allows. It is the same cause `src/test_support`'s `write_executable` documents.
- **The poll loop of `eventually` and `until`.**

**e2e's `Scale`** is a `Copy` value taken from `ctx.scale()`, so a `call_at` closure (`Send + 'static`, no `Ctx`) can carry it. Today's `slow: f64` fields in `local_server.rs` and `remote.rs` become `Scale`. It offers:
- `secs(f64)`;
- `poll(secs, what, probe) -> Result<T>`, today's `remote.rs` `poll`, moved;
- `eventually(secs, what, condition) -> Result`;
- `observe_for(secs, sample)`: a scaled sampling window, for the one case that watches for processes appearing (`remote.rs:815`);
- `Ctx::secs` stays, and `Ctx::sleep` is removed.

*Rejected:*
- **One shared test-support crate:** each crate below the app has a closed dependency list (`ports R3`), and a dev-dependency counts.
- **Scaling the real-process tests by `BRINDLE_E2E_SLOW`:** that variable is the e2e suite's, and CI sets it for the e2e shards only. A second name keeps "one factor" true inside each suite, and `testing R7` names it.
- **A larger fixed `TIMEOUT`:** that is "raising a timeout passing runs also wait for" (`testing R9`'s anti-pattern). Scaling changes only failing runs, and the limit is reached only when a test fails.

### 2. Three kinds of replacement

1. **A wait for an event** becomes `eventually` or `until` on that event. Unbounded loops become bounded.
2. **A "nothing happens" check ends on a positive marker** that shows the decision point has passed, then asserts the state. In order of preference:
   - **the decision's own log line**, where the code logs what it decided (the core's session lines; the holder's `the core it started is listening`);
   - **a sentinel:** a second subject that, unlike the first, must change once the deadline passes. Its change proves the code has evaluated that deadline. Examples: a session of an owner who never returns, whose hang-up shows `end_grace` ran; a lingering client that left after the subject did;
   - **a round trip** through the same ordered channel, so everything sent before it has been handled (a `LIST`, or a later message that `fake::link` sees in order).
3. **An elapsed-time assertion becomes the rule it stood for:**
   - the outcome value (`TimedOut`, `false`, an exit code);
   - an order of recorded events (as `drain_stops_at_the_budget` already does);
   - a bracket the test's own clock gives (`before ≤ started ≤ after`);
   - or nothing, when the surrounding bounded wait already proves it.

   Lower bounds the code guarantees (a lingering session is never hung up before its linger) stay: they hold on any machine.

### 3. The real-process tests, site by site

**`remote-agent/tests/core.rs`** (b16):

| Site | What it waits for or asserts | Replacement |
| --- | --- | --- |
| `:433` | stimulus: a slow client | `throttle(30 ms)` |
| `:519-527` (R8) | the flood's program is blocked in its write; ten 20 ms samples | `stays_blocked(flood_pid)`, rebuilt on `Quiet` over context switches: no switch at all in the window means the program never ran, which doesn't depend on polling speed |
| `:537-541`, `:1939` Windows (R8) | each echo round under 5 s | drop the bound; the round completing within the scaled limit is the rule (a core blocked by the stalled client never answers) |
| `:607` | the core's decision for the `detach` session after its client died | wait for `session {kept}: detached` in the log, then assert alive |
| `:638` | same, after `drop(client)` | `session {session}: detached` |
| `:665` | the grace period has ended (12 s `PAST_GRACE`) | sentinel: before the kill, a client with token `never` spawns a `kill` session; wait for `session {sentinel}: its owner didn't come back; hanging up` (`end_grace` handles every orphan in one pass), then assert `pid` alive |
| `:899-905`, `:2080-2086` Windows | the `DETACH` was read (unbounded loop that swallows refusals) | `until("the attach after DETACH", TIMEOUT, ‖ other.attach(..).ok())` |
| `:1128` (R8) | `started <= before + 60` | `before <= started <= after`, `after` read when `list` returns |
| `:1153` | the core handled the explicit-detach client's departure; nothing to decide | wait for the log's `left (` line, then assert alive |
| `:1322` | past the grace period after a restart | the `:665` sentinel |
| `:1424` | nothing more after a take-over | drop the sleep: `take_over` logs `which it replaces` and closes the stale connection in one handler, so after `first.closes()` nothing is pending; the `say` round trips show both sessions |
| `:1446`, `:1453` | the linger decision; then the linger's deadline passed | linger 10 s, so a reconnect on a loaded machine stays inside it; wait for `its client left; it waits 10s`, then assert alive; after the return, a sentinel lingering client that left after the subject never returns, and its `didn't come back in time` line shows the subject's old deadline has passed; then assert alive |
| `:1473-1483` (R8) | the plain session ends at once; the lingering one only after its linger | wait for `session {waits}: its client left; it waits 4s`, then `gone(waits_pid)`; keep `left.elapsed() >= 3 s` (a guaranteed lower bound); drop the racing `alive(waits_pid)`; check that the `didn't come back in time` line follows the `waits` line |
| `:1506`, `:1509`, `:1512` (R7, R8) | `detached_secs >= 2`; `< 60` | `until` a listing with `detached_secs >= 2`; replace `< 60` with `<= since_drop.elapsed().as_secs() + 1` |
| `:1558` | the flood is being written to this connection | the draining thread sends on a channel after its first 64 KiB, and the test waits on it within `limit(TIMEOUT)` |
| `:1640` (R8) | told within 3 s | drop it; told at all, with nothing printed, is the rule |
| `:1669-1673` (R8) | an idle session isn't looked at (relies on `sleep 3` inside 7 s) | the program waits for a file (`while [ ! -e go ]; do sleep 0.1; done; exec sleep 1000`); the test creates it and waits until `program_of(pid)` is `sleep`; then a sentinel interactive session gets input and runs `sleep`; once its `PROCESS` arrives (a look after the change), a `LIST` round trip, then no `PROCESS` for the idle session was queued |
| `:1739-1744` | skip when the user manager can't make scopes (made opt-in by change 4) | `skip(Capability::Scopes, reason)` (decision 5) |
| `:1932` Windows | the flood filled every queue (3 s) | Windows `stays_blocked(pid)`: `Quiet` over the program's `WriteTransferCount` (`GetProcessIoCounters`), the same item as Unix's in `support/windows.rs` |
| `:2221-2224` Windows | skip without the packaged ConPTY | `skip(Capability::PackagedConpty, …)` |
| `:2260` Windows | the console window's owner changed | `until`, with `Window::pump()` in the probe |

**The other real-process tests:**

| Site | What it waits for or asserts | Replacement |
| --- | --- | --- |
| `holder.rs:400` | stimulus | `throttle(self.pause)` |
| `holder.rs:669` (R8) | SIGHUP, not the later SIGKILL, ended it | drop the elapsed bound; the `killed by signal Some(SIGHUP)` count (`:676`) is the rule |
| `holder.rs:1343` Windows (R8) | the close, not the grace's `TerminateProcess`, ended it | drop the bound; `exit_code != TERMINATED` (`:1342`) is the rule |
| `holder.rs:1449` Windows | without a core, the program blocks once the tail is full | Windows `stays_blocked(pid)` |
| `holder.rs:1492` Windows | skip | `skip(Capability::PackagedConpty, …)` |
| `holder.rs:1516` Windows | the owner changed | `until` with `pump` |
| `manage.rs:125`, `:129` (R8) | age and detached time under a minute | a `duration_column(text) -> Option<u64>` parser (digits, then `s`/`m`/`h`/`d` parts) that the columns must satisfy; the attached session's detached column is `-` |
| `manage.rs:184` | `detached_secs` reached 1 | `until` `list` shows no `0s` and `list --json` has `detached_secs >= 1` |
| `relay.rs:166` (R8) | the silent client's relay ended, upper bound | drop it; `exits_within(IDLE + TIMEOUT)` bounds it, and `>= IDLE / 2` stays |
| `relay.rs:221-237` | the flood's program is blocked while heartbeats go on | one `eventually` whose probe sends a heartbeat when due and feeds `Quiet` with the program's context switches; done once `IDLE * 3` has passed and the program is quiet |
| `support/relay.rs:133` | the relay exited | `until` |
| `support/mod.rs:977` | the replay matched | `until`, with the detach between tries in the probe |
| `upgrade.rs:366-381` (R7, R8) | a core came back within 10 s | `eventually(TIMEOUT, …)` (scaled) in place of the literal 20 s; drop the elapsed bound; assert the holder's log has `the core it started is listening` (the rule: the holder's own core took over) |

### 4. Unit tests, site by site

| File | Sites | Replacement |
| --- | --- | --- |
| `src/remote/local/connection.rs` (b25 #20) | `:2463` | drop the sleep: the two requests have no link to finish on, and their success once the link comes (asserted later) shows they waited |
| | `:2867`, `:2938`, `:2980`, `:3339` (`idle * 3`, `* 5`) | `State` gains `last_look: Option<Instant>`, written by `end_when_unused` on each look; it decides nothing. The test marks `t0` when the connection becomes idle-eligible, waits until `last_look >= t0 + idle`, and asserts it isn't dead (a positive marker that the idle decision was taken past the limit) |
| | `:3431` (`200 ms`, then `!is_finished`) | an order: the gate records when the `DETACH` passed, the flushing thread records when `flush` returned, and the test asserts the first came before the second |
| (b25 #21) | `quiet` (`:2240`) and its 27 call sites | `settled(&far)`: a round trip through the fake link (the client sends `LIST` and waits for the fake's answer), then `rx.try_recv()` must be empty. `quiet` is removed |
| (b25 #22) | `:2381` `heartbeats >= 2` | `>= 1`, with `assert!(timings.dead >= timings.heartbeat * 2)` beside it: a heartbeat before the link is declared dead is the rule |
| (b25 #23) | `:3445` elapsed `< 1 s` | flush with a deadline an hour away, on a background thread; `eventually(limit(10 s), is_finished)` and `false` |
| | `eventually` (`:2281`) | the crate's `test_support::wait` |
| `src/remote/local/session.rs` (b26) | `:338` `temp_dir` (#9) | `test_support::TempDir` |
| | `:716-718`, `:750-752` (#11) | `eventually(… pid().is_some())` |
| | `:852` (#12) | the fake core's gate held closed until after the `start_with` returns, as `a_tab_closed_while_attaching…` does; `pid() == None` is the rule |
| | `:961` (#10) | wait until the connection has the request waiting (`waiting(&connection, 1)`, as in `connection.rs`) |
| | `:491, 589, 687, 727, 766, 830, 971, 972, 1031` (#13) | a barrier the test controls: a later message whose in-order arrival proves the earlier one wasn't sent, as `layouts_are_sent_once_per_change…` does with `DETACH` |
| `src/remote/local/relay.rs:701` (b26 #13) | still running | `link.wire` gets a probe that records the read as pending; the test ends the wire and then asserts the read ended, dropping the 300 ms "still running" window, which a later `Ok(0)` already orders |
| `src/terminal/pace.rs:143-150` (b28 #13) | the budget passes during the calls | `pub fn drain_with(rx, first, handle, now: impl FnMut() -> Instant)`, with `drain` calling it with `Instant::now`; the test's clock advances `BUDGET / 2 + MS` per call, and the same assertions hold without sleeping |
| `src/terminal/links.rs:293-329` (b28 #12) | pid-named directory, trailing removal | `TempDir` named by test |
| `src/config_edit.rs:749-814` (b00 #12) | same | `TempDir` |
| `src/remote/mod.rs:272-366`, `src/session.rs:737-741, 790, 817, 891, 1179, 1205` (b27 #25) | same, plus a stale directory from a recycled pid | `TempDir`, created fresh |
| `src/session.rs:817-836` (b27 #24) | the `Lock::acquire` retry | kept if decision 7's numbers show the transient; the numbers are written beside it |
| `remote-agent/src/server/conn.rs:444` (b10 #15) | "the sender is waiting by now" | drop it: the reader polls, and `flush_for` is right whichever starts first |
| `remote-agent/src/conpty/api.rs:215-223` (b09 #10) | no cleanup | `test_support::TempDir` |
| `…/platform/windows/conn.rs` (b13 #7, #8) | `:428` (1 ms pacing), `:508`, `:609` (a delayed writer), `:496`, `:602` (`elapsed >= 45/50 ms`), `:618` (`< 5 s`) | the writer waits on a channel the reader sends just before it blocks; the reader's result (`TimedOut`, or the bytes) is asserted, not its elapsed time; the pacing becomes a gate the reader opens per chunk |
| `…/windows/event_loop.rs` | `:675` (delayed wake), `:536` (`>= 25 ms`), `:680` (`< 20 s`) | a channel handshake before the wake; the wait's returned events (none, then the wake) are asserted; the upper bound goes |
| `…/windows/daemon.rs` | `:966`, `:1073`, `:1088` (`!ends_within`), `:988` (`>= READY_LIMIT`) | survival is asserted after the starter has exited (the event that could end it), with a process-handle query and no window; the timeout case asserts the returned error; `eventually` (`:702`) moves to `test_support` |
| `…/windows/listen.rs` (b14 #3) | `:856` | `eventually` until the waiting connects are refused with `ERROR_PIPE_BUSY` |
| | `:955`, `:986` | a reader gate (a channel per chunk) in place of wall-time pacing |
| `…/windows/lock.rs` (b14 #4) | `:68-73`, `:88`, `:94-102` | `TempDir`, and a child guard that kills the 120 s child on drop |
| `…/windows/pipe.rs:1475` (b14 #5) | `< 20 s` | drop it: `ev.wait(…, Some(30 s))` returning one writable event is the rule |
| `remote-protocol/src/win.rs:955` | poll with an unscaled 20 s limit | `test_support::eventually(limit(20 s), …)` |

The Windows modules first run on Windows CI (`testing R17`). Change 10 moves decisions out of `pipe.rs`, `listen.rs` and `daemon.rs`, but these tests stay in the seams.

### 5. Skips are recorded, and required where CI has the capability

`support::skip(capability: Capability, reason: &str)` in `remote-agent/tests/support/wait.rs`:
- **`enum Capability { Scopes, PackagedConpty }`**, closed and matched without `_ =>`. Each has a fixed name (`scopes`, `packaged-conpty`).
- **When `BRINDLE_TEST_REQUIRE`** (a comma-separated list of those names) **names the capability,** it panics with the reason: CI wanted it there.
- **Otherwise it records the skip:**
  - it appends `<test name>\t<capability>\t<reason>` to `$CARGO_TARGET_TMPDIR/skipped.log` (`env!`, set by Cargo for integration tests; one `write` per line, opened with `append`);
  - it writes the same line to stderr;
  - the test returns.
- The test name is `thread::current().name()`, which libtest sets.
- **CI:**
  - the test steps of `ci.yml`'s `test` job and `build-windows.yml` remove the log before `cargo test` and print it after (`if: always()`), so the job's log lists every skip;
  - Windows sets `BRINDLE_TEST_REQUIRE=packaged-conpty`, because `fetch-conpty.ps1` runs first, but only after task 6.3 has confirmed the DLL is beside the test helper there;
  - Linux leaves `scopes` unrequired: whether the runner has a delegating user manager is checked on the first run, and recorded.

*Rejected:*
- **`#[ignore]` with an opt-in:** `testing R3` names libtest's `#[ignore]` as the wrong tool. A capability check also has to run to know.
- **A panic always:** developers without ConPTY or a user manager would see red for their machine, not for the code.

### 6. e2e: every delay through `Scale`, shell-side delays gated on a condition

| Site | Today | Replacement |
| --- | --- | --- |
| `notify.rs:37` (b33 #1), `status.rs:93` (b35 #2) | the shell sleeps 3 s, hoping the second tab has opened | the first tab's command waits for `<case dir>/go` (`while [ ! -e …/go ]; do sleep 0.1; done; …`). After `new_tab`, a `--send 'touch …/go\r'` step runs in the new, active tab, so the report fires only once the second tab exists |
| `status.rs:38` (b35 #3) | `read -t 3` | `read -t {T}` with `T` from `ctx.secs(3.0)`, as `query.rs` does |
| `win.rs:495-502` (b35 #4) | `Start-Sleep -Milliseconds 500` | a loop on `[Console]::KeyAvailable` until the reply's terminator, within `ctx.secs(5.0)` written into the script |
| `win.rs:539-541` (b35 #5) | `ping -n 8` against a scaled dump | count from `ctx.secs(4.0) + 4` |
| `win.rs:765`, `:777` (b35 #6) | `took < ctx.secs(6) + 5` | dropped. A starved window gives no dump, and the run's own deadline fails it. Losing "slow but not frozen" detection is accepted: the bug the case guards was a freeze |
| `local_server.rs:577` (b33 #2) | 3 s for the state file | `scale.poll` on `state_dir`'s file content |
| `local_server.rs:817` (b33 #3) | 3 s of recorder ticks | `scale.poll` on `list --json` until a layout names both keys |
| `local_server.rs:1045`, `:1098` (b33 #4, #5) | let it draw | `scale.poll` on a marker file that the session's own command writes after `line-01` |
| `local_server.rs:1142` (b33 #6) | late output | `scale.poll` until the notification log is unchanged across two reads that bracket a `list` round trip of the core |
| `local_server.rs:1145` (b33 #8) | 3 s of "nothing" | wait for the replay's end marker (the restored status record in `list`'s layout or the dump), then compare the log |
| `local_server.rs:1172` (b33 #7) | pure padding | deleted (`srv.attached` and `wait_for("before-2")` already hold) |
| `remote.rs:959-960` (b34 #4) | dump at `INTERVAL * 2.5 + 0.7`, pause at `INTERVAL * 2.5 - 2.0` | see below |
| Unscaled limits: `local_server.rs:1618` (5 s), `win.rs:115` (2 s), `win.rs:712` (120 s), `brindle.rs:227` (10 s), `brindle.rs:546` and `win.rs:360` (`+ 30.0`) | | through `Scale` / `ctx.secs` |
| The hand-written poll loops: `local_server.rs:96, 1491, 1517, 1620, 1711`; `remote.rs:821, 1388, 1643, 1913`; `instance.rs:60`; `win.rs:122, 368, 376, 733` | `thread::sleep(100–200 ms)` loops | `scale.poll` / `eventually`; `remote.rs:815-821`'s ssh watch becomes `scale.observe_for` |
| `multi_window.rs:14`, `restore.rs:196` (`ctx.sleep`) | "give the first one time to start" | `scale.poll` until the case's server directory lists the first run's session(s). A listed session means the first Brindle has started, and it takes `session.lock` before GPUI starts |

**`palette-slow-host`** relates two times to app constants that don't stretch: the listing gives up after `LIST_LIMIT` (2 s), and a paused link dies after `Timings::dead` (30 s). Stretching those offsets breaks the case at `BRINDLE_E2E_SLOW` ≥ 3.
- The pause becomes a condition. The `call_at` that already polls for Brindle's relay in the container pauses it as soon as it sees the relay, and the second, computed `call_at` goes.
- The palette's step follows by one step interval. The case asserts `ctx.secs(INTERVAL) < 25.0`, under `Timings::dead`, and otherwise returns `Outcome::Skip("BRINDLE_E2E_SLOW stretches the palette step past the link's 30 s limit")`.
- The dump is taken through a new **`Run::dump_after_last_step(secs)`**. It sets the dump time to the last step's scaled time plus `secs`, unscaled. Brindle counts steps and dump from the same origin (the window's first appearance), so their gap is exactly `secs` on Brindle's own clock.
- `testing R5` gains: "An offset measured against one of the app's own unscaled limits is not a test delay: it is written unscaled, through a named helper, with the limit named beside it."

**Perf `--shard`** (b36 #2):
- `parse_shard` and `take_shard` move from `tests/e2e/main.rs` to `tests/common/mod.rs`, with their self-test entries, which both runners list.
- The perf runner takes `--shard K/N` round-robin after filters, as e2e does.
- Change 7's `runner_gaps` gains `"--shard"`, and its planted test gains a runner without it.

### 7. The two `testing R9` items get numbers

- **`face_top` in `tabs-strip-colors`** (b35 #7). It runs headless under `scripts/with-xvfb`, which opens no desktop window.
  - **Reproduce:** with the band moved back onto the face's top edge (`FACE_TOP + 0.0`, a scratch edit), loop the case 100 times. Record pass or fail, each dump's `grid origin`/`scale`, and the device row of the face's top edge found in the capture.
  - **If it fails:** the rows give the cause. The hypothesis to test first, labelled inferred (`testing R11`): `Strip::locate` derives the strip's top from the grid's logical origin, and `px()` rounds `(top + y) * scale` separately for each rectangle, so a fractional logical origin can put the edge one device row off. Fix it at the cause in `Strip` (derive device rows from one rounded device origin, or find the edge in the capture), put the band back on the edge, and report "N failures in 100 before, 0 in 100 after".
  - **If 100 runs show no failure:** file a GitHub issue with the numbers, the run conditions and the hypothesis, and cite it in `face_top`'s comment, which keeps the 1 px mitigation labelled as one.
- **`lock_is_exclusive`'s retry** (b27 #24).
  - Loop the `brindle` unit-test binary (200 runs, 8 in parallel, as change 4's task 2.1 does) with the retry removed in a scratch build, then with it.
  - Write "N in 200 without the retry, 0 in 200 with it" into its comment.
  - If the loop never reproduces the failure, keep the retry: its mechanism is the one `write_executable` and `retry_busy_exec` document. The comment then says the loop didn't reproduce it (`testing R11`).

### 8. The checks in `tests/architecture.rs`, with their docs

Both are pure scans over text, tested on planted input (`testing R1`, `R13`). They are written on change 7's matcher, so `use std::thread; … thread::sleep`, `use std::thread::sleep as nap; … nap(` and `std::thread::sleep` are all one needle, `std::thread::sleep`.

**`test_waits_go_through_the_wait_helper`** (`testing R7`):
- It reads every file under `remote-agent/tests/` and `tests/e2e/cases/`.
- It fails on any line whose resolved paths match `std::thread::sleep`, except in `remote-agent/tests/support/wait.rs`.
- The failure names the helper to use (`eventually`, `until`, `Quiet`, `throttle`, `retry_busy_exec`, or e2e's `Scale`).
- `every_allowed_sleeper_exists` keeps the allowed path real.
- `a_sleep_in_a_test_is_found` plants direct, imported, renamed and full-path sleeps, plus a `// thread::sleep` comment that must not count.

**`tests_set_no_process_environment`** (`ports R10`, `testing R7`):
- It reads every file under `tests/` and `remote-agent/tests/`, plus the test lines of every crate's `src/`. Those are the complement of change 7's `production_lines`, built on change 5's test-item scanner.
- It fails on `std::env::set_var` or `std::env::remove_var`.
- Production lines are out of scope, so `src/main.rs:305` (`parse_args_from`, a b01 `fcis R9` SHOULD) isn't caught. Change 12 or 13 owns that.
- `a_set_var_in_a_test_is_found` plants each form inside a `#[cfg(test)] mod tests`, and one in production code that must not count.

**Docs in the same commit:**
- `testing-architecture.md`:
  - R7 names the factor ("within a limit scaled by `BRINDLE_TEST_SLOW`; e2e by `BRINDLE_E2E_SLOW`");
  - R5 gains the unscaled-offset sentence (decision 6);
  - Enforcement gains: "`tests/architecture.rs`: `test_waits_go_through_the_wait_helper` (R7: no `std::thread::sleep` in `remote-agent/tests/**` outside `support/wait.rs`, the one module that sleeps, scaled, and none in `tests/e2e/cases/**`, whose waits go through `Scale`), and `tests_set_no_process_environment` (R7, `ports R10`). A sleep in a `#[cfg(test)]` module of `src/` is held by review."
  - Its last Enforcement bullet's "No lint keeps `std::thread::sleep` … out of unit tests" is narrowed to match.
- `ports-and-adapters.md`'s Enforcement names `tests_set_no_process_environment` for R10.
- `.claude/rules/tests.md` names both factors and the check.
- `CLAUDE.md`'s e2e and server-test paragraphs name `BRINDLE_TEST_SLOW`, `BRINDLE_TEST_REQUIRE` and `skipped.log`.

### Layers, crates and rules

- **Test code only,** apart from two production seams:
  - **`pace::drain_with`:** the clock becomes an argument, so the decision is testable without sleeping (`fcis R2`, `R8`).
  - **`State.last_look`:** a recorded observation in the connection's shell. It decides nothing (`fcis R1`), and change 13's `idle_step` would make it unneeded.
- **Crates:**
  - No crate reference changes (`ports R1`–`R3`), so `test_support` modules are per crate.
  - `cfg(unix)`/`cfg(windows)` in test-support code is test code, outside `ports R4` (decision D1, change 5).
- **No wire, config or state-file change,** so `process R4`–`R6` and `R15` don't apply. A core restart, a Brindle crash and an upgrade behave as before.
- **Types:** `Capability` is a closed enum (`types R1`, `railway R3`). A malformed `BRINDLE_TEST_SLOW` or `BRINDLE_TEST_REQUIRE` is a test-setup error, so it panics with the variable's name.
- **Rules satisfied:** `testing R3`, `R5`, `R7`, `R8`, `R9`, `R10`, `R11`, `R13`, `R17`; `ports R10`.

## Risks / Trade-offs

- **[Log lines become test contracts]** A reworded core or holder line breaks a test, not a user. → Each marker is matched in one support helper (`core_logged(dir, session, Decision)`), so a rewording touches one place, and the core's lines are already asserted in tests today.
- **[Sentinels lengthen some tests]** The linger and grace tests now wait out the linger or grace once (10 s and 10 s). They waited 5 s and 12 s before. → Accepted. The wait is the property under test, and it ends on the sentinel's line, not on padding.
- **[`Quiet` is still a window]** "Blocked" means no context switch (or write) for `BLOCKED_FOR`. → A process that didn't run at all in a quarter-second is blocked, whatever the load. This is the one absence the suite still measures, in one place, and it is labelled so (`testing R11`).
- **[Dropping wall-clock bounds weakens two checks]** `flood_responsive`/`close_mid_output` and the stalled-client rounds detect a freeze, not a slowdown. → The regressions behind them were freezes. A slowdown is the perf suite's question (`testing R14`).
- **[Windows modules change without a local run]** → Windows CI's `cargo test` is their first run (`testing R17`). Report them as CI's.
- **[The `face_top` loop may not reproduce]** → Decision 7 files the issue instead. The case keeps its mitigation, labelled.
- **[Overlap with changes 2, 6, 9 and 10]** in the same test files. → Whichever lands second rebases. New tests in those changes use the helpers once this change has landed, and the sleep check catches them in `remote-agent/tests/**`.

## Migration Plan

None. No config, state or protocol change. Rollback is a revert. `BRINDLE_TEST_SLOW` and `BRINDLE_TEST_REQUIRE` are new, optional test variables.
