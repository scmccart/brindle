# Proposal

## Why

The 2026-11-26 agent code review (`docs/architecture/audits/2026-11-26-agents.md`; change 3 of `2026-11-26-agents-remediation.md`) found that the bounds on what an agent sends hold field by field but not in sum, so the two invariants the code's comments rely on are false:

- **A-1** (`2026-11-26-agents/a.md`): one text entry has no bound. A reply or thought streamed past about 13 MiB makes the trim drop every older entry, the log then stays over the 16 MiB the spec requires, and near 64 MiB the whole checkpoint is `TooLarge`, which `server/core/agents.rs` and two archived designs call unreachable.
- **A-2 / B-1** (`a.md`, `b.md`): the tool content limits count text bytes, not what goes on the wire. A diff of 300,000 empty lines, 32 locations of 64 KiB, a permission with unbounded options or option ids, or 32 terminal items with long commands each make one transcript event past the 1 MiB frame. `pack_events` then refuses the whole batch: the live event never reaches the client, an attach whose page holds the entry replays nothing, and a pending permission of that size can never be answered. Terminal ids are kept at any length.
- **B-2** (`b.md`): a tool call's raw input is pretty-printed whole and cleaned whole before its 16 KiB cut. One deeply nested 8 MiB line allocates about 2 GiB on the core's one loop thread, and an out-of-memory kill takes every session with it.
- **B-7** (`b.md`): the transcript crate defines the tool content bounds and its rule text says the fold enforces them, but only the reducer does. A client folding a buggy or newer core's events keeps whatever it is sent.
- **D-8** (`d.md`): a hunk's line numbers are counted with `+= 1` on a `u32` from the wire, so a hunk starting at `u32::MAX` panics the UI thread in a debug build.
- `a.md`'s structural observation 4 and `b.md`'s observation 4: the bounds that must line up live in three crates, tied together only by comments.

While tracing A-2, two more unbounded fields turned up and join it: the ACP session id the agent gives in `session/new` (an id, echoed in every request, carried in the `Conversation` event) and a stop reason the agent names itself (carried in `Turn` and a held slot).

## What Changes

- **One place for the bounds, two invariants, tested.** A new pure module `remote-agent/src/acp/limits.rs` states the invariants: every transcript event the reducer emits, and every event of a replay, page or fetch, encodes within `brindle_transcript::MAX_EVENT`, which fits a `TRANSCRIPT` or `AGENT_PAGE` frame; and a full log, a full partial line and the rest of a session's state fit `MAX_CHECKPOINT_LEN`. Constant assertions derive each from the constants, and a reducer property test feeds adversarial updates and permissions and checks every event, every replay and page, the log bound and the checkpoint after every step.
- **A text entry is at most 1 MiB (A-1, choice C1).** A run that would pass it ends, and its text continues in a new entry of the same kind, so nothing is cut and the trim always works on whole entries. A zone shows such a reply as two (or more) bubbles. `TooLarge` becomes unreachable, and the comments say why.
- **Limits count what goes on the wire (A-2).** A content item's size is its text plus its framing (`ToolItem::wire_len`: 4 bytes a diff line, 64 a hunk, an output's command and directory); the diff stops allocating lines once its item is full; paths (locations, diffs, a move's source, an output's directory) are cut at 4 KiB; a permission keeps at most 16 options, leaves out one whose id is over 1 KiB, empty or holds a control character (an id is echoed, so it is never cut), cuts option names at 4 KiB, and says what it left out; terminal ids over 1 KiB are ignored; a session id over 1 KiB fails the start; a stop reason's name is cut as titles are.
- **One oversized event fails its session and says why (choice C2, simplified).** The reducer measures each event before applying it. One past `MAX_EVENT` is not applied, and the step faults through the existing path: the zone shows `ACP handling failed:` followed by the event's kind, its entry and its size against the limit, and the core logs the same. The bounds make this unreachable; the guard makes a future mistake loud and self-describing, where today the batch is dropped without a word.
- **Raw input is formatted within its bound (B-2):** `serde_json::to_writer_pretty` writes into a writer that stops past `RAW_LIMIT`, a string value is cut there too, and the reducer cleans only what it keeps.
- **The fold enforces the tool content bounds it defines (B-7):** `MAX_ITEMS` and `CALL_LIMIT` (dropping into `more`), `ITEM_LIMIT` per item, `RAW_LIMIT`, `MAX_LOCATIONS` and the new `MAX_PATH`, so the core and every client share one definition. Within the bounds the fold changes nothing, so Brindle's own core sees no difference.
- **Hunk line numbers can't overflow (D-8):** the client numbers lines with checked arithmetic, and the transcript decoder refuses a hunk that starts at 0 or whose lines would be numbered past `u32::MAX`.
- **Wire (choice C4):** client protocol 4 is unreleased, so its bounds are amended in place. No event, field or message changes; no fixture changes.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`:
  - "The log is bounded": a text entry past 1 MiB continues in a new entry, and the log is within its bound after every update, however long one reply is.
  - "Tool call content is bounded": sizes count each item's text with its framing; paths are cut at 4 KiB; a diff of many empty lines is bounded.
  - "Clients hold the same tool content as the server": a client applies the same bounds to what it is sent.
  - "Permission requests wait for the client's choice": options are bounded in count, id and name, and what was left out is said.
  - "Tool call locations, raw input and raw result are kept": raw input is formatted only up to its bound.
  - New requirement "Every transcript event fits one message".

## Impact

- **Code:**
  - `brindle-transcript` (`transcript/src/lib.rs`): `MAX_EVENT`, `MAX_PATH`, `HUNK_FRAMING`, `ToolItem::wire_len` (replacing `text_len`), `Event::encoded_len`, `escaped_len` made public, the fold's clamps on `Added`, `ToolChanged` and `ToolItems`, the `Hunk` decoder's check; `transcript/src/fuzz.rs`.
  - `brindle-remote-protocol` (`client.rs`): the two envelope lengths become public constants (`TRANSCRIPT_ENVELOPE`, `AGENT_PAGE_ENVELOPE`). No encoding changes.
  - `brindle-remote-agent`: the new `acp/limits.rs`; `acp/reduce.rs` (`emit`, `add`, `chunk`, `add_text`, `tool_detail`, `tool_content`, `permission`, `created`, `prompted`, `raw_text`); `acp/diff.rs` (`Keep`); `acp/v1/decode.rs` (`raw`); `server/core/agents.rs` (comments, and a failure logged).
  - `brindle` (`src/agent/tool.rs`, `diff_rows`).
- **Tests:** unit tests beside each change; the property test in `acp/limits.rs`; `transcript/src/fuzz.rs`; the perf cases `agent-markdown-stream` and `agent-tool-output` before and after (the per-event measure).
- **Docs:** `docs/code-map.md` (the transcript and reducer entries), `.claude/rules/transcript.md` (what the fold bounds and what the producer bounds), `docs/architecture/functional-core-imperative-shell.md` and `tests/architecture.rs` (`PURE_MODULES` gains `acp/limits.rs`).
- **Overlap with the seven sibling changes:**
  - `agents-client-state-fixes` (1) and `agents-zone-focus-and-scroll` (2) touch `src/agent/model.rs` and the view; this change's only client edit is `tool.rs`'s `diff_rows`. Two bubbles for a reply past 1 MiB need nothing from either.
  - `agents-process-lifecycle` (4) adds an "agent gone" step that emits `Answered` for waiting permissions, and a fault switch point after the first `emit`; it uses `emit` as this change leaves it (its signature unchanged; an oversized event faults the step). It lands after this one. Both changes modify `agent-sessions` "Permission requests wait for the client's choice", each adding its own text: whichever is archived second carries the other's text in its delta.
  - `agents-module-structure` (5) splits `reduce.rs` (moving `clean`, `bound` and the new `cleaned_len` to `reduce/text.rs`) and `transcript/src/lib.rs`; it moves this change's code, and `acp/limits.rs` stays as it is.
  - `agents-checkpoint-deltas` (6) changes the segment's fields and `Output`'s storage (A-4). This change's checkpoint invariant is about a whole checkpoint, which keeps its form; the fold's clamp of an `Output` item uses the same suffix rule `Output::append` does, which change 6 must keep.
  - `agents-absorb-cost` (7): none.
  - `agents-client-request-marks` (8) takes C-6, first planned here: the core stops taking a client's requests while its queue is over its mark. It shares no code with this change; the 1 MiB text entry bounds the size of the one answer that crosses its mark.
- **Overlap with open changes:**
  - `agents-commands` (PR #151, unmerged) changes `acp/reduce.rs`, `src/agent/model.rs` and the view, and adds bounded command and option lists to the agent's state. Its events join this change's derivation table and property test when it is merged; this change's first task re-opens every cited site on `main`.
  - `agents-plans` adds plan entries (64 KiB of Markdown, 256 items): the plan's events must fit `MAX_EVENT`, and its row joins the derivation.
  - `agents-prompt-images` raises the 256 KiB prompt limit to 5 MiB for attachments. The slot's `Queue` event carries the prompt's text, so that change must keep attachments out of the slot's event (or by reference) to stay within `MAX_EVENT`; the derivation's `Queue` row says so.
  - `agents-client-terminals` has Brindle run an agent's terminals. A tool call's log entry keeps at most three outputs here, which is a limit on what the log and the zone show; that change must keep what the agent reads from a terminal independent of what the entry kept.
  - `agents-prompt-context` keeps attachment text outside the log (16 MiB a session); it adds state the checkpoint invariant must count if it is checkpointed.
