# Tasks: agents-process-lifecycle

Each task names the command that verifies it. Every new test is named as a sentence and waits on a condition or a gate file, never on time (`testing R7`, `R9`). A test written to show a fault fails on `main` first; design decision 14 says what happens when it doesn't.

## 1. Re-open the sites and show the faults

- [x] 1.1 Re-read the cited sites on `main`, after #151 (`agents-commands`) and change 3 (`agents-event-bounds`) have landed. In design.md's Context, note any line or shape that moved since `5f335dc`. The sites:
  - `acp/reduce.rs`: `end`, `restart_for_login`, `respawn`, `refuse`, `fail`, `finish`, `line`'s `Malformed::InvalidRequest`, and `message`'s `RequestKind::Unoffered`.
  - `acp/rpc.rs`: `restart`, `cancel_permissions`.
  - `server/core/clients.rs`: `drop_client`, `take_over`.
  - `server/core/sessions.rs`: `session_lost`, `reattach`, `replay`, `hang_up`, `abandon_restart`, `start_again`, `restarting`.
  - `server/core/holders.rs`: `place_waiting`, `fail_waiting`.
  - `server/core/agents.rs`: `guarded_step`, `agent_step`, `agent_apply`, `agent_stderr`, `client_agent_restart`, `restart_after_login`.
  - `server/holder.rs`: `read_limit`, `read_output`, `tick`, `update_interests`.

  Verify: `git log -1 --oneline origin/main` names the base the notes were taken on.

  Done: `677dffd`, with `agents-event-bounds` below the branch (`02fd25c`); design.md "As built", "The sites, re-read".
- [x] 1.2 A-3, pure, failing first:
  - Add the invariant (design decision 3) to the reducer's test `Agent::step`: the log's pending permission entries equal `Rpc`'s waiting ones after every step. Add a `#[cfg(test)] Rpc::waiting_permissions`.
  - Add `a_permission_waiting_at_a_login_restart_is_recorded_cancelled_before_the_agent_starts`: a permission at entry 7, the prompt answered `-32000`, `Input::Restart`; shape `["answered", "added notice", "agent starting"]`.
  - Add `a_list_out_at_a_login_restart_is_refused_saying_the_agent_is_started_again`.

  Verify: `cargo test -p brindle-remote-agent acp::reduce` fails on these and on no unrelated test.

  Done: 3 failed (these two and 1.3's), 423 passed, on the base.
- [x] 1.3 Observation 6, pure, failing first: `a_fold_error_writes_nothing_the_faulted_step_asked_for`. A step whose own events fail to fold after it queued a `session/prompt` leaves `Output::stdin` empty. Verify: `cargo test -p brindle-remote-agent acp::reduce::tests::a_fold_error_writes_nothing` fails.

  Done: failed on the base (`stdin` held the prompt).
- [x] 1.4 C-1, failing first: `a_continue_whose_kill_client_drops_before_placement_ends_and_a_stop_is_answered`. The client drop must land while the restart waits for a holder.
  - Use `remote-agent/tests/agents.rs` if a gate can hold placement there (no holder of the core's build running, with the holder start held).
  - Otherwise use the core's in-process harness in `server/core.rs`'s tests, which controls holder connections, as `WaitingAttach` does.

  Assert that the session stops being listed, that `STOP` gets its `OK`, and that no agent process was started for it. Verify: the test fails on `main` (the session stays `Starting`, and `STOP` gets no answer before the test's limit).

  Done: in the in-process harness (`server/core.rs`, `restart_unplaced`; design.md flag 5); failed on the base.
- [x] 1.5 C-3, failing first. Add `BRINDLE_TEST_ACP_PANIC_LATE` to `server/fault.rs`:
  - debug builds only;
  - listed by `honoured()` and in `the_helper_lists_its_switches_only_in_debug_builds`;
  - its injection inside `guarded_step`'s `catch_unwind`, after `reduce::step` returns, when the input held the marker and the output holds an event.

  Then add `a_late_panic_in_acp_handling_replays_the_log_and_survives_a_core_kill` in `remote-agent/tests/agents.rs`:
  - a prompt holding the marker;
  - the client receives an unrequested replay holding the user entry and the failed state;
  - the agent's record has no `session/prompt`;
  - the agent's exit then folds on the client;
  - after `kill_and_restart_core`, the adopted log equals the one before.

  Verify: `cargo test -p brindle-remote-agent --test agents a_late_panic` fails on `main` (`TestClient`'s fold panics on an event that doesn't fold); `cargo test -p brindle-remote-agent fault` passes with the new listing.

  Done: failed on the base; the fault tests pass with the new switch listed. The test's shape differs (design.md flag 8).
- [x] 1.6 C-4, failing first, in `remote-agent/tests/agents.rs`:
  - `an_owner_returning_to_an_exited_agent_is_replayed_its_exit_before_the_session_exit`: a `kill` agent session with no continue, its connection dropped and lingering; the agent ends its turn and exits with status 1 behind a gate the test opens; the owner reconnects with its token.
  - `an_owner_returning_after_a_core_kill_is_replayed_the_adopted_exit`: the agent exits while no core runs.

  Assert the replay's turn end, the exit with status 1 and its standard error, then `EXIT`. Verify: `cargo test -p brindle-remote-agent --test agents an_owner_returning` fails on `main` (only `EXIT` arrives).

  Done: both failed on the base.
- [x] 1.7 C-5, failing first: `a_refused_request_is_answered_once_across_a_core_kill` in `remote-agent/tests/agents.rs`:
  - the fake agent sends `fs/read_text_file` with id 7, expects the `-32601`, then waits on a gate file;
  - the test kills the core once the agent's record holds the answer, starts a new one, and opens the gate;
  - the record holds exactly one answer for id 7.

  Verify: `cargo test -p brindle-remote-agent --test agents a_refused_request` fails on `main` with two answers.

  Done: failed on the base with two answers. The test reads the agent's record through the new `record-all` step (design.md flag 7).
- [x] 1.8 C-7, failing first, in `remote-agent/tests/holder.rs`, in the Unix and the Windows `mod` (the existing `prints_twice`-style program helpers): `a_paused_pipe_sessions_last_output_reaches_the_core_before_its_exit`. A pipe session is paused; its program, released by a gate file, writes 48 KiB of numbered lines and exits. Every line arrives as `OUTPUT`, in order, before `EXITED`. Verify: `cargo test -p brindle-remote-agent --test holder a_paused_pipe_session` fails on `main` with lines missing (Linux here; Windows in CI's `windows` job).

  Done: failed on the base on Linux, with lines missing; Windows not run here.
- [x] 1.9 For each of 1.2–1.8, record in design.md's "As built" note whether it failed on `main`. Where one passed, apply decision 14 for that finding before going on. Verify: the note names each test and its result.

  Done: every one failed on the base, so decision 14 dropped nothing (design.md "As built", "What failed on `main`").

## 2. The reducer (pure)

- [x] 2.1 Tests first, in `acp/reduce.rs` and `acp/rpc.rs`:
  - `an_exit_mid_turn_fails_the_turn_and_cancels_the_waiting_permissions` keeps its shape;
  - `a_handling_fault_cancels_and_answers_the_waiting_permissions` (shape `answered`, `turn failed`, `agent failed`; `stdin` holds the cancelled answers);
  - `a_refused_load_cancels_a_permission_asked_during_it_and_unblocks_the_turn`;
  - `a_restart_after_an_exit_answers_nothing_already_closed`;
  - `closing_forgets_every_request_and_keeps_counting`, replacing `a_restart_forgets_every_request_and_keeps_counting`.

  Verify: `cargo test -p brindle-remote-agent acp::` fails only on these and 1.2's tests.

  Done: the tests were written together with 2.2, not run failing alone (design.md flag 12).
- [x] 2.2 Implement design decision 1:
  - `Closing::{Gone, LeftRunning}`;
  - `Rpc::close` in place of `Rpc::restart`;
  - `Refused::Restarted`;
  - `close_process`, called from `end`, `restart_for_login` (after the `CannotContinue` check), `refuse` (then `unblock`), `fail` (`LeftRunning`) and `respawn`.

  No `_ =>` arm on `Closing` (`railway R3`). Verify: `cargo test -p brindle-remote-agent acp::` passes, 1.2 included.

  Done: passes (commit `ec8dcac`).
- [x] 2.3 Implement design decision 2: `finish` clears `out.stdin` before `fail` when `fault` is set, keeping `events` and `listed`. Verify: 1.3's test passes; `cargo test -p brindle-remote-agent acp::reduce` passes.

  Done: passes (commit `ec8dcac`).
- [x] 2.4 Implement design decision 8, with tests first:
  - tests `an_unoffered_request_is_checkpointed_before_its_answer` and `an_invalid_request_is_checkpointed_before_its_answer` (`Output::checkpoint == Some(Moment::Answered)`, and the answer in `stdin`);
  - `Moment::Answered` with its doc, the `mark` arm, `policy::agent_checkpoint_due`'s arm, and the moment list in `policy.rs`'s tests.

  Verify: `cargo test -p brindle-remote-agent acp:: server::core::policy` passes.

  Done: the two tests didn't compile without `Moment::Answered`, and pass with it.
- [x] 2.5 Extend `random_steps_checkpointed_whole_or_appended_decode_to_the_live_session` (`acp/checkpoint.rs`) as design decision 3 says:
  - the new choices (login demand, restart, exit, restart after exit, fault, a client's answer, `$/cancel_request`, a list and its answer);
  - a helper that brings a restarted agent to ready;
  - the invariant asserted after every step.

  Keep the seeds and the run count. Verify: `cargo test -p brindle-remote-agent acp::checkpoint` passes, and fails if `close_process` is skipped in `restart_for_login` (checked once by hand, then restored).

  Done: passes; with `close_process` skipped in `restart_for_login` it failed on the invariant, and the code was restored.
- [x] 2.6 Refuse a checkpoint that breaks the invariant (design decision 3), test first: `a_checkpoint_with_a_pending_permission_rpc_doesnt_await_is_unreadable` in `acp/checkpoint.rs` builds one by hand and expects `Unreadable::Entry { name: "permissions", entry }` naming the entry; then add the check to `restore` beside today's converse one. Verify: `cargo test -p brindle-remote-agent acp::checkpoint`.

  Done: the test failed before the check (the checkpoint read) and passes with it. The refusal's text was reworded (design.md flag 9).

## 3. The core's models and policy (pure)

- [x] 3.1 Tests first, in `server/core/sessions.rs` and `policy.rs`:
  - `each_life_answers_its_questions` gains both `Restarting` stages;
  - `only_a_restart_not_yet_started_is_restarting` becomes `a_waiting_restart_is_in_no_holder_and_a_placed_one_is`;
  - one `lost_fate` test per row, e.g. `a_placed_restart_with_its_client_attached_is_placed_again`, `a_placed_restart_with_no_client_ends_as_lost`, `a_waiting_restart_is_untouched_by_a_holders_loss`, `a_queued_spawn_is_untouched_by_a_holders_loss`, `a_starting_spawn_with_its_client_is_requeued_even_hung_up`, `a_running_agent_is_stepped_lost`, `a_terminal_with_its_client_is_reported_lost`, `a_terminal_with_no_client_ends_as_its_exit`;
  - one `after_loss` test per outcome;
  - `gone_fate`, `grace_fate`, `restart_refusal` and `detach_ends` cases for `Restarting`.

  Verify: `cargo test -p brindle-remote-agent server::core` fails only on these.

  Done: written before 3.2's types existed, so they failed to compile first. `a_queued_spawn_is_untouched_by_a_holders_loss` was dropped with `lost_fate`'s `queued` parameter, and an exit kept for its owner got a row of its own, `an_exit_kept_for_its_owner_outlives_its_holder` (design.md flags 1 and 2). The fate, refusal and detach tests that walk `lives()` cover both `Restarting` stages, since the list now holds nine lives.
- [x] 3.2 Implement design decisions 4 and 5's types and functions:
  - `Restart`, `RestartStage`, `Life::Restarting`, and `Life`'s answers;
  - `SpawnRequest` without `Restart`, and `Session::spawning` without `Option`;
  - `policy::{LostFate, lost_fate, AfterLoss, after_loss}`.

  Every `match` on `Life` names `Restarting` (`railway R3`). Verify: `cargo test -p brindle-remote-agent server::core::policy server::core::sessions` passes.

  Done: passes (commit `387d5cc`).

## 4. The core's handlers (shell)

- [x] 4.1 Apply design decision 4's site table and route `session_lost` through `lost_fate` and `after_loss`, with the handler only gathering and applying (`fcis R1`). Log `session N: restart dropped, its client is gone`. Verify:
  - 1.4's test passes;
  - `cargo test -p brindle-remote-agent --test agents restart continue login` passes, including `a_close_before_the_old_agent_exits_drops_the_restart`, `an_unrunnable_restart_returns_the_session_to_ended_for_another_try` and `a_holder_killed_under_a_continuable_agent_leaves_it_to_continue`.

  Done: 1.4's test passes, and so does the whole `agents` suite (116), the three named included. A hang-up of a waiting restart ends the session (design.md flag 3).
- [x] 4.2 Add the real-process cases for the remaining spec scenarios, tests first:
  - `a_detached_continue_whose_client_drops_before_placement_runs_and_is_attached_ready`;
  - `a_permission_waiting_at_a_login_restart_is_cancelled_and_refused_afterwards`;
  - `a_list_out_at_a_login_restart_is_refused_and_the_next_goes_to_the_new_agent`.

  Verify: `cargo test -p brindle-remote-agent --test agents` passes.

  Done: all three failed on the base and pass now (design.md flags 6 and 15).
- [x] 4.3 Implement design decision 6 in `server/core/agents.rs`:
  - `Stepped::{Clean, Faulted}` from `guarded_step`;
  - `forget_stored` on `Faulted`;
  - `Delivery::{Events, Replay}` on `agent_apply`;
  - the fresh replay to an attached client with no replay pending;
  - the same in `agent_adopted`.

  Extend `a_fault_fails_the_agent_with_its_reason_and_leaves_it_running` to check that the checkpoint after a fault is whole. Verify:
  - 1.5's test passes;
  - `cargo test -p brindle-remote-agent --test agents a_panic_in_acp_handling` passes;
  - add and pass `a_permission_waiting_at_a_handling_fault_is_answered_cancelled`, which uses `BRINDLE_TEST_ACP_PANIC` after a permission request.

  Done: the unit test `a_fault_fails_the_agent_with_its_reason_and_leaves_it_running` (`agents.rs`) checks that a fault leaves nothing appendable, so the next checkpoint is whole. 1.5's test passes, and fails when `forget_stored` is removed (a mutation, restored). The new permission test failed on the base and passes.
- [x] 4.4 Implement design decision 7:
  - `AgentSession::has_ended`, with a unit test in `reduce.rs`;
  - the `reattach` arm by `SessionKind`;
  - `replay`'s deferral for an `ExitedAwaitingOwner` agent whose exit isn't stepped;
  - `agent_stderr` sending the pending replay.

  Verify: 1.6's tests pass; `cargo test -p brindle-remote-agent --test agents --test core` passes.

  Done: passes (agents 116, core 59). The holder's loss right after a kept exit needed `LostFate::KeepExit` (design.md flag 2).
- [x] 4.5 Verify design decision 8 end to end: 1.7's test passes. Verify: `cargo test -p brindle-remote-agent --test agents a_refused_request`.

  Done: passes.

## 5. The holder (shell, with its pure rule)

- [x] 5.1 Tests first, in `server/holder.rs`'s own tests:
  - `read_limit_lets_a_draining_pipe_session_read_past_a_pause_up_to_its_bound`;
  - `read_limit_lets_a_draining_pipe_session_read_past_the_cores_mark_up_to_its_bound`;
  - `read_limit_keeps_the_tails_room_while_draining`;
  - `read_limit_is_unchanged_for_a_following_session`;
  - the existing `read_limit_*` tests updated for the `Reads` argument.

  Verify: `cargo test -p brindle-remote-agent server::holder` fails only on these.

  Done: written first, they failed to compile (no `Reads`, `DRAIN_PAST_HOLD` or `held`), and pass with 5.2. `only_a_reaped_pipe_session_drains` was added for `reads`.
- [x] 5.2 Implement design decision 9: `Reads`, `DRAIN_PAST_HOLD`, `Life::Reaped { held }`, `read_output` counting held reads, `update_interests` passing `Reads`, and the log line once the bound is spent. PTY sessions are always `Following`. Verify:
  - 1.8's test passes on Linux: `cargo test -p brindle-remote-agent --test holder`;
  - on Windows, the same command in CI's `windows` job.

  Done: on Linux the holder suite passes (43), 1.8's test included (commit `78317d9`). Windows is left to CI's `windows` job.
- [x] 5.3 `a_paused_pipe_session_whose_child_keeps_writing_ends_within_the_drain_limit` (Unix and Windows `mod`s): the program starts a child that inherits stdout and writes without end, then exits. `EXITED` arrives, and the bytes read past the pause are at most `DRAIN_PAST_HOLD` plus one read. Verify: `cargo test -p brindle-remote-agent --test holder a_paused_pipe_session`.

  Done: passes on Linux. It can't fail on the base (a paused session read nothing there), so a mutation shows that it guards the bound: with an unbounded drain, 6,113,165,312 bytes were read past the pause (design.md flag 13). The Windows program has not been run.

## 6. End-to-end

- [x] 6.1 No new e2e case: the client is unchanged, and every behaviour here has a real-process test. Run the agent cases to check that today's zone handles a mid-session replay and a replay followed by `EXIT`. Verify: `systemd-run --user --scope scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- agent` passes.

  Done: with the `local-server` cases too, 119 passed, 0 failed, 2 skipped. The skipped cases were `remote-agent-ssh` and `remote-agent-login-terminal`, because the static musl fake agent isn't built here.
- [x] 6.2 Manual checks: none new. No behaviour here needs a real key or pointer.

  Done: none added.

## 7. Docs and checks

- [x] 7.1 Update `docs/code-map.md`:
  - the terminal server core's entry: `Life::Restarting` and its stages, `lost_fate`, the replay and whole checkpoint after a caught panic, and the replay to a returning owner;
  - the holder's entry: the drain past a pause and its bound;
  - the reducer's entry: the closing step and the invariant.

  Verify: `git diff --stat docs/code-map.md`.

  Done: the server core's, holder's and reducer's entries, plus the fake agent's `record-all` step.
- [x] 7.2 Update `docs/architecture/process-architecture.md`:
  - `R19`: a waiting permission is also answered `cancelled` when the session's ACP handling has failed, and recorded cancelled when its process is hung up to be restarted;
  - `R9`: a reaped pipe session drains past a pause or mark, up to 1 MiB;
  - `R20`: name `BRINDLE_TEST_ACP_PANIC_LATE` if the rule lists switches.

  Verify: `cargo test -p brindle --test architecture` passes.

  Done: R19, R9 and R20 updated, along with the line on what the real-role tests cover; the architecture test passes (51).
- [x] 7.3 Write design.md's "As built" notes: 1.9's results, the C-1 test's home (harness or real process), and any site that moved. Verify: `openspec validate agents-process-lifecycle --strict`.

  Done: valid. The delta's "Permission requests wait for the client's choice" now carries `agents-event-bounds`' option bounds and its two scenarios, merged by hand.
- [ ] 7.4 Run the checks:
  - `cargo test --workspace --locked`;
  - `cargo clippy --locked --workspace --all-targets -- -D warnings`;
  - `cargo test -p brindle-remote-agent --bin brindle-fake-acp-agent` (if the fake agent's script steps changed);
  - CI's `windows` job (cargo test and clippy on Windows);
  - the e2e run in 6.1.

  Partly done, on Linux: the workspace tests pass (brindle 1311, the helper's 696 unit tests, agents 116, core 59, holder 43, manage 14, relay 16, upgrade 18, the fake agent 31, architecture 51, and the shared crates'); clippy is clean; the e2e self-test passes; 6.1's run passes. Left unticked until CI's `windows` job has run, which this branch hasn't reached yet: it is the first run of 1.8's and 5.3's Windows programs.
