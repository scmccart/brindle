# Proposal

## Why

The test suites fail intermittently, so CI turns red on unrelated PRs and people learn to rerun rather than read failures. A survey of the failed CI runs since 2026-10-31 and the open issues #32 and #36 found eleven distinct flakes. Each failed run had a same-commit sibling run that passed. Every one now has a root cause. Seven were reproduced locally with the real test binaries, and the others were traced from the CI artifacts. Most are races in the tests, but three are small product bugs that the tests exposed.

## What Changes

**e2e harness (Linux)**
- `local-server-close-window` (#32, 7 CI failures): `request_close` drops its X connection right after sending `WM_DELETE_WINDOW`, and Xvfb discards a disconnected client's unread requests. The harness will wait for the server to process the request before disconnecting.
- `dump-grid-geometry`: the harness waits a fixed 3 s for a viewable window, unscaled by `BRINDLE_E2E_SLOW`, and returns early without killing Brindle. The wait will scale and lengthen, and an early return will kill and reap the child.

**Unit and integration tests**
- `a_new_core_gets_the_sessions_checkpoints_and_tails`: `dup2(fd, 3)` is a no-op when the ready pipe is already fd 3, which leaves it close-on-exec. The fd will be cleared of close-on-exec in that case. Brindle's own `start_core` has the same bug and gets the same fix (product).
- `a_core_that_never_listens_is_unreachable` and `a_core_that_fails_to_start_shows_why`: `ETXTBSY`, because a child forked by another test thread holds the fake helper's write fd. Writing the test scripts will wait until no writer remains. `terminal/links.rs` tests have the same exposure.
- `a_session_with_more_output_than_a_tail_holds_rebuilds_exactly`: the test kills the core before its final checkpoint reaches the holder. It will wait for a round trip first.
- `owner_and_policy_changes_survive_a_core_restart` (#36): an attach's new owner is queued for the holder behind the client's replay, so a core killed right after the replay forgets the owner. The core will get an attach's ownership change to the holder before the client's replay (product). The test keeps its kill-right-after-attach shape and becomes the regression check.
- `session::tests::lock_is_exclusive` (#36): a child forked by another test shares the lock's open file description until its exec, so the lock outlives `drop`. The test will retry the retake for a bounded time.
- `config_edit` tests `save_refuses_malformed_files` and `edits_refused_on_malformed_file` share one temp directory. Each will get its own.
- `terminal/cmdline.rs` tests leave a `/tmp/brindle-default-*` directory behind on every run. They will remove it.

**Windows e2e**
- `windows-conpty-host`: the probe for the console-host child fires once, 5 s after the harness's spawn, which is before a cold first launch has started. It will poll until the child appears, Brindle exits, or the dump time passes. The `conpty_fallback` and `caption` probes get the same treatment.
- `windows-pwsh-integration`: Windows PowerShell 5.1 sometimes shows no prompt before the fixed 30 s dump. The cause is reasoned, not reproduced: likely a cold first start. The case will warm 5.1 up headless and time it. The debug hooks will make the failure explain itself: `--wait-for`'s limit scales with `--step-interval`, and the dump names any wait that timed out.

**Debug hooks (product)**
- `--wait-for` waits for 30 step intervals rather than a fixed 30 s. The dump prints a `--- wait-for timeout` line for each wait that gave up, and a `--- wait-for pending` line for one still waiting.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `command-line`: `--wait-for`'s limit scales with `--step-interval`, and `--dump-screen-after` reports waits that timed out or are still pending.
- `terminal-server`: an attach's new owner and policy are kept from the moment the attaching client receives the start of its replay, so a core restart right after an attach still gives the session back to the new owner.

## Impact

- Tests: `tests/e2e/capture.rs`, `tests/e2e/brindle.rs`, `tests/e2e/cases/win.rs`, `remote-agent/tests/{support/mod.rs,core.rs}`, and the unit tests in `src/remote/local.rs`, `src/session.rs`, `src/config_edit.rs`, `src/terminal/cmdline.rs`, `src/terminal/links.rs`.
- Product: `src/remote/local.rs` (`start_core`'s ready-fd handoff), `remote-agent/src/server/core.rs` (flush order for ownership changes), `src/main.rs` (`--wait-for` limit and dump line).
- Closes #32 and #36.
- Added during implementation, once verification loops showed them failing at ordinary parallelism: five holder tests whose timing assumptions fail under load (one, the hang-up test, also on an idle machine), and the client heartbeat-count test (task group 10).
- Out of scope, to be filed as follow-ups:
  - `instance.rs` setting `XDG_RUNTIME_DIR` for the whole test process, which makes `remote::local` tests create real systemd scopes on a desktop;
  - session programs inheriting an ignored SIGHUP when the tests run under `nohup`;
  - picking up GPUI's blank-new-window fix (zed-industries/zed#62081) with a future gpui upgrade.
