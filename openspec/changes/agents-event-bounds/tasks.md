# Tasks

Tests come first in each group, written as sentences (`testing R15`). No test sleeps (`testing R9`). Verify commands run from the repository root.

## 1. Re-open the sites and show the plausible findings

- [x] 1.1 Once `agents-commands` (PR #151) is on `main`, rebase and re-open every site design.md's Context cites (`reduce.rs` `emit`, `add`, `chunk`, `add_text`, `tool_detail`, `tool_content`, `permission`, `created`, `prompted`, `raw_text`, `trim`; `diff.rs` `Keep`; `v1/decode.rs` `raw`, `requested_option`, `new_session_result`, `prompt_result`; `transcript/src/lib.rs` `apply`, `trim_to`, `replay_entry`; `client.rs` `pack_events`; `src/agent/tool.rs` `diff_rows`). Correct the line numbers in design.md and add a derivation row for each event `agents-commands` added (its command and option lists). Verify: design.md's table names every `Event` variant on `main`.
  - Done at `486f6e6` (nothing to rebase: #151 and #153 were already under this branch). Context's table and Findings carry `486f6e6`'s lines, the table gains `agents-commands`' bounds, and decision 1's derivation a `Commands`/`Options` row and the list of every variant.
- [x] 1.2 (B-7, shown first) In `transcript/src/lib.rs`'s tests, add the test "a fold of a tool call update carrying 40 items and a 100 KiB raw input holds at most 32 items and a raw input of at most 16 KiB". Run it at the rebased commit and record that it fails. Verify: `cargo test -p brindle-transcript a_fold_of_a_tool_update_past_the_bounds` fails before task 3.2 and passes after. If it passes before, B-7 is dropped and the rule text alone is corrected.
  - `a_fold_of_a_tool_update_past_the_bounds_holds_them` failed at `486f6e6` (`content.len() <= MAX_ITEMS`: 40 items) and passes after 3.2.
- [x] 1.3 (D-8, shown first) In `src/agent/tool.rs`'s tests, add the test "the rows of a hunk starting at `u32::MAX` with two context lines are drawn without a panic, the second line unnumbered". Record that it panics in a debug build. Verify: `cargo test -p brindle agent::tool::a_hunk_at_the_last_line_number` panics before task 7.1 and passes after. If it doesn't panic, record that and keep task 3.3's decoder check alone.
  - `a_hunk_at_the_last_line_number_draws_without_a_panic` panicked at `486f6e6` (`attempt to add with overflow`, `tool.rs:292`); the package has no library, so the command is `cargo test -p brindle --bin brindle agent::tool::…`. Passes after 7.1.
- [x] 1.4 (A-1 and A-2, shown first) In `remote-agent/src/acp/reduce.rs`'s tests, add the tests "a 300,000-line empty diff makes no event past `MAX_EVENT`", "a permission with 20 options named with 64 KiB each makes no event past `MAX_EVENT`", and "a 20 MiB thought streamed in 64 KiB chunks leaves the log within `LOG_LIMIT` after every step". Record that each fails at the rebased commit. Verify: `cargo test -p brindle-remote-agent acp::reduce::tests::` names the three failing.
  - All three failed at `486f6e6` (`--bin brindle-remote`, the package has no library): a `tool_items` event of 1,200,147 bytes, an `added permission` of 1,311,664, and a log of 16,777,244 bytes. They were written against a literal 1,047,552 before `MAX_EVENT` existed, and use it since 4.1.

## 2. The shared contract (`brindle-transcript`, `brindle-remote-protocol`)

- [x] 2.1 Write the tests first, in `transcript/src/lib.rs`'s test module:
  - "`Event::encoded_len` equals `serde_json::to_vec(event).len()` for every fixture event and for random ones from `fuzz.rs`'s generator";
  - "`escaped_len` of a cleaned string is at most twice its bytes";
  - "`wire_len` of a diff counts 4 bytes a line and 64 a hunk";
  - "`wire_len` of an output counts `ITEM_LIMIT`, its command and its directory, whatever its text holds";
  - "`ContentBudget` keeps at most 32 items whose `wire_len`s add up to at most `CALL_LIMIT`, trying later items after one that doesn't fit, and counts the rest".
  - As `the_encoded_length_of_a_random_event_is_its_encodings` (and `the_encoded_length_of_every_fixture_event_is_its_encodings` in `tests/events.rs`), `cleaned_text_escapes_to_at_most_twice_its_bytes_and_cleaned_len_is_cleans` (which holds task 4.2's `cleaned_len` test too, flag 2), `the_wire_length_of_a_diff_counts_its_lines_and_hunks_framing`, `the_wire_length_of_an_output_is_its_limit_command_and_directory` and `a_content_budget_keeps_what_fits_and_counts_the_rest`. They named items 2.2 adds, so they failed by not compiling.
- [x] 2.2 Add `MAX_EVENT`, `MAX_PATH`, `LINE_FRAMING`, `HUNK_FRAMING`, `SIGNAL_RESERVE` and `ENTRY_FRAMING`; make `escaped_len` public; add `Event::encoded_len` (a counting writer, no allocation); replace `ToolItem::text_len` with `wire_len`; add `ContentBudget` and `ToolItem::bound`. Verify: `cargo test -p brindle-transcript`.
  - In `33cd838`; also `cleaned_len`, `Entry::kind` and `Event::kind`. `cargo test -p brindle-transcript`: 73 unit and 9 fixture tests pass.
- [x] 2.3 Add `ENTRY_FRAMING` to `Kept.size`, and make `replay_entry` group a tool entry's items into `ToolItems` pieces by the live split's rule. Tests first: "a replay of a tool entry of 32 small items sends one `ToolItems`"; "a fold of grouped pieces equals a fold of one item an event"; "a format 2 checkpoint written with one item an event still decodes to the same session" (`checkpoint.rs`'s existing fixtures). Verify: `cargo test -p brindle-transcript` and `cargo test -p brindle-remote-agent acp::checkpoint`.
  - `a_replay_groups_a_tool_entrys_items_by_the_live_split`, `a_fold_of_grouped_pieces_equals_a_fold_of_one_item_an_event` and `checkpoint.rs`'s `a_format_2_checkpoint_written_with_one_item_an_event_still_decodes` (the session's base rewritten with its grouped `tool_items` split one an event; `checkpoint.rs` keeps no fixture files, so the test builds the old form). The existing replay test became `a_tool_entry_replays_without_content_then_its_items_grouped`, and the tests that measured the log add `ENTRY_FRAMING`. `acp::checkpoint`: 63 pass.
- [x] 2.4 In `remote-protocol/src/client.rs`, make the envelopes public as `TRANSCRIPT_ENVELOPE` and `AGENT_PAGE_ENVELOPE`, used by `pack_transcript` and `pack_page`. Test first: "each envelope constant equals the encoded length of an empty batch with the largest session id". Verify: `cargo test -p brindle-remote-protocol`, and `cargo test -p brindle-remote-protocol --test transcripts` passes with the fixtures unchanged.
  - `each_envelope_is_an_empty_batchs_length_with_the_largest_ids` (and a `const` assertion that the page's is no smaller). 146 unit and 4 fixture tests pass; `git diff --stat` shows no fixture changed.

## 3. The fold's clamps and the hunk check (B-7, D-8)

- [x] 3.1 Write the tests first, in `transcript/src/fuzz.rs`:
  - "after any sequence of random events, some past every bound, no tool entry holds content past its bounds";
  - "clamping is the identity on content within the bounds, and a clamped entry clamps to itself";
  - "folding a tool call's pieces equals folding their concatenation, with content past the bounds";
  - "a hunk value that starts at 0, or whose lines would be numbered past `u32::MAX`, is a decode error and never a panic".
  - As `the_fold_never_holds_tool_content_past_its_bounds`, `clamping_is_the_identity_within_the_bounds_and_idempotent`, `folding_a_tool_calls_pieces_equals_folding_their_concatenation_past_the_bounds` and `a_hunk_near_the_edges_decodes_numbered_within_u32_or_is_refused`, over generators of content past every bound (`big_tool` and its parts). The random items' hunks start from 1, as the producer's do, so the logs they build encode and decode; the edge starts are the last test's.
- [x] 3.2 Clamp in `Transcript::apply`'s `Added` (tool), `ToolChanged` and `ToolItems` arms, through `ToolItem::bound` and `ContentBudget`, raising `more` by what was dropped; cut locations, paths, `input` and `result`. Verify: task 1.2's test and `cargo test -p brindle-transcript fuzz`.
  - `Entry::bound_tool`, called by the three arms before the entry is measured; `ToolOutput`'s exit is clamped too (flag 6). Task 1.2's test passes; `fuzz`: all pass.
- [x] 3.3 Decode `Hunk` through a private `HunkForm` (`#[serde(try_from)]`) that refuses design.md decision 7's values. Verify: `cargo test -p brindle-transcript`; `cargo test -p brindle-transcript --test events` passes with `tests/fixtures/v4/` unchanged.
  - The errors read "a hunk's old side starts at line 0; lines count from 1" and "a hunk's new side starts at line S and numbers N lines, past line 4294967295". `--test events`: 9 pass, fixtures unchanged.

## 4. The reducer's bounds (`brindle-remote-agent`, pure)

- [x] 4.1 Create `remote-agent/src/acp/limits.rs`:
  - the invariants' doc and design.md's table of bounds;
  - `ENTRY_LIMIT`, `FIELD_LIMIT`, `MAX_OPTIONS`, `MAX_OPTION_ID`, `MAX_OPTION_NAME`, `MAX_TERMINAL_ID`, `MAX_SESSION_ID`;
  - `LOG_LIMIT`, `LOG_TRIM_TO` and `MAX_PROMPT` moved from `reduce.rs`, re-exported there;
  - `STATE_SLACK`, and the `const` assertions of decision 1's sums (`MAX_EVENT + AGENT_PAGE_ENVELOPE ≤ MAX_FRAME_PAYLOAD_SIZE`; the largest entry ≤ `LOG_TRIM_TO`; the checkpoint sum ≤ `MAX_CHECKPOINT_LEN`).

  Add it to `PURE_MODULES` in `tests/architecture.rs` and to the whole pure modules table in `docs/architecture/functional-core-imperative-shell.md`. Verify: `cargo build -p brindle-remote-agent` and `cargo test -p brindle --test architecture`.
  - The option bounds are `MAX_PERMISSION_OPTIONS`, `MAX_PERMISSION_ID` and `MAX_PERMISSION_NAME`, since `agents-commands` already has `MAX_OPTIONS` and `MAX_OPTION_*` for session options (flag 1). The `const` assertions also hold each row's largest event within `MAX_EVENT`. Builds; the architecture test passes (51), `PURE_MODULES` at 69.
- [x] 4.2 (`cleaned_len`, the diff) Tests first:
  - "`cleaned_len(t)` equals `clean(t).len()` for random strings with controls" (`reduce.rs`);
  - "a 300,000-line empty diff keeps at most 16,384 lines, counts the rest as omitted, and allocates no line it doesn't keep" (`diff.rs`; the last part through `Keep`'s count of lines cleaned);
  - "both texts past `MAX_DIFF_LINES` give one hunk within `ITEM_LIMIT` counted by `wire_len`".

  Then change `Keep::push` to measure before it allocates, and drop `tool_content`'s `clean_line` loop. Verify: `cargo test -p brindle-remote-agent acp::diff` and `acp::reduce`.
  - `cleaned_len` lives beside `clean` in `brindle-transcript` (flag 2), tested there. `diff.rs`'s `a_300000_line_empty_diff_keeps_what_fits_and_allocates_no_more`, `both_texts_past_the_guard_give_one_hunk_within_the_item_limit` and `a_compared_diffs_lines_are_cleaned` failed against today's `Keep` with a counter added (300,000 lines kept; controls not cleaned) and pass. `unrelated_large_texts_finish_and_are_cut_at_the_item_limit` now sums `Hunk::wire_len`. Task 1.4's diff test passes.
- [x] 4.3 (`text_run`, A-1) Tests first, in `reduce.rs`:
  - "a 2.5 MiB reply streamed in 32 KiB chunks becomes three agent entries of at most `ENTRY_LIMIT`, whose texts joined are the reply, each cut at a line break when one lies in the room";
  - "a 1 MiB reply stays one entry" (today's scenario);
  - "a shown `session/load` replay of a 1.5 MiB user message gives two user entries";
  - "a 20 MiB thought leaves the log within `LOG_LIMIT` after every step and `checkpoint::encode` is `Ok`".

  Then route `chunk` and `add_text` through `text_run`. Verify: task 1.4's third test and `cargo test -p brindle-remote-agent acp::reduce`.
  - `a_long_reply_continues_in_new_entries_cut_at_line_breaks` (also with 30,000-byte chunks, flag 7), `a_1_mib_reply_stays_one_entry`, `a_shown_replay_of_a_long_user_message_gives_two_user_entries`, and task 1.4's thought test with `checkpoint::encode` added. The first, third and fourth failed (one entry; a 16,777,308-byte log), and pass. The real-role `a_log_past_16_mib_drops_its_oldest_entries` sent messages of 1 MiB + 3 bytes, which now split; its messages are exactly 1 MiB.
- [x] 4.4 (A-2, the fields) Tests first, in `reduce.rs`:
  - "a permission with 20 options of 64 KiB names keeps 16, names cut at 4 KiB, and a notice says 4 were left out";
  - "an option whose id is 2 KiB, empty or holds `\u{1b}` is left out, and a request left with none is answered `cancelled`";
  - "a terminal id of 2 KiB is ignored, its output not kept";
  - "40 locations of 64 KiB paths keep 32, each cut at `MAX_PATH`";
  - "a fourth full output in one call is counted in `more`, not opened";
  - "a `session/new` answer with a 2 KiB session id fails the start saying why, and nothing is sent in its name";
  - "a stop reason named with 100 KiB is cut to `FIELD_LIMIT`";
  - "the piece split counts an output's command and directory".

  Then bound `permission`, `tool_detail`, `tool_content`, `open_output`, `describe_output`, `created` and `prompted` as design.md decision 3 says. Verify: task 1.4's first two tests and `cargo test -p brindle-remote-agent acp::reduce`.
  - `a_permission_keeps_16_options_their_names_cut`, `an_option_whose_id_cant_be_sent_back_is_left_out`, `a_terminal_id_past_its_bound_is_ignored`, `locations_keep_32_their_paths_cut`, `a_fourth_output_in_one_call_is_counted_in_more`, `a_session_id_past_its_bound_fails_the_start` (2 KiB, empty, a control character), `a_long_stop_reasons_name_is_cut` and `the_piece_split_counts_an_outputs_command_and_directory`. All but the last failed on this branch before the bounds. The piece split was already fixed by `33cd838`'s `text_len` → `wire_len` rename, which the fold needed, so it was shown failing at `486f6e6` in a scratch checkout (`[2]`: one piece) and passes here (`[1, 1]`). Task 1.4's first two tests pass.
- [x] 4.5 (the emit guard, C2) Tests first, in `reduce.rs`, with a test-only seam that lowers the guard's limit (a `Step` field set by the test constructor, not an environment variable, `testing R7`):
  - "an `Added` past the limit is not applied, and the session fails with `ACP handling failed: a added (tool) event for entry N is L bytes; one transcript event may be at most M`", the whole message compared;
  - "an `Appended` past the limit fails the session naming `appended`, its entry and its size";
  - "a `Queue` event past the limit fails the session naming the session's queue event";
  - "after the oversized event the step emits only `fail`'s events, and a client's fold of the step's events equals the server's log".

  Then add the measure to `emit` (design decision 4): an oversized event sets `fault` with its reason and isn't applied. `emit`'s and `add`'s signatures don't change. In `server/core/agents.rs`, make `agent_apply` log `session {id}: {message}` when a step's events hold `AgentState::Failed`, with a core test that a fold error's reason reaches the log. Verify: `cargo test -p brindle-remote-agent acp::reduce server::core`.
  - The seam is `Step::event_limit`, set by a private `run(session, input, now, limit)` that `step` calls with `MAX_EVENT` and the test `Agent` with its `event_limit` (flag 14). `an_added_event_past_the_limit_fails_the_session_naming_it`, `an_appended_event_past_the_limit_fails_the_session_naming_it` and `a_queue_event_past_the_limit_fails_the_session_naming_it` compare the whole message and the step's shape (`turn failed`, `agent failed`; the test `Agent` checks every step's client fold against the log). The message takes "an" before a vowel (`an added (tool) event`, flag 9). They failed (the events applied) and pass. The log line is a pure `failure_line` in `agents.rs` (flag 13), tested by `a_failure_in_a_steps_events_reaches_the_log`, which failed to compile before it existed. `acp::reduce` and `server::core` pass.
- [ ] 4.6 (B-2) Tests first, in `v1/decode.rs`:
  - "a `rawInput` of 120 nested arrays around 200,000 zeros decodes to at most `RAW_LIMIT + 4` bytes, and the capped writer is offered at most its limit plus one write";
  - "a 1 MiB raw string is cut at `RAW_LIMIT + 4` at a character boundary";
  - "a raw value within the limit decodes exactly as before" (the existing raw tests, unchanged).

  In `reduce.rs`: "`raw_text` of a long value holding controls ends with `…` within `RAW_LIMIT`". Then write `Capped` and change `raw` and `raw_text`. Verify: `cargo test -p brindle-remote-agent acp::v1` and `acp::reduce`.

## 5. The invariants' tests

- [ ] 5.1 In `acp/limits.rs`'s tests, write `the_frame_and_checkpoint_bounds_follow_from_the_constants` (design.md decision 8: each event at its largest, `"`-filled, within `MAX_EVENT`; the largest session's `checkpoint::encode` is `Ok`; replay plus map at most twice `encoded_len()` for each entry kind's smallest form). Verify: `cargo test -p brindle-remote-agent acp::limits`.
- [ ] 5.2 In the same module, write `every_event_fits_a_frame_over_adversarial_agent_output`: 2,000 seeded steps over design.md decision 8's generators, asserting its seven properties after every step and `checkpoint::encode` every 100 steps. Verify: `cargo test -p brindle-remote-agent acp::limits`. It must run within the debug test suite's usual time (under 30 s); if not, cut the step count and say so in the test's doc.

## 6. The core's comments and send sites (shell)

- [ ] 6.1 Rewrite the comments that claimed the invariants so they cite `acp::limits` I1 and I2: `Agent.unstorable` and `agent_checkpoint` (`server/core/agents.rs`), `send_transcript`, `pack_events`, `checkpoint.rs`'s **Bounds** paragraph, `replay_entry`, `ITEM_LIMIT`. No behaviour changes. Verify: `cargo test -p brindle-remote-agent` and `cargo clippy --locked --workspace --all-targets -- -D warnings`.

## 7. The client (`brindle`)

- [x] 7.1 Number `diff_rows`' lines with `checked_add`, leaving a side unnumbered once it overflows. Verify: task 1.3's test and `cargo test -p brindle agent::tool`.
  - In `33cd838`. Task 1.3's test passes, with `agent::tool`'s others (`--bin brindle`).

## 8. Cost and the whole

- [ ] 8.1 Before task 4.5, run `cargo build --release && BRINDLE_PERF=1 scripts/with-xvfb cargo test --test perf -- agent-markdown-stream agent-tool-output` and keep the run directory. Run it again after task 4.5 with `BRINDLE_PERF_BASELINE=<that dir>`. Record both `[stream]` tables' times in design.md decision 4 (`testing R10`). If the change is past the runs' noise, apply decision 4's fallback and run again.
- [ ] 8.2 Run `cargo test --workspace --locked`, `cargo clippy --locked --workspace --all-targets -- -D warnings` and `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- agent` (under `systemd-run --user --scope`). Verify: all pass, and the fixtures under `transcript/tests/fixtures/v4/` and `remote-protocol/tests/fixtures/` are unchanged (`git diff --stat` shows none).

## 9. Docs

- [ ] 9.1 Update `docs/code-map.md`: the transcript entry (the fold's clamps, `MAX_EVENT`, `wire_len`, `ContentBudget`, the grouped replay, `ENTRY_FRAMING`); the reducer entry (`acp/limits.rs`, `text_run`, the emit guard, the option and id bounds, `Capped`).
- [ ] 9.2 Update `.claude/rules/transcript.md`: which bounds the fold enforces (the tool content bounds) and which the producer does (entry size, ids, options, other fields).
- [ ] 9.3 Verify: `openspec validate agents-event-bounds --strict` passes, and each task above is checked.

## Workflow follow-up

- Archive with `openspec archive agents-event-bounds` in the implementation PR, syncing the delta specs.
