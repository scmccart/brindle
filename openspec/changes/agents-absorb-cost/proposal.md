# Proposal

## Why

The 2026-11-26 agent code review (`docs/architecture/audits/2026-11-26-agents.md`; batch reports `docs/architecture/audits/2026-11-26-agents/d.md` and `e.md`; change 7 of `2026-11-26-agents-remediation.md`) found that what an agent zone does on the UI thread grows with the conversation, and that part of it runs under the lock every terminal tab's output waits on:
- **D-2:** `Agent::apply` (`src/agent/mod.rs`) holds `AgentShared`'s lock across the whole `ChatModel::absorb`, Markdown rendering included. The connection to an endpoint has one reader thread for all its sessions, and it takes that lock for every `TRANSCRIPT` frame, so while an agent streams, or a reconnect or a fetched page renders 256 KiB, the output of every other tab on the endpoint waits.
- **D-3 (plausible):** absorbs are paced per wake, not per frame, and each one parses the growing message whole again. A reply that trickles in can cost one parse per arriving frame, against `agents-markdown` decision 6's "one parse per frame". A tool output delta clones its whole tool entry (up to 256 KiB) and rebuilds every detail part. The perf case measures a burst the core merges, so it can't show this.
- **D-9:** a zone holds three to four copies of its conversation (the shared fold, the model's entries, the rendered blocks, the tool caches), against `agents-profiles`' recorded "about twice the core's 16 MiB".
- **`e.md`'s observation on per-absorb view work:** `AgentView::sync` scans every block for pending permissions on every absorb, and the model reads every held entry again (`refresh_facts`, `refresh_rows`).

The remediation plan settled the shape of the fix (its decision D1): the reader thread keeps folding into `AgentShared`'s transcript, and the zone shrinks what it does under that lock. The plan first filed E-5 (long tables and lists as one list item, a code block's text copied per render) here too; it shares no code with the absorb, so it is its own change, `agents-markdown-long-blocks`.

## What Changes

- **Absorb in two halves (D-2, D1).** Under `AgentShared`'s lock the zone takes the changes and copies only what changed: an appended text's tail, a tool output's new bytes, a tool's replaced items, a new or replaced entry, a reset's or a fetched page's entries. It lets go, and only then applies the copy, renders and splices, from its own copy. The second half takes no transcript, so it can't read the shared fold.
- **Render once per draw (D-3).** Absorb copies and marks entries; the zone renders and splices the marked entries once, at the end of the update that drained them. A test pins renders per drained batch. Before anything else, the change measures renders against frames with a trickle mode for the perf suite (the fake ACP agent spacing its chunks), and, only if those numbers show more renders than frames, adds a frame gate: an update less than a frame after the last draw leaves growing entries for one timer to draw.
- **Tool output by its delta (D-3).** A `ToolOutput` change copies the new bytes and appends them to the zone's copy with the transcript's own `Output::append`, and redraws only the output's last part (all of that output's parts when its front was dropped).
- **Fewer copies (D-9).** The tool text cache no longer keeps a second copy of each item's source; a tool detail's text part no longer copies its item; a reference link's target is shared by its uses. The recorded bound becomes what is held: the shared fold, the zone's copy, the rendered text, and the open rows' detail.
- **Facts kept on change (`e.md`'s observation).** The model keeps the pending permissions, the latest user message, the newest tool entry per call and the unfinished tool rows as values it updates when an entry is added, replaced, trimmed or reset, and redraws only the rows whose state can have changed. The view reads the pending set instead of scanning every block.
- **Measurements, before and after (`testing R10`, `R14`):** `agent-markdown-stream` gains its trickled run and render counts; new perf cases time a shell's key echo beside an agent zone that is quiet or streaming.

No wire, transcript protocol, checkpoint, config or state file change. `brindle-transcript` gains two pure, public items (`Transcript::entries_from`, a public `Output::append`); neither changes an encoding.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-zones`:
  - modified "Markdown while it streams": a reply that streams in one zone doesn't hold up the output of other tabs on the same endpoint.

## Impact

- **Code:**
  - `src/agent/mod.rs`: `Agent::apply`'s two halves; the draw at the end of each drained update; the render counts logged when the zone is left; the frame gate's timer if the numbers call for it.
  - `src/agent/model.rs` (`src/agent/model/blocks.rs` once `agents-module-structure` has split it): `ChatModel::copy_changes` (the first half), `absorb` taking the copy, `draw`, the undrawn marks, `Facts`, incremental `refresh_rows`, `texts` without sources, `pending_permissions` from the set, `render_stats`.
  - `src/agent/shared.rs`: unchanged types; `Changes` is read as `agents-client-state-fixes` leaves it.
  - `src/agent/tool.rs`: `DetailPart::Text` without its source; one output's parts on their own.
  - `src/agent/markdown.rs`: interned link targets.
  - `src/agent/view.rs`: the pending set.
  - `transcript/src/lib.rs`: `Transcript::entries_from`; `Output::append` public.
  - `src/terminal/pace.rs`: `draw_wait`, only if the frame gate is built.
  - `remote-agent/src/fake_acp/{script,main}.rs`: a `pace <ms>` step.
  - `tests/perf/agent.rs`, `tests/perf/main.rs`, `tests/perf/report.rs`: the trickled run, render counts, the latency cases.
- **Docs:** `docs/code-map.md` (the agent entries: the two halves, what a zone holds), `CLAUDE.md` (the perf cases, the fake agent's `pace`), the `agents-markdown` and `agents-profiles` designs' recorded bounds, which this change corrects in its design (the archive stays as written).
- **Recorded decisions changed:** `agents-markdown` decision 6's cost model and fallback; `agents-profiles`' memory line ("about twice"); `agents-tool-content` decision 10's "the zone draws that output's last part again" now holds without cloning the entry.
- **Overlap with the open changes:**
  - **`agents-commands` (PR #151, unmerged):** it changes `acp/reduce.rs`, `src/agent/model.rs` and the view before this change is implemented. Its commands and options are session state outside the entries; whatever of it `absorb` reads from the transcript, the first half copies when its change is flagged, as it copies the slot. The first task re-opens every cited site on `main`.
  - **`agents-plans`:** plan state the model reads from the transcript is copied under the lock as the slot is.
  - **`agents-prompt-context`, `agents-prompt-images`:** `Entry::User` gains attachments; the first half copies a new user entry whole, as any new entry, so nothing changes here.
  - **`agents-chat-selection` (proposal only), `agents-acp-v2`:** none; the first changes the view's text, the second the reducer, not the client's fold.
- **Overlap with the sibling remediation changes:**
  - **`agents-module-structure` (5) lands first:** it moves the code this change rewrites into `src/agent/model/blocks.rs`; the tasks name today's functions and say where each lands.
  - **`agents-zone-focus-and-scroll` (2):** it makes the working row's splice the model's (in `finish`) and moves scroll anchoring into `MessageScroller`. This change moves that splice to the end of `draw`, still the last of the splices, so its decision 5's ordering holds; its `permission_block` reads this change's pending set.
  - **`agents-client-state-fixes` (1):** it replaces `Changes::from_replay` with `status_from` and reconciles prompts on a reset. The first half carries `Changes` through unchanged, so its reconciliation runs in the second half on the copy.
  - **`agents-checkpoint-deltas` (6):** it changes `Output.text` to `OutputText` and `Output::append` to return a length. The output delta here reads the visible text and the dropped count, which that change keeps.
  - **`agents-event-bounds` (3):** its per-entry text limit (1 MiB) bounds the largest single entry a reset or page copies.
  - **`agents-process-lifecycle` (4):** none.
  - **`agents-markdown-long-blocks` (9), split out of this change (E-5), lands after it:** it splits long tables and lists into continuation parts and computes a code block's shown text once (`Code::new`), the `shown_code` copy decision 8's table lists. It builds on this change's `draw` (a grown message spliced from its first differing part) and takes `agent-latency-quiet` as its baseline; if it lands first, it adds that case itself, and task 2.5 here adds only `agent-latency-trickle`. No requirement is modified by both.
