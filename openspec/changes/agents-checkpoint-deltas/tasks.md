# Tasks

Each group writes its tests first and watches them fail before the code they check. Test names are sentences (`testing R15`). Nothing sleeps, retries or asserts on elapsed time (`testing R7`–`R9`), and every generator has a fixed seed. "Design N" means decision N of `design.md`.

## 1. Re-open the sites and take the numbers before

- [x] 1.1 Re-open the cited sites on `main`, after `agents-commands` (PR #151) and changes 3, 4 and 5 (`agents-event-bounds`, `agents-process-lifecycle`, `agents-module-structure`) have merged. Find where each item now lives:
  - `state_fields`, `encode_segment`, `decode_2`, `restore` and `push_record`, which may be in `checkpoint/{format,state,adopt}.rs`;
  - `Output::append`, which may be in `transcript/src/output.rs`;
  - `tools`, `outputs`, `respawn` and `trim`, which may be in `ToolBook` or `reduce/tools.rs`;
  - `raw`, as change 3 left it in `v1/decode.rs`;
  - change 4's "agent gone" step;
  - `agents-commands`' state fields.

  Update `design.md`'s Layer map and decisions 2, 4, 10 and 12 with the paths and names as found, and classify each state field `agents-commands` added by decision 4's rule. Verify by `grep` that every function the design names exists, and record the commit you read in the design's As built.

  Done: read at `ad696ca` (#157's head, before it merged; flag 1); the Layer map lists the sites, decisions 2, 4, 10 and 12 name them, and `option_keys` and `modes` stay whole.
- [x] 1.2 In `remote-agent/src/server/core/policy.rs`'s `mod tests`, add `checkpoints_of_a_tool_heavy_conversation_and_a_long_line_write_a_small_multiple_of_what_they_added` (design 7). It uses the existing `Checkpointing` helper, extended to count `records` and to check each appended segment against `RECORD_ALLOWANCE` (1 KiB) plus its merged events' encoded length plus the partial's new bytes. Run it on the unchanged code with `cargo test -p brindle-remote-agent --release server::core::policy::tests::checkpoints_of_a_tool_heavy -- --nocapture`, watch it fail on assertion 1, and record the printed `written`, `added` and `records` under "As built (before)" in `design.md`. To get the total before the per-segment assertion stops the run, run it once with that assertion commented out locally; don't commit that edit.

  Done (4e91d6f): it fails at segment 8 (1,844 bytes for 793 of events); without assertion 1, 2,123,919,660 bytes written for 13,072,007 added in 11,018 records (flag 4 on the events' measure).
- [x] 1.3 Add the measurement runs to the perf suite before any production change (design 15):
  - `tests/perf/agent.rs`:
    - `tool_output` gains the `small` run: 1 MiB in 32,768 `terminal_output_delta`s of 32 bytes, written through `output_script`. `[stream]` gains `small_ms`.
    - New `tool_heavy(ctx)`: `tool_heavy_script(record, gate)` sends 3,000 tool calls behind the gate, each a `tool_call`, a `session/request_permission`, a `$/cancel_request` for it and a completing `tool_call_update`. Then a final `agent_message_chunk`, then one 8 MiB `tool_call_update` line written as 128 `send-raw` pieces of 64 KiB and its newline. It measures from the gate to Brindle's exit on a `--wait-for` of the final message, reporting a `[tool_heavy]` table, and skips without the fake agent.
    - `self_test_tool_heavy`, which checks the script's step count and that the pieces join to one line.
  - `tests/perf/main.rs`: register the case `agent-tool-heavy`, and `self_test_tool_heavy` in `self_test()`.

  Verify that `BRINDLE_PERF=1 cargo test --test perf -- --self-test` passes. Then take the baseline on a release build of the unchanged code with `cargo build --release && scripts/with-xvfb env BRINDLE_PERF=1 cargo test --test perf -- agent-`, and record the run directory and the `agent-attach-long`, `agent-tool-output` and `agent-tool-heavy` numbers in `design.md`'s As built.

  Done (a713e328): self-test passes; baseline run `target-cd/tmp/perf/20261128T071449Z` (flags 2 and 3: `send-part`, the last message after the line).

## 2. Shared contracts: the transcript's output text and replay, the protocol's packers

- [x] 2.1 Write the tests first in `transcript/src/lib.rs`'s `mod tests` (or `output.rs` after change 5) and watch them fail (`cargo test -p brindle-transcript`):
  Done: the two output tests in `output.rs`, the replay test in `page.rs`; they failed to build (no `OutputText`, no `replay_ref`).
  - `an_output_fed_a_mebibyte_in_32_byte_pieces_keeps_its_buffer_within_twice_the_limit`: it checks `OutputText`'s buffer length after every append.
  - `output_text_compares_encodes_and_clones_as_its_visible_text`: equality with `&str`, `serde_json::to_string`, `Debug`, a clone holding no dropped prefix, and `Deserialize` from a string.
  - `replaying_by_reference_encodes_the_same_bytes_as_replay`: over 500 `fuzz::random_transcript`s, plus trimmed and windowed ones, `to_vec(&t.replay_ref()) == to_vec(&t.replay())`.

  The existing `output_appended_in_pieces_folds_as_appended_whole`, `output_appends_and_keeps_whole_lines_within_the_limit` and `fuzz.rs`' tests must keep passing unchanged.
- [x] 2.2 Implement `OutputText` and the new `Output::append` returning the escaped length of what it dropped (design 10). Change the fold's `Event::ToolOutput` arm to use that length. Implement `Transcript::replay_ref` with the private `EventRef` mirror (design 8). In the same commit, update every reader so the workspace builds: `src/agent/tool.rs` (`output.text.as_str()`, and `.to_string()` where a `String` is wanted), `src/agent/model.rs`, `transcript/src/fuzz.rs`, and the `reduce.rs` and `checkpoint.rs` tests. Verify with `cargo test -p brindle-transcript`, `cargo test -p brindle agent::`, `cargo test -p brindle-remote-agent acp::`, and that `transcript/tests/events.rs`' fixture tests pass with no fixture changed (`git status transcript/tests/fixtures` clean).
  Done: d04e785b; `replay_ref` walks the replay rather than mirroring `Event` (flag 5); fixtures unchanged, 76 transcript, 260 `agent::` and 440 `acp::` tests pass.
- [x] 2.3 Write the protocol tests first and watch them fail (`cargo test -p brindle-remote-protocol`):
  - `holder.rs`: `sliced_checkpoint_frames_are_the_bytes_of_the_checkpoint_messages`, for `BlobForm::Whole` and `Append`, for blobs of 0, 1, `CHUNK_LEN` and `3 × CHUNK_LEN + 7` bytes. Each `SlicedFrame`'s type, length, head and body must equal `ToHolder::checkpoint(…)` or `checkpoint_append(…)`'s `encode()`.
  - `client.rs`:
    - `transcript_frames_of_encoded_events_are_the_bytes_of_pack_transcript`;
    - `page_frames_of_encoded_events_are_the_bytes_of_pack_page` (an empty page and a multi-frame page included);
    - `an_encoded_event_past_a_frame_is_refused_as_pack_transcript_refuses_it`;
    - `json_object_refuses_bytes_that_dont_start_and_end_as_an_object`.
  - `fuzz.rs`: random bytes through `JsonObject::new` and the packers never panic.

  Done: all six, failing to build before 2.4 (`encoded_objects_and_their_packers_never_panic_on_random_bytes` in `fuzz.rs`).
- [x] 2.4 Implement `holder::{BlobForm, SlicedFrame, checkpoint_frames}` and `client::{JsonObject, NotAnObject, transcript_frames, page_frames}` (design 8, 9). Each frame's envelope is written by hand, and the grouping is shared with `pack_events` through one sizing function. Update the module docs: `holder.rs` (the checkpoint can be queued from slices, with the same bytes) and `client.rs` (encoded events are packed without being read). Verify with `cargo test -p brindle-remote-protocol`, including `--test transcripts`, with no fixture changed.
  Done: 8978c0a7; 152 lib and 4 transcript tests pass, fixtures unchanged.

## 3. Pure reducer and codec: the tool epoch, the diff memo, owned arguments

- [ ] 3.1 Write the tests first in `remote-agent/src/acp/reduce.rs` (or `reduce/tools.rs`) and watch them fail (`cargo test -p brindle-remote-agent acp::reduce`):
  - `the_tool_index_is_always_the_logs_tool_entries_from_the_epoch`: a property over 300 seeded runs of 60 random steps (tool calls with repeated ids, updates, permissions, trims forced by long chunks, exits and restarts by `Input::Restart`), asserting `s.tools == tool_index(&s.transcript, s.tool_epoch)` after every step.
  - `a_restart_moves_the_tool_epoch_past_the_earlier_agents_entries`.
  - `the_memo_hands_back_the_kept_diff_only_for_the_same_path_and_texts_at_the_same_item`: tests the pure lookup `DiffMemo::reused(entry, index, digest, old) -> Option<ToolItem>` directly. There is no counter or seam around `diff::hunks` (`ports R6`).
  - `a_diff_repeated_unchanged_records_its_digest_and_emits_nothing`: after the first update the memo holds the item's digest, and the repeat, in the update and then in the permission's `toolCall`, emits no `ToolItems`.
  - `a_diff_whose_text_changed_is_diffed_again`.
  - `the_diff_memo_forgets_trimmed_entries_and_everything_at_a_restart`.
  - `a_restored_session_repeating_a_diff_emits_nothing`: decode a checkpoint, so the memo is empty, then repeat the diff.
  - The existing `a_permissions_tool_call_brings_its_content_but_never_its_title` and `a_permission_requests_title_falls_back_to_its_tool_calls` must stay green across the move of the call.
- [ ] 3.2 Implement `tool_epoch`, `tool_index`, `DiffMemo` (always-equal `PartialEq`, documented as design 11 says), the memo's use in `tool_content`, its pruning in `trim` and in the respawn or "agent gone" step, and the permission's call moved (design 2, 11). Verify with 3.1 and the whole `acp::reduce` module.
- [ ] 3.3 Make `Dialect`'s decoders take owned values (design 12): `dialect.rs`'s signatures, `v1/decode.rs`'s `split`, `tool_detail`, `tool_item`, `chunk`, locations and `raw`, and the reducer's `message`, `response` and per-purpose handlers. Add `v1/decode.rs` tests:
  - `a_tool_update_moved_into_the_decoder_keeps_every_field`;
  - `a_permission_request_moved_into_the_decoder_keeps_its_call_and_options`.

  Verify with `cargo test -p brindle-remote-agent acp::` and `cargo test -p brindle --test architecture` (`acp_versions_stay_in_their_modules`).

## 4. Pure checkpoint: the delta shape, earlier checkpoints refused, one buffer

- [ ] 4.1 Write the tests first in `remote-agent/src/acp/checkpoint.rs` (or `checkpoint/`) and watch them fail (`cargo test -p brindle-remote-agent acp::checkpoint`). The `Stored` test helper carries `Written`.
  - `a_segment_carries_no_tool_index_and_its_size_doesnt_grow_with_the_tool_calls`: segments at the 10th and the 2,000th tool call differ only by their events.
  - `a_long_line_is_written_once_across_its_segments_and_decodes_whole`.
  - `a_partial_line_that_ended_or_was_cut_resets_its_delta`: `Collecting`, then a newline and a new line, then `TooLong`, then `Resyncing`.
  - `methods_are_written_only_when_they_change_and_an_emptied_list_reads_empty`.
  - `outputs_are_patched_per_terminal_and_a_closed_one_is_removed`.
  - `a_restored_session_rebuilds_its_tool_index_from_the_log_and_epoch`.
  - `a_checkpoint_from_before_the_delta_shape_is_unreadable`: today's `PINNED` bytes are refused as `Unreadable::Field { name: "tool_epoch" }`, and a format-1 header as a format this server doesn't read (design 1).
  - `a_small_sessions_format_2_bytes_are_pinned`: `PINNED`, written again once from the new encoder.
  - `a_base_and_two_segments_are_pinned`: a new pinned chain, written once from the new encoder, with a partial line's delta and a `methods` change in its segments.
  - `a_segment_whose_partial_doesnt_continue_the_last_is_unreadable` (`PartialGap`), with its `Display` text checked in `the_new_refusals_read_as_what_went_wrong`.
  - `a_record_without_an_epoch_is_unreadable`.
  - `an_outputs_patch_closing_a_terminal_it_doesnt_hold_is_unreadable`.
  - `a_checkpoint_past_the_limit_is_refused_while_it_is_written`: the `Limited` adapter stops before `out` passes the limit.
  - `a_full_log_a_full_partial_line_and_a_full_prompt_fit_in_64_mib`, which already exists, must keep passing.
  - `random_steps_checkpointed_whole_or_appended_decode_to_the_live_session` and the tool-content round trips must keep passing with the new API, and gain restarts and long split lines among their random steps.
  - `fuzz_decoding_never_panics_on_random_or_mutated_checkpoints` gains a delta-shaped chain with a long partial line and an outputs patch to mutate.
- [ ] 4.2 Implement design 1 to 6 and 8 in `checkpoint.rs`:
  - `Written`, `LineMark`, `WrittenChanges`, `Encoded` and `Segment`;
  - `encode` and `encode_segment(session, events: Vec<Event>, seq, &Written)`;
  - `StateOut` streaming `ReplayRef` or the merged events through `Limited` into `out`;
  - `decode_2`'s delta fold, `restore`'s rebuilt index, the refusal of a record with no epoch, and `Unreadable::PartialGap`;
  - `decode_1`, `FORMAT_1` and their tests removed, and `Unreadable`'s format message naming format 2 alone.

  Rewrite the module doc's format section: the delta shape, that earlier checkpoints are refused, decision 4's table and its rule for new fields, and the `lines.from` delta. Verify with `cargo test -p brindle-remote-agent acp::`.

## 5. Pure policy: one fact for what is stored, the bound asserted

- [ ] 5.1 Change `policy::Stored::Known` to carry `written: Written` (no longer `Copy`): `Stored::whole(len, written)`, `Stored::appended(self, len, changes)`, and `agent_appendable` and `agent_checkpoint_form` taking `&Stored` (design 5). Update the policy tests that build `Stored` (`Written::default()` where the content doesn't matter), and `Checkpointing` in the tests. Verify that 1.2's test now passes with its three assertions, and that `checkpoints_of_a_64_mib_conversation_write_a_small_multiple_of_what_it_added` and every `agent_checkpoint_form` test pass (`cargo test -p brindle-remote-agent server::core::policy`). Record the printed after numbers in `design.md`'s As built.

## 6. Shell: the core encodes once and queues slices

- [ ] 6.1 Write the real-role tests first in `remote-agent/tests/agents.rs`, with the fake ACP agent and gates on files, no sleeps, and watch the first fail on the current shell (`cargo build -p brindle-remote-agent --bin brindle-fake-acp-agent && cargo test -p brindle-remote-agent --test agents`):
  - `a_core_killed_while_a_long_line_arrives_applies_the_line_once`: the agent writes a 2 MiB `tool_call_update` line in 64 KiB `send-raw` pieces, the core is checkpointed past 512 KiB at least twice, SIGKILL, the agent finishes the line, a new core adopts, and the client's copy holds the update once.
  - `tool_calls_after_a_restart_keep_their_entries_across_a_core_kill`: the spec scenario "Tool calls carried on after a core restart".
  - `a_restarts_first_checkpoint_is_whole_after_a_partial_line`: design Risks, checked through the holder's stored bytes after the restart.
- [ ] 6.2 In `server/core/agents.rs`:
  - `agent_apply` encodes each event once (`serde_json::to_vec` into `JsonObject`), sends `transcript_frames` through a new `Core::send_client_frame` (`clients.rs`), then moves the events into `unsaved`.
  - `agent_checkpoint` passes `std::mem::take(&mut agent.unsaved)` and `&Written` to `encode_segment`, applies the `Segment`'s changes or the `Encoded`'s `Written` to `Stored`, and queues `holder::checkpoint_frames` through a new `Core::send_holder_sliced` (`holders.rs`), which calls `Conn::queue_sliced` (`server/conn.rs`).
  - `send_transcript` and the attach page (`sessions.rs`) and `client_agent_fetch` use `transcript_frames` and `page_frames` with events encoded once.

  The module doc's Checkpoints paragraph says segments carry deltas. Verify with 6.1, `cargo test -p brindle-remote-agent --test agents`, `--test upgrade`, `--test core` and `--test holder`.

## 7. Docs and checks

- [ ] 7.1 Update the docs:
  - `docs/code-map.md`: the checkpoint's delta shape and tool epoch, the core's `Written` in `Stored`, the encode-once and sliced frames, `OutputText`, the diff memo.
  - `CLAUDE.md`: the perf suite's case list, adding `agent-tool-heavy` and `agent-tool-output`'s `small` run.
  - `docs/architecture/process-architecture.md`'s "How it applies here", if it describes segments as whole state.

  No architecture rule or exception changes. Verify by reading each changed paragraph against the code.
- [ ] 7.2 Run the workspace checks: `cargo test --workspace --locked`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, and `cargo test -p brindle --test architecture`.
- [ ] 7.3 Run the agent e2e cases headless, outside Brindle's scope: `systemd-run --user --scope scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- agent`. No case changes; they pin that nothing a user sees moved.
- [ ] 7.4 Run the perf cases after, against 1.3's baseline: `cargo build --release && scripts/with-xvfb env BRINDLE_PERF=1 BRINDLE_PERF_BASELINE=<1.3's run dir> cargo test --test perf -- agent-`. Record `agent-attach-long`, `agent-tool-output` (`streamed`, `whole`, `small`) and `agent-tool-heavy` before and after in `design.md`'s As built. They report; they don't pass or fail (`testing R14`).
- [ ] 7.5 Run `openspec validate agents-checkpoint-deltas --strict`.

## Workflow follow-up

- Archive the change in the implementation PR (`docs(openspec): archive agents-checkpoint-deltas`), syncing the `agent-sessions` delta into `openspec/specs/agent-sessions/spec.md`.
- Tell `agents-plans`' author (or its next writer) that `dismissed` must be written only when it changed under design 4's rule.
