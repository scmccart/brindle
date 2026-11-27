# Design

## Context

See proposal.md for why. The code as it is at `ba18a14` (after `agents-commands`, PR #151), which this design was checked against; the review's line numbers in `e.md` are from `5f335dc`, and the ones below are today's:

- **One item per top-level block.** `markdown::render` (`src/agent/markdown.rs:54`) walks pulldown-cmark's events and makes one `Part { range, block: Arc<RichBlock> }` per top-level block: `Walk::emit` (`:619-629`) pushes a part when no container frame is open. A list opens a `Frame::List` (`:366-373`) and each item a `Frame::Item` (`:374-381`); a table is a `Leaf::Table` whose rows `TagEnd::TableRow` collects (`:382-392`, `:460-465`). Tables are bounded in columns (`MAX_COLUMNS`, 32) only; lists, in nesting (`MAX_DEPTH`) only, not in items. `blocks_of` (`src/agent/model.rs:359-373`) makes one `BlockBody::AgentPart { block }` per part, and `AgentView`'s list draws one item per block. `Response`'s `Draw::list` and `Draw::table` (`ui/src/components/response.rs:557-641`) build the whole list or table as one element tree.
- **What gpui does with it.** gpui 0.2.2's `ListState::layout_items` (`elements/list.rs:604-660`) renders and lays out every item in view on every frame (`if visible_height < available_height || size.is_none()`, line 642), using an item's cached size only off screen. So a 1,000-row, 4-column table in view is about 5,000 divs and 4,000 `StyledText`s rebuilt and laid out per frame: per absorb, per motion tick (10 a second while the working row shows) and per hover redraw. `agents-markdown` decision 5's "the list lays out only the changed tail" holds for measurement of off-screen items only.
- **Table columns.** `Draw::table` lays its cells out with `grid_cols(n)`, which gpui turns into `repeat(n, minmax(0, 1fr))` (`gpui-0.2.2/src/taffy.rs:308-312`): equal columns, whatever the cells hold. It draws a line under the header and every body row but the last (`ruled = row < last_row`), and one rounded border round the grid.
- **Code's shown text.** `CodeBlock::render` (`ui/src/components/code_block.rs:94-97`) calls `shown_code` (`:41-46`), which copies the whole code into a new `SharedString` on every render when it ends in a line break, which a fence's code always does. `Code` (`response.rs:111-114`) has public `label` and `text` fields, built as literals in `markdown.rs` (`:386`, `:597`), `src/agent/view.rs` (`:1008`, `:1016`, a tool output's run and a raw value) and the gallery.
- **The dump.** `ChatModel::part_lines` (`model.rs:3177-3192`) prints one `--- block entry=N part=K` line per rendered part, `K` the part's index, its text `markdown::shown_text` (`markdown.rs:725`), then one `--- link` line per `markdown::block_links` (`:773`), which for a table reads the head's cells and then the rows'. `last_code_block` (`model.rs:2995`) returns `Option<&str>` through `markdown::last_code` (`:808`).
- **Where a growing message's splice is computed.** Today `ChatModel::update_entry` (`model.rs:1829-1894`) renders a grown agent message again, keeps the cached parts equal to the fresh ones from the start, and splices the blocks from the first that differs. `agents-absorb-cost` (remediation change 7) moves that into `ChatModel::draw`, run once per drained update, with the same comparison (its decision 4). `agents-module-structure` (change 5) moves `blocks_of`, `render_entry` and `last_code_block` into `src/agent/model/blocks.rs` and `part_lines` into `src/agent/model/dump.rs` (its module table).

## Goals / Non-Goals

**Goals:**
- Show, with numbers, whether a long table or list in view costs a shell beside it key-echo latency, before changing anything.
- If it does: a long top-level table or list costs, per frame, about as much as 50 rows or items of it, and still reads, dumps and copies as one block.
- A code block's shown text computed once, when it is built.

**Non-Goals:**
- Splitting code fences, thoughts, quotes, nested lists or tool text items (Open Questions 1 and 2).
- Caching rendered elements between frames. gpui 0.2.2 has no view cache, and the list's own item cache is per item; making each item small is what is in reach.
- Anything in the absorb path, its lock or its pacing (`agents-absorb-cost`).

## Layers

| Part | Layer | Crate |
| --- | --- | --- |
| `TABLE_ROWS`, `LIST_ITEMS`, continuation parts, `Part::continues`; code built through `Code::new` | pure module | `brindle` (`src/agent/markdown.rs`) |
| `BlockBody::AgentPart { continues }`; `last_code_block` | GPUI-free model | `brindle` (`src/agent/model/blocks.rs` after change 5) |
| `part_lines` grouping continuation parts | GPUI-free model | `brindle` (`src/agent/model/dump.rs` after change 5) |
| `DetailPart::Output` and `DetailPart::Raw` carrying a built `Code` | pure module | `brindle` (`src/agent/tool.rs`) |
| `Piece`, `Table::with_piece`/`piece`; `Code::new`/`label`/`shown`/`copy_text` | component data | `brindle-ui` (`components/response.rs`) |
| drawing a table piece; `CodeBlock::render` reading `Code::shown` | component | `brindle-ui` (`components/response.rs`, `code_block.rs`) |
| a continued part's spacing | view | `brindle` (`src/agent/view.rs`) |
| `agent-latency-table` | perf harness | `tests/perf/` |
| `agent-markdown-long-table` | e2e case | `tests/e2e/cases/agent.rs` |

Every decision is in the pure renderer or the GPUI-free model; the components take plain values (`Piece`, `Code`) and know nothing of Markdown or agents (`ports R2`). No crate reference changes (`ports R1`, `R3`).

## Decisions

### 1. Measure first; split only if the numbers show the cost (E-5 is plausible; `testing R10`, `R14`)

E-5's structure is confirmed by reading (one item per top-level block, gpui laying out every item in view each frame); its cost is estimated. So the change measures it before any product change:

- **`agent-latency-table`** (`tests/perf/agent.rs`, a `Measured::Latency`, no new report table): a tab of the perf suite's `Shell` with the agent profile `Fake` split beside it (`--action split_agent_right`), the agent's reply ending with a 1,000-row, 4-column table and a 1,000-item numbered list, in view, and the turn held running (the agent's `end_turn` waits on a `wait-file` gate the case opens only after the bench), so the working row's motion redraws the window ten times a second while the bench types; focus back in the shell (`--action focus_zone_left`), then `--latency-bench N`. That is E-5's per-frame cost as a user beside it feels it.
- **The baseline** is `agent-latency-quiet`, which `agents-absorb-cost` adds (its decision 9): the same layout beside an idle agent zone with a short reply, its turn ended. This change lands after that one (decision 8).
- **The criterion.** Both cases are run three times each on `main` before the change. The fault is shown when `agent-latency-table`'s p50 and p90 are both above `agent-latency-quiet`'s by more than the spread (largest minus smallest) of the three quiet runs' p50 and p90.
- **If it is shown,** decisions 2–6 are made and both cases are run again after them; the numbers before and after go in the as-built notes.
- **If it isn't,** the split (decisions 2–6) is not made. `agent-latency-table` stays in the suite as the pin, the as-built notes record the numbers and that the cost wasn't measurable at 1,000 rows on that machine, and the change is closed with decision 7 alone (which doesn't depend on the measurement). The spec deltas leave with the split: they are archived only if the split is made.

This replaces what the text had when E-5 was part of `agents-absorb-cost`, where the split was made even if the case showed nothing, on the grounds that it bounds a cost that grows with the table. The remediation rule for a plausible finding is that the fix waits on the fault being shown.

*Rejected:* a unit test as the showing. What E-5 claims is time per frame in gpui's layout, which only a run with a window shows; a test counting elements per item would show the structure, which is already confirmed by reading.

### 2. Long tables and lists in continuation parts (`types R1`, `R7`)

`a_long_table_renders_as_parts_of_fifty_rows` fails first on today's one part.

**In `markdown::render`.** `TABLE_ROWS = 50` and `LIST_ITEMS = 50`, like `tool::DETAIL_LINES`. When a top-level table closes with more than `TABLE_ROWS` body rows, `emit` pushes one part per run of 50 rows; when a top-level list closes with more than `LIST_ITEMS` items, one part per run of 50 items. `Part` gains `continues: bool` (it continues the part before it), the one fact the app keeps. The walk records each top-level row's and item's start offset (`Tag::TableRow`, and `Tag::Item` while the frame below it is the top-level list), so each part's range starts at its first row or item and runs to the next part's start, the last to the block's end. A table or list that grows while it streams therefore keeps its earlier parts equal (range and block) and splices only its last part, and the part before it once, when it stops being the last (the same rule as a message's bubble end, `agents-markdown` as built). The splice itself is computed where a grown message's splice is computed (decision 8), from the first part that differs; nothing there changes.

### 3. The pieces in `brindle-ui` (`ports R2`)

- `pub enum Piece { Whole, First, Middle, Last }` and `Table::with_piece(piece)` / `Table::piece()`. A `Table` keeps its head in every piece (its width and alignment come from it), and draws it only in `Whole` and `First`; its frame's top border and rounding only in `Whole` and `First`, its bottom only in `Whole` and `Last`; a line under every body row except the table's last (so under a `First` or `Middle` piece's last row too). Columns are `grid_cols(n)`, equal fractions of the same width, so every piece's columns line up without measuring cells.
- A list needs no piece: a numbered continuation is `ListKind::Numbered { start: start + 50k }` (saturating), and a bullet or task list is the same in every part.
- `Piece` is a closed enum: a table is exactly one of the four, and `Draw::table` matches it without a wildcard (`types R1`, `railway R3`). It rules out a piece that draws a header and joins the frame above it.

### 4. The view

`BlockBody::AgentPart { block, continues }`. A continuing part has no `pt_3` gap above it (a table's has none, a list's has the list's own `gap_1`), and `item_padding` treats it as inside its bubble (it already does for any part after the first that isn't the last).

Thoughts stay one item, as `agents-markdown` decision 5 requires (a collapsible can't span items), so a table in a thought is drawn whole; tool text items too. Nested lists and quotes are not split.

### 5. The dump and copying see one block

`part_lines` groups a part with the continuing parts after it: one `--- block` line, `part=K` counting top-level blocks, `kind` the first part's, its text the parts' `shown_text`s joined by line breaks (`shown_text` of a `Middle` or `Last` table leaves out the head), and the `--- link` lines of all of them under that `K`. That is the `command-line` spec's "one line per top-level block". The message copy button and Copy Last Agent Message copy the source, unchanged; `last_code_block` finds code in the last part, as before.

**A continued table's links.** A `Middle` or `Last` piece keeps the head (decision 3), and `block_links` reads a table's head cells, so the grouped `--- link` lines would repeat the head's links once per piece. `block_links` of a `Middle` or `Last` table piece reads its body rows only, as its `shown_text` leaves out the head. (This was missing from the text as first written in `agents-absorb-cost`, which said `block_links` was unchanged per part.)

### 6. Rejected ways to bound a long block

- **A repeated header on every piece:** the table would read as several, and the spec asks for one.
- **Measuring column widths across parts:** equal columns are what `grid_cols` already does, and measuring needs every cell laid out, which is the cost being removed.
- **Splitting inside `Response` at draw time:** the list item, not the element, is what the list virtualizes, so the split has to make items.

### 7. Code's shown text once (`types R7`)

`Code`'s fields become private behind `Code::new(label, text: String)`, which pops one final line break into `ends_in_break` and keeps the rest as the shown `SharedString`; `label()`, `shown()` and `copy_text()` (the shown text and the break) read it. `CodeBlock::render` clones the shown `SharedString` (a pointer) and builds the copy text only on a press. `markdown.rs` builds code through it; `tool.rs`'s output runs and raw values carry a built `Code` in their `DetailPart`s (`Output { item, head, code }`, `Raw { which, code }`), so the view builds none. `last_code_block` returns `Option<String>` (`copy_text`), and `shown_text` of a code block is its `copy_text`, so the dump and the clipboard read as today. `shown_code` goes.

The private fields make a `Code` whose shown text and copy text disagree unbuildable: both come from the one text `new` was given. This decision is made whatever decision 1's numbers show: the copy on every render is certain by reading, and removing it changes nothing a user sees. It is also one of the copies `agents-absorb-cost`'s decision 8 lists (its D-9 table), where it is marked as removed by this change.

### 8. Where this lands, and landing before `agents-absorb-cost`

The plan orders this change ninth, after `agents-absorb-cost` (7): it builds on that change's `ChatModel::draw`, which computes a changed message's splice from the first part that differs, and on `agents-module-structure`'s modules (`src/agent/model/blocks.rs`, `dump.rs`), and its baseline is change 7's `agent-latency-quiet`.

It shares no code with change 7's two absorb halves, its lock or its frame gate, so it can land first. If it does:
- A growing table's last part is spliced by today's `ChatModel::update_entry` (`model.rs:1829-1894` at `ba18a14`, or wherever change 5 put it), whose comparison from the first differing part decision 2 relies on; change 7 then moves that comparison into `draw` unchanged.
- This change adds `agent-latency-quiet` itself, as change 7's decision 9 describes it, and change 7's task that adds the latency cases adds only `agent-latency-trickle`.
- Change 7's D-9 table's `shown_code` row then reads "already removed".

### 9. The wire, the state and the server (`process R1`–`R6`, `R12`)

No message, protocol version, checkpoint, config key or state file changes. Parts are computed on the client from an entry's text and live only in a zone's memory; the next attach renders them again. The server is untouched: a core restart, a Brindle crash and an upgrade do what they do today. A renderer panic still shows the message's text as one plain paragraph (`render_entry`, under `catch_unwind`), so a fault in the new splitting shows as unstyled text, not a broken zone (`railway R5`).

### 10. Recorded decisions this changes

- `agents-markdown` decision 5: "a block longer than 4 KiB is one item" no longer holds for top-level tables and lists of more than 50 rows or items (decision 2), and its "the list lays out only the changed tail" holds for off-screen items only (Context).

No row of the remediation plan's decisions (D1, D2) or choices (C1–C5) touches this change.

### 11. Tests (`testing R1`, `R13`, `R15`)

- **Perf:** `agent-latency-table` against `agent-latency-quiet` (decision 1), before and after.
- **Pure, `markdown.rs`:** `a_long_table_renders_as_parts_of_fifty_rows`, `a_long_numbered_list_continues_its_numbers_across_parts`, `a_bullet_list_of_120_items_is_three_parts_of_the_same_kind`, `continuation_parts_cover_the_block_in_order`, `a_growing_table_keeps_its_earlier_parts`, `a_nested_list_and_a_table_in_a_quote_stay_whole`, `a_split_tables_shown_text_is_the_whole_tables`, `a_continued_table_piece_lists_its_rows_links_only`.
- **Pure, `tool.rs`:** `a_tool_outputs_and_raw_values_carry_a_built_code`.
- **Model, `blocks.rs` and `dump.rs` tests:** `a_split_table_is_one_block_line_in_the_dump`, `a_split_list_is_one_block_line_numbered_on`; the existing `the_last_code_block` with `last_code_block` returning `Option<String>`; `streaming_at_every_split_point_ends_with_the_whole_texts_parts` and `streaming_a_long_message_re_renders_only_what_changed` keep passing.
- **`brindle-ui`:** `code_block.rs` (`code_shows_its_text_without_the_last_line_break_and_copies_it_whole`); `response.rs` (`a_table_is_whole_until_given_a_piece`, `a_continued_table_draws_no_header_and_joins_its_frame`, `pieces_of_one_table_line_up_their_columns`, with `debug_bounds` of `response-cell` selectors); the gallery shows a table split into pieces (`-- --check`).
- **View:** `a_continued_part_sits_flush_under_the_one_before`.
- **e2e:** `agent-markdown-long-table` (`tests/e2e/cases/agent.rs`): a reply holding a 120-row table and a 120-item numbered list; the dump has one `--- block kind=table` line with every row and one `kind=list` line numbered 1 to 120. The existing `agent-markdown-*`, `agent-tool-*`, `agent-thoughts` and `agent-scroll` cases run unchanged.
- **Manual (`CLAUDE.md`):** a 1,000-row table and a 1,000-item list in an agent zone read as one table and one list (one header, columns aligned, no seam between rows 50 and 51), scrolled with the wheel, in brindle-light, brindle-dark and solarized-dark.

## Findings traced

- **E-5:** the one-item-per-block structure is confirmed (`markdown.rs:619-629`, `model.rs:359-373`, gpui `layout_items`); the cost is estimated. `agent-latency-table` measures it before the split (decision 1). The code block's copy per render is confirmed (`code_block.rs:41-46`, `:97`).

## Risks / Trade-offs

- **[Continuation parts change what a `Response` sees]** → a part is still a whole `RichBlock`; only a top-level table's piece and a list's start differ, and the unit and e2e tests pin the dump and the drawing.
- **[A piece's columns could drift from its neighbours']** → every piece is laid out at the same width with `grid_cols(n)`, equal fractions; `pieces_of_one_table_line_up_their_columns` pins it, and the manual check looks at the seam.
- **[A 50-row run with very tall cells is still a large item]** → cells wrap within equal columns, so a run's height grows with its text; the bound is in rows, as `DETAIL_LINES` is in lines. A pathological table of 50 huge cells is no worse than today's whole table.
- **[The numbers may not show the cost]** → then the split isn't made (decision 1), and the perf case stays to show it if a later change makes items costlier.

## Migration Plan

Nothing to migrate: no stored state changes. The change lands after `agents-absorb-cost` (7) and `agents-module-structure` (5) on `main` (decision 8 says what changes if it lands before 7). The perf case lands first in the same PR, so the "before" numbers are taken on the code as it was. Rolling back is reverting the PR.

## Open Questions

1. **Should code fences split too?** A 1,500-line fence is one item, laid out whole each frame it shows (`agent-markdown-stream`'s fence is one). `CodeBlock::joined` already draws runs as one frame for tool output. *Recommendation:* if `agent-latency-table`'s numbers make the case for tables, add a fence of 1,500 lines to that case; if it costs as much, split fences into runs of `DETAIL_LINES` lines in a follow-up, the first run carrying the label and a copy button that copies the whole block (each part holding the block's one `SharedString` and its line range).
2. **Tables inside thoughts and tool text items** stay one item. *Recommendation:* leave them until a thought can be split (a collapsible spanning list items), which `agents-chat-selection`'s cross-block work may bring.
