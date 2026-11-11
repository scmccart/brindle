# Tasks

Root-cause reports, kept for the implementer: /tmp/claude-1000/-home-scmccart-src-brindle/09fe43d2-d6c1-4c46-b53c-30ca7978b10b/scratchpad/{close-window-rca,unit-flakes-rca,windows-rca}.md (repro programs under `repro/` and `harness/` beside them). Copy anything needed into the PR description; the scratchpad is not permanent.

## 1. Ready-fd handoff (holder tests and `start_core`)

- [x] 1.1 Add a helper that passes a pipe fd to a child as a given number: `dup2` when the numbers differ, and clear `FD_CLOEXEC` when they match. Use it in `remote-agent/tests/support/mod.rs` `start` and in `src/remote/local.rs` `start_core`. Verify with a unit test that forces the pipe onto the target fd (close fds below it first) and checks that the child sees the fd open.
- [ ] 1.2 Verify that `cargo test -p brindle-remote-agent --test holder` passes 600 consecutive runs, with 4 copies at a time, and no `--ready-fd 3 is not an open file descriptor` failure. Before the fix, about 1 in 600 runs failed.

## 2. `ETXTBSY` on test scripts

- [x] 2.1 After writing each executable test script in `src/remote/local.rs` tests (`script()`) and `src/terminal/links.rs` tests, wait (bounded at 5 s) until a read lease can be taken, then release it. If leases are unsupported (`EINVAL`), fall back to the bounded wait without one and log it. Verify that `cargo test -p brindle --bin brindle remote::local` and `terminal::links` pass.
- [x] 2.2 Verify that `a_core_that_never_listens_is_unreachable` and `a_core_that_fails_to_start_shows_why` pass 3,500 runs of the `brindle` test binary, 4 copies at a time, with no "can't run" failure. Before the fix, 13 of 3,520 runs failed.

## 3. Ownership survives a core killed right after an attach

- [x] 3.1 In the core, count bytes written per holder connection. Record where an attach's `SET_META` ends, and hold that client's replay until the holder's count passes it, or the holder connection is lost. Flush holders before clients in `flush_all`. Verify with `cargo test -p brindle-remote-agent --test core` and the existing upgrade, holder and manage tests.
- [x] 3.2 Add the spec scenario "Core killed right after an attach" as a test in `remote-agent/tests/core.rs`: attach from B, SIGKILL the core on `REPLAY_START`, reconnect both, assert that B gets the session and A doesn't. Keep `owner_and_policy_changes_survive_a_core_restart` as it is, without adding a round trip. Verify both pass 40 runs with 8 binaries at a time. Before the fix the existing test failed 24 of 40 under that load.

- [x] 3.3 In the holder's `flush_core`, read and handle the frames already available before dropping the core on a write error. Add a holder test: a session printing continuously (`yes`), a stand-in core that sends `SET_META` and closes without reading, and the meta checked as stored. Verify that the test fails without the change (about 14 of 20 runs) and passes 40 of 40 with it, and that the remote-agent suite passes.

## 4. The truncated tail

- [x] 4.1 In `a_session_with_more_output_than_a_tail_holds_rebuilds_exactly`, end the script with `read x; printf 'ack\r\n'`, and have the test send a line and wait for `ack` before killing the core. Leave the assertion at `core.rs:773` unchanged. Verify 400 runs of the test pass with no "tail was truncated" log. Before the fix, 14 of 400 runs failed.

## 5. Session lock, `config_edit` and `cmdline` tests

- [x] 5.1 In `session::tests::lock_is_exclusive`, retry the post-drop `Lock::acquire` for up to 5 s. Verify 1,600 full runs of the `brindle` test binary have no failure of this test. Before the fix, 8 of 1,600 runs failed.
- [x] 5.2 Give `save_refuses_malformed_files` and `edits_refused_on_malformed_file` (`src/config_edit.rs`) temp directories of their own. Verify 1,600 runs with no failure of either test. Before the fix, 17 of 1,600 runs failed.
- [x] 5.3 Make the `terminal/cmdline.rs` tests remove their `/tmp/brindle-default-*` directories with a drop guard. Verify that the count of `/tmp/brindle-default-*` entries is the same before and after `cargo test -p brindle --bin brindle terminal::cmdline`.
- [x] 5.4 Make `terminal::pace::tests::drain_stops_at_the_budget` independent of how long its sleeps actually take: assert the budget rule against the times the callbacks actually ran, not a fixed `[0, 1]`. This was found in group 5's loop: 22 of 1,600 runs failed under load with `left: [0]`. Keep `drain` unchanged unless the test shows it is wrong. Verify 1,600 runs of the test under 4-way parallel load with no failure, and confirm that it still fails if `drain` ignores the budget.

## 6. Debug hooks: scaled `--wait-for` and wait lines in the dump

- [x] 6.1 Make the `--wait-for` limit 30 step intervals in `src/main.rs`, and record each wait's outcome (timed out, pending) where the dump can read it. Verify with a unit test of the limit computation, and update the `--wait-for` log message to print the actual limit.
- [x] 6.2 Print `--- wait-for timeout "TEXT"` and `--- wait-for pending "TEXT"` in the dump, escaped as selection text is. Teach the e2e dump parser the lines and add a parser self-test. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`.
- [x] 6.3 Add e2e cases for the spec scenarios "A timed-out wait in the dump" and "A pending wait in the dump". Verify them under `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- wait-for`.
- [x] 6.4 Update CLAUDE.md's `--wait-for` description (30 step intervals, the new dump lines). Verify that it matches `specs/command-line/spec.md`.

## 7. e2e harness on Linux

- [x] 7.1 Write the headless reproduction from design.md first, without Brindle: one connection maps a window and counts client messages, while 500 short-lived connections each send one message and disconnect. Run it under `scripts/with-xvfb`, without and with `.check()`. If it loses no messages without `.check()`, stop and report back before changing the case. Keep the reproduction in the scratchpad, not the repo.
- [x] 7.2 In `tests/e2e/capture.rs` `request_close`, replace `send_event` + `flush` with `send_event(...)?.check()`. Verify that `local-server-close-window` passes 30 consecutive runs under `scripts/with-xvfb` with `BRINDLE_E2E_SLOW=2` and CPU load (`stress-ng --cpu $(nproc)`, if installed). Record the pre-fix failure rate from 20 runs under the same load.
- [x] 7.3 Make `capture`'s viewable-window wait 10 s scaled by `BRINDLE_E2E_SLOW`, count `capture_at` times from the window's first appearance (a watcher thread started at spawn), and kill and wait Brindle on every early return after spawn in `tests/e2e/brindle.rs`. Verify with a `BRINDLE_E2E_BIN` wrapper that sleeps 8 s before exec'ing Brindle: `dump-grid-geometry` passes with it, and after an induced failure no `brindle` process is left running.

## 8. Windows e2e (on the Windows machine, through the `brindle-windows` session)

- [x] 8.1 Turn the `during` probes of `windows-conpty-host`, `conpty_fallback` and `caption` (`tests/e2e/cases/win.rs`) into polls, every 100 ms, until the condition holds or Brindle exits (after its dump), within the run's deadline. Assert on the last snapshot. Verify on Windows that the three cases pass, and that delaying Brindle's start by 8 s no longer fails `windows-conpty-host`. A temporary sleep at the top of `main` stands in for a cold start, since on Windows a wrapper changes the probed pid. These runs open windows: ask the user first.
- [x] 8.2 Add the timed headless Windows PowerShell 5.1 warm-up to `windows-pwsh-integration`, writing `powershell-warmup.txt` to the case directory. Verify on Windows that the case passes and that the file is written.
- [ ] 8.3 Verify the Windows build and the Windows-only unit tests compile and pass (`cargo test --workspace` on Windows), since groups 1, 2 and 6 touch `cfg(unix)` and shared code.

## 10. Holder and client timing tests

These turned up while verifying groups 1–2: three holder tests failed in 16 of 600 runs at only 4 at a time, also on an idle machine.

- [ ] 10.1 `hangup_kills_a_child_that_outlives_its_exited_leader`: the parent prints the child's pid before the child has run `trap '' HUP`, so the hang-up can kill it first (1,995 of 2,000 in miniature). Have the child print its own pid after the trap (`(trap '' HUP; exec sh -c 'echo "bg=$$."; exec sleep 1000') & wait`). Verify 600 runs of the holder binary, 4 at a time, with no failure of this test.
- [ ] 10.2 `a_new_core_gets_the_sessions_checkpoints_and_tails` (`holder sockets: []`): find out why the holder is empty when the test expects it, and fix the test's ordering without weakening what it checks. Verify the same way.
- [ ] 10.3 `a_checkpoint_keeps_the_output_it_doesnt_cover` (`cat` exited 1 instead of dying of SIGHUP): find the cause. If the product is right, make the test accept the outcomes the spec allows; if it isn't, fix the product and say so. Verify the same way.
- [ ] 10.4 `without_a_core_a_program_blocks_instead_of_losing_output` (a fixed 500 ms sleep) and `a_tail_past_its_limit_is_truncated`, which failed only with 24 binaries at once: replace fixed sleeps with waits for the condition, bounded. Verify 600 runs at 8 at a time with no failure of either.
- [ ] 10.5 `remote::client::tests::an_idle_writer_sends_heartbeats_and_then_the_goodbye` (6 heartbeats where 2–5 were expected in 900 ms; 1 of 3,500): make the count bound follow the time that actually elapsed. Verify 3,500 runs of the test, 4 at a time with 4 busy loops, with no failure.

## 9. Integration

- [ ] 9.1 Run `cargo test --workspace --locked` 20 times in a row on Linux. Verify there are no failures, and list any new flake found, with its test name and failure text, in the PR description.
- [ ] 9.2 Run the full e2e suite under `scripts/with-xvfb` with `BRINDLE_E2E_SLOW=2` 3 times. Verify there are no failures.
- [ ] 9.3 After the PR is pushed, check its CI. Verify that `test`, all e2e shards and `windows-e2e` are green. On a red job, read its log before rerunning anything.
- [ ] 9.4 Draft follow-up issues for the out-of-scope items in proposal.md (`XDG_RUNTIME_DIR` in `instance.rs` tests, ignored SIGHUP under `nohup`, the GPUI blank-window fix). Show the drafts to the user, and file them only with their approval.

## Workflow follow-up

- Land through a squash-merged PR whose title follows commitlint, with `Closes #32` and `Closes #36` in the description. The user merges.
- Archive the change (`docs(openspec): archive fix-flaky-tests`) after merge, syncing the delta specs.
