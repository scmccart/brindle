# Proposal

**Depends on:** `agents-profiles` (the agent zone's copy of the log and its scrolling, and through it `agents-sessions`' transcript log, its 16 MiB bound, its checkpoint and its replay), implemented and archived.
**Next:** `agents-acp-v2`, speaking ACP v2. The five follow-ups of `agents-profiles` are independent of one another, so that order is a suggestion.

**Status: proposal stage.** This change has no design, specs or tasks yet. It is taken further once `agents-profiles` is archived (and with it `agents-launch-commands` and `agents-sessions`).

## Why

In step 1 an agent session's whole conversation log is kept, sent and copied whole. The terminal server caps it at 16 MiB and drops the oldest entries past that, every attach replays the whole log before anything live, every checkpoint rewrites the whole log into the holder, and each agent zone keeps its own copy beside the fold. For the user that means: a long conversation loses its beginning; attaching or restoring a tab with a long conversation, especially over SSH, waits for the whole log; and each long conversation costs Brindle about twice the log in memory. A user mostly reads the end of a conversation, so the rest could stay on the server until it is scrolled to.

## What Changes

- Attaching an agent session sends a page of the newest entries, and the agent zone fetches older entries when the user scrolls up to them.
- The server's checkpoints append to the stored log instead of rewriting it, so a checkpoint's cost no longer grows with the conversation.
- Both are additive: a later client protocol version and a new checkpoint format version; clients on protocol 4 keep the whole replay.

Open questions, to answer before a design:
1. **The page:** how much an attach sends (entries or bytes), and how a client asks for older entries (a new client message, by entry id).
2. **The bound:** whether the 16 MiB cap stays, rises or goes once checkpoints append, and what bounds the stored log then (the holder accepts at most a 64 MiB checkpoint).
3. **The fold's guarantee.** Step 1 promises that folding a replay gives the server's log (`fold(replay(t)) == t`). With paging the client holds a suffix: what the fold's property becomes, and how an entry the client hasn't fetched is updated (a tool call updated by id, a permission's outcome) or referred to.
4. **The checkpoint format:** how appended segments are stored and compacted, what an older core does with them (`process R15`), and what an upgrade or a truncated tail does.
5. **The zone:** what it shows above the newest page, how scrolling up waits for a fetch, whether it drops pages it fetched to stay bounded, and what the dump says.
6. **Who else uses it:** whether a page-sized replay is also what a small client needs (`add-pocket-client` notes a smaller replay as something its device may need from the relay).

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
- `agent-zones` (added by `agents-profiles`): "Scrolling the conversation", for entries fetched on demand.
- `terminal-server`: "Protocol versions", for the new client protocol version.

## Impact

- `transcript/` (`brindle-transcript`): the replay, the fold over a partial log, and their property tests.
- `remote-agent/src/acp/` and the core: the checkpoint format, its segments and their compaction, the page on attach and the fetch.
- `remote-protocol/`: a new client protocol version, transcript tests and fuzzing for its messages.
- `src/agent/`: the zone's copy, its fetches and scrolling.
