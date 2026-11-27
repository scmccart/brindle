# Design

## Context

See `proposal.md` for why. The line numbers below are `main` at `5f335dc`, before `agents-commands` (PR #151) and changes 1–4 of the remediation plan land; task 1.1 re-reads them on the `main` this change is implemented against, and each table names where the earlier changes' additions go.

The files and their production sizes at `5f335dc`:

| File | Lines (production / total) | Test module from |
|---|---|---|
| `remote-agent/src/acp/reduce.rs` | 3,187 / 8,125 | 3188 |
| `remote-agent/src/acp/checkpoint.rs` | 1,242 / 3,102 | 1243 |
| `transcript/src/lib.rs` | 1,756 / 3,724 | 1757 |
| `src/agent/model.rs` | 3,293 / 5,914 | 3294 |
| `remote-agent/src/server/core/sessions.rs` | 2,041 / 2,451 | 2042 |
| `src/workspace.rs` (palette part) | about 330 (lines 391–408, 1456–1780) | — |
| `src/zone_view.rs` (login part, zone tree) | about 120 (1129–1230, 900) and 240 (55–270, 379–400) | 2250 |

Two facts shape every move:
- **Rust privacy follows the module tree.** A private item is visible in its module and all descendants. A struct whose definition stays in the parent (`reduce/mod.rs`'s `AgentSession` and `Step`, `transcript/src/lib.rs`'s `Transcript`, `model/mod.rs`'s `ChatModel`) keeps its private fields reachable from `impl` blocks in child modules. A method moved into a child and called from the parent or a sibling needs `pub(super)`, which is exactly the reach a private method of the parent has today. So the moves widen nothing.
- **A checkpoint's bytes and a transcript's bytes are fixed by format 2 and protocol 4** (`agent-sessions`, "Transcript event forms are fixed for each protocol version"; `process R4`–`R6`). A refactor that changes what a record holds in some phase is a format change, not a move.

## Goals / Non-Goals

**Goals:**
- Each file holds one concern (`ports R13`), with its tests in its own `mod tests` (`fcis R8`).
- The moves are reviewable as moves: one commit per source file, and `git diff --color-moved=dimmed-zebra --color-moved-ws=allow-indentation-change` shows nothing but moved lines, `mod` lines, `use` lines and `pub(super)` / `pub(crate)` markers.
- The type changes the plan lists are each one commit with their tests written first.
- `tests/architecture.rs` checks what the cuts make checkable.

**Non-Goals:**
- Any behaviour, wire, checkpoint, transcript, config, state or dump change.
- Giving each model module its own fields (`d.md` observation 1's "each owns its fields and returns its own `Absorbed` part"): decision 20.
- The serde mirror of the checkpoint state (`b.md` observation 1): decision 18.
- `chose`, `lists` and `methods` moved into the phase and greeting (`a.md` observation 5): decision 19.
- The client's single fold (`d.md` observation 2): the plan's decision D1 keeps the reader's fold.

## Layers

| Part | Layer | Crate, file |
|---|---|---|
| `reduce/` and its modules, `ToolBook`, `Process`, `Output::then`, `Moment::wins` | pure (whole pure modules) | `brindle-remote-agent`, `src/acp/reduce/` |
| `checkpoint/` and its modules | pure (whole pure modules) | `brindle-remote-agent`, `src/acp/checkpoint/` |
| `forms::Failure`, `Dialect::failure`, `v1::decode::failure` and the two codes | pure, ACP version seam (`ports R14`) | `brindle-remote-agent`, `src/acp/{forms,dialect}.rs`, `src/acp/v1/decode.rs` |
| `policy::agent_request`, `Gated` | pure policy | `brindle-remote-agent`, `src/server/core/policy.rs` |
| `handle_client`'s gate, `checkpoint_screen`, `checkpoint_agent`, the restart lifecycle | shell (the core's loop handlers) | `brindle-remote-agent`, `src/server/core/{clients,sessions,agents}.rs` |
| `event.rs`, `fold.rs`, `page.rs`, `output.rs` | pure (whole crate, `fcis R4`) | `brindle-transcript` |
| `model/` and its modules, `leave_on_quit`, `replay_ended` | GPUI-free model (whole pure modules) | `brindle`, `src/agent/model/` |
| `conversations.rs`'s spelled-out arms | GPUI-free model | `brindle`, `src/agent/conversations.rs` |
| `Hold` | shell (the `Agent` entity) | `brindle`, `src/agent/mod.rs` |
| `zone_tree.rs` | pure (new whole pure module) | `brindle`, `src/zone_tree.rs` |
| `workspace/conversations.rs`, `zone_view/login.rs` | shell (entity `impl` blocks) | `brindle` |

No crate gains a dependency, and no reference changes direction (`ports R1`, `R3`). No platform `cfg` moves: the cut ranges of `workspace.rs`, `zone_view.rs` and `sessions.rs` hold none (`zone_view.rs`'s are at 720, 1327 and 1335; `sessions.rs`'s at 1726 and 1743, in `client_reparent`, which stays), so `R4_EXCEPTION` is unchanged and the new files carry no `cfg` (`ports R4`).

## Decisions

### 1. How a file is cut (`ports R13`, `fcis R8`)

- **The parent keeps the state and the entry points.** A struct whose fields many concerns touch stays in the parent module (`mod.rs`, or `lib.rs` for the transcript crate); each child holds an `impl` block of the methods of its concern, plus the free functions, types and constants only that concern uses.
- **Visibility is the narrowest that compiles, and never wider than today:**
  - a method or free function moved to a child and called from outside it: `pub(super)` (in the transcript crate's root, `pub(crate)`, the reach a private crate-root item has today);
  - a type, constant or function that was `pub` keeps `pub`, and the parent re-exports it (`pub use lifecycle::{Phase, Start, …}`), so every path outside the module (`crate::acp::reduce::Phase`, `brindle_transcript::Event`, `crate::agent::model::LoginRow`) is unchanged;
  - re-exports are explicit lists, never globs, so a later `pub` item doesn't leak by accident.
- **Tests move with the code they test**, into the child's own `#[cfg(test)] mod tests`. Test helpers used by more than one child go to a test-only harness module (`#[cfg(test)] mod harness;`, items `pub(super)`), as `src/agent/test_support.rs` does today. A test that crosses concerns (the captured sessions, the log bound, the activity rule) stays in the parent's tests.
- **A test changes only in its `use` lines and module path.** The one exception is decision 13's test constant (`AUTH_REQUIRED` moves out of `forms.rs`).
- **One commit per source file**, each built and tested on its own (`cargo test -p <crate>`), so a bisect lands on one move.

### 2. `acp/reduce.rs` → `acp/reduce/` (`a.md` observation 1)

`reduce/mod.rs` keeps the session and the step:

| Module | Items (line at `5f335dc`) | Joins from earlier changes |
|---|---|---|
| `mod.rs` | the module doc; `BAD_RUN_QUIET` 106, `RunKind` 330, `Run` 339, `BadRun` 346, `AgentSession` 356 and its `impl` 383–495, `Input` 499, `Command` 530, `Refused` 561 and `Display`, `Moment` 668, `Output` 689, `step` 707, `Step` 764; `Step`'s emitting group 773–866 (`finish`, `emit`, `mark`, `add`, `add_text`, `notice`, `turn`, `agent`, `hold_waiting`), its state readers 869–942 (`running`, `prompt_awaited`, `readiness`, `failure`, `reading`, `ended`), `stdout` 956, `output_lost` 979, `tick` 1254, `bad_line` 1264, `flush_bad` 1273, `line` 1284, `message` 1309, `mute` 1369, `update` 1398, `retitle` 1447, `chunk` 1466, `response` 2115, `command` 2680, `trim` 2907; free `running` 2961, `muted_notice` 323 | change 4's fault rule in `finish` and `Moment::Answered`; change 3's emit guard; decision 10's `Process` |
| `lifecycle.rs` | `Phase` 117, `Start` 181, `Log` 230, `Replay` 237, `Greeting` 247, `Choice` 278, `choose` 289, `RESTARTED` 304, `RESTARTED_AFTER_LOGIN` 308, `REPLAY_SHOWN` 311, `NOT_REPLAYED` 315, `CANT_CONTINUE` 319, `Ending` 748; `spawned` 945, `exited` 1048, `lost` 1053, `end` 1067, `restart` 1119, `respawn` 1164, `restart_for_login` 1182, `refuse` 1210, `fail` 1234, `initialized` 2126, `speaking` 2166, `start` 2210, `continue_refused` 2256, `wait_for_choice` 2267, `choose_conversation` 2279, `created` 2377, `continued` 2403, `ready` 2483, `greeting_now` 2540 | change 4's closing step (`close_process`, `Closing`) |
| `turn.rs` | `prompt` 2751, `unqueue` 2774, `resend` 2785, `cancel` 2809, `prompted` 2598, `end_turn` 2639, `send_prompt` 2664, free `hold_reason` 2971 | `agents-commands`' option and command handling (`session/set_config_option`, `session/set_mode`), unless it reads better in its own `options.rs`: task 1.1 decides from its size |
| `tools.rs` | `MAX_OPEN_OUTPUTS` 79, `OpenOutput` 84, `ToolFields` 3181; `tool` 1490, `tool_call` 1495, `tool_change` 1510, `add_tool` 1571, `change_tool` 1585, `change_tool_fields` 1595, `tool_detail` 1649, `output` 1750, `describe_output` 1761, `open_output` 1795, `keep_open` 1820, `tool_content` 1839; free `placeholder` 3002, `clean_line` 3173 | decision 9's `ToolBook`; change 3's diff `Keep` changes |
| `permission.rs` | `permission_tool` 1545, `permission` 1980, `cancel_now` 2074, `withdraw` 2088, `unblock` 2102, `answer` 2851 | — |
| `login.rs` | `Login` 258, `AfterLogin` 266; `keep_methods` 2191, `login_required` 2518, `needs_login` 2529, `authenticated` 2568, `prompt_needs_login` 2616, `login` 2702, `cancel_login` 2740; free `methods_notice` 2983 | — |
| `list.rs` | `MAX_LISTED` 198, `MAX_LISTED_ID` 201, `MAX_LISTED_TIME` 203, `MAX_CURSOR` 205, `Page` 210, `ListAnswer` 221; `drop_list` 1223, `list` 2313, `listed` 2353; free `page` 3087, `same_dir` 3120 | — |
| `text.rs` | `MAX_SESSION_TITLE` 111; free `clean` 3015, `agent_name` 3033, `session_title` 3046, `bound` 3127, `cut_at` 3140, `text_item` 3153, `raw_text` 3160 | change 3's `cleaned_len`; when change 1 has moved `clean` to `brindle-transcript`, `text.rs` imports it rather than defining it |
| `harness.rs` (`#[cfg(test)]`) | the test module's `Agent` 3197 and its `impl`, and the helpers 3368–3503 (`result`, `error`, `update`, `chunk`, `said`, `tool_call`, `tool_update`, `permission`, `waiting`, `held`, `ended`, `shape`, `entry_kind`, `turn_name`, `agent_name`, `fixture_line`) | decision 13's test constant `AUTH_REQUIRED` |

- **`LOG_LIMIT`, `LOG_TRIM_TO` and `MAX_PROMPT`:** change 3 moves them to `acp/limits.rs` and leaves re-exports in `reduce.rs` "until `agents-module-structure` tidies them". This change points `policy.rs` and the tests at `crate::acp::limits` and drops those re-exports. `acp/limits.rs` itself stays as change 3 leaves it.
- **Tests, by section of today's `mod tests`:** tool content (3504–3860) and tool calls (4899–5018) → `tools.rs`; start sequence (3861–4096), exits and faults (5463–5579) and continuing (6361–7203, recovery 2.1–2.5 and titles across restarts) → `lifecycle.rs`; prompts (4097–4278), slot (4279–4589) and cancel (4590–4641) → `turn.rs`; permissions (4642–4898) → `permission.rs`; title (5128–5318) → `text.rs`; agents-login 3.3 and login 5.3 (7204–7816) → `login.rs`; choosing and listing (7817–) → `list.rs`; chunk runs (5019), messages the core doesn't take (5095), bad lines (5319), the log's bound (5580), activity (5713) and the captured sessions (5802–6360) stay in `mod.rs`.
- **Sizes after the move**, at `5f335dc`: `mod.rs` about 1,000 production lines, `lifecycle.rs` about 750, `tools.rs` about 550, the others 100–300 each.

### 3. `acp/checkpoint.rs` → `acp/checkpoint/` (`b.md` observation 1)

| Module | Items (line at `5f335dc`) |
|---|---|
| `mod.rs` | the module doc; `encode` 222, `encode_within` 226, `encode_segment` 246, `decode` 559, `decode_1` 575, `decode_2` 591; re-exports |
| `format.rs` | `MAGIC` 89, `FORMAT` 92, `FORMAT_1` 95, `PREFIX_LEN` 98, `HEADER_LEN` 101, `RECORD_HEADER_LEN` 104, `BASE` 107, `SEGMENT` 110, `TooLarge` 119, `Unreadable` 137 and their `Display`, `push_record` 376, `Record` 629, `next_record` 637, `take_u32` 656, `take` 663 |
| `state.rs` | writing: `merged` 268, `state_fields` 309, `phase_value` 410, `with_start` 457, `login_value` 475, `greeting_value` 487, `conversation_value` 496, `continuation_name` 504, `log_name` 513, `purpose_name` 522, `run_kind_name` 534, `rpc_value` 542, `open_output_value` 857; reading: `state_object` 672, `field` 689, `text` 693, `number` 699, `optional_text` 705, `object` 716, `array` 720, `pair` 731, `restore` 740, `restore_open_output` 878, `restore_phase` 913, `restore_start` 964, `restore_login` 987, `restore_greeting` 1001, `restore_continuation` 1009, `restore_log_kind` 1021, `restore_log` 1030, `fold_log` 1037, `restore_rpc` 1049, `restore_run` 1078, `restore_lines` 1090 |
| `adopt.rs` | `NOT_RESTORED` 114, `not_restored` 1115, `Adopted` 1141, `adopt` 1170 (and `merge` 1226 until decision 11 replaces it) |
| `harness.rs` (`#[cfg(test)]`) | the helpers 1265–1380 |

- `state.rs` keeps each record's writer and reader together, which `agents-checkpoint-deltas` changes as pairs.
- **Tests:** round trips (1381–1597), agents-login 3.4 (1598–2093) and tool content (2929–) → `state.rs`; bounds (2094–2138), "don't read" (2139–2327), format 2 segments (2509–2881) and the fuzz test (2882–2928) → `mod.rs`; adoption (2328–2508) → `adopt.rs`.
- Its fields read from `AgentSession` (`pub(super)` in `acp::reduce`, reachable from `acp::checkpoint`) are unchanged; decision 10's `Process` gives its fields the same reach.

### 4. `transcript/src/lib.rs` → `event.rs`, `output.rs`, `fold.rs`, `page.rs` (`b.md` observation 5)

`lib.rs` keeps the crate doc, the lint attributes, `#[cfg(test)] mod fuzz;`, the `Transcript` struct (1003) with its `Default`, the private `Kept` (958) and `Trim` (966), and the re-exports. A private item of the crate root is visible to every module of the crate, so `Kept`, `Trim` and `Transcript`'s fields need no new visibility.

| Module | Items (line at `5f335dc`) |
|---|---|
| `event.rs` | `TEXT_CHUNK` 54, `EntryId` 60, `Entry` 80, `ToolKind` 159, `ToolStatus` 175, `ITEM_LIMIT` 186, `MAX_ITEMS` 190, `CALL_LIMIT` 194, `RAW_LIMIT` 197, `MAX_LOCATIONS` 200, `ToolItem` 206, `FileChange` 234, `FileOp` 259, `Hunk` 271, `DiffLine` 280 and its serde, `Exit` 393, `Location` 400, `mod change` 408, `is_false` 428, `is_zero_u32` 432, `is_zero_u64` 436, `NoticeLevel` 443, `PermissionOption` 451, `PermissionKind` 462, `PermissionOutcome` 472, `Slot` 482, `HoldReason` 495, `TurnState` 514, `StopReason` 536, `AgentState` 548, `LoginMethod` 600, `LoginKind` 614, `Conversation` 634, `Continuation` 653, `Event` 699 (`to_value`, `from_value`), `json_kind` 809, `DecodeError` 822 |
| `output.rs` | `Output` 332 (`append`, `cut`), `output_fields_len` 1602 |
| `fold.rs` | `FoldError` 834, `Activity` 948; `impl Transcript`: `new`, `apply` 1042, `trim` 1213, `trim_entries` 1235, `index` 1267, `kept_mut` 1273, `resize_entry` 1281, the accessors (`slot` … `encoded_len`), `activity` 1544, `trim_to` 1575; free `entry_change` 1621, `chunks` 1726, `escaped_len` 1747 |
| `page.rs` | `PAGE_BYTES` 973, `Window` 978, `Fetched` 986; `impl Transcript`: `window` 1337, `replay` 1347, `page` 1360, `fetch` 1388, `newest_within` 1403, `prepend` 1422, `suffix` 1488, `replay_from` 1506; free `replay_entry` 1640, `replay_text` 1711 |

- Change 3's `MAX_EVENT`, `MAX_PATH`, `HUNK_FRAMING`, `ToolItem::wire_len` and `Event::encoded_len` go to `event.rs`, its fold clamps to `fold.rs`, and its public `escaped_len` stays in `fold.rs`, re-exported. Change 1's `clean` goes to `event.rs` beside the text it bounds.
- **Tests:** the general section (1757–2697) is split by the function a test exercises: decoding and encoding (`Event::to_value`, `from_value`, `DecodeError`) → `event.rs`; fold, trim, activity and login methods → `fold.rs`; replay and conversation → `page.rs`. The paging section (2698–3168) → `page.rs`. The tool content section (3169–) → `fold.rs`, except its output tests (`Output::append`, `cut`) → `output.rs`. Shared helpers (`added`, `agent`, `tool`, `fold`, `ready`, `ended`, `long`, `window`, `paged`, `text_item`, `output_item`, `diff_item`, …) → `#[cfg(test)] mod harness;`.
- **The public API is unchanged:** every item that is `pub` in `lib.rs` today stays reachable at `brindle_transcript::<name>`. A new integration test, `transcript/tests/public_paths.rs`, names each of them in one `use` and asserts nothing else: it fails to compile if a re-export is missing. `fcis R4`'s crate rule (`transcript_does_no_io_and_reads_no_clock`) already walks the whole crate.

### 5. `src/agent/model.rs` → `src/agent/model/` (`d.md` observation 1)

The modules follow `d.md`'s list, as children of `model/` (decision 21 says why not siblings). `ChatModel` (1123) and its fields stay in `model/mod.rs`.

| Module | Items (line at `5f335dc`) | Joins from earlier changes |
|---|---|---|
| `mod.rs` | `AGENT_PROTOCOL` 35, `RequestId` 39, `BLOCK_LIMIT` 44, `Absorbed` 1095, `Before` 1114, `ChatModel` 1123, free `running` 1212; `new` 1222, the setters and getters 1388–1475 (`set_command` … `agent_label`), `before` 1502, `absorb` 1687, `set_link` 2237, `life_event` 2241, `finish` 2249 | change 1's `SessionHandle`, `session_running`, `StatusFrom` use in `finish`; change 2's working-row splice in `finish`; decision 15's `replay_ended` |
| `phase.rs` | `StartFailure` 48, `agent_start` 75, `too_old_text` 84, `AgentPhase` 90, `Link` 106, `SessionLife` 115, `Alert` 545, `Names` 560, `batch_form` 568, `failure_text` 579, `DisabledWhy` 645, `HOLDER_LOST` 690, `PromptStatus` 807; `not_started` 2169, `ended` 2175, `lost` 2180, `broken` 2186, `restarting` 2205, `respawning` 2224, `prompt_status` 2507, `alert` 2851 | change 1's `respawning` changes |
| `blocks.rs` | `PermissionButton` 122, `BlockBody` 130, `Rich` 159, `RenderedText` 162, `ShownPart` 167, `ToolDraw` 177, `Block` 187, `Splice` 198, `parts` 210, `is_allow` 244, `buttons` 254, `user_blocks` 337, `blocks_of` 357, `render_entry` 410, `text_items` 426; `prepend_entries` 1664, `update_entry` 1776, `redraw` 1846, `keep_texts` 1869, the tool rows 1896–2082 (`refresh_facts`, `tool_running`, `tool_state`, `shown_parts`, `tool_draw`, `held`, `refresh_rows`, `set_policy`, `toggle_tool`, `toggle_tool_details`, `tool_copy_text`), `keep_render` 2083 … `toggle_thoughts` 2156, `last_agent_message` 2759, `last_code_block` 2770, `message_text` 2778 | change 2's `item_count`, `permission_block`; change 1's toggles cleared on a new log |
| `ledger.rs` | `Sent` 859, `Replied` 987; `in_flight` 2537, `sent` 2576, `replied` 2583, `unsent` 2596, `server_note` 2602, `answered` 2633 | change 1's `NotSent` handling, `ReplyError`, `Uncertain`, `Taken`, `taken`, `reconcile_prompt` |
| `queue.rs` | `Resend` 440, `QueueRow` 448, `stop_word` 457, `hold_words` 469, `hold_word` 481, `EXITED`, `NEEDS_LOGIN`, `LOGGING_IN`, `CHOOSING` 492–496, `without_session` 500, `resend_of` 509, `queue_row` 530, `Unqueue` 1000; `can_send` 2678, `unqueue` 2689, `resend` 2707, `cancel` 2713, `queued_text` 2787 | — |
| `prompt.rs` | `Why` 830, `ALREADY_QUEUED` 842, `UNSENT` 846, `Submit` 850, `First` 1012; `settle_first` 2294, `first_shown` 2337, `submit` 2545 | change 1's `FirstState` |
| `login.rs` | `LoginRow` 1029, `MethodChoice` 1040, `LoginChoice` 1049, `CancelLogin` 1061, `LoginZoneEnd` 1071, `AfterLogin` 1080; `needs_login` 2343, `login_row` 2365, `logins_usable` 2406, `logging_in` 2428, `login_note` 2434, `choose_login` 2440, `cancel_login` 2465, `login_zone_ended` 2482 | — |
| `recovery.rs` | `ProfileFound` 695, `Offer` 706, `NoOffer` 717, `continue_offer` 770; `conversation` 2192, `offer` 2198, `keeps_session` 2612 | — |
| `choosing.rs` | `ListFor` 883, `LaunchChoice` 892, `OnChoosing` 901, `ListDone` 910, `ListerState` 921, `ZoneChoice` 933, `Choosing` 1198; `choosing` 1264, `forget_conversation` 1278, `is_choosing` 1283, `started_choosing` 1289, `choosing_reason` 1296, `zone_choices` 1310, `can_choose` 1331, `chose` 1336, `can_list` 1345, `listed` 1351, `list_refused` 1373, `zone_run` 2804, `lister_state` 2830 | change 1's `Choose`; decision 15's `leave_on_quit` |
| `paging.rs` | `no_earlier_text` 962, `Earlier` 971, `Fetch` 980; `earlier` 1559, `fetch_due` 1569, `retry` 1579, `fetch` 1587, `fetching` 1602, `fetch_unsent` 1613, `fetched` 1625, `follow_window` 1643 | change 1's paging state |
| `attention.rs` | `WorkingRow` 266, `kind_word` 288, `notice_title` 306, `permission_body` 323, `AgentNotice` 1088; `shown_status` 1476, `seen` 1492, `working_row` 1511, `tool_in_progress` 1543, `close_busy` 2502, `answer` 2723, `pending_permissions` 2740, `banner` 2795, `error_text` 2857, `pending_body` 2884, `tool_kind` 2904, `notice` 2921 | — |
| `dump.rs` | `describe` 2940, `wait_text` 2946, `part_lines` 2952, `lines` 2969, `login_lines` 3072, `entry_line` 3110; free `tool_lines` 3163, `state_word` 3241, `turn_word` 3259, `tool_kind_word` 3270, `tool_status_word` 3285 | `agents-commands`' dump lines |
| `harness.rs` (`#[cfg(test)]`) | the test `Zone` 3307 and its helpers to 3453 | — |

- `agents-commands`' commands, options and completion go to a new `model/options.rs` when they are a concern of their own after it merges (task 1.1 decides), else to `prompt.rs` (completion) and `mod.rs` (the lists).
- **Tests:** the sections go whole: recovery (4663–4930) → `recovery.rs`; agents-login (4931–5168) → `login.rs`; paging (5169–5370) → `paging.rs`; tool details (5371–5749) and rendering (4384–4662) → `blocks.rs`; choosing (5750–) → `choosing.rs`. The general section (3454–4383) is split by the model method each test drives first: `every_phase`, `each_failure_has_its_alert`, `a_version_mismatch_stays_failed_after_the_session_ends` → `phase.rs`; `each_entry_is_drawn_as_its_blocks`, `absorbing_changes_splices_only_what_changed`, `thoughts_open_while_latest_and_streaming`, `the_copy_actions_read_the_zones_copy`, `the_last_code_block` → `blocks.rs`; `the_first_prompt_is_sent_once_on_the_first_live_ready`, `the_first_prompt_becomes_the_draft_on_a_failure_or_a_replay`, `a_new_prompt_beside_the_slot`, `no_slot_change_alters_the_draft` → `prompt.rs`; `every_slot_and_hold_reason_with_each_phase`, `unqueue_and_resend` → `queue.rs`; `an_unsent_request_is_refused_and_a_first_prompt_becomes_the_draft` → `ledger.rs`; `a_login_demanded_by_a_prompt` → `login.rs`; `a_zone_keeps_its_session_only_while_it_is_open` → `recovery.rs`; `a_permissions_rejects_come_first`, `permissions_are_answered_once`, `the_shown_status_and_the_seen_rule`, `notices_on_blocked_done_and_error`, `the_working_row_follows_the_turn`, `notification_words`, `a_permission_notice_gives_the_context` → `attention.rs`; `the_dump_describes_the_zone`, `an_endpoint_note_shows_below_a_restores_note`, `the_dump_shows_each_part_and_link` → `dump.rs`; `the_title_is_the_conversations_else_the_profiles`, `a_respawn_falls_back_to_the_profiles_name_once_its_conversation_arrives` → `mod.rs`. A test added by an earlier change goes by the same rule.
- **Sizes after the move**, at `5f335dc`: `blocks.rs` about 800 production lines (the part `agents-absorb-cost` rewrites), `mod.rs` about 450, the others 100–350.

### 6. The agent restart lifecycle → `core/agents.rs` (`c.md` observation 2)

`sessions.rs`'s `send_restart` (865), `start_again` (989), `restart_failed` (1010), `abandon_restart` (1023), `agent_ended` (1282) and `end_ended` (1584) move to `core/agents.rs`, as change 4 leaves them (`Life::Restarting { restart, stage }`, `SpawnRequest` without `Restart`, and `restarting` (752) with its test deleted). Methods called from `sessions.rs` (`session_lost`, `ended`) stay `pub(super)`; `send_restart`, `abandon_restart` and `end_ended`, private today, become `pub(super)` only if a caller stays in `sessions.rs`.

- `policy::lost_fate`: change 4 writes it in `policy.rs`, which is where the core's pure policy lives, so it stays; task 1.1 checks that change 4 landed it, and if not, this change extracts it as change 4's decision 5 describes.
- **Not done:** `c.md`'s `Content::on_end` / `on_lost` hooks. The terminal and agent arms are already one `match` on the session's content in `session_lost` and `ended`; an arm calling into `agents.rs` gives the same separation without a dispatch layer around a decision (`ports R6`).

### 7. The conversations palette → `src/workspace/conversations.rs` (`e.md`)

`ConversationsPalette` (395–408) and the `impl Workspace` block of `conversation_facts` (1456) through `open_placed` (1749–1780): `active_zone_at`, `tab_zones`, `open_zone_conversations`, `continue_agent_conversation`, `open_conversations`, `conversations_palette`, `conversations_lister_changed`, `conversations_listed`, `show_conversations`, `take_conversations`, `close_palette_chooser`, `conversations_confirmed`, `clear_palette_filter`, `show_zone`, `open_here`, `open_placed`. The `PaletteKind::Conversations` variant (373) and the action registration stay in `workspace.rs`, calling `pub(super)` methods. `workspace.rs` imports `crate::agent::conversations` items only, without `self`, so the child module's name doesn't clash. It is shell (an entity's `impl`), so it doesn't join `PURE_MODULES`.

### 8. The terminal-login zone code → `src/zone_view/login.rs` (`e.md`)

`on_login_zone` (1129), `login_zone_of` (1144), `login_zones` (1150), `open_login_zone` (1160), `end_login` (1200) and the free `login_axis` (900) with its test `a_login_zone_goes_beside_and_is_never_recorded`. `Zone::login_for` (888) stays a field of `Zone`, which stays in `zone_view.rs`.

### 9. `ToolBook` (`a.md` observation 3)

```rust
pub(in crate::acp) struct ToolBook {
    pub(in crate::acp) tools: BTreeMap<String, EntryId>,
    pub(in crate::acp) outputs: BTreeMap<String, OpenOutput>,
}
impl ToolBook {
    pub(super) fn forget_through(&mut self, through: EntryId); // trim's two retains (reduce.rs 2929–2930)
    pub(super) fn clear(&mut self);                           // respawn's two clears (1168, 1170)
}
```

In `reduce/tools.rs`. `AgentSession.tools` and `.outputs` become `AgentSession.book` (inside `Process`, decision 10). The checkpoint writes the same `"tools"` and `"outputs"` keys from it, so its bytes don't change. Tests in `tools.rs`: `forget_through_keeps_only_later_calls_and_outputs`, `clear_empties_both_indexes`. `agents-checkpoint-deltas` adds its `tool_epoch` and diff memo here.

### 10. `Process` (`a.md` observation 2; built on change 4's closing step)

```rust
pub(in crate::acp) struct Process {
    pub(in crate::acp) lines: Lines,
    pub(in crate::acp) run: Option<Run>,
    pub(in crate::acp) bad: Option<BadRun>,
    pub(in crate::acp) book: ToolBook,
}
```

In `reduce/mod.rs`, as `AgentSession.process`. `respawn` becomes change 4's closing step followed by `self.s.process = Process::new()`, and `Process::new()` is exactly today's reset (`lines` new, `run` and `bad` `None`, both indexes empty), so a field added to `Process` later is reset without anyone remembering to.

- **`Rpc` stays on the session.** `Rpc::restart` (change 4: `Rpc::close`) keeps `next_id` so request ids are never reused across processes (`types R3`); a `Process` owning the `Rpc` would reset it. The closing step forgets the awaited requests through `Rpc::close`, as change 4 designs it.
- **What a respawn does is unchanged**, which the existing continuing tests (2.1–2.5) and the captured sessions pin. A new test in `lifecycle.rs`, `a_respawn_leaves_no_state_of_the_old_process`, fills each field of `Process` and checks a respawn leaves `Process::new()`'s values.
- The checkpoint reads `process.lines`, `process.run`, `process.bad` and `process.book` where it read the flat fields, and writes the same keys (`"partial"`, `"covers"`, `"run"`, `"tools"`, `"outputs"`, `"bad"`), so its bytes don't change. The round-trip and segment tests pin that.
- **Checkpoint, core restart, crash, upgrade:** nothing new. `Process` is in-memory structure only; a restored session gets the same values as today.

### 11. `Output::then` and one `Moment` precedence (`b.md` observation 2)

`Moment::wins(earlier: Option<Moment>, later: Option<Moment>) -> Option<Moment>` (in `reduce/mod.rs`): `BeforeRequest` from either side, else the later one, else the earlier one, which is the rule `Step::mark` (796) and `checkpoint::merge` (1226) both write today. `Step::mark` calls it. `Output::then(self, later: Output) -> Output` replaces `merge`: events and stdin appended, `reply` the later one when it has one, `checkpoint` by `Moment::wins`, `hang_up` either, `wake` the later one, `listed` the later one else the earlier one. `merge` ignores `listed`; `adopt` steps `Input::OutputLost` and `Input::Spawned` on a restored session, which has no list request out (the restore drops `Purpose::List`), so neither output carries `listed` and the two agree. Tests in `mod.rs`: `before_request_wins_from_either_side`, `then_keeps_the_later_reply_and_wake`; the adoption tests pin `adopt`.

If change 4's `Moment::Answered` gives `mark` another precedence, `wins` is where it is written once.

### 12. Where `pub use` paths change

None outside the moved modules' own `use` lines and the decision 2 limits tidy. `crate::acp::reduce::*`, `crate::acp::checkpoint::*`, `brindle_transcript::*`, `crate::agent::model::*` and `crate::zone_view::*` keep every path. `src/zone_tree.rs`'s items are re-exported from `zone_view` (decision 17), and the agent's pure modules switch their imports to `crate::zone_tree`.

### 13. `Dialect::failure` (B-6; `ports R14`)

```rust
// acp/forms.rs
pub enum Failure {
    AuthRequired,
    VersionRefused { supported: Option<Vec<u16>> },
    Other,
}
// acp/dialect.rs
pub fn failure(self, error: &RpcError) -> Failure
```

- `v1::decode::failure` reads the code: `-32000` is `AuthRequired`; `-32022` is `VersionRefused` with the `data.supported` list when it is a non-empty list of `u16`, else `None`; anything else is `Other`. It absorbs `v1::decode::supported_versions` (232), which returns `Some` only for `-32022` with that list, so `failure` decides exactly what the five sites decide today. `Dialect::supported_versions` goes.
- `AUTH_REQUIRED` and `UNSUPPORTED_VERSION` move from `forms.rs` (16–17) to `v1/decode.rs`, private. `forms.rs` keeps only the JSON-RPC 2.0 standard codes its `Refusal` uses (`-32601`, `-32600`), which are the envelope's, not a version's.
- **The sites:** `initialized` (2144–2160) matches `failure(error)`: `VersionRefused { supported: Some(versions) }` gives the versions text, `VersionRefused { supported: None }` "supports no ACP version Brindle speaks", `AuthRequired | Other` "couldn't start: …". `listed` (2362), `created` (2389), `continued` (2447) and `prompted` (2606) test `self.s.rpc.dialect().failure(&error) == Failure::AuthRequired`. The match is exhaustive (`railway R3`).
- **Tests:** in `v1/decode.rs`, `minus_32000_is_a_login_demand`, `minus_32022_with_versions_names_them`, `minus_32022_without_a_list_names_none`, `any_other_code_is_another_failure`. The reducer's tests keep building `-32000` errors through `harness.rs`'s test constant `AUTH_REQUIRED: i64 = -32000`, the one test edit beyond `use` lines; `v1/decode.rs`'s own tests (659, 1896) use its private constant.
- `agents-acp-v2` adds `Dialect::V2`'s arm to `failure` and its own codes in `acp/v2/`.

### 14. `Hold` replaces `Held` beside `requests` (`d.md` observation 4; `types R1`)

```rust
enum Hold {
    Open(Box<dyn Requests>),
    NoServer, // the start failed (`renew`'s and `new`'s `Err(unsupported)`)
    Left,     // quit or closed: nothing more is sent
}
```

`Agent.held` and `Agent.requests` become `Agent.hold`. `None` requests today mean the start failed, so `requests.take()` alone would merge "start failed" with "left"; the third variant keeps them apart.

- `quit_with` and `close` move to `Left` with `std::mem::replace`, using the requests they take; `renew` closes an `Open` hold when `close_old`, then installs the new one. The sends (`restart`, `choose`, the first prompt, `cancel`, `send`, `fetch`) go through one `fn requests(&self) -> Option<&dyn Requests>`, `Some` only for `Open`. `server_session` reads `Open` where it read `Holding`.
- **One difference, unreachable today:** a send after leaving now goes nowhere, where today it would still be written. Nothing sends after `quit_with` or `close` (the zone is dropped right after), so no test or behaviour sees it.
- **Tests:** `Agent` is an entity, so its tests are in `src/agent/tests.rs` (`gpui::TestAppContext`): `a_quit_leaves_and_sends_nothing_more`, `a_failed_start_refuses_a_prompt_as_today`.

### 15. `leave_on_quit`, `replay_ended` and the `_ =>` arm (the rest of D-11; `fcis R1`, `R10`, `railway R3`)

- **`ChatModel::leave_on_quit(&self) -> Leaving`**, `enum Leaving { Close, Detach }`, in `model/choosing.rs`: `Close` while choosing (`started_choosing() || is_choosing()`), else `Detach`. `quit_with` matches it. Test: `a_zone_still_choosing_closes_its_session_on_quit_and_others_detach`.
- **`ChatModel::replay_ended(&mut self) -> Option<Absorbed>`**, in `model/mod.rs`: `Reconnecting` → `Some(self.set_link(Link::Up))`, `Up` → `None`. `apply`'s `Replay(false)` arm calls it. Test: `a_replay_ends_a_reconnect_and_nothing_else`. If change 1's replay-end record (D-10) already made this a model call, this part is done and task 1.1 says so.
- **`login_zone_ended`'s match on `profile_found`** stays in the entity: it chooses which entity call to make (dispatch), and `ChatModel::login_zone_ended` already returns the decision.
- **`conversations.rs`'s `failed` (484–492)** spells out its arms: `(ListState::Listed, More::Loading(cursor))` keeps the page and fails the next one; `(ListState::Starting | ListState::Listing | ListState::Listed | ListState::Failed(_), More::None | More::Available(_) | More::Loading(_) | More::Failed { .. } | More::Capped)` fails the list, written so that the first arm's pair still reaches the first arm. A new variant of either enum then fails to compile here. The existing tests cover both arms.
- D-11's `choose` part is change 1's.

### 16. One protocol-4 gate in `handle_client` (`c.md` observation 4; `fcis R1`, `railway R3`)

```rust
// core/policy.rs
pub(super) struct Gated { pub request: AgentRequest, pub req: Option<RequestId>, pub session: Option<SessionId> }
pub(super) fn agent_request(message: &ToCore) -> Option<Gated>
```

Exhaustive over `ToCore` with no `_ =>` arm: the agent messages (`SpawnAgent`, `AgentPrompt`, `AgentCancel`, `AgentPermission`, `AgentUnqueue`, `AgentResend`, `AgentRestart`, `AgentLogin`, `AgentFetch`, `AgentList`, `AgentChoose`, and `agents-commands`' `AgentSetOption`) give their `AgentRequest`, `req` and session; every other message gives `None`. `handle_client` (`clients.rs` 312) applies `agent_request_refusal(protocol, request)` once, before its dispatch, and answers `ERROR` with the same `req` and session each site uses today: `SpawnAgent` `(Some(req), None)`, `AgentRestart` and `AgentFetch` `(Some(req), Some(id))`, the commands `(req, Some(id))`. The four handlers drop their own gates (`agents.rs` 486, 521, 655) and `agent_fetch_refusal` drops its `protocol` argument, keeping its view check. A new agent message then can't skip the gate: it must be classified to compile.

- Each site checks the protocol first today, so the order of checks, and the `ERROR` a client before protocol 4 gets, are unchanged. The fixtures `client-agent-fetch-old-client.hex` and `client-agent-list-old-client.hex` pin the bytes.
- `SET_LAYOUT`'s protocol-3 check (`sessions.rs` 1832) has the same shape but one message; it stays.
- **Tests** in `policy.rs`: `every_agent_message_is_gated_with_its_request_and_session` (one row per agent message), `no_terminal_message_is_gated` (one row per other message).

### 17. `ZoneId`, `split_target` and the zone tree → `src/zone_tree.rs` (`d.md` observation 5; `fcis R5`)

`ZoneId` (55), `Direction` (59), `SplitAxis` (68), `UnitRect` (77), `ZoneTree` (117) and its `impl` (126–270: `new`, `split`, `remove`, `leaves`, `rects`, `focus_target`, all GPUI-free), `split_host` (379), `split_cwd` (388), `split_target` (397), with their tests. They need only `config::Profile`, `remote::Host` and serde. `zone_view.rs` re-exports them (`pub use crate::zone_tree::{…}`), so `layout.rs`, `session.rs` and `close_confirm.rs` are untouched; `agent/conversations.rs` and `agent/close.rs` import `crate::zone_tree`. `can_split_zone` (272) takes gpui sizes and stays. `zone_tree.rs` joins `PURE_MODULES` and `fcis`'s table (decision 22). `illegal-states-unrepresentable.md`'s example "`ZoneId(u64)` in `zone_view.rs`" names the new file.

- **`TabId` stays** in `src/workspace/tabs.rs`, which is already a whole pure module (`PURE_MODULES`); `conversations.rs` importing it through `crate::workspace::TabId` reaches a pure module through a re-export, not a view's own code. `d.md` observation 5's concern is met without moving it; the import switches to `crate::workspace::tabs::TabId` so the path names the pure module.

### 18. Rejected: the serde mirror of the checkpoint state (`b.md` observation 1)

A `#[derive(Serialize, Deserialize)]` mirror (`StateV2`, …) would replace the hand-written `*_value` and `restore_*` pairs. Rejected for this change, because it can change the bytes:
- `serde_json::Map`'s key order depends on feature unification: gpui 0.2.2 enables `preserve_order`, so the app's build orders keys by insertion and a helper built without gpui (`cargo build -p brindle-remote-agent`, the static musl helper) sorts them; a derive writes struct field order in both. The hand-written writer's output today and a derive's agree only if every record's fields are written in an order that happens to match.
- `Unreadable`'s messages name the field that failed (`restore_*`'s `field`, `text`, `number`), and tests pin those messages; serde's errors read differently.
`agents-checkpoint-deltas` already adds a serialize mirror where streaming needs it (the log and the events) and says the rest stays with this change's split. With `state.rs` holding each writer beside its reader, a later change can replace one pair at a time and show the bytes unchanged by the round-trip tests.

### 19. Changed: `chose`, `lists` and `methods` stay where they are (`a.md` observation 5)

Moving `chose` into `Start::Continue`'s origin and `lists` and `methods` into the `Greeting` changes what a checkpoint records:
- `Phase::Exited` and `Phase::Spawning` carry no greeting, but `methods` and `lists` are written whenever they are set (`state_fields` 360, 365), including in those phases; a greeting-owned field would be dropped there.
- `chose` survives an exit today, and a restart whose continue is refused returns to choosing with it still set; an origin on `Start::Continue` ends with the start.
Each difference is observable after a core restart (a login method list or a choice lost), so the move is a format change under `process R4`–`R6`, outside a no-behaviour change. Their meaning is documented beside the fields instead. No written change takes the move: `agents-checkpoint-deltas` reworks how `methods` is recorded (only when it changed) but keeps all three as top-level record fields, and the move is kept as the proposal-only change `agents-reducer-phase-flags`.

### 20. Changed: model modules don't own their fields (`d.md` observation 1)

`d.md` proposes that each module own its fields and return its own `Absorbed` part. That is not a move: `phase`, `turn`, `slot`, `entries` and `pending` are read by most of the modules (the dump, attention, the queue and the prompt all read the phase and the turn), so a field-owning split either copies them or passes them everywhere. This change moves the methods and leaves the fields on `ChatModel`. `agents-absorb-cost` splits the absorb into two halves and keeps pending permissions as a set; it is the change that gives `blocks.rs` its own state, where the two halves need it. The rest of the proposal, each concern owning its fields, is kept as the proposal-only change `agents-model-state-ownership`.

### 21. Changed: `model/` children, not `agent/` siblings (`d.md` observation 1)

`d.md` names `agent/phase.rs`, `agent/blocks.rs`, …, with login joining `agent/login.rs` and choosing `agent/conversations.rs`. As siblings of `model.rs`, their `impl ChatModel` blocks could reach `ChatModel`'s fields only if those were `pub(super)`, visible to all of `agent/` including the entity (`mod.rs`) and the view (`view.rs`), which today must go through methods (`types R11`). As children of `model/`, they reach the private fields and nothing else gains access. `agent/login.rs` (the terminal-login command line) and `agent/conversations.rs` (the palette's list model) keep their own concerns.

### 22. `tests/architecture.rs` and the rule docs (`fcis`, `ports R14`)

Each change to a check lands with its doc in one commit ("change the rule in its doc and the test in the same commit"):
- **`PURE_MODULES`:** `remote-agent/src/acp/reduce.rs`, `remote-agent/src/acp/checkpoint.rs` and `src/agent/model.rs` are replaced by every file of their directories (the harness files included, which are test-only but pure), and `src/zone_tree.rs` is added; the array's length changes with it. `fcis`'s whole-pure-modules table writes them in its brace form (`remote-agent/src/acp/reduce/{mod,lifecycle,turn,tools,permission,login,list,text,harness}.rs`), which `the_pure_module_list_matches_the_fcis_table` expands. `the_pure_modules_read_nothing_ambient` then walks the new files. Its comment says a directory module joins file by file, so a new file in it can't skip the check.
- **`acp_versions_stay_in_their_modules`:** `versionless` becomes every file under `acp/reduce/` and `acp/checkpoint/` (through `sources`) plus `rpc.rs`. `ports R14`'s enforcement line names `acp/reduce/`, `acp/checkpoint/` and `acp/rpc.rs`.
- **New, `acp_error_codes_stay_in_their_version`:** the production code lines (comments dropped) of every file under `remote-agent/src/acp/` outside a version directory (`v1/`, later `v2/`) name no `AUTH_REQUIRED`, no `UNSUPPORTED_VERSION`, no `.code ==` or `.code !=`, and no `-320` (JSON-RPC's implementation-defined range, `-32000` to `-32099`; the standard codes `-32600`, `-32601` and `-32700` in `envelope.rs` and `forms.rs`'s `Refusal` are other prefixes). At `5f335dc` it would report the reducer's five sites and `forms.rs`'s two constants; after decision 13 it reports none. A companion test, `the_error_code_check_finds_a_planted_code`, runs the same scan over a planted line, so a scan that silently matches nothing fails. `ports R14` gains the sentence "A version's error codes are read only in its module; the shared code matches `forms::Failure`."
- **Unchanged:** `only_the_acp_modules_name_the_schema_crate` (walks `acp/` already), `transcript_does_no_io_and_reads_no_clock` (walks the crate), `platform_cfgs_stay_in_their_seams` and `R4_EXCEPTION` (no `cfg` moves; the walk covers the new files, which have none), `only_the_markdown_module_names_the_parser`.

### 23. Docs

- `docs/code-map.md`: the agent entry's model paragraphs (lines 51–61) name `model/`'s modules; the reducer and checkpoint paragraph (42) names `reduce/` and `checkpoint/` and `Process`, `ToolBook`, `Dialect::failure`; the transcript's names its modules; the server core's names `agents.rs`'s restart lifecycle, `checkpoint_screen`/`checkpoint_agent` and the gate in `handle_client`; `workspace.rs` (68) names `workspace/conversations.rs`; `zone_view.rs` (70) names `zone_view/login.rs` and `zone_tree.rs`.
- `docs/architecture/functional-core-imperative-shell.md`'s table, `ports-and-adapters.md`'s R14, `illegal-states-unrepresentable.md`'s `ZoneId` line: decisions 17 and 22.
- `.claude/rules/remote-agent.md`: "the reducer, `rpc.rs` and the checkpoint" become `acp/reduce/`, `acp/rpc.rs` and `acp/checkpoint/`, and the error-code rule is added beside `ports R14`'s line.
- `CLAUDE.md` names none of the moved files' paths except through the dump's `ChatModel::…` items, which keep their names; it is checked and left unless a path appears.
- `docs/architecture/README.md`'s read-by-layer table uses globs and needs no change.

### 24. `checkpoint_screen` and `checkpoint_agent` (`c.md` observation 5; `types R7`)

`Core::checkpoint(id, at: AgentCheckpoint)` (`sessions.rs` 1191) serves both kinds: a terminal's caller passes `AgentCheckpoint::Stepped(None)` as a sentinel (`checkpoint_due` 1214: "A terminal's checkpoint is always whole"). It splits into:
- `checkpoint_screen(id)` in `sessions.rs`, for terminals (`checkpoint_due`, `checkpoint_at`'s terminal arm);
- `checkpoint_agent(id, at)` in `agents.rs`, with today's liveness check (`agent_step` 822, `checkpoint_at`'s agent arm);
- the shared liveness test, `checkpointable(id) -> bool`, used by both;
- the unchecked writer `agent_checkpoint` (`agents.rs` 864, used by `send_restart` after its own check) renamed `write_agent_checkpoint`, private to `agents.rs` now that `send_restart` lives there.

`checkpoint_all` and `checkpoint_at` dispatch on the session's content. What is written, and when, is unchanged; the core's checkpoint tests in `remote-agent/tests/upgrade.rs` and `core.rs` pin it. Core restart, crash and upgrade see the same checkpoints as today.

### 25. How "no behaviour change" is shown

- `cargo test --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings` and the whole e2e suite under `scripts/with-xvfb` pass, with the e2e cases unchanged.
- `git diff main -- '**/tests/fixtures/**'` is empty: no transcript, protocol or checkpoint fixture changes, and the fixture tests (`remote-protocol/tests/transcripts.rs`, the checkpoint round trips) pin the bytes.
- Test code differs from `main` only in `use` lines, module paths, the `harness` modules' headers and decision 13's test constant; the new tests are additions. A task lists each test file's diff with `--stat` and `--color-moved` to show it.
- The dump's format is untouched: `dump.rs` is a move, and the e2e cases read every line.
- Each move commit, under `git diff --color-moved=dimmed-zebra --color-moved-ws=allow-indentation-change`, shows only moved lines plus `mod`, `use` and visibility lines.

### 26. Order and rebases

After `agents-commands` (PR #151) and changes 1–4; before `agents-checkpoint-deltas`, `agents-absorb-cost` and `agents-client-request-marks` (change 8, C-6, which `agents-event-bounds` first planned and which adds `policy::takes_frames` and a check in `process_client_frames` before a frame reaches this change's gate in `handle_client`). If one of changes 1–4 is still open when this starts, the moves wait for it rather than rebase it, since each of them edits a file this change cuts. A change in flight that rebases over a move (`agents-prompt-context`, `agents-plans`, `agents-prompt-images`, `agents-acp-v2`, `agents-chat-selection`) resolves the conflict by taking the move first and replaying its edit in the module that now holds the code, which decisions 2–5 name per concern. The same rule applies to this change if `main` moves under it: redo the move on the new `main` (it is mechanical), then replay.

## Risks / Trade-offs

- **[A move hides a behaviour change]** → one commit per file, `--color-moved` review, and no test edited beyond `use` lines; type changes are separate commits with their own tests.
- **[`pub(super)` spreads]** → decision 1's rule: no item's reach grows past today's, and re-exports are explicit lists of what was `pub` before, checked per commit in review (and, for the transcript crate, by `tests/public_paths.rs`).
- **[Checkpoint bytes drift through `Process` or `ToolBook`]** → the writer keeps its keys and order; the round-trip and format-2 segment tests, and the checked-in fixtures, pin the bytes. Decisions 18 and 19 keep the two riskiest ideas out.
- **[Rebase pain for in-flight changes]** → decision 26; the moves are mechanical and the tables say where each concern now lives.
- **[The error-code check is a text scan]** → its planted-line test keeps it honest; a false positive is fixed by moving the code into `v1/`, which is the rule.

## Migration Plan

None: no wire, checkpoint, transcript, config or state change. A helper with this change and one without speak the same protocols and read each other's checkpoints. Rolling back is reverting the PR.

## Open Questions

1. **`agents-commands`' reducer and model code: own modules or existing ones?** Recommendation: `reduce/options.rs` and `model/options.rs` if each is over about 150 lines after #151 merges, else `turn.rs` and `prompt.rs`. Task 1.1 decides from the merged code; it doesn't change the approach.
2. **One PR or several?** Recommendation: one PR with one commit per file and per type change, since each commit builds and tests alone and `--color-moved` reviews per commit. If review prefers, the server half (decisions 2, 3, 6, 9–11, 13, 16, 24) and the client half (4, 5, 7, 8, 14, 15, 17) split cleanly into two PRs, the server first.
