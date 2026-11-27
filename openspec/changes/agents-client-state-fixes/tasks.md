# Tasks

Run e2e cases headless and outside Brindle's systemd scope: `systemd-run --user --scope scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- FILTER`. Test names are given as they will read (`testing R15`). Every task writes its test first and watches it fail before the fix.

## 1. Re-open the sites and show each fault

- [x] 1.1 Re-open every site `design.md` cites on `main` after `agents-commands` (PR #151) merged: `src/agent/model.rs` (`Sent`, `pending`, `settle_first`, `prompt_status`, `can_send`, `absorb`, `respawning`, `Earlier`, `fetching`, `fetched`), `src/agent/mod.rs` (`Requests`, `apply`, `apply_absorbed`, `send`, `send_fetch`, `choose`), `src/agent/shared.rs` (`fold`, `prepend`), `src/agent/sink.rs`, `src/agent/login.rs` (`reparses`), `src/remote/local/session.rs` (`run`, `Session::running`), `src/remote/local/connection.rs` (`Waiting`, `State::link_lost`, the `Spawned` and `ReplayStart` arms, `Running::agent_request`, `Running::fetch`), and `remote-agent/src/acp/reduce.rs` (`clean`). Note every line that moved or changed meaning in this change's `design.md` under a dated "Re-read on main" note. Done when each decision's sites are confirmed or the design is amended.
- [x] 1.2 Add the fault switch (design decision 4):
  - Add `pub const SESSION_START_DELAY: &str = "BRINDLE_TEST_SESSION_START_DELAY"` to `src/remote/mod.rs`.
  - Add `#[cfg(debug_assertions)] fn start_delay() -> Option<Duration>` to `src/remote/local/session.rs`. It reads the variable once at the start of `run` and takes whole milliseconds only, as `main.rs`'s `test_listing` does.
  - In `run`, sleep for the delay after `started` is `Ok` and before `phase.lock()`, commented as the fault injector's delay.
  - List the switch in `main.rs`'s `fault_switches()`, and update `fault_switches_are_listed_only_in_debug_builds` to expect it.
  - Add `TestSwitch::SessionStartDelay` to `tests/e2e/switches.rs` (host `Brindle`), with its row in the switches self-test.
  - Verify with `cargo test -p brindle fault_switches` and `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`.
- [x] 1.3 Add the two e2e cases of design decision 4 to `tests/e2e/cases/agent.rs`, registered in `cases/mod.rs` with `needing(…, &[TestSwitch::SessionStartDelay], …).needs_fake_agent()`:
  - `agent-first-prompt-before-the-session-runs` (`first_prompt_before_the_session_runs`).
  - `agent-prompt-before-the-attached-session-runs` (`prompt_before_the_attached_session_runs`).
  - Set the switch to `ctx.scale().secs(1.5)` in milliseconds.
  - Each case checks for no `refusal=` and checks `prompts_sent`.
  - Run each 20 times on this branch before any fix, and record the failures (expected 20 of 20, each dump showing `refusal="the agent session isn't connected; nothing was sent"`) in the design's as-built notes.
- [x] 1.4 Show D-5:
  - Add `a_lost_link_fails_queued_fetches_and_carries_other_queued_requests` to `connection.rs`'s tests. It inserts a queued `Waiting::Agent` for a fetch and one for a prompt, calls `State::link_lost`, and expects the fetch failed and the prompt still queued.
  - Verify that it fails with `cargo test -p brindle a_lost_link_fails_queued_fetches`.
- [x] 1.5 Show D-4 with a failing test, `a_prompt_lost_with_the_link_and_found_in_the_replay_clears_its_draft`, in `model.rs`'s tests:
  - Send a prompt.
  - Reply `Err("the terminal server went away (…)")`.
  - Set the link to `Reconnecting`.
  - Replay a log holding the user entry.
  - Expect no refusal and a `ClearDraft`.
- [x] 1.6 Show D-6 and D-7 with failing tests in `model.rs`'s tests:
  - `a_new_conversation_forgets_the_old_rows_and_thoughts`: open a finished tool row by hand and close a thought, then `respawning` and feed a new log with a tool call and a streaming thought at the same ids; expect the row closed by policy and the thought open.
  - `a_respawn_that_doesnt_start_leaves_no_fetch_loading`: page a zone, then `fetching`, `respawning` and `not_started`; expect `earlier()` not `Loading`.
- [x] 1.7 Show D-10 with failing tests:
  - `a_live_permission_after_a_replay_raises_its_notice`: in one batch of changes, a replay of an idle ready agent, then live `Turn { Blocked }` and a permission entry; expect a notice.
  - `a_live_ready_after_a_replay_sends_the_first_prompt`: a first prompt, a replay showing `Starting`, then a live `Ready` in the same batch; expect `First::Send` once the link is up.
  - Put both in `model.rs`'s tests, using `AgentShared`'s `replay_start`, `apply` and `replay_end` before one `take_changes`.
- [x] 1.8 Show D-11's dropped choice with a failing entity test, `a_choice_while_reconnecting_says_why`, in `src/agent/tests.rs`: a choosing zone (fed `Choosing`), `sink.link_down()`, then `agent.choose(Some("c1"))`. Expect no `Call::Choose` and a choosing reason naming the link.
- [x] 1.9 Show D-12:
  - Add rows to `login.rs`'s `commands_that_reparse_their_arguments_are_withheld`: `pwsh -Command agy acp`, `pwsh -c …`, `pwsh -ec …`, `powershell agy acp`, `powershell.exe -Command …`, `wsl agy acp`, `WSL.EXE agy acp`. Expect `Some(Withheld)`.
  - Verify that they fail with `cargo test -p brindle agent::login`.
- [x] 1.10 Record the baseline:
  - Loop `agent-queue` and `agent-continue-profile-gone` on `main`: 4 parallel loops of 100 whole e2e runs each, under `scripts/with-xvfb` with `BRINDLE_E2E_SLOW=2`, each loop in its own `systemd-run --user --scope`.
  - Record failures out of 400 per case in the as-built notes.
  - Any test of 1.4 to 1.9 that passes on `main` marks its finding "traced, not shown": drop that fix (design, "Findings, traced") and keep the test.

## 2. Shared contract: one `clean`

- [x] 2.1 Move `clean` from `remote-agent/src/acp/reduce.rs` to `brindle-transcript` (`transcript/src/lib.rs`, `pub fn clean`), unchanged:
  - Make `reduce.rs` import it.
  - Move its unit test, or add `clean_replaces_control_characters_but_line_ends_and_tabs` if it has none.
  - Verify with `cargo test -p brindle-transcript` and `cargo test -p brindle-remote-agent`. The reducer's transcript tests pass unchanged.

## 3. Pure model (`src/agent/model.rs`, `shared.rs`, `login.rs`)

- [x] 3.1 Add the types of design decisions 1 and 2:
  - Add `NotSent`, `SessionHandle` and the texts `STARTING`, `PROMPT_NOT_TAKEN`, `PROMPT_MAYBE_TAKEN`, `CHOICE_LINK_DOWN` and `CHOICE_PENDING`. Move `LINK_DOWN` here as a `pub const`.
  - Add `ChatModel.handle`, `ChatModel::session_running`, and the `handle` checks in `can_send`, `prompt_status`, `fetch`, `can_choose` and `finish`'s `OnChoosing`.
  - In the tests' `Zone`, `new` calls `session_running`; add `Zone::starting`.
  - Tests: `a_first_prompt_waits_for_the_session_to_run`, `the_input_says_connecting_until_the_session_runs` and `a_launchs_list_waits_for_the_session_to_run` (a new row in `a_launch_lists_for_the_latest_or_opens_the_list`).
  - Verify with `cargo test -p brindle agent::model`.
- [x] 3.2 Replace `unsent` with `ChatModel::not_sent(sent, why) -> Replied`, following design decision 1's table. Test it with `each_request_that_isnt_sent_does_what_its_case_says`, one row per cell of the table, and update `an_unsent_request_is_refused_and_a_first_prompt_becomes_the_draft` to `Gone`. Verify with `cargo test -p brindle agent::model`.
- [x] 3.3 Replace `first` and `first_shown` with `FirstState`, and change `settle_first(status_from)` to design decision 3's order. Keep `the_first_prompt_is_sent_once_on_the_first_live_ready`, `the_first_prompt_becomes_the_draft_on_a_failure_or_a_replay` and `the_first_prompt_waits_for_a_login_and_goes_once` passing. Verify with `cargo test -p brindle agent::model`.
- [x] 3.4 Change `Changes::from_replay` to `status_from: StatusFrom` in `shared.rs` (design decision 8):
  - Set it in `fold`, reset it in `replay_start`, and leave it alone in `prepend`.
  - Add `the_newest_status_event_says_where_it_was_folded` to `shared.rs`'s tests.
  - Update `a_replay_starts_over_and_carries_the_slot`, `a_prepended_page_is_recorded_as_from_a_replay_and_a_bad_one_changes_nothing`, and `sink.rs`'s `a_replay_and_the_life_events_reach_the_zone_in_order` and `a_two_frame_page_is_folded_once_at_its_last_frame`.
  - In `model.rs`, pass the field to `finish`, `notice` and `settle_first`.
  - 1.7's tests pass, and so do `notices_on_blocked_done_and_error` and the `Replays raise nothing` tests.
  - Verify with `cargo test -p brindle agent::`.
- [x] 3.5 Make a prompt whose reply was lost wait for the replay (design decision 5):
  - Add `ReplyError`, `PromptSent`, `ChatModel::sent_prompt`, `Uncertain`, `Taken`, and the pure `taken(prompt, entries, window, slot)`.
  - Add `Absorbed::replied`, filled in `absorb`'s reset branch. `lost`, `broken` and `not_started` decide `CantTell`, and `respawning` drops the prompt.
  - `in_flight` counts an uncertain prompt.
  - Tests: 1.5's test, plus:
    - `a_prompt_lost_with_the_link_and_queued_in_the_slot_clears_its_draft`
    - `a_prompt_lost_with_the_link_and_missing_from_the_replay_is_refused_then`
    - `a_prompt_lost_with_the_link_past_the_replayed_page_may_have_been_sent`
    - `a_cleaned_prompt_is_found_in_the_replay` (a prompt with a control character)
    - `a_first_prompt_lost_with_the_link_becomes_the_draft_only_when_not_taken`
    - `a_failed_reply_is_still_a_refusal_at_once`
  - Verify with `cargo test -p brindle agent::model`.
- [x] 3.6 Paging (design decision 7):
  - Add the stored `Paging` and derive `earlier()`; `Earlier::Loading` loses `request`.
  - Change `fetching`, `fetched`, `fetch_due`, `retry` and the dump's `earlier=` line.
  - 1.6's second test passes, and so do `a_paged_zone_fetches_while_the_row_shows_and_folds_each_page_in_front`, `a_failed_fetch_asks_again_only_when_the_row_comes_back_or_is_pressed` and `a_reset_drops_a_fetch_in_flight_and_its_late_reply` (updated for the pattern).
  - Verify with `cargo test -p brindle agent::model`.
- [x] 3.7 Make `respawning` clear `toggled`, `by_hand` and `auto_open`, set `handle = Starting`, and drop `uncertain`. Make `absorb`'s trim prune `toggled` (design decision 9). 1.6's first test passes, and so does `a_respawn_keeps_the_old_conversation_until_the_new_session_replaces_it`. Verify with `cargo test -p brindle agent::model`.
- [x] 3.8 Add `ChatModel::choose() -> Choose` with design decision 10's reasons, tested by `a_choice_that_cant_go_says_why` (one row per reason) and by `a_palettes_list_and_a_choice` still passing. Verify with `cargo test -p brindle agent::model`.
- [x] 3.9 Extend `login::reparses` as design decision 11's table says, with a pure helper `names_switch(args, spellings)`. 1.9's rows pass, along with `commands_that_pass_arguments_through_are_offered` (`pwsh -File`, `pwsh -cwa`, `powershell -File`, `wsl --exec`, `wsl -e`) and `an_ssh_host_profile_is_not_withheld`. Verify with `cargo test -p brindle agent::login`.

## 4. Shell: the session, the connection and the entity

- [x] 4.1 Change the `SessionSink` trait in `connection.rs`:
  - Add `fn running(&mut self) {}`.
  - Change `reply` to take `Result<(), ReplyError>`.
  - Update `src/remote/local/agent.rs` and the terminal sink, which ignores both.
  - Update `connection.rs`'s test sinks, `fake::failing_sink` included.
  - Verify with `cargo build -p brindle`.
- [x] 4.2 In `connection.rs`:
  - Add `Carry` to `Waiting::Agent`: `NextLink` from `Running::agent_request`, `Never` from `Running::fetch`.
  - `State::link_lost` returns `LinkLost { unanswered, unsent }`.
  - Add `Waiting::unanswered`.
  - The link-down handler fails `unanswered` with `ReplyError::Unanswered` and `unsent` with `Failed(LINK_DOWN)`.
  - `ToClient::Error` and `Waiting::fail` reply `Failed`.
  - `Running::fetch` returns `Err(NotSent::LinkDown)` while the link is down, and `agent_request` returns `Err(NotSent::Gone)`.
  - 1.4's test passes, and so does `a_lost_link_fails_written_requests_and_carries_written_attaches` (updated to the two lists).
  - Verify with `cargo test -p brindle remote::local::connection`.
- [x] 4.3 In `session.rs`:
  - `Session::running` and every agent request return `Result<_, NotSent>` (design decision 1).
  - `run` raises `sink.lock().running()` after `*phase = Phase::Running(..)`, with the phase lock dropped first.
  - Change `AgentLink` in `src/remote/mod.rs` to match.
  - Tests:
    - `the_running_event_comes_once_the_session_can_send`: a spawn on the scripted core whose `Ready` comes with `SPAWNED`; when `SessionRunning` is taken, `session.prompt` is `Ok`.
    - `an_attached_sessions_running_event_follows_its_replay_start`.
    - `agent_requests_go_only_while_the_session_runs`: `Err(Starting)` while connecting, `Err(Gone)` after a failed start.
    - `an_agent_request_fails_with_the_link_and_the_zone_hears_it`: `ReplyError::Unanswered`.
    - `a_fetch_in_flight_fails_with_the_link`: `Unanswered`.
    - `a_fetch_while_the_link_is_down_fails_at_once`: returns `Err(NotSent::LinkDown)`, and no reply follows.
    - `an_attached_agent_session_replays_its_transcript` and `a_fetchs_frames_are_folded_in_front_before_its_reply`: `status_from` replaces `from_replay`.
  - Verify with `cargo test -p brindle remote::local::session`.
- [x] 4.4 In `src/agent/sink.rs`:
  - Add `AgentEvent::SessionRunning` (`Kind::Life`) and `Sink::running`.
  - Change `AgentEvent::Reply`'s outcome to `Result<(), ReplyError>`.
  - Test with `the_running_event_is_a_life_event_queued_in_order`.
  - Verify with `cargo test -p brindle agent::sink`.
- [x] 4.5 In `src/agent/mod.rs`:
  - `Requests` methods return `Result`.
  - `apply` handles `SessionRunning` with `session_running`.
  - `apply_absorbed` hands `Absorbed::replied` to `apply_replied`.
  - Every caller goes through `not_sent`: `send`, `send_fetch`, the first prompt, `restart`, `cancel`, `cancel_login` and `choose`.
  - `Agent::prompt` and the first prompt record `sent_prompt`.
  - `choose` applies `Choose`.
  - In `src/agent/test_support.rs`:
    - `FakeRequests.refuse: Rc<Cell<Option<NotSent>>>` replaces `down`.
    - `Zone::new` calls `sink.running()`, and `Zone::starting` doesn't.
    - `fake_connect` calls `running()` on each new sink.
  - Tests in `src/agent/tests.rs`:
    - `a_first_prompt_waits_for_the_session_to_run_and_goes_once`
    - `enter_before_the_session_runs_keeps_the_text_without_a_refusal`
    - 1.8's test passes.
    - Every existing test in the file passes with `refuse` in place of `down`.
  - Verify with `cargo test -p brindle agent::tests`.

## 5. UI and end-to-end

- [x] 5.1 Make `src/agent/view.rs` match `Earlier::Loading { .. }` with no request, in `refresh_motion` and in the row's drawing. In `view/tests.rs`, the paging test takes the fetch's id from `Call::Fetch` order, not from `Earlier::Loading`. Verify with `cargo test -p brindle agent::view`.
- [x] 5.2 Check the e2e cases:
  - Run `agent-first-prompt-before-the-session-runs` and `agent-prompt-before-the-attached-session-runs` 20 times each; expect 0 failures.
  - Run the whole `agent-` filter once; it passes (`agent-broken` included, on a debug build).
  - Record the counts in the as-built notes.
- [ ] 5.3 Repeat 1.10's loop on the branch (4 × 100 runs each of `agent-queue` and `agent-continue-profile-gone`). Record failures out of 400 before and after in the design's as-built notes and the PR body. Say plainly whether `main` failed locally; if it didn't, the switch cases are the evidence (`testing R11`).

## 6. Docs and checks

- [x] 6.1 Update `docs/code-map.md`:
  - The `agent/` entry: `SessionRunning`, `NotSent`, `ReplyError`, the uncertain prompt, `StatusFrom`, derived `earlier()`, `Choose`.
  - The `remote/` client entry: `Carry`, `link_lost`'s two lists, `Session::running`'s `Result`, the fault switch.
- [x] 6.2 Update `CLAUDE.md`'s e2e paragraph. Next to `agent-broken`'s fault switch, name the two new cases and `BRINDLE_TEST_SESSION_START_DELAY`: they skip on a release build, as `agent-broken` does.
- [x] 6.3 Note the amended withheld list in this change's as-built notes (design decision 11). No architecture rule or exception changes, so `docs/architecture/` is untouched.
- [x] 6.4 Add one line to the designs of `agents-prompt-context` and `agents-prompt-images` (or, if they are archived by then, to their follow-ups): the uncertain prompt's `taken` must compare the attachment list as well as the text (design decision 5).
- [x] 6.5 Run the checks:
  - `cargo test --workspace`
  - `cargo clippy --locked --workspace --all-targets -- -D warnings`
  - `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`
  - The `agent-` e2e filter under `scripts/with-xvfb`.
  - Report each result.
