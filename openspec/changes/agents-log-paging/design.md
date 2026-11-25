# Design

## Context

See proposal.md for why. The code this design builds on is `agents-sessions` and `agents-profiles` as built (archived 2026-11-25), plus `agents-markdown`, `agents-session-recovery` and `agents-login`, which land before this change:

- **The log.** `brindle_transcript::Transcript` keeps the kept entries in a `VecDeque<Kept { id, entry, size }>`, with `size` each entry's encoded length (`Entry::encoded_len`) and `encoded_len()` their sum. `apply` is the one fold; `replay()` rebuilds a log from nothing (`Trimmed` first, one `Added` per entry with text cut into `Appended`s of at most `TEXT_CHUNK`, 64 KiB, the `Answered` outcomes, `Queue` when the slot isn't empty, `Turn`, `Agent`); `trim_to(limit)` drops the oldest entries whole, keeping the newest. Entry ids are contiguous from the first kept one to `next_id() − 1`: they are given in order and dropped only from the front.
- **The bound.** The reducer (`remote-agent/src/acp/reduce.rs`) trims after every step to `LOG_LIMIT`, 16 MiB, so once a log is at the cap nearly every new entry trims one old one.
- **The replay.** `ATTACH` of an agent session is answered `REPLAY_START`, `TRANSCRIPT`… (`client::pack_transcript`: whole events per frame, under the 1 MiB frame limit), `REPLAY_END`, then live `TRANSCRIPT`s; a reconnect after a core restart replays the client's sessions the same way with no `req`. Brindle's sink (`src/agent/sink.rs`) folds each frame into `AgentShared` on the reader thread; `REPLAY_START` resets the fold; `ChatModel::absorb` copies the changes into its `entries` and `blocks` and emits `Splice`s to the `MessageScroller` (GPUI `list`, `ListAlignment::Bottom`).
- **The checkpoint.** `acp::checkpoint::encode` writes `BACP`, format 1 (`u16` BE), the state's length (`u32` BE), the JSON state (the log as `replay()` events, `Rpc`, phase, run, tools, `Lines` with `covers`), then the partial line's bytes. The core checkpoints an agent session at every reducer `Moment` (`Started`, `BeforeRequest`, `TurnEnded`, `QueueChanged`, `Permission`, `Stopped`), at step-down, on detach, and past 512 KiB of stdout (`policy::agent_checkpoint_due`). Each is the whole state, sent as `CHECKPOINT_BEGIN`/`CHUNK`/`END` (H1), which the holder assembles (`holder::Assembler`, one open blob per session) and stores in place of the last one on `END`, dropping `covers` bytes from the tail. A holder keeps at most `MAX_CHECKPOINT_LEN`, 64 MiB.
- **Versions.** v0.3.0, the latest release, speaks client protocols 1–3 and H1 1–2. Client protocol 4 and H1 3 come with `agents-sessions` and are unreleased; the agent changes ship together in one release (the user's decision), so both are still open to change.
- **Upgrades.** A core speaks H1 1–3 and adopts every holder; holders of another build are told to `DRAIN` and keep their sessions until they end, the core still feeding and checkpointing them. On Windows those holders run from per-build copies of the helper (`server_dir::helper_copies`), so an old build's holder can run for days beside a new core. A core spawns pipe sessions only on a holder of its own build at H1 3 (`policy::placement`).

## Goals / Non-Goals

**Goals:**
- An attach, a restore and a reconnect send a page of about 256 KiB however long the conversation, and the zone fetches the rest only as the user scrolls up to it.
- A checkpoint's cost tracks what changed since the last one, not the log's length.
- The fold's guarantee is restated for a suffix and pinned by property tests.

**Non-Goals:**
- Raising or removing the 16 MiB bound, or spilling the log to disk.
- Dropping fetched pages from the zone to bound its memory (the server's bound caps it).
- Fetching from anywhere but the top: no jump to an entry, no search over unfetched entries.
- Any work for `add-pocket-client`, which speaks client protocol 1 and has no agent sessions. The page budget stays a field the client sets.
- Terminal sessions: their checkpoints and replays don't change.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Event::Window`, `Transcript::{page, fetch, prepend, window}`, the fold below the window, `PAGE_BYTES`, the new `FoldError`s, their property tests | pure | `brindle-transcript` (`transcript/src/lib.rs`, `fuzz.rs`) |
| `ATTACH.page?`, `AGENT_FETCH`, `AGENT_PAGE` with `pack_page`; transcripts and fuzzing | codec | `brindle-remote-protocol` `client.rs` |
| `CHECKPOINT_APPEND`; transcripts and fuzzing | codec | `brindle-remote-protocol` `holder.rs` |
| Checkpoint format 2: a base and segments; `encode`, `encode_segment`, `decode` of formats 1 and 2 | pure | `brindle-remote-agent` `acp/checkpoint.rs` |
| The trim's low mark | pure | `acp/reduce.rs` |
| Whole or append (`CheckpointForm`), the page budget's clamp, fetch refusals | pure | `server/core/policy.rs` |
| Storing appended checkpoints: concatenate, length checks | shell (loop handler) | `server/holder.rs` |
| Paged replays, answering `AGENT_FETCH`, keeping the unsaved events and what the holder stores | shell (loop handler) | `server/core/agents.rs`, `core/sessions.rs`, `core/clients.rs` |
| Routing `AGENT_PAGE` to the session's sink; `AGENT_FETCH` refused while the link is down | shell (connection thread) | `src/remote/local/session.rs`, `connection.rs` |
| Folding a page into `AgentShared` | shell (reader thread) | `src/agent/sink.rs`, `shared.rs` |
| `Earlier`, `fetch_due`, the rows' splices, the dump | pure (GPUI-free model) | `src/agent/model.rs` |
| Sending the fetch, the Earlier row | shell (entity, view) | `src/agent/mod.rs`, `view.rs` |
| The scroller's top row and keeping the view on an insert above | component | `brindle-ui` `components/message_scroller.rs` |

No crate gains a dependency, and references keep pointing one way (`ports R1`, `R3`): `remote-protocol` carries the page's events as opaque JSON objects and never names `brindle-transcript`, so the budget's clamp lives in the core's policy, beside the constant it reads from `brindle-transcript`. No platform `cfg` is added (`ports R4`).

## Decisions

### 1. The user's answers, and where each is worked out

1. **Pages by byte budget of whole entries** (decisions 3, 5): the attach sends the newest whole entries up to 256 KiB encoded, at least the newest entry, always reaching the oldest pending permission; `ATTACH.page` asks for less; `AGENT_FETCH` returns older entries with `more`.
2. **The 16 MiB bound stays** (decision 6), trimmed to a low mark so compaction on a trim stays rare.
3. **`Window { from, earlier }`** (decisions 3, 4): `Trimmed` still means gone; the fold ignores events below its floor; a fetch returns current state; the two laws are property tests.
4. **Append-only checkpoints in this change** (decisions 7–9): a base and appended segments, `CHECKPOINT_APPEND` in H1 3, checkpoint format 2, compaction at step-down, on a trim, and when segments outgrow the base; released holders and per-build copies in decision 10.
5. **The zone** (decision 11): an "Earlier messages" row that fetches when scrolled into view, with a spinner; the view kept; fetched pages kept; `earlier=N|loading|none` in the dump (decision 12).
6. **No pocket work**; the budget is the client's field (decision 5).
- **Cross-cutting:** client protocol 4 and H1 3 are amended in place, with no fallback for builds of them that were never released (decision 2).

### 2. Protocol 4 and H1 3 are amended, not succeeded (`process R4`, `R5`)

The agent changes ship in one release, so protocol 4 and H1 3 have never been released: `process R5`'s "a format that has shipped is frozen" doesn't bind them yet. This change adds to them in place instead of opening protocol 5 and H1 4:

- **Client protocol 4** gains the message `AGENT_FETCH`, the reply `AGENT_PAGE`, the optional field `ATTACH.page`, and the event kind `Window`. The range stays 1–4. The `agent-sessions` requirement "Transcript event forms are fixed for each protocol version" holds for the released form of 4, which now includes `Window`.
- **H1 3** gains `CHECKPOINT_APPEND`. The range stays 1–3.
- **No fallback** for a peer built from these changes before this one: there is no capability flag, no "whole replay" mode, no per-holder probe. Section "What a pre-release build meets" in decision 10 says what such a peer sees anyway.
- **Released versions keep their rules.** Clients on 1–3 never see agent sessions; holders on H1 1–2 never hold pipe sessions, so they never receive an agent session's checkpoint, whole or appended. Nothing sent on a connection below 4 (client) or below 3 (H1) changes, and the existing fixtures for them stay byte for byte.
- **What would force a real bump:** a release cut after some agent changes merge but before this one. Then protocol 4 and H1 3 are frozen at that release, and this change needs protocol 5 (for `AGENT_FETCH`, `AGENT_PAGE` and `Window`) and H1 4 (for `CHECKPOINT_APPEND`), with the core sending a protocol 4 client today's whole replay. The release PR (release-please) must not be merged until every agent change is in (Migration Plan).
- **Message numbers.** `agents-session-recovery` (`AGENT_RESTART`) and `agents-login` (`AGENT_LOGIN`) add client messages to protocol 4 first. `AGENT_FETCH` takes the next free client → core type after theirs (21 if neither has), and `AGENT_PAGE` the next free core → client type (43 if none). `CHECKPOINT_APPEND` is H1 type 16, the next after `SPAWN_PIPES`. The transcripts pin whichever numbers they get.
- **Fixtures.** The fixtures of the amended versions are written only when missing and never edited, as today: every new message gets new fixtures; existing ones stay valid, since an attach whose whole log fits in a page encodes exactly as before (no `Window`, no `page` field).

### 3. The page (pure, `brindle-transcript`; `fcis R1`, `R3`)

`PAGE_BYTES = 256 * 1024` is the most a page holds, counted in `Kept::size` (an entry's encoded length, which the bound counts too).

`Transcript::page(budget: usize) -> Vec<Event>` is `replay()` cut at a start entry `from`:
1. Walk the kept entries newest first, taking whole entries while their sizes add up to at most `budget`; the newest is always taken, however large.
2. Lower `from` to the oldest permission entry whose `outcome` is `None`, when that is older (at most 16 are pending, so this is bounded by the reducer's own cap).
3. Emit `Trimmed` (when entries were ever dropped), then `Window { from, earlier }` when `from` is past the first kept entry (`earlier` = how many kept entries precede `from`, which with contiguous ids is `from − first kept`), then each entry from `from` as `replay_entry` emits it, then `Queue` (when not `Empty`), `Turn`, `Agent`.

With a budget of `PAGE_BYTES` and a log that fits, `page` equals `replay()`, so short conversations replay exactly as before.

`Transcript::fetch(before: EntryId, budget: usize) -> Fetched { events: Vec<Event>, more: bool }` takes the newest whole kept entries with ids below `before`, the same way (at least one when any exists), and emits them as `replay_entry` does, in id order: each in its current state. `more` says whether kept entries remain below the first one returned. A `before` at or below the first kept entry gives no events and `more: false`.

`replay()` stays: the checkpoint's base is the whole log.

**The event.** `Event::Window { from: EntryId, earlier: u64 }` (JSON `{"event":"window","from":N,"earlier":M}`): the entries from `from` on follow; `earlier` kept entries before `from` exist on the server and can be fetched. It appears only in a page, after any `Trimmed` and before the first `Added`, never live. *Rejected:* reusing `Trimmed` with an optional `fetchable` field, which would give an existing event a second meaning (`process R5`), and leaving the client to infer the window from the first `Added`'s id, which would make "dropped" and "not sent" look alike.

### 4. The fold over a suffix (pure, `brindle-transcript`; `railway R1`, `R5`; `types R1`)

`Transcript` gains `window: Option<Window { from: EntryId, earlier: u64 }>` (private, read through `window()`): `None` means the transcript holds every kept entry, which is always the case on the server. The rules `apply` adds:

| Event | With a window |
| --- | --- |
| `Window { from, earlier }` | Valid only on a transcript with no entries and `from ≥ next` (after a `Trimmed`, `next` is past the dropped ids); sets `next = from` and the window. Otherwise `FoldError::WindowAfterEntries` or `WindowBehind { from, next }`. |
| `Appended`, `ToolChanged`, `Answered` on an id below `from` | Ignored: `Ok(())`, nothing changes. The entry isn't held; a later fetch brings its current state. |
| `Added` | Unchanged: it must be `next`, which is never below `from`. |
| `Trimmed { through }`, `through < from` | Records the trim; `earlier` becomes `from − through − 1`, the window going when that is 0. |
| `Trimmed { through }`, `through ≥ from` | Drops held entries through `through` as today; the window goes. |

`Transcript::prepend(events: &[Event], more: bool) -> Result<(), FoldError>` folds a fetched page in front: the events are folded into a scratch transcript starting at their first `Added`'s id; its last entry must be `from − 1` (else `FoldError::PageGap { last, from }`); its entries go in front, `from` moves to the first, `earlier` drops by their count, and the window goes when `more` is false. `more: false` while `earlier` would stay above 0, or `more: true` with no entries, is `FoldError::WindowMismatch`. A page while there is no window is `FoldError::NoWindow`. A refused page changes nothing.

**Why the ignore rule is safe.** The server sends every live event for every entry; the client drops the ones below its floor. A fetch reads the log at the moment the core handles it, and its reply goes out on the same ordered client queue as the live events, so every change before that moment is in the reply and every change after it arrives after the reply, when the entry is held. Pending permissions are always inside the page (decision 3), so an answer never names an entry the client lacks, and the reducer's chunk run always targets the newest entry, which the page always holds.

**Invalid states ruled out.** A window exists only on a transcript that was paged; the server's transcript never has one, since it is never fed a `Window`. `earlier` can't be 0 inside a window (the window goes instead). A fetched page can't leave a gap: `prepend` refuses one.

**The laws**, as property tests in `brindle-transcript` over `fuzz::random_transcript` with the crate's own `XorShift` (no new dependency), across budgets from 0 to beyond the log:
- `fold(page(t, b)) == t.suffix(from)`, where the test-only `suffix` is `t` with the entries before `from` taken out into a window.
- Folding `fetch(from, b)` pages in front until `more` is false gives `t` (equal, window gone).
- For every valid live event `e`: `fold(page(t)) + e == (t + e).suffix(from')`, the floor moving only by a trim; and a fetch after `e` returns the entries as `t + e` holds them.
- The page holds every pending permission and the newest entry; its entries' sizes add up to at most `b` unless one entry alone or the pending permissions push it past.

### 5. `AGENT_FETCH` and `AGENT_PAGE` (`process R4`–`R7`, `R9`; `railway R3`)

| type | name | direction | payload | reply |
| ---: | --- | --- | --- | --- |
| 21† | `AGENT_FETCH` | client → core | `req, session, before, max_bytes` | `AGENT_PAGE`… or `ERROR` |
| 43† | `AGENT_PAGE` | core → client | `req, session, events: [object], last?: { more }` | — |
| — | `ATTACH.page?` | client → core | the page budget in bytes; omitted when unset | — |

† or the next free numbers, decision 2.

- **The reply** is one or more `AGENT_PAGE` frames for the `req`, each holding whole events under the frame limit (`client::pack_page`, as `pack_transcript` packs a replay; a 64 KiB `Appended` is the largest event); the last frame carries `last: { more }`, the others leave it out. An empty page is one frame with no events and `last`. In Rust `last` is `Option<PageEnd { more: bool }>`.
- **The budget.** `max_bytes` and `ATTACH.page` are clamped by the core to `PAGE_BYTES` (`policy::page_budget(asked: Option<u32>) -> usize`; absent means `PAGE_BYTES`). Brindle omits `page` and fetches with `PAGE_BYTES`. A reconnect's replays (no `ATTACH`) use `PAGE_BYTES`.
- **Refusals** (`policy::agent_fetch_refusal`, exhaustive): no such session; a terminal session ("session N isn't an agent session"); a client that doesn't have it attached (the text the other agent messages use); a client still waiting for that session's replay ("session N's replay hasn't ended"). Each is an `ERROR` with `req` and `session`.
- **Who may fetch.** Only the attached client, as for prompts; a fetch changes nothing on the server, so it needs no checkpoint and no ordering with the holder.
- **Ended sessions.** A session `agents-session-recovery` keeps after its agent exits keeps its log in the core, so fetches work on it unchanged.
- **Queues (`process R9`).** A reply goes through the client's existing queue and marks (1 MiB / 256 KiB) like a replay. Its size is one budget, or one entry or the pending permissions when they alone pass it, so at most the 16 MiB log, as a replay is today. Brindle keeps at most one fetch in flight per zone.
- **The relay** copies frames by header and needs no change.
- **Transcripts** (`remote-protocol/tests/transcripts.rs`): `client-agent-attach-paged` (`ATTACH` with `page`, a replay with `Trimmed` and `Window`, then live), `client-agent-fetch` (two fetches, a multi-frame reply, the last with `more: false`, an empty page), `client-agent-fetch-refused` (each refusal). `every_message_variant_and_form_appears_in_a_transcript` gains `AgentFetch`, `AgentPage` and both `last` forms. The event form is pinned by `transcript/tests/fixtures/v4/window.json`. `fuzz.rs` covers the new decoders.

### 6. The bound keeps 16 MiB and trims to a low mark (`process R9`)

`LOG_LIMIT` stays 16 MiB. The reducer now trims, when the log passes `LOG_LIMIT`, down to `LOG_TRIM_TO`, 12 MiB (`trim_to(LOG_TRIM_TO)`), instead of to `LOG_LIMIT`.

Why: the user's answer compacts the checkpoint on a trim (decision 9), since a base that still holds dropped entries would keep their bytes until the next rewrite. Trimming to the cap itself trims on nearly every new entry once a log is full, which would make every checkpoint of a full log a whole 12–16 MiB rewrite, the cost this change removes. With a 4 MiB gap, a full log trims, and compacts, once per 4 MiB of new entries, and each compaction writes at most 12 MiB, 3× the growth. Clients see bigger but rarer `Trimmed` events, which they fold as today. The cost is up to 4 MiB of the oldest history dropped earlier than before. *Rejected:* folding trims into segments and letting the size rule alone compact (stored blob at most twice the base, still under 64 MiB), which goes against the user's "on a trim"; it stays the fallback if the low mark is refused. **This is a choice beyond the answers: confirm it.**

### 7. Checkpoint format 2: a base and segments (pure, `acp/checkpoint.rs`; `process R15`; `railway R5`)

| bytes | what |
| --- | --- |
| 4 | `BACP` |
| 2 | format, big-endian: `2` |
| then, repeated | a record |
| 1 | the record's kind: `B` (base) or `S` (segment) |
| 4 | its sequence number, big-endian: the base 0, each segment one more than the last |
| 4 | the JSON state's length, big-endian |
| that many | the JSON state |
| 4 | the partial line's length, big-endian (0 unless `Collecting`) |
| that many | the partial line's bytes |

- **The base's state** is format 1's state object, unchanged: the log as `replay()` events plus `Rpc`, phase, run, tools, `Lines`.
- **A segment's state** has the same fields except `log`, which becomes `events`: the events the session applied since the previous record, in order. Every other field is whole and replaces the previous record's.
- **Reading** folds the base's log, then each segment's `events` in order onto it, and takes the other fields and the partial line from the last record. Then everything format 1 checks is checked on the result (entries the run, tools and waiting permissions name, `Rpc` parts, the partial).
- **New ways not to read** (`Unreadable`, each with its text after "the conversation could not be restored: "): no base first (`NoBase`), a record kind other than `B` or `S` (`RecordKind { kind }`), a sequence number out of order (`Sequence { expected, found }`), a record running past the end (`Truncated`, reused), a segment's events that don't decode or fold (`Log`, reused). The adoption is `not_restored`'s, as today.
- **Format 1 is still read**, so a holder storing a checkpoint from a build before this change (only pre-release builds wrote format 1) adopts. `encode` writes only format 2.
- **API:** `encode(session) -> Result<Vec<u8>, TooLarge>` writes the header and a base; `encode_segment(session, events: &[Event], seq: u32) -> Result<Vec<u8>, TooLarge>` writes one record with no header; `decode` reads both formats. `TooLarge` keeps its meaning for the whole blob; a segment past the limit makes the core write a whole checkpoint (decision 9).
- *Rejected:* a segment holding the entries added since the last checkpoint (instead of the events), which can't express a change to an older entry (a tool call finishing, an answer) without restating it, and segments for the log alone with the rest kept in the base, which would rewrite the base for every `Rpc` change.

### 8. `CHECKPOINT_APPEND` in H1 3 (`process R1`, `R3`, `R7`)

| type | name | direction | payload |
| ---: | --- | --- | --- |
| 16 | `CHECKPOINT_APPEND` | core → holder | session, `len: u64 BE` (version 3) |

It opens a blob as `CHECKPOINT_BEGIN` does, of the same kind (so `MAX_OPEN_BLOBS` and "one open blob per session" cover both), followed by `CHECKPOINT_CHUNK`s and `CHECKPOINT_END { covers }`. On `END` the holder, instead of replacing the stored checkpoint, appends the bytes to it and covers the tail as for a whole one. It refuses, with an `ERROR` for the session, keeping the stored checkpoint and the tail as they were (as for a whole checkpoint it drops):
- an append to a session whose stored checkpoint is empty ("nothing to append to"), so no blob without a header can be stored;
- an append that would make the stored checkpoint pass `MAX_CHECKPOINT_LEN` (64 MiB).

Both are length checks: the holder still parses nothing and holds no policy (`process R1`). It is one message type, one flag on the open blob and one `extend`, handled under the holder's per-message `catch_unwind` as `CHECKPOINT_BEGIN` is (`Unit::Checkpoint`, `process R3`). On a connection below H1 3 it is refused as `SPAWN_PIPES` is. Adopting is unchanged: `WELCOME` is followed by the stored checkpoint, base and segments as one blob, and the tail.

**Why in the holder at all:** the alternative is for the core to keep sending whole checkpoints (decision 9's cost) or to make the holder understand records, which `process R1` forbids. Folding it into H1 3 now spares a later H1 4 and a release of holders that can't append. Transcripts: `holder-checkpoint-append` (a whole checkpoint, two appends, an adoption returning the concatenation), `holder-checkpoint-append-refused` (onto empty, past 64 MiB). `fuzz.rs` covers the decoder.

### 9. When the core appends and when it rewrites (pure policy, `fcis R1`; shell in `core/agents.rs`)

The core keeps, beside each agent session's `AgentSession`, what its holder stores and what changed since:

```rust
enum Stored {
    Unknown,                                        // never written, adopted, or the holder refused one
    Known { base: u64, appended: u64, seq: u32 },   // bytes of the base, of the segments, the last seq
}
enum CheckpointForm { Whole, Append }
```

and `unsaved: Vec<Event>`, every event its steps applied since the last checkpoint (the shell pushes each `Output`'s events; a test pins that the reducer changes the log only through them). `policy::agent_checkpoint_form(stored, at, segment_len, trimmed, holder) -> CheckpointForm`, where `holder` is `HolderBuild::{Own, Other}`, is `Whole` when:
- `stored` is `Unknown`: a new session, an adopted one, or after the holder sent an `ERROR` naming the session (any error: the core can't tell which message it answers, and a whole checkpoint is always safe);
- `at` is `StepDown` (the next core adopts one base, compact);
- `unsaved` holds a `Trimmed` (decision 6);
- `appended + segment_len > base` (the segments would outgrow the base);
- `base + appended + segment_len > MAX_CHECKPOINT_LEN`, or `encode_segment` returned `TooLarge`;
- the holder is of another build (decision 10);

and `Append` otherwise. `agent_checkpoint_due` (when) is unchanged; this decides how. A whole checkpoint sets `Known { base: len, appended: 0, seq: 0 }`; an append adds its length and bumps `seq`; both clear `unsaved`.

**What it costs.** Between two whole checkpoints of base size B, appends write at most B, and the next whole one is at least as large as the base it replaces grew to. So bytes written per byte of log growth are bounded by a small constant (about 2 below the cap, about 4 at it with decision 6's low mark), against today's copy of the whole log at every moment, which at 16 MiB and several moments per turn is hundreds of times the turn's output. A pure test pins it: driving the policy and encoders through a synthetic 64 MiB conversation (moments and 512 KiB stdout steps), total checkpoint bytes stay under 4× the log bytes added, plus one base per trim.

**What stays in memory only (`process R2`).** `Stored` and `unsaved` live in the core. A core restart loses both, and loses nothing: the new core adopts the session from the holder's base, segments and tail, starts at `Unknown`, so its first checkpoint is whole, and starts `unsaved` empty, since everything before the adoption is in what the holder stores.

### 10. What survives what (`process R2`, `R3`, `R12`)

| Event | What happens |
| --- | --- |
| core crash or `SIGKILL` | The holder stores the last whole checkpoint plus the segments it accepted. The next core decodes them (decision 7), feeds the tail as today, and writes a whole checkpoint first. A segment the holder refused leaves a sequence gap only if the core sent another append before it read the `ERROR`: the decoder then refuses the checkpoint (`Sequence`) and the session is adopted as "could not be restored". Holder errors on checkpoints are bugs in the core, so this window is not expected to open. |
| upgrade (`STEP_DOWN`) | The old core writes every agent session's checkpoint whole, then hands over; the new core reads format 2. |
| Brindle crash or quit | Nothing changes on the server. The zone's fetched pages are gone; the next attach or restore sends the newest page again. |
| holder crash | Its sessions are lost, as today. |
| an older core adopting a newer holder | Only a pre-release core (format 1 only) could meet a format 2 checkpoint: it adopts the session as "could not be restored" (`Unreadable::Format`), and the agent keeps running. A released core (H1 ≤ 2) is never shown pipe sessions. |
| a holder of another build (draining, Windows per-build copies included) | Every checkpoint of its agent sessions is whole. A released holder (H1 2) never holds one; a pre-release holder at H1 3 may lack `CHECKPOINT_APPEND`, and the build rule means the core never needs to know. Such a holder runs from its own copy until its sessions end, as today. |
| `check --holder-protocols` during an upgrade | Unchanged: the new binary speaks H1 1–3, so it vets every running holder as before. |

**What a pre-release build meets** (no fallback, decision 2; listed so the user knows what dogfooding sees):
- a pre-change Brindle attaching a long session on a new core gets a `Window` it can't decode, and its zone says it can no longer show the session (the session stays, detached); sessions whose log fits a page replay as before;
- a new Brindle on a pre-change core gets whole replays with no `Window`, so it never fetches, and the `page` field is ignored;
- a pre-change core adopting a new holder reads its format 2 checkpoints as unreadable (above).

### 11. The zone (`fcis R1`, `R5`; `types R1`; `process R13`, `R14`; `railway R6`)

**The model** (`src/agent/model.rs`, GPUI-free):

```rust
enum Earlier {
    None,
    Available { count: u64 },
    Loading { count: u64, request: RequestId },
    Failed { count: u64, why: SharedString },
}
```

set from the transcript's window on every absorb, with `Loading` and `Failed` kept until the page or the reply arrives. The rows the scroller shows are the blocks only; the Earlier row is the scroller's top row (below). `fetch_due(row_in_view, was_in_view, link: Link) -> Option<Fetch { before: EntryId, max_bytes: usize }>` decides: `Available` asks whenever the row is in view; `Failed` asks only when the row comes into view again (`was_in_view` false) or on a press (`retry()`); nothing is asked while `Loading`, while the link is `Reconnecting`, or with no window. `before` is the window's `from`. The entity sends `AGENT_FETCH` with the request and calls `fetching(request)`; `Err(why)` from the reply makes it `Failed`; the page itself arrives as a change.

**The fold.** The sink buffers a reply's `AGENT_PAGE` frames by `req` and, at `last`, calls `AgentShared::prepend`, which runs `Transcript::prepend` and records `Changes::prepended: Option<usize>` (entries added in front), then sends the request's `Reply { Ok }`. A `FoldError` from `prepend` breaks the session as any fold error does (agent-zones "A session Brindle can no longer show"). `REPLAY_START` resets the fold, the window and `Earlier` (a reconnect replays the newest page again, dropping fetched pages, as a reset drops every block today), and a late reply for a request the model no longer waits for is dropped.

**The splice and the view.** `absorb` turns `prepended` into one `Splice { range: 0..0, count }` (the new blocks, in front). `MessageScroller` draws the Earlier row in its list's top padding rather than as an item, so the list's items are exactly the blocks, and GPUI's `ListState::splice` of `0..0` moves the scroll position past the inserted items (it shifts `item_ix` when the range ends at or before it): every block in view stays where it was, and the Earlier row scrolls up out of view with the new content. While the zone follows its end (`Follow::AtEnd`, bottom-aligned), inserting above changes nothing in view either. The scroller reports whether its top padding is in view through a callback the zone registers (`process R14`: computed from the layout it already did, no work in `render`). When the window goes, the padding goes. **Spike first** (task 5.1): a GPUI `VisualTestContext` test that a block's bounds are unchanged after inserting items above it, with the padding row in view and out of it. If GPUI moves the view, the fallback is the Earlier row as item 0 with the view re-anchored on the first block, which may shift by up to the row's height.

**The row** shows "Earlier messages" with the lucide `loader-circle` spinner (an existing `brindle-ui` icon frame) while `Loading`, and "Earlier messages couldn't be loaded: {why}" while `Failed`, pressable to retry. It takes no focus: the keyboard path is shift-pageup, which scrolls it into view, and that fetches.

**Failures shown where they happen (`railway R6`).** A refused fetch and a link that goes down mid-fetch (the client session answers a fetch it can't send, or loses with the link, with `Err`) show on the row; nothing else in the zone changes. `src/remote/local/session.rs` refuses `AGENT_FETCH` while the link is down instead of queuing it as it queues prompts, since a page asked of a connection that then replays would not fit the new fold.

**Memory.** Fetched pages are kept until the zone closes or replays; the zone's two copies (fold and model) are bounded by the server's 16 MiB as before, and in the common case hold one page. Parsing (`agents-markdown`) a fetched page costs one parse of at most a page's entries, once, on the UI thread inside `absorb`, less than today's attach of a whole log.

### 12. The dump (`testing R2`; `command-line`)

The `--- conversation` line becomes `--- conversation entries=N at=end|away earlier=N|loading|none[ fetch-error="…"]`. `earlier` goes after `at` so that `wait_text` (which drops `at=`) reads `entries=N earlier=…`, and existing waits on `conversation entries=N` and dumps checked for `entries=N at=end` keep matching. `earlier=N` is `Available` or `Failed`'s count, `loading` is `Loading`, `none` is no window; `fetch-error` appears only while `Failed`, escaped as the other quoted fields are. `--- entry` lines list only the entries the zone holds, so their ids show where the page starts. CLAUDE.md's dump paragraph gets the new form (task 5.6).

### 13. The sibling changes

- **`agents-markdown`:** its parts split within an entry; pages split between whole entries, so they never meet. A message bigger than the page is sent whole and still cut into parts by its renderer. Its `--- block` lines sit inside the entry lines, before `--- conversation`.
- **`agents-session-recovery`:** a `session/load` replay into an empty log can rebuild far more than a page; that is server-side, and the client still gets a page. A session kept after its agent exits has no holder session, so no checkpoint; a restart into a new holder session starts `Unknown`, so its first checkpoint is whole. `AGENT_RESTART` and `AGENT_FETCH` are independent messages of protocol 4.
- **`agents-login`:** nothing shared but protocol 4's message numbers (decision 2).
- **Archive order:** this change's deltas modify requirements the earlier three may also modify (`terminal-server` "Protocol versions" among them). Whichever archives later re-copies the block from the synced main spec before archiving.
- **`agents-acp-v2`:** unaffected; it changes the reducer, not the log or the wire.

### 14. Tests (`testing R1`, `R7`, `R8`, `R13`, `R14`, `R16`)

Each behaviour in the layer that answers it:
- **Unit and property (pure):** the page, fetch and prepend laws and the fold below the window (`brindle-transcript`); format 2 round trips, every new `Unreadable`, format 1 still read, a pinned format 2 byte string (`acp/checkpoint.rs`); the low mark (`reduce.rs`); `agent_checkpoint_form` for each `Whole` reason, the page budget's clamp, the fetch refusals, and the write-cost bound over a synthetic 64 MiB conversation (`policy.rs`); `Earlier`, `fetch_due`, the prepend splice and the dump line (`src/agent/model.rs`); the sink's frame buffering and the shared fold's `prepended` (`sink.rs`, `shared.rs`); the client session's refusal while the link is down (`session.rs`).
- **Component:** the scroller keeps a block's bounds when items are inserted above (`message_scroller.rs`, `VisualTestContext`).
- **Transcripts and fuzzing:** decisions 5 and 8.
- **Real role tests** (`remote-agent/tests/`, with the fake ACP agent): a paged attach of a long log, fetched back to the start equal to the whole log, a refused fetch from another client, and a tool call updated outside the page then fetched current (`agents.rs`); appends then `SIGKILL` and reconnect (`agents.rs`); a step-down writing whole and the new core carrying on (`upgrade.rs`); the holder's append, its refusals, an append on an H1 2 connection, and adoption returning base plus segments (`holder.rs`); a draining holder of another build getting only whole checkpoints (`core.rs`).
- **e2e** (`tests/e2e/cases/agent.rs`, fake agent, `--wait-for`, no sleeps): `agent-earlier-page`, `agent-earlier-scroll`, `agent-earlier-reattach`.
- **Perf:** a perf-suite case `agent-attach-long` (`testing R14`): time from launch to the first page of a 16 MiB conversation shown, against a baseline taken on the build before this change.
- **Manual:** wheel scrolling to the top of a long real conversation (`testing R16`).

### 15. Docs

`CLAUDE.md` (the dump paragraph; the manual check), `docs/code-map.md` (`brindle-transcript`'s page and window; the core's paged replays, fetch, `Stored` and appends; the holder's append; the zone's Earlier row), `docs/architecture/process-architecture.md` ("How it applies here": what the holder stores is a checkpoint and segments appended to it), and the module docs of `remote-protocol/src/client.rs` and `holder.rs` (their tables and the protocol 4 and H1 3 paragraphs).

### 16. Choices made beyond the user's answers (flagged)

1. **Trimming to a 12 MiB low mark** (decision 6), so "compact on a trim" doesn't rewrite every checkpoint of a full log.
2. **Appends only to a holder of the core's own build** (decision 10): stricter than "H1 3", so a pre-release H1 3 holder without `CHECKPOINT_APPEND` is never sent one, with no probe.
3. **Any holder `ERROR` naming an agent session makes the next checkpoint whole** (decision 9), and **a sequence number per record**, so a lost segment is refused at adoption rather than misread.
4. **The holder refuses an append onto an empty checkpoint and one past 64 MiB** (decision 8).
5. **Segments carry events, not entries** (decision 7), and **the core keeps the unsaved events** beside the session.
6. **`max_bytes` and `ATTACH.page` are clamped to 256 KiB**, not refused, and **a reconnect's replays use 256 KiB** (no `HELLO` field) (decision 5).
7. **A fetch is refused while the session's replay is pending**, and **Brindle refuses a fetch while the link is down** instead of queuing it (decisions 5, 11).
8. **The reply is `AGENT_PAGE` frames with `last: { more }`** on the final one (decision 5).
9. **The Earlier row is drawn in the scroller's top padding**, with a spike and a fallback (decision 11); **a failed fetch retries when the row comes into view again or is pressed**, and the dump adds `fetch-error="…"` while failed.
10. **`earlier=` goes after `at=`** on the conversation line (decision 12).
11. **Fixtures of the amended versions stay append-only**, as before; no existing fixture is edited (decision 2).
12. **A perf-suite case `agent-attach-long`** (decision 14).

## Risks / Trade-offs

- [A release is cut between agent changes, freezing protocol 4 and H1 3 without paging] → hold the release PR until every agent change lands; if it happens anyway, this change moves to protocol 5 and H1 4 (decision 2).
- [GPUI moves the view when items are inserted above] → the spike comes first (task 5.1); the fallback shifts by at most one row.
- [A fetch races a trim] → both go through the core's one ordered client queue; `prepend` refuses a gap, and a page below a trim is empty with `more: false` (decision 4); the property tests cover trims between fetches.
- [A huge single entry or many pending permissions make a page large] → bounded by the 16 MiB log, as a replay is today, and sent through the client queue's marks.
- [Dropping up to 4 MiB more history at a time] → decision 6; confirm or fall back to trims inside segments.
- [A segment lost between a holder's refusal and the core reading it] → the sequence number turns it into an honest "could not be restored" instead of a wrong log; holder refusals of checkpoints are core bugs.
- [Covers after a dropped checkpoint] → appends behave exactly as whole checkpoints do when the holder drops one (stored checkpoint and tail kept); the accounting after a dropped checkpoint is today's and out of scope here.
- [Pre-release builds in daily use] → decision 10 lists what they see; none loses a session, a long conversation in an old Brindle needs the newer one to show.
- [Two copies in the zone of a fully fetched long log] → still bounded by 16 MiB each, as before; paging only makes the common case smaller.

## Migration Plan

Additive for every released version: clients on 1–3 and holders on H1 1–2 see nothing new. Within the one release that brings agent sessions, protocol 4 and H1 3 carry paging and appends from the start. A newer helper upgrades the running core (`process R12`); the old core writes whole checkpoints at step-down; the new core appends only to its own build's holders and reads checkpoints of format 1 and 2. **The release PR must wait until `agents-markdown`, `agents-session-recovery`, `agents-login` and this change are all merged.** Rollback to a build before this change: its core reads this change's format 2 checkpoints as unreadable, so agent sessions adopted by it show "the conversation could not be restored" and keep running; terminal sessions are unaffected. No config or state file changes.

## Open Questions

None that change the specs, the approach or the tasks. Decision 16 lists the choices made beyond the user's answers for confirmation; the first (the 12 MiB low mark) is the one most worth a look.
