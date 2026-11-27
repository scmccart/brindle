# Batch B: the ACP codec, the checkpoint and the shared transcript

## Coverage

Read in full (production code):
- `remote-agent/src/acp/checkpoint.rs` 1–1242
- `remote-agent/src/acp/v1/decode.rs` 1–652, `v1/encode.rs`, `v1/mod.rs`
- `remote-agent/src/acp/rpc.rs` 1–305, `envelope.rs` 1–185, `lines.rs` 1–165
- `remote-agent/src/acp/dialect.rs` (all), `forms.rs` 1–40
- `transcript/src/lib.rs` 1–1756 (production ends at 1756; tests from 1757)
- `transcript/src/fuzz.rs`: its generators and test list (what it covers)

Read in part, to follow paths:
- `remote-agent/src/acp/reduce.rs`: `AgentSession` and `Phase` (84–420), `add`/`notice` (812–846), `respawn` (1160–1175), `tool_call`/`add_tool` (1490–1580), `tool_detail`/outputs/`tool_content` (1648–1975), `permission` (1979–2066), `start` (2205–2250), list (2330–2346), `authenticate` (2720–2735), `trim` (2904–2932), `clean`/`bound`/`cut_at`/`raw_text` (3015–3170)
- `remote-agent/src/server/core/agents.rs` 766–910 (`agent_step`, `agent_apply`, `send_transcript`, `agent_checkpoint`)
- `remote-agent/src/server/core/policy.rs` 560–690 (`agent_checkpoint_due`, `Stored`, `agent_appendable`, `agent_checkpoint_form`)
- `remote-agent/src/server/core/sessions.rs` 1950–1995 (the attach replay)
- `remote-protocol/src/client.rs` 780–812 (`pack_transcript`), `holder.rs` `TAIL_LIMIT`
- `src/agent/sink.rs` 150–240 (the client's fold caller)
- `openspec/changes/archive/2026-11-26-agents-log-paging/design.md` decisions 6–10 and 16

Also checked: no crate in `Cargo.lock` enables serde_json's `unbounded_depth` or `arbitrary_precision`, so every `from_str`/`from_slice` here keeps the 128-level recursion limit and numbers past `u64` read as `f64`.

Clean, in one line each:
- `lines.rs`: one scan per chunk, no rescans, amortized growth, an over-long line is dropped without buffering and the next line starts clean; only a stale doc link (`super::wire::decode`, now `envelope::decode`).
- `rpc.rs`: ids are never reused (a restart keeps `next_id`; every request marks `Moment::BeforeRequest`, so the checkpointed `next_id` is past every id sent; `List` isn't checkpointed but its id is still counted); at most one list, one prompt and one start request are out at a time, so `outgoing` stays small; permissions are capped at 16.
- `envelope.rs`: total; ids bounded at 1 KiB; invalid UTF-8 is replaced.
- `v1/encode.rs`: every string goes through JSON escaping; ids are echoed as sent.
- Checkpoint coverage: every `AgentSession` field and every `Phase` variant's fields are encoded and restored. `BadRun.last` (restored as `now`), `Purpose::List` (dropped) and a continue start's title (taken from the log) are the only differences, and each was a recorded decision.
- Transcript fold: total. Every arm returns a `FoldError` or applies. Ids it doesn't hold, duplicates, out-of-order events and events below a `Window` are refused or ignored without a panic. An `Appended` costs O(chunk) (`escaped_len`, no re-encode). The core and the client run the same `apply`.

## Findings

### B-1: Events the reducer can emit exceed a 1 MiB frame, so a permission is never shown and attaches fail
- Severity: medium
- Confidence: confirmed
- Category: errors
- Where:
  - `remote-agent/src/acp/v1/decode.rs:601-607, 618-632`: `permission_request` keeps every option, with no limit on how many or on the `optionId` length.
  - `remote-agent/src/acp/reduce.rs:1998-2007`: each option name is bounded to 64 KiB, but neither the option count nor `option.id` is.
  - `remote-agent/src/acp/v1/decode.rs:451-464` and `reduce.rs:1672-1680`: 32 locations of up to 64 KiB each, 2 MiB before escaping.
  - `remote-protocol/src/client.rs:794-801`: `pack_transcript` refuses the whole batch if any one event is too big.
  - `remote-agent/src/server/core/agents.rs:840-856`: its doc says "the log's bounds rule it out", which is not true.
  - `core/sessions.rs:1958,1982`: the attach replay goes through the same call.
- Scenario: an agent sends `session/request_permission` with 20 options, each with a 64 KiB name, or one option whose `optionId` is 2 MiB. The same applies to a `tool_call` with 32 long `locations`.
  - The reducer adds an `Entry::Permission` of about 1.3 MiB or more, applies it to the core's log and checkpoint, and marks the turn `Blocked`.
  - `send_transcript` gets `Err` from `pack_transcript`. The live `Added` never reaches the client, so the user sees a blocked turn with no permission row to answer.
  - Every later attach or reconnect sends `page()`, which always includes a pending permission. `pack_transcript` then refuses the whole page, so the zone attaches with an empty log. Later live events for entries it doesn't hold then fail to fold.
  - This survives a core restart, because the entry is in the checkpoint. It ends only when the permission is answered (impossible from the UI) or cancelled, and the entry is trimmed. While the oversized entry is newest, or reached by a fetch, those fail the same way.
- Fix:
  - Bound what decodes: at most 16 options (`MAX_LOGIN_METHODS`-style), leaving out an option whose `optionId` is over 1 KiB (it must be echoed, so it can't be cut), and a total bound on location path bytes (for example 4 KiB per path). `railway R5`.
  - Add a reducer property test asserting that every emitted event's encoding is at most `MAX_FRAME_PAYLOAD_SIZE` minus the `TRANSCRIPT` envelope. Run it over the existing random-step fuzz with long inputs. This ties the bound to the frame limit, which today only a comment does.
  - Optionally make `pack_transcript`'s caller replace a single oversized event with a notice rather than drop the whole batch (`railway R6`: confine the failure to one entry).

### B-2: Raw tool input is pretty-printed without a bound before the 16 KiB cut: one 8 MiB line can allocate about 2 GiB in the core
- Severity: medium
- Confidence: confirmed
- Category: memory
- Where:
  - `remote-agent/src/acp/v1/decode.rs:506-512` (`raw`: `serde_json::to_string_pretty(value)`)
  - `decode.rs:465-466` (called for `rawInput` and `rawOutput` on every `tool_call`, every `tool_call_update` and a permission's `toolCall`)
  - `remote-agent/src/acp/reduce.rs:1681-1682, 3160-3170` (`raw_text` runs `clean` over the whole string, a second full copy, before `cut_at(…, RAW_LIMIT)`)
- Scenario: a `tool_call` line (at most `MAX_LINE`, 8 MiB) whose `rawInput` is about 124 nested arrays, the innermost holding about 4 million `0`s.
  - It is valid JSON within serde_json's 128-level limit, and parsing it costs about 128 MiB of `Value`s. The whole-update clone in `split` (B-5) doubles that.
  - Pretty-printing writes each element as `,\n` plus 248 spaces plus `0`, about 251 bytes where the input has 2. That is a 1.0 GiB `String`. `clean` then allocates another 1.0 GiB, and `cut_at` keeps 16 KiB of it.
  - This runs on the core's one loop thread, which also serves every terminal session. An OOM kill takes down the whole terminal server core, not only the session (`process R3`'s unit is the session).
  - Even without nesting, the pretty form is 1.5–3× the compact one, and every repeated `tool_call_update` that carries `rawInput` pays it in full.
- Fix: produce at most `RAW_LIMIT` (plus a few bytes) while formatting. Use `serde_json::to_writer_pretty` into a small `Write` adapter that errors once past the limit, and treat that error as cut. In `raw_text`, cut before `clean`, or clean only the kept prefix. `railway R5` (no unbounded allocation from outside bytes). Test gap: a deep, wide `rawInput` whose pretty form is hundreds of times its size, checked against a peak-size bound.

### B-3: Each checkpoint segment rewrites the whole non-log state, so cost grows with the square of the tool calls
- Severity: medium
- Confidence: confirmed
- Category: performance
- Where:
  - `remote-agent/src/acp/checkpoint.rs:246-264, 309-372` (`encode_segment` calls `state_fields`: the `tools` map of every tool call the log holds, `methods`, `outputs`, `rpc`, the partial line)
  - `remote-agent/src/acp/reduce.rs:366-367, 1579, 2929` (`tools` grows by one per tool call and shrinks only on a trim or a respawn)
  - `remote-agent/src/server/core/policy.rs:564-578, 669-688`
- Scenario: a long permission-gated session. Each tool call brings two `Moment::Permission` checkpoints (the request and its answer), plus `BeforeRequest`/`TurnEnded` per turn.
  - With T tool calls in the log, each segment carries about 40·T bytes of `tools` (`"toolu_…":12345,`), plus `methods` (up to `MAX_LOGIN_METHODS_LEN`, 256 KiB, unchanged since `initialize`). It adds only a few hundred bytes of events.
  - At T = 5,000 (about 15 MiB of log, under `LOG_LIMIT`), each segment is about 200 KiB. Over the session that is roughly 2T × 40T ≈ 2 GB JSON-encoded on the core thread and sent to the holder.
  - The segments outgrow the base after base/200 KiB moments (about 75 for a 15 MiB base), which forces a whole re-encode of the log. So the log is also rewritten about once per 37 tool calls.
  - The design (`agents-log-paging` decision 9, "What it costs") claims bytes written per byte of log growth stay below a small constant. That claim counts only the events and leaves out this fixed per-segment state. I dispute that part of the decision: it holds only while the non-log state is small next to each segment's events.
- Fix: make a segment's non-log fields deltas. `tools` becomes `tools_added` (call → entry) since the previous record, removals following from `Trimmed`/respawn as the reducer does them. `methods` is left out of a segment unless it changed (it changes only on `initialize`). `decode` folds them as it folds `events`. This is still format 2 if a missing `tools` means "unchanged". Test gap: decision 9's pinned 64 MiB synthetic conversation should include thousands of tool calls with a permission each, and assert the 4× bound.

### B-4: A long line arriving across many reads re-sends its whole partial in every 512 KiB checkpoint
- Severity: low
- Confidence: confirmed
- Category: performance
- Where:
  - `remote-agent/src/acp/checkpoint.rs:309-314, 376-403` (every record carries the whole `Partial::Collecting` bytes)
  - `remote-agent/src/server/core/agents.rs:810-822` (`Stepped(None)` is due past `CHECKPOINT_AFTER` of stdout)
  - `remote-protocol/src/holder.rs:160` (`TAIL_LIMIT` 1 MiB forces those checkpoints)
- Scenario: an agent writes a 4 MiB `tool_call_update` (a 2 MiB file's `oldText`/`newText`) or an 8 MiB line. Every 512 KiB of it triggers a checkpoint carrying the partial line so far: 0.5, 1, …, 8 MiB.
  - That is 18 MiB copied and sent for a 4 MiB line, and about 68 MiB for an 8 MiB line.
  - With a large log, the growing segments outgrow the base after a few, adding 2–3 whole 16 MiB log encodes per such line.
  - It is bounded by `MAX_LINE`, but quadratic in line length.
- Fix: let a segment carry only the partial bytes added since the previous record (for example a `partial_from` in its `lines` object, the decoder concatenating). This keeps format 2's framing and makes the cost linear. The alternative, skipping `Stepped(None)` checkpoints mid-line, is ruled out by `TAIL_LIMIT`.

### B-5: v1 decode deep-clones whole updates, including the fields it then discards
- Severity: low
- Confidence: confirmed
- Category: performance
- Where:
  - `remote-agent/src/acp/v1/decode.rs:387-398` (`split` clones the whole update, `content`, `rawInput` and `rawOutput` included, then removes them from the clone)
  - `decode.rs:487` (`tool_item` clones each item again for `from_value`)
  - `decode.rs:360-364` (`chunk` clones every message chunk)
  - `decode.rs:610` (a permission's `toolCall` takes the same path)
  - `decode.rs:70, 245, 286, 294` (whole-result clones)
- Scenario: a 4 MiB `tool_call_update` diff. The envelope's parse holds `oldText`/`newText` once, `split` copies them, and `tool_item` copies them again before `from_value` moves them into the form. That is three resident copies, plus `Value` overhead. An 8 MiB image chunk in `agent_message_chunk` is copied once more only to be shown as `[image]`. This multiplies B-2's peak too.
- Fix: the `Dialect` methods take `params`/`result` by value (`envelope::Message` already owns them). `split` then `remove`s from the owned map, and items are moved out with `std::mem::take` before `from_value`. This is a signature change confined to `dialect.rs` and `v1/decode.rs` (`ports R14` unchanged).

### B-6: ACP version facts outside `v1/`: the reducer compares JSON-RPC error codes itself
- Severity: low
- Confidence: confirmed
- Category: placement
- Where:
  - `remote-agent/src/acp/forms.rs:16-17` (`AUTH_REQUIRED` -32000, `UNSUPPORTED_VERSION` -32022)
  - `remote-agent/src/acp/reduce.rs:2155, 2362, 2389` (`error.code == …`)
  - `remote-agent/src/acp/dialect.rs:35-37` (`spoken` reads `"protocolVersion"`; that one is documented as version-neutral, and acceptable)
- Scenario: `agents-acp-v2` (proposed) adds a dialect. If v2 signals "authentication required" or "unsupported version" differently, the reducer, not `v2/`, must change. `.claude/rules/remote-agent.md` says the reducer "reads `acp/forms.rs` and `acp/envelope.rs`, never a version". Nothing else in the reducer names a code. `agents-acp-v2/proposal.md` doesn't mention this.
- Fix: `Dialect::failure(&RpcError) -> forms::Failure::{AuthRequired { message }, Unsupported { supported: Option<Vec<u16>> }, Other}`. It absorbs `supported_versions`, and the constants move into `v1/decode.rs` (`ports R14`).

### B-7: Tool content bounds are defined in the transcript crate but the shared fold doesn't enforce them
- Severity: low
- Confidence: confirmed
- Category: types
- Where:
  - `transcript/src/lib.rs:186-200` (`MAX_ITEMS`, `CALL_LIMIT`, `RAW_LIMIT`, `MAX_LOCATIONS`)
  - `lib.rs:1076-1139` (`ToolChanged` and `ToolItems` apply any number of items, any raw length, any number of locations)
  - enforced only in `remote-agent/src/acp/reduce.rs:1674, 1906, 3160`
- Scenario: `.claude/rules/transcript.md` says "A tool entry's content … is bounded in the fold". `docs/code-map.md` says each is "bounded in the fold with what it left out said". Only `Output`'s `ITEM_LIMIT` is: `Output::append` keeps the newest 64 KiB and counts the rest in `omitted`. A client folding a buggy or newer core's `ToolItems` with 10,000 items keeps them all, and its `more` disagrees with what it holds. Today the only producer is the reducer, so this is a latent mismatch between the rule and the code, not a live bug. It is also why B-1's frame bound has no single owner.
- Fix: either refuse in the fold (`FoldError::ItemsPastBound`) or clamp there (counting into `more`), so both sides share one definition. Or correct the rule and the code-map to say the producer bounds and the fold trusts. In both cases add the frame-size relation from B-1.

## Structural observations

1. **`checkpoint.rs` holds four jobs in 1,240 lines:**
   - record framing (`push_record`, `next_record`, the format 1 and 2 headers);
   - the state schema, encoded by hand with `json!` and decoded by hand with about 25 `restore_*` functions;
   - cross-reference validation;
   - adoption policy (`adopt`, `not_restored`, `merge`).

   Each new field must be added in two hand-written places, as `lists`, `chose`, `outputs` and `methods` each were. I would split it into `checkpoint/format.rs` (records and their `Unreadable` kinds), `checkpoint/state.rs` and `checkpoint/adopt.rs`. In `state.rs` I would replace the hand pairs with a private serde-derived mirror (`StateV2`, `PhaseV2`, … with `#[serde(default)]` for the additive fields), so encode and decode are symmetric by construction. The reducer's types stay serde-free.
2. **`merge` (`checkpoint.rs:1226-1241`) combines `reduce::Output`s and encodes the `Moment` precedence rule (`BeforeRequest` wins).** That rule belongs to `Output`/`Moment` in `reduce.rs`, as `Output::then(self, later)`. The reducer surely needs the same composition, and two copies of a checkpoint-ordering rule can drift.
3. **The segment's "every other field whole" schema couples checkpoint cost to the size of the non-log state (B-3).** Design the record so each field is either an event stream or a delta from the previous record. The decoder then folds every field the way it folds `events`, and the base and segment forms share one state reader.
4. **The bounds that must line up live in three crates, with no tie between them:**
   - `transcript`: `TEXT_CHUNK`, `ITEM_LIMIT`, `CALL_LIMIT`, `PAGE_BYTES`;
   - `reduce`: `LOG_LIMIT`, `MAX_PROMPT`, `bound`, `MAX_LISTED`;
   - `remote-protocol`: `MAX_FRAME_PAYLOAD_SIZE`, `MAX_CHECKPOINT_LEN`, `TAIL_LIMIT`.

   "Any one event fits a frame" and "a full log plus a full partial line fits a checkpoint" are stated only in comments, and the first is false (B-1). A `const` assertion or a test in the core that derives each from the others would make the next bound change fail loudly.
5. **`transcript/src/lib.rs` keeps 1,756 production lines in one file:** the types and their wire forms, the fold and trim, paging (`page`, `fetch`, `prepend`, `Window`), and `Output`'s suffix bounding. I would split it into `event.rs` (types and serde), `fold.rs`, `page.rs` and `output.rs` behind the same `lib.rs` re-exports. The public API is unchanged, and the fold's totality is easier to review in isolation.
6. **`v1/decode.rs` takes `&Value` everywhere and clones to reach `serde_json::from_value` (B-5).** A by-value `Dialect` API would let each decoder own and dismantle its input. That is cheaper, and it removes the "split out the hand-read fields from a clone" pattern.
