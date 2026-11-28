# Design

## Context

Every site below was first opened at `5f335dc`, then re-opened (task 1.1) at `486f6e6`, this change's base: `main` with `agents-commands` (#151) and `agents-client-state-fixes` (#153) merged, under `agents-zone-focus-and-scroll` (#154). The line numbers are `486f6e6`'s.

### The bounds today, and where they live

| Bound | Value | Where | What it bounds |
| --- | --- | --- | --- |
| `TEXT_CHUNK` | 64 KiB | `transcript/src/lib.rs:54` | the text of one `Added` or `Appended` (`chunks`) |
| `ITEM_LIMIT` | 64 KiB (`= TEXT_CHUNK`) | `lib.rs:186` | a `Text`'s text, a diff's lines' *text* (`Keep::push`, `diff.rs:117-129`), an `Output`'s kept suffix (`Output::append`, `lib.rs:353`) |
| `MAX_ITEMS` | 32 | `lib.rs:190` | items in a tool call (reducer only, `reduce.rs:2179`) |
| `CALL_LIMIT` | 256 KiB | `lib.rs:194` | the items' `text_len` together, at listing time (`reduce.rs:2179`) |
| `RAW_LIMIT` | 16 KiB | `lib.rs:197` | raw input and result (`raw_text`, `reduce.rs:3646`), after a whole pretty-print and a whole `clean` |
| `MAX_LOCATIONS` | 32 | `lib.rs:200` | locations (reducer only, `reduce.rs:1947`); each path only `bound` (64 KiB) |
| `MAX_COMMANDS`, `MAX_OPTIONS`, `MAX_CHOICES` | 256, 32, 256 | `lib.rs:697-703` | the agent's slash commands and session options (`agents-commands`; reducer only, `bounded_commands` and `bounded_options`, `reduce.rs:826-921`, which also keep each list's encoding within `TEXT_CHUNK`) |
| `MAX_OPTION_ID`, `MAX_OPTION_NAME`, `MAX_OPTION_DESCRIPTION` | 1 KiB, 256 B, 1 KiB | `reduce.rs:778-786` | a session option's id and choice values (left out past it), names and descriptions (cut) |
| `PAGE_BYTES` | 256 KiB | `lib.rs:1054` | a page's or fetch's entries, by `Kept.size` |
| `LOG_LIMIT` / `LOG_TRIM_TO` | 16 / 12 MiB | `reduce.rs:95`, `:101` | the log, by `Kept.size` (each entry's own encoding) |
| `MAX_PROMPT` | 256 KiB | `reduce.rs:104` | a prompt, after `clean` |
| `MAX_SESSION_TITLE` | 256 chars | `reduce.rs:113` | a conversation's title |
| `MAX_LISTED`, `MAX_LISTED_ID`, `MAX_LISTED_TIME`, `MAX_CURSOR` | 256, 1 KiB, 64 B, 1 KiB | `reduce.rs:200-207` | a list's page |
| `bound` | 64 KiB (`TEXT_CHUNK`) | `reduce.rs:3613` | every other string: titles, call ids, paths, commands, notices, messages, option names, signal names |
| `MAX_OPEN_OUTPUTS` | 64 | `reduce.rs:81` | outputs still appended to |
| `MAX_LOGIN_METHODS`, `MAX_LOGIN_ID`, `MAX_LOGIN_DESCRIPTION`, `MAX_LOGIN_ARGS`, `MAX_LOGIN_ARG`, `MAX_LOGIN_METHODS_LEN` | 16, 1 KiB, 4 KiB, 32, 4 KiB, 256 KiB encoded | `acp/forms.rs:94-108` | login methods, at decode (`v1/decode.rs:116-148`) |
| `MAX_PENDING_PERMISSIONS` | 16 | `acp/rpc.rs:21` | waiting permissions |
| `MAX_ID_LEN` | 1 KiB | `acp/envelope.rs:17` | JSON-RPC ids |
| `MAX_LINE` | 8 MiB | `acp/lines.rs:13` | one line of agent stdout; the partial line is at most `MAX_LINE + 1` |
| `MAX_DIFF_LINES` | 200,000 | `acp/diff.rs:17` | lines compared |
| `plain::MAX_STRING` | 64 KiB | `acp/plain.rs:18` | an escape string skipped; the held line is at most `ITEM_LIMIT` (`plain.rs:187`) |
| `MAX_FRAME_PAYLOAD_SIZE` | 1 MiB | `remote-protocol/src/lib.rs:56` | one frame |
| `TRANSCRIPT` / `AGENT_PAGE` envelope | 34 / 83 bytes | `client.rs:762`, `:781` (private) | the bytes around a frame's events |
| `MAX_CHECKPOINT_LEN` | 64 MiB | `remote-protocol/src/holder.rs:162` | what a holder stores |
| `TAIL_LIMIT` | 1 MiB | `holder.rs:160` | the stdout a holder keeps past its checkpoint |
| `MAX_STDERR_TAIL` | 16 KiB | `holder.rs:151` | the stderr ring |

Two invariants are stated only in comments: "any one event fits a frame" (`send_transcript`'s doc, `agents.rs:857-859`; `replay_entry`'s, `lib.rs:1794`; `ITEM_LIMIT`'s, `lib.rs:182-185`) and "the log and a partial line fit a checkpoint" (`checkpoint.rs:57-60`; `Agent.unstorable`'s "the bounds make this unreachable", `agents.rs:141`). Both are false today (A-1, A-2).

### Findings, traced

- **A-1, traced.** `chunk` (`reduce.rs:1739-1760`) appends every piece to the run's entry with no size check; `trim_to` (`lib.rs:1689-1710`) never drops the newest entry. Confirmed as reported.
- **A-2 / B-1, traced.** Each site holds: `Keep::push` counts `line.text().len()`; the fallback (`diff.rs:34-43`) allocates every line before `push` sees it; `FileChange::text_len` undercounts the same way; locations are 32 × `bound` paths; `requested_option` (`v1/decode.rs:630-644`) keeps any number of options with any id; the piece split (`reduce.rs:2207-2220`) ignores an output's command and directory; terminal ids are map keys at any length (`reduce.rs:1923-1940`, `:1957-1980`, `:2155-2176`). Two more fields turned up while tracing every event's fields:
  - the ACP session id `new_session_result` returns (`v1/decode.rs:252-259`) is unbounded; `ready` (`reduce.rs:2759`) puts it in a `Conversation` event, the `Phase`, and every later request;
  - `prompt_result`'s `StopReason::Other { name }` (`v1/decode.rs:321`) is unbounded and reaches `Turn` and a held slot (`HoldReason::Stopped`).
  And one more fault in the same class: `CALL_LIMIT` is checked when items are listed (`reduce.rs:2179`), when a new output's text is empty; outputs then grow to `ITEM_LIMIT` each through `ToolOutput`, and `open_output` (`reduce.rs:2068`) adds outputs checking only `MAX_ITEMS`. A tool call can hold 32 full outputs, 2 MiB of text, against the spec's "256 KiB of items".
- **B-2, traced.** `raw` (`v1/decode.rs:518-524`) pretty-prints whole; `raw_text` (`reduce.rs:3646`) cleans whole before `cut_at`. Confirmed.
- **B-7, traced** (taken from the report without a second reading; read now). `apply`'s `ToolChanged` and `ToolItems` arms (`lib.rs:1163-1226`) copy what they are given; only `Output::append` bounds. `.claude/rules/transcript.md` and `docs/code-map.md` say the fold bounds. Confirmed; task 1.2 shows it with a failing test first.
- **D-8, traced** (taken without a second reading; read now). `diff_rows` (`src/agent/tool.rs:284-317`) does `old += 1` / `new += 1` on the hunk's `u32` starts, and `Hunk` (`lib.rs:270-275`) derives `Deserialize` with no check. Confirmed; task 1.3 shows it first.

## Goals / Non-Goals

**Goals:**
- The two invariants, stated in one module, derived from the constants by constant assertions, and checked by a property test over adversarial agent output.
- No single agent field, entry or step can make an event pass `MAX_EVENT`, the log pass `LOG_LIMIT` after a step, or a whole checkpoint pass `MAX_CHECKPOINT_LEN`.
- Raw values cost at most their bound to keep.
- The tool content bounds have one definition, which the reducer applies and the fold enforces on both sides.
- No panic on a hunk's numbers from the wire.

**Non-Goals:**
- The checkpoint segment's cost (B-3, B-4: `agents-checkpoint-deltas`), `Output`'s memmove (A-4: same), repeated diffs (A-5).
- A per-entry bound on the client beyond what the fold already does (D-9 is `agents-absorb-cost`'s).
- Joining a reply split at 1 MiB back into one bubble on the client (choice C1 accepts two).
- A new protocol version: nothing here changes an encoding.
- A client's own requests under its queue's marks (C-6): `agents-client-request-marks`.

## Layers

| Part | Kind | Crate, file |
| --- | --- | --- |
| `MAX_EVENT`, `MAX_PATH`, `ENTRY_FRAMING`, `HUNK_FRAMING`, `LINE_FRAMING`, `SIGNAL_RESERVE`, `ToolItem::wire_len`, `ContentBudget`, `ToolItem::bound`, `Event::encoded_len`, `escaped_len` (made public), the fold's clamps, `Kept.size` with its framing, `replay_entry`'s grouped items, the `Hunk` decoder's check | pure, shared contract | `brindle-transcript`, `transcript/src/lib.rs` |
| `TRANSCRIPT_ENVELOPE`, `AGENT_PAGE_ENVELOPE` made public | codec | `brindle-remote-protocol`, `remote-protocol/src/client.rs` |
| `acp/limits.rs`: the reducer's limits, the invariants' doc, constant assertions, the derivation and property tests | pure (whole pure module) | `brindle-remote-agent` |
| `ENTRY_LIMIT` split (`text_run`), the emit guard (an oversized event faults the step, naming itself), option, terminal-id, session-id and stop-name bounds, `raw_text`'s order, `cleaned_len` | pure | `remote-agent/src/acp/reduce.rs` |
| `Keep` measuring before it allocates | pure | `remote-agent/src/acp/diff.rs` |
| `raw` through a capped writer | pure (decoder) | `remote-agent/src/acp/v1/decode.rs` |
| comments on `TooLarge`; a failed step's message logged (`agent_apply`) | shell (the core's loop) | `server/core/agents.rs` |
| `diff_rows` numbering | pure helper | `brindle`, `src/agent/tool.rs` |

No crate gains a dependency. `brindle-transcript` still names only `serde` and `serde_json`; it can't name the frame size, so the relation `MAX_EVENT + AGENT_PAGE_ENVELOPE ≤ MAX_FRAME_PAYLOAD_SIZE` is asserted in `remote-agent`, the one crate that sees both (`ports R3`).

## Decisions

### 1. The bounds in one place: two invariants, `MAX_EVENT`, and `acp/limits.rs`

`brindle-transcript` gains `pub const MAX_EVENT: usize = 1024 * 1024 - 1024` (1,047,552): the most one event of protocol 4 encodes to. It is the event's own contract, so it lives with the event; the transcript crate never names the frame. `remote-protocol`'s `client.rs` makes its two envelope lengths public as `TRANSCRIPT_ENVELOPE` (34) and `AGENT_PAGE_ENVELOPE` (83), and `pack_transcript`/`pack_page` use them.

A new whole pure module, `remote-agent/src/acp/limits.rs`, is where the bounds meet:
- Its doc states the invariants:
  - **I1:** every event the reducer emits, and every event of `Transcript::replay`, `page` and `fetch` of a log the reducer built, encodes to at most `MAX_EVENT`, so it fits a `TRANSCRIPT` or `AGENT_PAGE` frame (`MAX_EVENT + AGENT_PAGE_ENVELOPE ≤ MAX_FRAME_PAYLOAD_SIZE`, and `AGENT_PAGE_ENVELOPE ≥ TRANSCRIPT_ENVELOPE`).
  - **I2:** after every step, the log is within `LOG_LIMIT`, and a whole checkpoint of any session the reducer can reach encodes within `MAX_CHECKPOINT_LEN`.
- It owns the reducer's limits: `LOG_LIMIT`, `LOG_TRIM_TO` and `MAX_PROMPT` move here from `reduce.rs` (re-exported there, so `policy.rs`'s and the tests' paths keep working until `agents-module-structure` tidies them), and the new `ENTRY_LIMIT`, `FIELD_LIMIT` (the 64 KiB `bound` uses, `= TEXT_CHUNK`), `MAX_OPTIONS`, `MAX_OPTION_ID`, `MAX_OPTION_NAME`, `MAX_TERMINAL_ID`, `MAX_SESSION_ID`.
- Its doc carries the table of every bound in the Context above, each with its home, so the next field added has one place to look.
- It holds `const` assertions of the linear relations (below) and the two tests of decision 8.
- It joins `fcis`'s "Whole pure modules" table and `PURE_MODULES` in `tests/architecture.rs`.

**The derivation of I1.** Every string the reducer emits is cleaned (`clean`: each control character but `\n`, `\t` and a final `\r` becomes U+FFFD), so it encodes to at most twice its bytes (`"`, `\`, `\n`, `\t`, `\r` take two; U+FFFD and every other character encode as their own bytes). The exceptions are option ids (refused with a control character, decision 3, so also at most twice) and `refuse`'s message, which `bound` cuts without cleaning (at most six times: `\u00XX`). `F` = 128 bytes, the most an event's tag, id and field names take. `T` = `TEXT_CHUNK` = 65,536.

| Event | Largest fields | At most (bytes) |
| --- | --- | --- |
| `Added` of user, agent or thought text; `Appended` | one `chunks` piece | 2T + F = 131,200 |
| `Added` of a notice | `bound` text | 2T + F = 131,200 |
| `Added` of a tool call (live, and replayed without content); `ToolChanged` | call 2T, title 2T, 32 × (path 2 × `MAX_PATH` + 40), input and result 2 × 2 × `RAW_LIMIT`, kind, status, more | 591,328 |
| `ToolItems` | a piece of items whose `wire_len` adds up to at most T, or one larger item alone: at most `W` = an output's `ITEM_LIMIT` + command T + directory `MAX_PATH` + `SIGNAL_RESERVE` = 135,232; then 2 × W + 32 × 192 (item framing) + F | 276,736 |
| `ToolOutput` | a piece (T) or the filter's held line (`ITEM_LIMIT`), exit name `SIGNAL_RESERVE` | 2T + 128 + F = 131,328 |
| `Added` of a permission | call 2T, title 2T, 16 × (id 2 × 1,024 + name 2 × 4,096 + 48) | 426,880 |
| `Answered` | option id 2 × 1,024 | 2,176 |
| `Queue` | prompt 2 × `MAX_PROMPT`, a held slot's stop name 2T | 655,488 |
| `Turn` | a failure message or stop name, 2T | 131,200 |
| `Agent` | `LoginRequired`: message 2T + methods `MAX_LOGIN_METHODS_LEN` (measured encoded at decode, and cleaning never lengthens an encoding); `Failed` from `refuse`: 6T | 393,344 |
| `Conversation` | id 2 × `MAX_SESSION_ID`, directory (the agent was started in it: at most the platform's path limit, 96 KiB as UTF-8 on Windows) 2 × 98,304, title 2 × 4 × 256 | 201,344 + F |
| `Trimmed`, `Window` | two integers | F |
| `Commands`, `Options` (`agents-commands`, added at task 1.1) | the whole list, which `bounded_commands` and `bounded_options` keep within `TEXT_CHUNK` *encoded* (each item measured with `serde_json::to_vec`, `LIST_EVENT_ROOM` = 64 kept for the event's own fields) | T + F = 65,664 |

The largest is `Queue`, 655,488, which leaves 392,064 bytes under `MAX_EVENT`. Replayed events are the same rows: a text entry replays in `chunks` pieces; a notice and a permission replay as their live `Added` (a permission's outcome as `Answered`); a tool call as its header `Added` and its items grouped as the live split groups them (decision 2); the slot, conversation, turn, agent, commands and options as their latest event. `agents-plans` and `agents-prompt-images` each add a row when they land (proposal, Impact); `agents-commands`' row is above. The table names every `Event` variant at `486f6e6`: `Added` (its five entry kinds), `Appended`, `ToolChanged`, `ToolItems`, `ToolOutput`, `Answered`, `Queue`, `Turn`, `Agent`, `Conversation`, `Trimmed`, `Window`, `Commands`, `Options`.

**The derivation of I2.** Three facts:
- **The log after a step is within `LOG_LIMIT`.** `trim` runs at every step's end (`Step::finish`) once the log passes `LOG_LIMIT`, and `trim_to(LOG_TRIM_TO)` drops oldest entries while the log is over `LOG_TRIM_TO`, keeping the newest. It reaches `LOG_TRIM_TO` if the newest entry alone is within it. After decision 2, an entry is at most: a text entry 2 × `ENTRY_LIMIT` + `ENTRY_FRAMING` + 64 = 2,097,280; a tool entry its header (591,328) plus its content (2 × `CALL_LIMIT` + 32 × 192 = 530,432); a permission 426,880; a notice 131,200. The largest, 2,097,280, is under `LOG_TRIM_TO` (12 MiB), so every trim makes progress and ends within `LOG_TRIM_TO`.
- **A log's stored form is at most twice its measure.** A whole checkpoint stores the log as `replay()`'s events and the `tools` map (call → entry). `Kept.size` now counts `ENTRY_FRAMING` = 64 bytes an entry beside its own encoding (decision 2). Per entry, replay adds at most: an `Added` wrapper (`{"event":"added","id":<20 digits>,"entry":…}`, 51 bytes), an `Answered` (60), one `Appended` wrapper per 64 KiB of text (48), and one `ToolItems` wrapper (69) per piece, where grouping keeps pieces to at most 8 a call. The `tools` map adds the call id and 24 bytes for a tool entry, which already holds the call id. Checked over each entry kind's smallest form (an empty user entry is 25 bytes, an empty tool entry 68 plus its call), replay plus map is at most twice `Kept.size`. Without the framing and the grouping it is not: an empty user entry replays at 76 bytes for 25 measured (3.04 ×), and a tool entry of 32 empty text items at 3.4 ×.
- **The rest of the state is bounded.** The partial line is at most `MAX_LINE + 1` raw bytes. The methods are at most `MAX_LOGIN_METHODS_LEN`. Each of `MAX_OPEN_OUTPUTS` open outputs holds a terminal id (`MAX_TERMINAL_ID`), a held line (2 × `ITEM_LIMIT` encoded) and its filter's mode (128). The rest (the non-entry replay events, the phase with its greeting, login message and conversation, `Rpc`'s 16 permission ids and few requests, the run, the lines' state, the headers) is under `STATE_SLACK` = 3 MiB.

So a whole checkpoint is at most 2 × `LOG_LIMIT` + `MAX_LINE` + 1 + `MAX_LOGIN_METHODS_LEN` + `MAX_OPEN_OUTPUTS` × (`MAX_TERMINAL_ID` + 2 × `ITEM_LIMIT` + 128) + `STATE_SLACK` = 33,554,432 + 8,388,609 + 262,144 + 8,462,336 + 3,145,728 = 53,813,249 bytes, under `MAX_CHECKPOINT_LEN` (67,108,864) by about 12.7 MiB.

The `const` assertions write these sums from the constants, so raising any bound past them fails the build. The ratio 2 and `STATE_SLACK` are not constants of the code; decision 8's derivation test checks them by building each extreme.

### 2. A text entry is at most 1 MiB, and the log counts each entry's framing (A-1; choice C1)

`ENTRY_LIMIT` = 1 MiB of text. `chunk` and `add_text` both go through one new step, `text_run(kind, text)`:
- While the run's entry (same kind, still in the log) has room, it appends pieces (`chunks`) to it, each cut to the room left: at the last `\n` within it when there is one, else at a character boundary.
- When a piece doesn't fit whole, the rest starts a new entry of the same kind (`Added`, then `Appended`s), which becomes the run.
- Text is never cut or dropped; joined, the entries' texts are the text the agent sent.

Cutting at a line break keeps a Markdown paragraph or a code fence's line whole. A fence open across the cut renders unclosed in the first entry and its rest as text in the second; that is the accepted cost of C1, which shows two bubbles for such a reply. A user entry (a prompt is at most `MAX_PROMPT`) never splits from the client; one replayed by a shown `session/load` (`user_message_chunk`s) can, and goes through the same step. A muted load adds nothing, as today.

The run's length is read from the log (`Transcript::entry(run.entry)`'s text), so the check is O(1) a chunk and needs no new state in `Run` or the checkpoint.

`Kept.size` (the fold's measure of an entry, which `LOG_LIMIT`, `trim_to`, `PAGE_BYTES` and `newest_within` count) becomes the entry's encoding plus `ENTRY_FRAMING` = 64. Both sides fold it the same way; nothing is stored. This makes I2's "twice" hold (decision 1). A page of the default 256 KiB holds a little less: 64 bytes an entry, which matters only for logs of tiny entries. `replay_entry` groups a tool entry's items into `ToolItems` pieces by the live split's rule (at most `TEXT_CHUNK` of `wire_len` together, or one larger item alone) in place of one event an item; a fold of either form gives the same entry, so format 2 checkpoints written before still read.

`TooLarge` is then unreachable (I2), and the code says so with its reason: `Agent.unstorable`'s doc and `agent_checkpoint`'s cite `acp::limits` I2; `checkpoint.rs`'s **Bounds** paragraph gives the sum. The path stays as it is (a value, then `Input::Fault`; `railway R1`), now as a guard against a future bound. The archived designs aren't edited: `agents-sessions`' "As built in 4.5 and 4.6" ("which the bounds make unreachable") and `agents-log-paging`'s Risks ("a huge single entry … bounded by the 16 MiB log") were false until this change, and this design records that.

### 3. Limits count what goes on the wire (A-2)

**The measure.** `ToolItem::text_len` becomes `ToolItem::wire_len`, "text plus framing":
- a `Text`: its text's bytes;
- a `Diff`: its path's and source path's bytes, plus each kept line's bytes and `LINE_FRAMING` = 4 (the sign, two quotes, a comma), plus `HUNK_FRAMING` = 64 for each hunk (`{"old_start":4294967295,"new_start":4294967295,"lines":[]},` is 59);
- an `Output`: `ITEM_LIMIT` for its text (what it may grow to, not what it holds now), plus its command's and directory's bytes, plus `SIGNAL_RESERVE` = 64 for its exit.

Counting an output at its full `ITEM_LIMIT` makes the measure a function of fields no `ToolOutput` changes: the text grows only to `ITEM_LIMIT`, and an exit signal's name is cut at `SIGNAL_RESERVE`. So `CALL_LIMIT` holds as outputs fill, which it doesn't today, and decision 6's clamp in the fold can never drop an item the reducer kept. The cost: a tool call holds at most three outputs (with their commands) in its 256 KiB. Every captured agent opens one terminal a call. The limit was accepted on 2026-11-27: what is lost is fidelity in the log and the zone only. The outputs past the third aren't kept or shown, and the row counts them as left out; the agent runs its own commands and reads their output itself, since Brindle offers no `terminal/*`.

The escaping factor (at most 2 for cleaned text) is left out of the measure and taken into decision 1's derivation instead, so the user-facing numbers (64 KiB an item, 256 KiB a call) stay what the spec says.

**`ContentBudget`** (transcript, pure) holds the rule "at most `MAX_ITEMS` items whose `wire_len`s add up to at most `CALL_LIMIT`; an item that doesn't fit is left out and counted, and later ones are still tried". The reducer's `tool_content`, `open_output` and `describe_output` take it before every `ToolItems` they emit:
- `open_output` leaves a terminal's output out (counted in `more` by a `ToolChanged`) when the budget has no room;
- `describe_output` doesn't add a command and directory that would pass it.

The fold's clamp (decision 6) uses the same type, so the two can't drift.

**The diff.** `Keep::push` takes the raw line (`&str`) and its sign, measures `cleaned_len(line) + LINE_FRAMING` (and `HUNK_FRAMING` with a hunk's first kept line), and only then allocates `clean(line)`. Once full, it counts the rest without allocating; the fallback past `MAX_DIFF_LINES` adds the remaining line count in one step rather than pushing each. `cleaned_len` (beside `clean` in `reduce.rs`) is the byte length `clean` would return, computed without building it: control characters other than `\n`, `\t` and a final `\r` take 3 bytes. A unit test checks it against `clean(t).len()` over random strings. Hunks then come out of `diff::hunks` already cleaned, and `tool_content`'s `clean_line` loop goes. A file of 300,000 empty lines keeps at most 16,384 lines (64 KiB / 4).

**The piece split** (`tool_content`, `reduce.rs:1934-1947`) and `replay_entry` both split by `wire_len`.

**Paths.** `MAX_PATH` = 4 KiB (transcript): a location's path, a diff's path and a move's or copy's source, and an output's directory are cut there, at a character boundary, after `clean`. A command stays at `FIELD_LIMIT`.

**Permission options** are bounded in the reducer's `permission`, not in `v1/decode.rs`:
- At most `MAX_OPTIONS` = 16 known options, in the agent's order.
- An option whose id is empty, over `MAX_OPTION_ID` = 1 KiB, or holds a control character is left out, since a cut or cleaned id would answer something else.
- A name is cut at `MAX_OPTION_NAME` = 4 KiB, after `clean`.
- One notice says how many options were left out, beside today's notice for unknown kinds. A request left with none is answered `cancelled`, as one offering none is today.

The bound is Brindle's policy, not a fact of ACP v1, so it belongs where every dialect's requests meet (`ports R14`); the decoder's memory is already bounded by `MAX_LINE`. Login methods are bounded at decode because `auth_method` builds the form there, and they stay so.

**Terminal ids** over `MAX_TERMINAL_ID` = 1 KiB are ignored in `tool_detail` and `tool_content`: such a terminal's output isn't kept, and a cut id could merge two terminals. **The session id** `created` takes from `session/new` is refused past `MAX_SESSION_ID` = 1 KiB, empty, or holding a control character, through `refuse` ("the agent's session id is longer than 1 KiB, or isn't one Brindle can send back"), as `MAX_LISTED_ID` does for listed ones. **A stop reason's name** is `bound(clean(..))` in `prompted`, as titles are.

### 4. An oversized event fails the session and says why (choice C2, simplified on 2026-11-27)

`Step::emit` measures each event before applying it, with `Event::encoded_len` (`serde_json::to_writer` into a byte counter, no allocation). One over `MAX_EVENT` is not applied, and the step faults exactly as it does on a fold error: `self.fault = Some(reason)`, every later `emit` of the step does nothing, and `finish` calls `fail`. `emit`'s and `add`'s signatures don't change, and no caller handles a withheld event.

**The failure states its cause.** The reason is built where the size is known:
- for an entry's event: `a {kind} event for entry {id} is {len} bytes; one transcript event may be at most {MAX_EVENT}`, with `{kind}` the event's own name (`added`, `appended`, `tool_changed`, `tool_items`, `tool_output`, `answered`) and, for an `Added`, the entry's kind after it (`added (tool)`);
- for the session's events (`Queue`, `Turn`, `Agent`, `Conversation`): `the session's {kind} event is {len} bytes; one transcript event may be at most {MAX_EVENT}`.

`fail` shows it as it shows every fault, as `AgentState::Failed { message: "ACP handling failed: {reason}" }`. So the cause is in the zone's failed state, in the dump's `state=failed message="…"`, in every later replay, and in the checkpoint. The core also logs it: when a step's events hold a failed agent state, `agent_apply` writes `session {id}: {message}` to the core's log, which a fold error doesn't get today (only a caught panic is logged). `fail`'s own events are small, so the guard can't fire on them.

**Why fail rather than replace.** The plan's C2 first replaced an oversized `Added` with a notice and left other events out, with the rest of the step sent. That needed `emit` to report what it did, four callers to handle a withheld entry, and a rule for a withheld `Queue`, `Turn` or `Agent` event, where the log would fall behind what the reducer did. I1 makes the case unreachable and the property test of decision 8 asserts it, so the guard exists only to make a future bounds mistake loud and self-describing. The fault path does that with one check, is already tested, and can't leave the log and the reducer apart. The cost is the session, not one entry, for a bug the property test should catch first.

**Why in the reducer and not where frames are packed.** The core's log, its checkpoint and every client hold the same events only if an oversized event never enters the core's log. A refusal at `send_transcript` (`agents.rs:840-856`) or the replay (`sessions.rs:1955-1983`) would leave the core's log holding an entry no client can be sent, which is today's fault: the batch is dropped without a word, and an attach replays nothing. A replay rebuilt from the log needs no guard, because its events are bounded by decision 1's table: the per-field bounds, the fold's clamps (decision 6) and the grouping of decision 2. `pack_events` keeps refusing an oversized event, and `send_transcript`'s and `pack_events`' docs now cite I1 rather than "the log's bounds".

With `agents-process-lifecycle`'s fault rule, the faulted step writes nothing it asked for to the agent, and the agent's waiting permissions are answered `cancelled`.

**Cost.** One counting serialization an event, O(its size), on the core's loop thread. Every event is already serialized once for the client (`to_value`) and once into `unsaved`. The perf cases `agent-markdown-stream` (2,048 32-byte chunks, then one) and `agent-tool-output` (256 4 KiB deltas, then one) report their `[stream]` times before and after (`testing R10`); the change is accepted if they stay within the runs' noise. If they don't, the measure is skipped for `Appended` and `ToolOutput`, whose text is at most one `TEXT_CHUNK` piece by construction.

**As run (task 8.1, 2026-11-27, release builds under `scripts/with-xvfb`, private servers):** before, at `486f6e6` with only the tests of section 1 added (`target-eb/tmp/perf/20261127T235917Z`); after, at `b7ce15b`, tasks 2 to 4.5 in (`20261128T002956Z`, run with `BRINDLE_PERF_BASELINE` set to the first).

| `[stream]` | streamed (ms) | one chunk (ms) | difference (ms) | chunks |
| --- | --- | --- | --- | --- |
| `agent-markdown-stream` before | 2265.60 | 2260.01 | 5.59 | 2048 |
| `agent-markdown-stream` after | 2259.29 (−6.30) | 2258.37 (−1.64) | 0.92 | 2048 |
| `agent-tool-output` before | 2252.12 | 2254.38 | −2.25 | 256 |
| `agent-tool-output` after | 2252.91 (+0.79) | 2256.78 (+2.40) | −3.87 | 256 |

Each time includes one step interval (1 s) and the gate, so the runs' noise is several milliseconds; both changes are within it, one of them faster. The guard measures every event, and the fallback isn't needed.

### 5. Raw values are formatted only up to their bound (B-2)

`v1/decode.rs`'s `raw` writes `serde_json::to_writer_pretty(value)` into `Capped { bytes: Vec<u8>, limit }`, a `std::io::Write` whose `write` takes bytes up to `limit` and then returns an error (`ErrorKind::WriteZero`). serde_json stops at the first failed write, so neither the time nor the memory goes past the limit, beyond what decoding the line already took. The kept bytes are cut back to the last whole UTF-8 character. A JSON string value is cut the same way, without copying the rest. `limit` = `RAW_LIMIT + 4`, so a longer value comes out longer than `RAW_LIMIT`, and the reducer marks it cut. `raw_text` then cuts at `RAW_LIMIT` first, cleans the kept prefix (at most 3 × `RAW_LIMIT`), and cuts again to `RAW_LIMIT − "…".len()` with `…`, so the result is what today's order gives for every value whose cleaned prefix fits.

The test (`v1/decode.rs`): a `rawInput` of 120 nested arrays around 200,000 zeros, whose pretty form is about 50 MB. The decoded input is at most `RAW_LIMIT + 4` bytes, and `Capped`'s count of bytes offered is at most `limit` plus one write's length.

### 6. The fold enforces the tool content bounds it defines: clamp, not refuse (B-7)

The fold clamps:
- `Added` of a tool, `ToolChanged` and `ToolItems` apply `ToolItem::bound` to each item:
  - a `Text`'s text cut at `ITEM_LIMIT` with `cut`;
  - a `Diff`'s paths cut at `MAX_PATH`, and its lines past `ITEM_LIMIT` (counted as `Keep` counts them) dropped into `omitted`, emptied hunks removed;
  - an `Output`'s text kept to its suffix by `Output::append`'s own rule, its directory cut at `MAX_PATH`, command at `TEXT_CHUNK`, signal name at `SIGNAL_RESERVE`.
- The resulting list goes through `ContentBudget`. When it drops items, `more` becomes at least the number dropped.
- Locations are cut to `MAX_LOCATIONS` and their paths to `MAX_PATH`; `input` and `result` to `RAW_LIMIT` with `…`.

Within the bounds every clamp is the identity, and a clamped value clamps to itself. The clamp depends only on the resulting entry, so folding pieces still equals folding their concatenation.

Clamping rather than refusing: a refusal (`FoldError`) breaks the client's session for content it could show bounded. Clamping keeps the session and says what it left out (`cut`, `omitted`, `more`), as every other bound does (`railway R6`). Both sides run the same `apply`, so the core's log and a client's copy agree.

The fold enforces only the tool content bounds the transcript crate defines. The rest stay the producer's (the reducer's), and the rule text says so:
- `ENTRY_LIMIT`: a client can't split an entry, and the core never sends one past it.
- Option counts and ids: an id can't be cut.
- Call ids, titles, notices, messages: `FIELD_LIMIT`.
- The slot and the agent's state.

I1 holds for the core's log because the reducer bounds all of them; the property test checks it.

**An older core's events under this definition.** Protocol 4 is unreleased, so the only older cores are pre-release builds. Their events within the bounds fold as before. A pre-release core could send content past them: a diff of many empty lines; more than three outputs in a call, now that an output counts at its full `ITEM_LIMIT`; a long path. Those events would also have failed the frame in the cases A-2 names. The newer client clamps them, so its copy differs from that core's log. A later `ToolItems` or `ToolOutput` naming an item the client dropped then fails its fold, and the zone shows the session broken, as a frame failure does today. The core and Brindle upgrade together, so this lasts until the core's next upgrade. A checkpoint a pre-release core wrote folds through the same clamps on restore, so the restored log is bounded from then on.

### 7. Hunk numbers: checked on the client, bounded at decode (D-8)

- `diff_rows` (`src/agent/tool.rs`) keeps `Option<u32>` counters advanced with `checked_add`. Once a side overflows, its later rows have no number, which the row already draws for an added or removed line's other side. It panics on no input.
- `Hunk` decodes through a private form (`#[serde(try_from = "HunkForm")]`). The form is refused when a start is 0, or when the start plus the count of lines that side numbers, minus one, passes `u32::MAX`. A malformed event is a `DecodeError`, which the zone shows as its session broken (`railway R5`).
- The encoding is unchanged. The producer (`diff.rs`) never makes such a hunk: its starts are line indexes plus one, at most `MAX_LINE` lines, and the fallback starts at 1. `fuzz.rs`'s decoder fuzz gains hunk values near the edges.

Both are needed. The decoder's check makes every `Hunk` a client holds well-formed (`types R7`, parse at the boundary). The client's checked arithmetic keeps a view helper total, whatever its caller.

### 8. Tests: the derivation and the adversarial property

In `acp/limits.rs`'s test module:
- **`the_frame_and_checkpoint_bounds_follow_from_the_constants`** builds each event of decision 1's table at its largest, every field at its bound and filled with `"` so it escapes to twice its bytes, and asserts each encodes to at most `MAX_EVENT`. It builds the session of I2's sum:
  - a log of `LOG_LIMIT` in empty user entries with 20-digit ids (a `Trimmed` through `10^19` on the empty log sets the next id);
  - 64 open outputs with full held lines;
  - full methods, a full partial line;
  - the largest slot, conversation and agent state.

  It asserts `checkpoint::encode` returns `Ok`, and that replay plus map is at most twice `encoded_len()` for each entry kind's smallest form.
- **`every_event_fits_a_frame_over_adversarial_agent_output`**: a seeded XorShift (the pattern of `checkpoint.rs`'s `random_steps_checkpointed_whole_or_appended_decode_to_the_live_session`) steps a ready session through 2,000 random lines. Each is drawn from:
  - text chunks of 1 B to 2 MiB of quote-, backslash- and control-heavy text, alternating kinds;
  - `tool_call`s and updates with 0 to 40 items: empty-line diffs of up to 300,000 lines, both texts past `MAX_DIFF_LINES`, terminals with 64 KiB ids, commands and directories, 40 locations of 64 KiB paths, deep and wide `rawInput`;
  - `_meta` terminal infos, deltas and exits with long signal names;
  - permissions with 0 to 40 options, ids from empty to 2 KiB and with controls, 64 KiB names;
  - `session/new` and prompt answers with long session ids and stop reasons;
  - prompts up to `MAX_PROMPT`.

  After each step it asserts:
  - every emitted event's `encoded_len` ≤ `MAX_EVENT`;
  - every event of `replay()`, of `page(b)` for `b` in {1, 16 KiB, `PAGE_BYTES`} and of `fetch(next_id, PAGE_BYTES)` likewise;
  - `encoded_len() ≤ LOG_LIMIT`;
  - every tool entry within `ContentBudget` and every item within `ToolItem::bound`, so the fold's clamps were the identity on the core;
  - every open output names an `Output` item of its entry;
  - no fault was recorded (no `AgentState::Failed`);
  - `pack_transcript` of the step's events is `Ok`.

  Every 100 steps it asserts `checkpoint::encode` is `Ok`. The seed is fixed (`testing R11`).

`reduce.rs`'s `tool_content_is_bounded_before_any_event` keeps its fixed cases and gains the empty-line diff and long-path cases. `transcript/src/fuzz.rs` gains:
- `the_fold_never_holds_tool_content_past_its_bounds`: random events, some past every bound, then every tool entry is within them;
- `clamping_is_the_identity_within_the_bounds_and_idempotent`;
- the pieces law (`folding pieces equals folding their concatenation`) re-run with clamps.

### 9. Wire, checkpoint and versions (choice C4)

- **Client protocol 4** is amended in place: it has not shipped (v0.3.0 predates it). No event kind, field or enum value changes, and no encoding changes:
  - `MAX_EVENT`, the clamps and the per-field bounds are bounds on values the forms already carry;
  - the `Hunk` check refuses values the producer never makes;
  - the grouped `ToolItems` of a replay are an existing form.
- **Fixtures.** The fixtures that pin protocol 4's encodings, `transcript/tests/fixtures/v4/*.json` (through `transcript/tests/events.rs`) and `remote-protocol/tests/fixtures/client-agent-*.hex` (through `remote-protocol/tests/transcripts.rs`), are unchanged and must pass unchanged (`process R5`, `R6`). Their values are all within the bounds.
- **Checkpoint format 2** is unchanged: `Kept.size` isn't stored, and a base's log of one-item `ToolItems` (written before) and of grouped ones (written after) both fold.
- **H1** is unchanged.
- **`pack_transcript` and `pack_page`** gain no behaviour; their envelope constants become public.

### 10. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

- **A core restart** adopts the checkpoint. The log folds through the new clamps and measure, so a pre-release checkpoint over a bound comes back clamped, and its `Kept.size`s are recomputed with `ENTRY_FRAMING`. A run, tools map and outputs that name an entry or item a clamp removed already fail `checkpoint::decode`'s cross-checks as "names entry N, which the log doesn't hold". The decoder treats a removed item the same: the session is adopted as not restored, with one error notice. Only a pre-release checkpoint over a bound can do it.
- **A Brindle crash** changes nothing here. The next attach replays as before.
- **An upgrade** hands the core's sessions over by checkpoint, which is the core-restart case.
- **The holder** is untouched (`process R1`).

### 11. Archived decisions this changes

- **`agents-tool-content` decision 5.** "The reducer applies every bound before an event is made, except an output's tail" becomes: the reducer applies every bound, and the fold also enforces the tool content bounds, the identity on the reducer's events. "A path, a command, a `cwd`: as titles are" becomes 4 KiB for paths and directories. `CALL_LIMIT` counts `wire_len`, an output at its full `ITEM_LIMIT`. "`ITEM_LIMIT` equals `TEXT_CHUNK`, so any one item fits one event" is replaced by decision 1's table.
- **`agents-tool-content` decision 15 and the code comment at `lib.rs:1678`.** "One item per event, so any one event stays within a frame" becomes grouped pieces, each within `MAX_EVENT`.
- **`agents-sessions` decision 14** ("the log, 16 MiB encoded, oldest entries trimmed"). The log's measure counts each entry's 64-byte framing, and a text entry is at most 1 MiB.
- **`agents-log-paging` decision 3** ("the newest whole entries whose encoded lengths add up to at most `budget`"). Each length now includes `ENTRY_FRAMING`. Its Risks line on "a huge single entry" was false and is answered by decision 2.

## Alternatives rejected

- **Refuse or replace an oversized event where frames are packed** (`send_transcript`, the attach replay, `client_agent_fetch`): incoherent with the core's log (decision 4).
- **Replace an oversized event with a notice and send the rest of the step** (the plan's C2 as first written): recovery logic at four call sites for a case the bounds make unreachable (decision 4).
- **Refuse in the fold (`FoldError::ItemsPastBound`)** instead of clamping (B-7's other option): breaks a session for content it can show bounded (decision 6).
- **Correct the rule text to "the producer bounds, the fold trusts"** (B-7's third option): leaves the bound without one owner, which is how A-2 happened.
- **Bound options in `v1/decode.rs`**, as login methods are: puts Brindle's policy in one ACP version's module (`ports R14`), and every future dialect would repeat it.
- **Cut an over-long option id, terminal id or session id:** a cut id names something else; ids are refused or left out.
- **Count an output at its current text length** (today's rule): `CALL_LIMIT` then fails as outputs fill, and a fold clamp could drop an item the reducer still appends to.
- **Raise `CALL_LIMIT`** so that more full outputs fit: changes the spec's 256 KiB and the entry's bound for a case no captured agent shows (decision 3).
- **Measure every limit in exact encoded bytes** (escapes included): the user-facing numbers would shift with how many quotes a text holds, and `Output`'s suffix rule would need prefix sums of escaped lengths. The factor 2 in the derivation does the same work.
- **Keep the per-entry measure as it is and rely on realistic id lengths for I2:** the bound would depend on how many entries a session has made, which nothing bounds.
- **Store the base's log as entries rather than replay events:** a format change that belongs with `agents-checkpoint-deltas`; the framing and grouping get I2 without it.

## Risks / Trade-offs

- [The 64-byte framing makes the log keep fewer tiny entries] → for a log of 100-byte notices, about 40 % fewer; for ordinary conversations (entries of kilobytes) a few percent. Accepted for a provable checkpoint bound.
- [At most three outputs a tool call] → no captured agent opens more than one terminal a call. If one does, its later outputs are counted in `more` and the row says so (accepted, decision 3).
- [A reply split at 1 MiB shows as two bubbles, and a fence open across the cut renders oddly] → choice C1; such replies are rare and nothing is lost.
- [The emit guard's cost] → measured by the perf cases (decision 4), with a fallback.
- [`agents-commands`, `agents-plans` and `agents-prompt-images` add events] → each must add its row to decision 1's table and its generator to the property test; the proposal's Impact names this for each.

## Open Questions

1. **Should the client join consecutive entries of one reply split at `ENTRY_LIMIT` into one bubble?** Recommendation: no, per C1. A split is rare, and joining would need the model to tell a split from two messages, which the log doesn't record.
2. **Should the emit guard measure every event, or skip `Appended` and `ToolOutput`, which are bounded by construction?** Recommendation: measure every event unless the perf cases show a cost past their noise (decision 4). A guard that skips kinds would miss a future mistake in them.

## As built

Both open questions were decided as recommended: entries split at `ENTRY_LIMIT` stay two bubbles (flag 7 says where the cut falls), and the guard measures every event (decision 4's numbers below).

Flags, numbered for review:

1. **The permission bounds are `MAX_PERMISSION_OPTIONS`, `MAX_PERMISSION_ID` and `MAX_PERMISSION_NAME`** (16, 1 KiB, 4 KiB) in `acp/limits.rs`, not decision 3's `MAX_OPTIONS`, `MAX_OPTION_ID` and `MAX_OPTION_NAME`: `agents-commands` already uses those names for session options (`brindle_transcript::MAX_OPTIONS`, `reduce.rs`'s `MAX_OPTION_ID`, `MAX_OPTION_NAME`, `MAX_OPTION_DESCRIPTION`).
2. **`cleaned_len` is in `brindle-transcript`, beside `clean`,** not in `reduce.rs`: `diff.rs` measures with it and the fold's clamp counts what it counts, so it sits where `clean` is. Its test is the transcript's `cleaned_text_escapes_to_at_most_twice_its_bytes_and_cleaned_len_is_cleans`.
3. **The fold raises `more` by the number it drops** (decision 6 says "at least"). On the reducer's events the clamp drops nothing, so `more` is the reducer's. A producer past the bounds that replaces a call's content again and again would count its dropped items each time; only a pre-release core can be one.
4. **An empty hunk counts `HUNK_FRAMING` in the fold's clamp and is kept** while it fits. The reducer never makes one (`Keep::done` removes them), and the fold keeps what it is sent within the bounds.
5. **`FileOp::Other { name }` counts in `wire_len` and is cut at `MAX_PATH`**, as a move's or copy's source is.
6. **An output's clamp is `Output::append("")`,** the suffix rule itself, so it is the identity on any output the fold made. `ToolOutput`'s exit is clamped in the fold too (its signal name at `SIGNAL_RESERVE`), as the reducer cuts it.
7. **A chunk that fits whole is appended whole.** An entry can therefore reach exactly `ENTRY_LIMIT` mid-line (32 chunks of 32 KiB fill 1 MiB), and the next text starts a new entry at its start. The cut at the last line break applies to the chunk that crosses the limit. `a_long_reply_continues_in_new_entries_cut_at_line_breaks` runs both: 32 KiB chunks, and 30,000-byte ones that cross.
8. **A run whose entry holds no text appends anyway,** so the fold refuses it and the handling fails, as before (`a_fold_error_from_the_reducers_own_events_fails_the_handling` builds that bug). `text_run` reads the run's length as 0 there.
9. **The guard's message takes "an" before a vowel:** `an added (tool) event for entry 2 is L bytes; …`, `an appended event …`, `a tool_items event …`. Decision 4 and task 4.5 wrote `a {kind}`, which reads "a added". The kind is the event's own name, underscores kept.
10. **A terminal item whose id is past `MAX_TERMINAL_ID` is counted in `more`,** as an item left out, and its `_meta` forms are dropped before anything else reads them.
11. **The session id check also refuses an empty id,** with the same message, as decision 3 lists; a control character is refused too, since the conversation's event carries the id uncleaned.
12. **The unknown-kind option names** a permission notice lists are at most 16, each cut at `MAX_PERMISSION_NAME`; the notice itself is `bound`, as every notice is.
13. **The core's failure log is a pure `failure_line(id, events)`** in `server/core/agents.rs`, called by `agent_apply` before it sends the events, and tested there (`a_failure_in_a_steps_events_reaches_the_log`), since `log!` writes the core's stderr.
14. **The guard's test seam** is `Step::event_limit`, set by a private `run(session, input, now, limit)`: `step` passes `MAX_EVENT`, the tests' `Agent` its `event_limit` field.
15. **The diff's fallback stops at the first line that doesn't fit** and counts the rest with one `count()`, neither cleaned nor copied; `Keep::cleaned` counts the lines cleaned, which the 300,000-line test compares with the lines kept.
16. **`checkpoint.rs` keeps no fixture files,** so task 2.3's old-form checkpoint is built in the test: the session's base with each grouped `tool_items` split one item an event, which decodes to the same session.
17. **The piece split's fault was shown at the base, not on this branch:** `33cd838` renamed `text_len` to `wire_len`, which the fold's clamp needed and which already fixed the split. `the_piece_split_counts_an_outputs_command_and_directory`, run at `486f6e6` in a scratch checkout, gave one piece (`[2]`); here it gives two.
18. **The real-role test `a_log_past_16_mib_drops_its_oldest_entries`** sent messages of 1 MiB and 3 bytes, which now split in two entries each. Its messages are now exactly 1 MiB, so the test checks what it did.
19. **The derivation's `Conversation` row** keeps decision 1's argument for its directory (the agent was started in it, so it is within the platform's path limit); nothing in `acp::limits` bounds it, and the row says so.
20. **`raw` cuts a string at `RAW_CAP` = `RAW_LIMIT + 4`, not only a formatted value,** and the old test's assertion that a 9 MiB raw string reads whole was removed: the bound replaces it. `Capped` keeps what it took before the refused write and cuts it back to a whole character (`a_capped_value_keeps_whole_characters`); serde_json's error is the cap reached and isn't reported. `raw_text` still cuts at the ellipsis when its prefix was cut, so a value cut by `raw` always ends with `…`.
21. **The adversarial property runs 300 steps, not 2,000** (110 s in a debug build; 300 take about 20 s), with three more departures from decision 8, each in the test's doc:
    - the replay, pages and fetch are walked every 40 steps, since each walk takes about 0.4 s;
    - the bound checks on tool entries cover the entries the step's events changed, since the others were checked when they last changed;
    - one move in eight is an extreme one, every size and count at its generator's largest.

    The extreme moves and mostly sendable option ids came after undoing the option names' cut wasn't caught; with them it is, at step 55. A prompt move answers the oldest waiting permission or lets a held prompt go before it prompts, so the turns keep coming: before that, the first unanswered permission blocked every later turn. The test ends by asserting that the run produced each event kind it exists for. A long `session/new` id stays with `reduce.rs`'s `a_session_id_past_its_bound_fails_the_start`, since it ends the session.
22. **The derivation test's I2 session** holds a log of empty user entries up to `LOG_LIMIT` with 20-digit ids, 64 open outputs with full held lines and 1 KiB terminal ids, methods near `MAX_LOGIN_METHODS_LEN`, an 8 MiB partial line, and the largest slot, conversation (a 96 KiB directory) and turn. Its checkpoint is compared with `LARGEST_CHECKPOINT`, not only with `MAX_CHECKPOINT_LEN`.
