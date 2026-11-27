# Tasks

Tests come first in each group, written as sentences (`testing R15`). No test sleeps (`testing R9`). Verify commands run from the repository root.

## 1. Re-open the sites and show the plausible findings

- [ ] 1.1 Once `agents-commands` (PR #151) is on `main`, rebase and re-open every site design.md's Context cites (`reduce.rs` `emit`, `add`, `chunk`, `add_text`, `tool_detail`, `tool_content`, `permission`, `created`, `prompted`, `raw_text`, `trim`; `diff.rs` `Keep`; `v1/decode.rs` `raw`, `requested_option`, `new_session_result`, `prompt_result`; `transcript/src/lib.rs` `apply`, `trim_to`, `replay_entry`; `client.rs` `pack_events`; `src/agent/tool.rs` `diff_rows`). Correct the line numbers in design.md and add a derivation row for each event `agents-commands` added (its command and option lists). Verify: design.md's table names every `Event` variant on `main`.
- [ ] 1.2 (B-7, shown first) In `transcript/src/lib.rs`'s tests, add the test "a fold of a tool call update carrying 40 items and a 100 KiB raw input holds at most 32 items and a raw input of at most 16 KiB". Run it at the rebased commit and record that it fails. Verify: `cargo test -p brindle-transcript a_fold_of_a_tool_update_past_the_bounds` fails before task 3.2 and passes after. If it passes before, B-7 is dropped and the rule text alone is corrected.
- [ ] 1.3 (D-8, shown first) In `src/agent/tool.rs`'s tests, add the test "the rows of a hunk starting at `u32::MAX` with two context lines are drawn without a panic, the second line unnumbered". Record that it panics in a debug build. Verify: `cargo test -p brindle agent::tool::a_hunk_at_the_last_line_number` panics before task 7.1 and passes after. If it doesn't panic, record that and keep task 3.3's decoder check alone.
- [ ] 1.4 (A-1 and A-2, shown first) In `remote-agent/src/acp/reduce.rs`'s tests, add the tests "a 300,000-line empty diff makes no event past `MAX_EVENT`", "a permission with 20 options named with 64 KiB each makes no event past `MAX_EVENT`", and "a 20 MiB thought streamed in 64 KiB chunks leaves the log within `LOG_LIMIT` after every step". Record that each fails at the rebased commit. Verify: `cargo test -p brindle-remote-agent acp::reduce::tests::` names the three failing.

## 2. The shared contract (`brindle-transcript`, `brindle-remote-protocol`)

- [ ] 2.1 Write the tests first, in `transcript/src/lib.rs`'s test module:
  - "`Event::encoded_len` equals `serde_json::to_vec(event).len()` for every fixture event and for random ones from `fuzz.rs`'s generator";
  - "`escaped_len` of a cleaned string is at most twice its bytes";
  - "`wire_len` of a diff counts 4 bytes a line and 64 a hunk";
  - "`wire_len` of an output counts `ITEM_LIMIT`, its command and its directory, whatever its text holds";
  - "`ContentBudget` keeps at most 32 items whose `wire_len`s add up to at most `CALL_LIMIT`, trying later items after one that doesn't fit, and counts the rest".
- [ ] 2.2 Add `MAX_EVENT`, `MAX_PATH`, `LINE_FRAMING`, `HUNK_FRAMING`, `SIGNAL_RESERVE` and `ENTRY_FRAMING`; make `escaped_len` public; add `Event::encoded_len` (a counting writer, no allocation); replace `ToolItem::text_len` with `wire_len`; add `ContentBudget` and `ToolItem::bound`. Verify: `cargo test -p brindle-transcript`.
- [ ] 2.3 Add `ENTRY_FRAMING` to `Kept.size`, and make `replay_entry` group a tool entry's items into `ToolItems` pieces by the live split's rule. Tests first: "a replay of a tool entry of 32 small items sends one `ToolItems`"; "a fold of grouped pieces equals a fold of one item an event"; "a format 2 checkpoint written with one item an event still decodes to the same session" (`checkpoint.rs`'s existing fixtures). Verify: `cargo test -p brindle-transcript` and `cargo test -p brindle-remote-agent acp::checkpoint`.
- [ ] 2.4 In `remote-protocol/src/client.rs`, make the envelopes public as `TRANSCRIPT_ENVELOPE` and `AGENT_PAGE_ENVELOPE`, used by `pack_transcript` and `pack_page`. Test first: "each envelope constant equals the encoded length of an empty batch with the largest session id". Verify: `cargo test -p brindle-remote-protocol`, and `cargo test -p brindle-remote-protocol --test transcripts` passes with the fixtures unchanged.

## 3. The fold's clamps and the hunk check (B-7, D-8)

- [ ] 3.1 Write the tests first, in `transcript/src/fuzz.rs`:
  - "after any sequence of random events, some past every bound, no tool entry holds content past its bounds";
  - "clamping is the identity on content within the bounds, and a clamped entry clamps to itself";
  - "folding a tool call's pieces equals folding their concatenation, with content past the bounds";
  - "a hunk value that starts at 0, or whose lines would be numbered past `u32::MAX`, is a decode error and never a panic".
- [ ] 3.2 Clamp in `Transcript::apply`'s `Added` (tool), `ToolChanged` and `ToolItems` arms, through `ToolItem::bound` and `ContentBudget`, raising `more` by what was dropped; cut locations, paths, `input` and `result`. Verify: task 1.2's test and `cargo test -p brindle-transcript fuzz`.
- [ ] 3.3 Decode `Hunk` through a private `HunkForm` (`#[serde(try_from)]`) that refuses design.md decision 7's values. Verify: `cargo test -p brindle-transcript`; `cargo test -p brindle-transcript --test events` passes with `tests/fixtures/v4/` unchanged.

## 4. The reducer's bounds (`brindle-remote-agent`, pure)

- [ ] 4.1 Create `remote-agent/src/acp/limits.rs`:
  - the invariants' doc and design.md's table of bounds;
  - `ENTRY_LIMIT`, `FIELD_LIMIT`, `MAX_OPTIONS`, `MAX_OPTION_ID`, `MAX_OPTION_NAME`, `MAX_TERMINAL_ID`, `MAX_SESSION_ID`;
  - `LOG_LIMIT`, `LOG_TRIM_TO` and `MAX_PROMPT` moved from `reduce.rs`, re-exported there;
  - `STATE_SLACK`, and the `const` assertions of decision 1's sums (`MAX_EVENT + AGENT_PAGE_ENVELOPE ≤ MAX_FRAME_PAYLOAD_SIZE`; the largest entry ≤ `LOG_TRIM_TO`; the checkpoint sum ≤ `MAX_CHECKPOINT_LEN`).

  Add it to `PURE_MODULES` in `tests/architecture.rs` and to the whole pure modules table in `docs/architecture/functional-core-imperative-shell.md`. Verify: `cargo build -p brindle-remote-agent` and `cargo test -p brindle --test architecture`.
- [ ] 4.2 (`cleaned_len`, the diff) Tests first:
  - "`cleaned_len(t)` equals `clean(t).len()` for random strings with controls" (`reduce.rs`);
  - "a 300,000-line empty diff keeps at most 16,384 lines, counts the rest as omitted, and allocates no line it doesn't keep" (`diff.rs`; the last part through `Keep`'s count of lines cleaned);
  - "both texts past `MAX_DIFF_LINES` give one hunk within `ITEM_LIMIT` counted by `wire_len`".

  Then change `Keep::push` to measure before it allocates, and drop `tool_content`'s `clean_line` loop. Verify: `cargo test -p brindle-remote-agent acp::diff` and `acp::reduce`.
- [ ] 4.3 (`text_run`, A-1) Tests first, in `reduce.rs`:
  - "a 2.5 MiB reply streamed in 32 KiB chunks becomes three agent entries of at most `ENTRY_LIMIT`, whose texts joined are the reply, each cut at a line break when one lies in the room";
  - "a 1 MiB reply stays one entry" (today's scenario);
  - "a shown `session/load` replay of a 1.5 MiB user message gives two user entries";
  - "a 20 MiB thought leaves the log within `LOG_LIMIT` after every step and `checkpoint::encode` is `Ok`".

  Then route `chunk` and `add_text` through `text_run`. Verify: task 1.4's third test and `cargo test -p brindle-remote-agent acp::reduce`.
- [ ] 4.4 (A-2, the fields) Tests first, in `reduce.rs`:
  - "a permission with 20 options of 64 KiB names keeps 16, names cut at 4 KiB, and a notice says 4 were left out";
  - "an option whose id is 2 KiB, empty or holds `\u{1b}` is left out, and a request left with none is answered `cancelled`";
  - "a terminal id of 2 KiB is ignored, its output not kept";
  - "40 locations of 64 KiB paths keep 32, each cut at `MAX_PATH`";
  - "a fourth full output in one call is counted in `more`, not opened";
  - "a `session/new` answer with a 2 KiB session id fails the start saying why, and nothing is sent in its name";
  - "a stop reason named with 100 KiB is cut to `FIELD_LIMIT`";
  - "the piece split counts an output's command and directory".

  Then bound `permission`, `tool_detail`, `tool_content`, `open_output`, `describe_output`, `created` and `prompted` as design.md decision 3 says. Verify: task 1.4's first two tests and `cargo test -p brindle-remote-agent acp::reduce`.
- [ ] 4.5 (the emit guard, C2) Tests first, in `reduce.rs`, with a test-only seam that lowers the guard's limit (a `Step` field set by the test constructor, not an environment variable, `testing R7`):
  - "an `Added` past the limit is not applied, and the session fails with `ACP handling failed: a added (tool) event for entry N is L bytes; one transcript event may be at most M`", the whole message compared;
  - "an `Appended` past the limit fails the session naming `appended`, its entry and its size";
  - "a `Queue` event past the limit fails the session naming the session's queue event";
  - "after the oversized event the step emits only `fail`'s events, and a client's fold of the step's events equals the server's log".

  Then add the measure to `emit` (design decision 4): an oversized event sets `fault` with its reason and isn't applied. `emit`'s and `add`'s signatures don't change. In `server/core/agents.rs`, make `agent_apply` log `session {id}: {message}` when a step's events hold `AgentState::Failed`, with a core test that a fold error's reason reaches the log. Verify: `cargo test -p brindle-remote-agent acp::reduce server::core`.
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

- [ ] 7.1 Number `diff_rows`' lines with `checked_add`, leaving a side unnumbered once it overflows. Verify: task 1.3's test and `cargo test -p brindle agent::tool`.

## 8. Cost and the whole

- [ ] 8.1 Before task 4.5, run `cargo build --release && BRINDLE_PERF=1 scripts/with-xvfb cargo test --test perf -- agent-markdown-stream agent-tool-output` and keep the run directory. Run it again after task 4.5 with `BRINDLE_PERF_BASELINE=<that dir>`. Record both `[stream]` tables' times in design.md decision 4 (`testing R10`). If the change is past the runs' noise, apply decision 4's fallback and run again.
- [ ] 8.2 Run `cargo test --workspace --locked`, `cargo clippy --locked --workspace --all-targets -- -D warnings` and `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- agent` (under `systemd-run --user --scope`). Verify: all pass, and the fixtures under `transcript/tests/fixtures/v4/` and `remote-protocol/tests/fixtures/` are unchanged (`git diff --stat` shows none).

## 9. Docs

- [ ] 9.1 Update `docs/code-map.md`: the transcript entry (the fold's clamps, `MAX_EVENT`, `wire_len`, `ContentBudget`, the grouped replay, `ENTRY_FRAMING`); the reducer entry (`acp/limits.rs`, `text_run`, the emit guard, the option and id bounds, `Capped`).
- [ ] 9.2 Update `.claude/rules/transcript.md`: which bounds the fold enforces (the tool content bounds) and which the producer does (entry size, ids, options, other fields).
- [ ] 9.3 Verify: `openspec validate agents-event-bounds --strict` passes, and each task above is checked.

## Workflow follow-up

- Archive with `openspec archive agents-event-bounds` in the implementation PR, syncing the delta specs.
