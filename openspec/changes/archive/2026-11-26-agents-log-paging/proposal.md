# Proposal

**Depends on:** `agents-profiles` (the agent zone's copy of the log and its scrolling, and through it `agents-sessions`' transcript log, its 16 MiB bound, its checkpoint and its replay), `agents-markdown` (the zone's Markdown cache), `agents-session-recovery` (kept logs, restarts in a new holder session, `AGENT_RESTART` = 21) and `agents-login` (login states in every replay, `AGENT_LOGIN` = 22), all implemented and archived; and the open stack on `feat/agents-zone-attention` (`agents-markdown-layout-fixes`, `agents-zone-theme-scope`, `agents-session-title` and `agents-zone-attention`), which lands and archives first: this change reads `Conversation.title`, draws its spinner with that stack's `Loader` on its motion clock, and its command-line delta carries `agents-zone-attention`'s wording.
**Next:** `agents-acp-v2`, speaking ACP v2. The follow-ups of `agents-profiles` are independent of one another, so that order is a suggestion.

**Status: designed, refreshed.** Design, specs and tasks were written on 2026-11-25 and brought up to date on 2026-11-26 with the code built since (main at cdb972c and the stack at cc4f41a); design.md decision 16 lists the choices the refresh added. It lands after the stack, in the same release as the other agent changes.

## Why

An agent session's whole conversation log is kept, sent and copied whole. The terminal server caps it at 16 MiB and drops the oldest entries past that, every attach replays the whole log before anything live, every checkpoint rewrites the whole log into the holder, and each agent zone keeps its own copy beside the fold and renders every agent message's Markdown on attach. For the user that means: a long conversation loses its beginning; attaching or restoring a tab with a long conversation, especially over SSH, waits for the whole log; and each long conversation costs Brindle about twice the log in memory. A user mostly reads the end of a conversation, so the rest could stay on the server until it is scrolled to.

## What Changes

- Attaching an agent session sends a page of the newest entries, with the queue slot, the conversation (and its title), the turn and the agent's state as before, and the agent zone fetches older entries when the user scrolls up to them.
- The server's checkpoints append to the stored log instead of rewriting it, so a checkpoint's cost no longer grows with the conversation.
- Client protocol 4 and holder protocol H1 3, both unreleased, are amended in place (the agent changes ship in one release): `AGENT_FETCH` is client message 23, `AGENT_PAGE` core message 43 and `CHECKPOINT_APPEND` holder message 16. The checkpoint gets format 2. Released versions are unchanged.

Open questions, answered by the user (design.md decision 1):
1. **The page:** the newest whole entries up to about 256 KiB encoded, at least the newest entry, always reaching the oldest pending permission; an optional `ATTACH` field asks for less; older entries through `AGENT_FETCH {req, session, before, max_bytes}`, answered by entry events and `more`.
2. **The bound:** 16 MiB stays.
3. **The fold's guarantee:** a new event `Window { from, earlier }` (fetchable; `Trimmed` still means gone); the fold ignores events below its floor and a fetch returns current state; `fold(page(t)) == t.suffix(from)`, and fetched pages folded in front give `t`; pending permissions are always in the page.
4. **The checkpoint format:** in this change, a base and appended segments, `CHECKPOINT_APPEND` in H1 3 (the holder only concatenates), format 2, compaction at step-down, on a trim, or when segments outgrow the base.
5. **The zone:** an "Earlier messages" row that fetches when scrolled into view, with a spinner; the view kept; fetched pages kept; `earlier=N|loading|none` in the dump.
6. **Who else uses it:** no pocket work; the budget stays a field the client sets.
- **Versions:** protocol 4 and H1 3 are amended, with no fallback for pre-release builds of them.

The refresh keeps every answer. What it adds (design.md decision 16, items 13–21): the fixed message numbers; an empty page accepted after a trim took the window (the first design would have broken the session on that race); the conversation and its title in every page; the working row and the copy actions reading only the entries the zone holds; the motion clock running while a fetch waits, which widens `agents-zone-attention`'s "Motion only while an agent works"; fetched pages raising no notification; a restart's first checkpoint written whole; merged appends in a segment; and the fetch's protocol 4 refusal.

## What the full changes already say

- `agents-sessions`, design decision 14 ("The log's bound"): the log is capped at 16 MiB encoded (`LOG_LIMIT`); past it the oldest entries are dropped whole and a `Trimmed` event records through which id and how many, so a replay starts with the marker. The cap leaves room in the 64 MiB checkpoint for an 8 MiB partial line, a 256 KiB queued prompt and the JSON-RPC state; every checkpoint copies the whole log, so at 16 MiB a checkpoint per 512 KiB of output is a 32× copy, the most that design accepts. Paging is this change: "an attach that sends a page of the newest entries and fetches older ones on demand, and checkpoints that append segments instead of rewriting the log, both additive (a later protocol version, a new checkpoint format version)". Its Risks name this change as the next step for whole-log checkpoints.
- `agents-sessions`, design decision 7: `replay()` returns `Trimmed` first, one `Added` per entry with its text split into `Appended`s of at most 64 KiB, the outcomes, the queue slot, then the turn and agent states; a property test pins `fold(replay(t)) == t` (question 3). As built, `agents-session-recovery` added the `Conversation` between the slot and the turn, and `agents-session-title` its title.
- `agents-sessions`, `agent-sessions` requirements "Attach replays the transcript" (a replay of the whole log, after which the client's copy equals the server's) and "The log is bounded" (16 MiB, oldest entries dropped whole, the count recorded). This change modifies both.
- `agents-sessions`, requirement "Transcript event forms are fixed for each protocol version": a new event kind needs a new client protocol version once protocol 4 has shipped; until then it is amended in place.
- `agents-profiles`, design decision 8: the zone holds a copy beside the fold, so at most about twice the core's 16 MiB; `Trimmed` drops blocks through the id. Its Risks: "Two copies of a long log per zone → bounded by the 16 MiB cap; `agents-log-paging` shrinks both."
- `agents-profiles`, design decision 1: the zone mirrors the server's cap and marker.
- `agents-login`, design: `AGENT_LOGIN`'s number is "the next free one after recovery's `AGENT_RESTART` (and `agents-log-paging`'s `AGENT_FETCH`, if that lands first)". It took 22, so `AGENT_FETCH` takes 23.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `agent-sessions`: "Attach replays the transcript" and "The log is bounded", and new requirements for fetching older entries, keeping unfetched entries current, and appended checkpoints.
- `agent-zones`: "Scrolling the conversation", for entries fetched on demand, "Replays raise nothing", for the newest page and fetched pages, and a new requirement for the "Earlier messages" row. "Motion only while an agent works" (from `agents-zone-attention`, now archived), for the row's spinner (design.md decision 16, item 17; task 8.3).
- `command-line`: "Debug hooks in an agent zone", for the dump's `earlier=`, carrying `agents-zone-attention`'s `--- working` line.
- `session-restore`: "Restored agent zones get their sessions back", for the newest page in place of the whole conversation.
- `terminal-server`: "Protocol versions", for what client protocol 4 and holder protocol 3 now carry.

## Impact

- `transcript/` (`brindle-transcript`): the page, the fetch, the fold over a partial log, and their property tests.
- `remote-agent/src/acp/`: the checkpoint's format 2, its segments, and the trim's low mark.
- `remote-agent/src/server/core/`: the page on attach, the fetch, what the holder stores and when to append or compact.
- `remote-protocol/`: new messages in client protocol 4 and H1 3, transcript tests and fuzzing for them.
- `remote-agent/src/server/holder.rs`: storing appended checkpoints.
- `src/remote/`: sending the fetch and routing its pages.
- `src/agent/`: the zone's copy, its fetches, its Earlier row, the dump.
- `ui/src/components/message_scroller.rs`: the top row and keeping the view on an insert above.
- Tests: unit, property and real-role tests, transcripts, three e2e cases and one perf case; docs: CLAUDE.md, `docs/code-map.md`, `docs/architecture/process-architecture.md`, README.md.
