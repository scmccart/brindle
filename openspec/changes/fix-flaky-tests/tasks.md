# Tasks

Root-cause reports, kept for the implementer: /tmp/claude-1000/-home-scmccart-src-brindle/09fe43d2-d6c1-4c46-b53c-30ca7978b10b/scratchpad/{close-window-rca,unit-flakes-rca,windows-rca}.md (repro programs under `repro/` and `harness/` beside them). Copy anything needed into the PR description; the scratchpad is not permanent.

## 1. Ready-fd handoff (holder tests and `start_core`)

- [ ] 1.1 Add a helper that passes a pipe fd to a child as a given number: `dup2` when the numbers differ, and clear `FD_CLOEXEC` when they match. Use it in `remote-agent/tests/support/mod.rs` `start` and in `src/remote/local.rs` `start_core`. Verify with a unit test that forces the pipe onto the target fd (close fds below it first) and checks that the child sees the fd open.
- [ ] 1.2 Verify that `cargo test -p brindle-remote-agent --test holder` passes 600 consecutive runs, with 4 copies at a time, and no `--ready-fd 3 is not an open file descriptor` failure. Before the fix, about 1 in 600 runs failed.

## 2. `ETXTBSY` on test scripts

- [ ] 2.1 After writing each executable test script in `src/remote/local.rs` tests (`script()`) and `src/terminal/links.rs` tests, wait (bounded at 5 s) until a read lease can be taken, then release it. If leases are unsupported (`EINVAL`), fall back to the bounded wait without one and log it. Verify that `cargo test -p brindle --bin brindle remote::local` and `terminal::links` pass.
- [ ] 2.2 Verify that `a_core_that_never_listens_is_unreachable` and `a_core_that_fails_to_start_shows_why` pass 3,500 runs of the `brindle` test binary, 4 copies at a time, with no "can't run" failure. Before the fix, 13 of 3,520 runs failed.

## 3. Ownership survives a core killed right after an attach

- [ ] 3.1 In the core, count bytes written per holder connection. Record where an attach's `SET_META` ends, and hold that client's replay until the holder's count passes it, or the holder connection is lost. Flush holders before clients in `flush_all`. Verify with `cargo test -p brindle-remote-agent --test core` and the existing upgrade, holder and manage tests.
- [ ] 3.2 Add the spec scenario "Core killed right after an attach" as a test in `remote-agent/tests/core.rs`: attach from B, SIGKILL the core on `REPLAY_START`, reconnect both, assert that B gets the session and A doesn't. Keep `owner_and_policy_changes_survive_a_core_restart` as it is, without adding a round trip. Verify both pass 40 runs with 8 binaries at a time. Before the fix the existing test failed 24 of 40 under that load.

## 4. The truncated tail

- [ ] 4.1 In `a_session_with_more_output_than_a_tail_holds_rebuilds_exactly`, end the script with `read x; printf 'ack\r\n'`, and have the test send a line and wait for `ack` before killing the core. Leave the assertion at `core.rs:773` unchanged. Verify 400 runs of the test pass with no "tail was truncated" log. Before the fix, 14 of 400 runs failed.

## 5. Session lock, `config_edit` and `cmdline` tests

- [ ] 5.1 In `session::tests::lock_is_exclusive`, retry the post-drop `Lock::acquire` for up to 5 s. Verify 1,600 full runs of the `brindle` test binary have no failure of this test. Before the fix, 8 of 1,600 runs failed.
- [ ] 5.2 Give `save_refuses_malformed_files` and `edits_refused_on_malformed_file` (`src/config_edit.rs`) temp directories of their own. Verify 1,600 runs with no failure of either test. Before the fix, 17 of 1,600 runs failed.
- [ ] 5.3 Make the `terminal/cmdline.rs` tests remove their `/tmp/brindle-default-*` directories with a drop guard. Verify that the count of `/tmp/brindle-default-*` entries is the same before and after `cargo test -p brindle --bin brindle terminal::cmdline`.

## 6. Debug hooks: scaled `--wait-for` and wait lines in the dump

- [ ] 6.1 Make the `--wait-for` limit 30 step intervals in `src/main.rs`, and record each wait's outcome (timed out, pending) where the dump can read it. Verify with a unit test of the limit computation, and update the `--wait-for` log message to print the actual limit.
- [ ] 6.2 Print `--- wait-for timeout "TEXT"` and `--- wait-for pending "TEXT"` in the dump, escaped as selection text is. Teach the e2e dump parser the lines and add a parser self-test. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`.
- [ ] 6.3 Add e2e cases for the spec scenarios "A timed-out wait in the dump" and "A pending wait in the dump". Verify them under `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- wait-for`.
- [ ] 6.4 Update CLAUDE.md's `--wait-for` description (30 step intervals, the new dump lines). Verify that it matches `specs/command-line/spec.md`.

## 7. e2e harness on Linux

- [ ] 7.1 Write the headless reproduction from design.md first, without Brindle: one connection maps a window and counts client messages, while 500 short-lived connections each send one message and disconnect. Run it under `scripts/with-xvfb`, without and with `.check()`. If it loses no messages without `.check()`, stop and report back before changing the case. Keep the reproduction in the scratchpad, not the repo.
- [ ] 7.2 In `tests/e2e/capture.rs` `request_close`, replace `send_event` + `flush` with `send_event(...)?.check()`. Verify that `local-server-close-window` passes 30 consecutive runs under `scripts/with-xvfb` with `BRINDLE_E2E_SLOW=2` and CPU load (`stress-ng --cpu $(nproc)`, if installed). Record the pre-fix failure rate from 20 runs under the same load.
- [ ] 7.3 Make `capture`'s viewable-window wait 10 s scaled by `BRINDLE_E2E_SLOW`, and kill and wait Brindle on every early return after spawn in `tests/e2e/brindle.rs`. Verify with a `BRINDLE_E2E_BIN` wrapper that sleeps 8 s before exec'ing Brindle: `dump-grid-geometry` passes with it, and after an induced failure no `brindle` process is left running.

## 8. Windows e2e (on the Windows machine, through the `brindle-windows` session)

- [ ] 8.1 Turn the `during` probes of `windows-conpty-host`, `conpty_fallback` and `caption` (`tests/e2e/cases/win.rs`) into polls, every 100 ms, until the condition holds, Brindle exits, or the dump time passes. Assert on the last snapshot. Verify on Windows that the three cases pass, and that a `BRINDLE_E2E_BIN` wrapper delaying Brindle's start by 8 s no longer fails `windows-conpty-host`. These runs open windows: ask the user first.
- [ ] 8.2 Add the timed headless Windows PowerShell 5.1 warm-up to `windows-pwsh-integration`, writing `powershell-warmup.txt` to the case directory. Verify on Windows that the case passes and that the file is written.
- [ ] 8.3 Verify the Windows build and the Windows-only unit tests compile and pass (`cargo test --workspace` on Windows), since groups 1, 2 and 6 touch `cfg(unix)` and shared code.

## 9. Integration

- [ ] 9.1 Run `cargo test --workspace --locked` 20 times in a row on Linux. Verify there are no failures, and list any new flake found, with its test name and failure text, in the PR description.
- [ ] 9.2 Run the full e2e suite under `scripts/with-xvfb` with `BRINDLE_E2E_SLOW=2` 3 times. Verify there are no failures.
- [ ] 9.3 After the PR is pushed, check its CI. Verify that `test`, all e2e shards and `windows-e2e` are green. On a red job, read its log before rerunning anything.
- [ ] 9.4 Draft follow-up issues for the out-of-scope items in proposal.md (load-only holder failures, `XDG_RUNTIME_DIR` in `instance.rs` tests, ignored SIGHUP under `nohup`, the GPUI blank-window fix). Show the drafts to the user, and file them only with their approval.

## Workflow follow-up

- Land through a squash-merged PR whose title follows commitlint, with `Closes #32` and `Closes #36` in the description. The user merges.
- Archive the change (`docs(openspec): archive fix-flaky-tests`) after merge, syncing the delta specs.
