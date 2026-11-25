# Proposal

**Depends on:** `agents-profiles` (the agent zone's copy of the log and its scrolling, and through it `agents-sessions`' transcript log, its 16 MiB bound, its checkpoint and its replay), implemented and archived.
**Next:** `agents-acp-v2`, speaking ACP v2. The five follow-ups of `agents-profiles` are independent of one another, so that order is a suggestion.

**Status: designed.** Design, specs and tasks are written; it lands after `agents-markdown`, `agents-session-recovery` and `agents-login`, in the same release as them.

## Why

In step 1 an agent session's whole conversation log is kept, sent and copied whole. The terminal server caps it at 16 MiB and drops the oldest entries past that, every attach replays the whole log before anything live, every checkpoint rewrites the whole log into the holder, and each agent zone keeps its own copy beside the fold. For the user that means: a long conversation loses its beginning; attaching or restoring a tab with a long conversation, especially over SSH, waits for the whole log; and each long conversation costs Brindle about twice the log in memory. A user mostly reads the end of a conversation, so the rest could stay on the server until it is scrolled to.

## What Changes

- Attaching an agent session sends a page of the newest entries, and the agent zone fetches older entries when the user scrolls up to them.
- The server's checkpoints append to the stored log instead of rewriting it, so a checkpoint's cost no longer grows with the conversation.
- Client protocol 4 and holder protocol H1 3, both unreleased, are amended in place (the agent changes ship in one release), and the checkpoint gets format 2. Released versions are unchanged.

Open questions, answered by the user (design.md decision 1):
1. **The page:** the newest whole entries up to about 256 KiB encoded, at least the newest entry, always reaching the oldest pending permission; an optional `ATTACH` field asks for less; older entries through `AGENT_FETCH {req, session, before, max_bytes}`, answered by entry events and `more`.
2. **The bound:** 16 MiB stays.
3. **The fold's guarantee:** a new event `Window { from, earlier }` (fetchable; `Trimmed` still means gone); the fold ignores events below its floor and a fetch returns current state; `fold(page(t)) == t.suffix(from)`, and fetched pages folded in front give `t`; pending permissions are always in the page.
4. **The checkpoint format:** in this change, a base and appended segments, `CHECKPOINT_APPEND` in H1 3 (the holder only concatenates), format 2, compaction at step-down, on a trim, or when segments outgrow the base.
5. **The zone:** an "Earlier messages" row that fetches when scrolled into view, with a spinner; the view kept; fetched pages kept; `earlier=N|loading|none` in the dump.
6. **Who else uses it:** no pocket work; the budget stays a field the client sets.
- **Versions:** protocol 4 and H1 3 are amended, with no fallback for pre-release builds of them.

## What the full changes already say

- `agents-sessions`, design decision 14 ("The log's bound"): the log is capped at 16 MiB encoded (`LOG_LIMIT`); past it the oldest entries are dropped whole and a `Trimmed` event records through which id and how many, so a replay starts with the marker. The cap leaves room in the 64 MiB checkpoint for an 8 MiB partial line, a 256 KiB queued prompt and the JSON-RPC state; every checkpoint copies the whole log, so at 16 MiB a checkpoint per 512 KiB of output is a 32× copy, the most that design accepts. Paging is this change: "an attach that sends a page of the newest entries and fetches older ones on demand, and checkpoints that append segments instead of rewriting the log, both additive (a later protocol version, a new checkpoint format version)". Its Risks name this change as the next step for whole-log checkpoints.
- `agents-sessions`, design decision 7: `replay()` returns `Trimmed` first, one `Added` per entry with its text split into `Appended`s of at most 64 KiB, the outcomes, the queue slot, then the turn and agent states; a property test pins `fold(replay(t)) == t` (question 3).
- `agents-sessions`, `agent-sessions` requirements "Attach replays the transcript" (a replay of the whole log, after which the client's copy equals the server's) and "The log is bounded" (16 MiB, oldest entries dropped whole, the count recorded). This change modifies both.
- `agents-sessions`, requirement "Transcript event forms are fixed for each protocol version": a new event kind needs a new client protocol version.
- `agents-profiles`, design decision 8: the zone holds a copy beside the fold, so at most about twice the core's 16 MiB; `Trimmed` drops blocks through the id. Its Risks: "Two copies of a long log per zone → bounded by the 16 MiB cap; `agents-log-paging` shrinks both."
- `agents-profiles`, design decision 1: the zone mirrors the server's cap and marker.

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `agent-sessions` (added by `agents-sessions`): "Attach replays the transcript" and "The log is bounded", and a new requirement for fetching older entries.
- `agent-zones` (added by `agents-profiles`): "Scrolling the conversation", for entries fetched on demand, and a new requirement for the "Earlier messages" row.
- `command-line`: "Debug hooks in an agent zone", for the dump's `earlier=`.
- `terminal-server`: "Protocol versions", for what client protocol 4 and holder protocol 3 now carry.

## Impact

- `transcript/` (`brindle-transcript`): the replay, the fold over a partial log, and their property tests.
- `remote-agent/src/acp/` and the core: the checkpoint format, its segments and their compaction, the page on attach and the fetch.
- `remote-protocol/`: new messages in client protocol 4 and H1 3, transcript tests and fuzzing for them.
- `remote-agent/src/server/holder.rs`: storing appended checkpoints.
- `src/agent/`: the zone's copy, its fetches and scrolling.
