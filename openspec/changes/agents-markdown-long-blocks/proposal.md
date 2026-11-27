# Proposal

## Why

The 2026-11-26 agent code review (`docs/architecture/audits/2026-11-26-agents.md`; batch report `docs/architecture/audits/2026-11-26-agents/e.md`, finding **E-5**, *plausible*) found that virtualization in an agent zone stops at the top-level Markdown block. `markdown::render` makes one part per top-level block, the chat model one list item per part, and `brindle-ui`'s `Response` builds a table or list as one element tree. gpui 0.2.2's list renders and lays out every item in view on every frame, so a 1,000-row table or a 1,000-item list in view is about 5,000 divs and 4,000 styled texts rebuilt and laid out per frame: per absorb, per motion tick (10 a second while a turn runs) and per hover. A code block also copies its whole text on every render (`shown_code`). The cost is estimated, not measured, so this change measures it first.

The remediation plan (`docs/architecture/audits/2026-11-26-agents-remediation.md`) first filed E-5 under change 7, `agents-absorb-cost`. Its review of the written changes found that E-5 shares no code with that change's two absorb halves or its lock, and split it out: this is remediation change 9.

## What Changes

- **Show it first.** A perf case, `agent-latency-table`, times a shell's key echo beside an agent zone showing a 1,000-row table and a 1,000-item list while the turn runs, against `agent-latency-quiet` (the same beside an idle zone, added by `agents-absorb-cost`). If the table case shows no difference beyond noise on `main`, the split is not made: the case stays as the pin, and the change is closed with the numbers recorded.
- **Long tables and lists in parts (E-5).** `markdown::render` splits a top-level table of more than 50 body rows, and a top-level list of more than 50 items, into continuation parts, each its own list item. The parts draw as one table or list (one header, equal columns, one frame; numbering that continues), the dump prints one `--- block` line for them, and copying is unchanged.
- **Code's shown text once.** A code block's shown text (its text without the final line break) is computed once when the block is built (`Code::new`), not on every render; `shown_code` goes. This is made whatever the measurement shows: the copy per render is certain by reading, and its removal changes nothing a user sees.

No wire, transcript protocol, checkpoint, config or state file change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-zones`:
  - modified "Tables in the conversation": a table of any length reads as one grid, its header once at its top.
  - modified "Markdown in agent messages and thoughts": a list of any length reads as one list, and a scenario for a long numbered list, read as one list numbered on.
- `command-line`:
  - modified "Rendered blocks in the dump": a long table or list is one `--- block` line.

## Impact

- **Code** (paths after `agents-module-structure`, change 5; today's file in brackets):
  - `src/agent/markdown.rs`: `TABLE_ROWS`, `LIST_ITEMS`, continuation parts, `Part::continues`; code built through `Code::new`; `shown_text` and `last_code` read `Code`'s accessors.
  - `src/agent/model/blocks.rs` (`src/agent/model.rs`): `BlockBody::AgentPart { block, continues }`; `last_code_block` returning `Option<String>`.
  - `src/agent/model/dump.rs` (`src/agent/model.rs`): `part_lines` grouping continuation parts.
  - `src/agent/tool.rs`: `DetailPart::Output` and `DetailPart::Raw` carry a built `Code`.
  - `src/agent/view.rs`: a continued part's spacing; the tool detail's code blocks taken as built; `copy_last_code_block`'s caller.
  - `ui/src/components/response.rs`: `Piece`, `Table::with_piece`, `Table::piece`, drawing a piece; `Code::new`, `label`, `shown`, `copy_text`, private fields.
  - `ui/src/components/code_block.rs`, `ui/src/components/mod.rs`: `shown_code` removed; `CodeBlock::render` reads `Code::shown`.
  - `ui/examples/gallery.rs`: its `Code` literals.
  - `tests/perf/agent.rs`, `tests/perf/main.rs`: `agent-latency-table`.
  - `tests/e2e/cases/agent.rs`, `tests/e2e/cases/mod.rs`: `agent-markdown-long-table`.
- **Docs:** `docs/code-map.md` (the Markdown entry), `CLAUDE.md` (the perf case, the manual check for a long table and list).
- **Recorded decisions changed:** `agents-markdown` decision 5 ("a block longer than 4 KiB is one item", and "the list lays out only the changed tail", which holds for off-screen items only) for top-level tables and lists.
- **Order:** after `agents-absorb-cost` (7), as remediation change 9. Design decision 8 says what changes if it lands before change 7.
- **Overlap with the sibling remediation changes:**
  - **`agents-absorb-cost` (7)** lands first. Its `ChatModel::draw` computes each changed message's splice from the first part that differs, which is what keeps a growing table's earlier parts in place here; its `agent-latency-quiet` case is this change's baseline. Its D-9 copies table lists `CodeBlock::render`'s `shown_code` copy as removed by this change. It modifies `agent-zones` "Markdown while it streams", which this change doesn't touch.
  - **`agents-module-structure` (5)** lands first: it moves `blocks_of` and `last_code_block` into `src/agent/model/blocks.rs` and `part_lines` into `src/agent/model/dump.rs`. The tasks name today's functions and say where each lands.
  - **`agents-zone-focus-and-scroll` (2):** it moves scroll anchoring into `MessageScroller`. Continuation parts are ordinary list items to it: a growing table's last part is a splice it anchors like any other.
  - **`agents-event-bounds` (3):** its 1 MiB text entry limit bounds the largest message a table or list can come from; a reply split there into two entries is two messages, each split into parts on its own.
  - **`agents-client-state-fixes` (1), `agents-process-lifecycle` (4), `agents-checkpoint-deltas` (6):** none.
  - No other remediation change modifies a requirement this change modifies.
- **Overlap with the open changes:**
  - **`agents-plans`:** its plan block renders a Markdown plan with `markdown::render`; a long list in a plan gets continuation parts only if the plan block draws parts as the conversation does, which its design decides.
  - **`agents-chat-selection` (proposal only):** a selection inside a table or list that spans continuation parts crosses list items, which its step 2 (across a message's blocks) already has to handle.
  - **`agents-prompt-context`, `agents-prompt-images`, `agents-acp-v2`:** none; user messages keep their own parts, and nothing here reads the reducer.
