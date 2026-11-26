# Design

## Context

See proposal.md for why. This design was written on 2026-11-25, before most of the agent stack was built, and refreshed on 2026-11-26 against `main` at cdb972c (`agents-sessions` #122, `agents-profiles` #123, `agents-markdown` #127, `agents-session-recovery` #132, `agents-login` #133 and `remote-login-environment` #137, the agent changes archived) and the open stack `feat/agents-zone-attention` at cc4f41a (`agents-markdown-layout-fixes`, `agents-zone-theme-scope` and `agents-session-title`, archived there; `agents-zone-attention`, not yet archived). It lands after that stack. What it builds on, as built:

- **The log.** `brindle_transcript::Transcript` keeps the kept entries in a `VecDeque<Kept { id, entry, size }>`, with `size` each entry's encoded length (`Entry::encoded_len`) and `encoded_len()` their sum, plus the slot, the turn, the agent's state, `conversation: Option<Conversation>` and the trim so far. `Conversation { id, cwd, continuation, title }` is the agent's ACP session; its `title` (`agents-session-title`) is set by the reducer from `session_info_update` and travels in `Event::Conversation`, which a later one replaces whole. `apply` is the one fold; `replay()` rebuilds a log from nothing in this order: `Trimmed` (when entries were dropped), one `Added` per entry with text cut into `Appended`s of at most `TEXT_CHUNK` (64 KiB) and a permission's outcome as `Answered`, `Queue` (when the slot isn't `Empty`), `Conversation` (when there is one), `Turn`, `Agent`. `trim_to(limit)` drops the oldest entries whole, keeping the newest. Entry ids are contiguous from the first kept one to `next_id() − 1`: they are given in order and dropped only from the front. `fuzz.rs` has `random_transcript` and the crate's own `XorShift`.
- **The bound.** The reducer (`remote-agent/src/acp/reduce.rs`, `Step::finish` → `Step::trim`) trims to `LOG_LIMIT`, 16 MiB, after every step that leaves the log past it, so once a log is at the cap nearly every new entry trims one old one. A trim also forgets the dropped entries' tool calls and chunk run, and answers any dropped pending permission `cancelled` to the agent. The reducer changes the log only through `Step::emit`, which applies each event and pushes it to `Output::events`, and the trim, which pushes its `Trimmed` there too.
- **The replay.** `ATTACH` of an agent session goes through `Core::replay` (`core/sessions.rs`): a session still being adopted (`rebuild` not `Ready`) or whose meta is being stored waits as a `PendingReplay { req, size }`; otherwise the core sends `REPLAY_START`, the log as `TRANSCRIPT` frames (`Agent::replay` → `Transcript::replay`, packed by `client::pack_transcript`: whole events per frame, under the 1 MiB frame limit), `REPLAY_END`, then live `TRANSCRIPT`s. While a replay is pending, `Core::agent_apply` sends the client no live events, since the replay will hold them. A reconnect after a core restart replays the client's sessions the same way, with no `req`.
- **The zone.** Brindle's sink (`src/agent/sink.rs`) folds each frame into `AgentShared` (`src/agent/shared.rs`) on the connection's reader thread, recording `Changes { added_from, changed, trimmed_through, slot_changed, turn_or_agent, reset, from_replay }`; `REPLAY_START` resets the fold. `ChatModel::absorb` (`src/agent/model.rs`) copies the changes into its `entries`, renders agent messages and thoughts into its Markdown cache (`rendered: BTreeMap<EntryId, Vec<Part>>`, `agents-markdown`), draws `blocks`, and emits `Splice`s to the `MessageScroller` (`brindle-ui`, a GPUI `list` with `ListAlignment::Bottom`, `Follow::{AtEnd, Away}`, `scroll_page` for shift-pageup and shift-pagedown). With `agents-zone-attention`, the working row is the list's last item after the blocks while it shows (`view::working_splice`), its text the turn's newest tool call in progress among the held entries (`ChatModel::working_row`), and its `Loader` and `Shimmer` step on the window's motion clock (`src/motion.rs`), which runs while some entity wants it. Fold changes made during a replay set `from_replay`, which raises no notification.
- **The checkpoint.** `acp::checkpoint::encode` writes `BACP`, format 1 (`u16` BE), the state's length (`u32` BE), the JSON state, then the partial line's bytes. The state holds the log as `replay()` events (the conversation with its title among them), the ACP version, the `Phase` (with a continuation's start, a muted load's count, a login's message and method, a login restart), the advertised login `methods`, the `Rpc` parts, the chunk run, the tools, the bad-line count and `Lines` with `covers`. The core checkpoints an agent session at every reducer `Moment` (`Started`, `BeforeRequest`, `TurnEnded`, `QueueChanged`, `Permission`, `Stopped`, `Login`), at step-down, on detach, and past 512 KiB of stdout (`policy::agent_checkpoint_due`), and right after placing a restart's or a continuing session's `SPAWN_PIPES`. Each is the whole state, sent as `CHECKPOINT_BEGIN`/`CHUNK`/`END` (H1, `ToHolder::checkpoint`), which the holder assembles (`holder::Assembler`, one open blob per session) and stores in place of the last one on `END`, dropping `covers` bytes from the tail. A holder keeps at most `MAX_CHECKPOINT_LEN`, 64 MiB. A state too large to store marks the `Agent` `unstorable` and fails its ACP handling.
- **Recovery** (`agents-session-recovery`). An agent that exits, or whose holder is lost, with a continuable conversation is kept as `Life::AgentEnded`, its log in this core's memory only: no program, no holder, no checkpoint. `AGENT_RESTART` (client type 21) runs a new agent under the same session id on any H1 3 holder of the core's build (`SpawnRequest::Restart`), in a new holder session whose stdout counts from 0 (`Agent::restarted`), into the same log; `SPAWN_AGENT` with a `conversation` starts a fresh log (`Agent::continuing`). After `initialize` the reducer resumes or loads: a kept log's load replay is muted (counted, not applied, then one notice), a fresh log's is shown, adding every replayed message as entries.
- **Login** (`agents-login`). `AGENT_LOGIN` (client type 22) runs ACP's `authenticate`; the agent's `LoginRequired { message, methods }` and `LoggingIn` states travel in `Event::Agent`. A terminal login's `AGENT_RESTART` while the agent needs a login hangs it up (`Phase::Restarting`) and starts the restart on its exit (`Core::restart_after_login`), as an ended agent's restart.
- **Versions.** v0.3.0, the latest release, speaks client protocols 1–3 and H1 1–2. Client protocol 4 and H1 3 came with `agents-sessions` and are unreleased; the agent changes ship together in one release (the user's decision), so both are still open to change. Protocol 4's client → core types run to 22 (`AGENT_LOGIN`), its core → client types to 42 (`TRANSCRIPT`); H1 3's core → holder types run to 15 (`SPAWN_PIPES`), its holder → core types to 41 (`STDERR_TAIL`).
- **Upgrades.** A core speaks H1 1–3 and adopts every holder; `policy::adoption(holder_build, core_build)` keeps a holder of its own build (`Adoption::Keep`) and tells one of another build to `DRAIN` (`Adoption::Drain`), which keeps its sessions until they end, the core still feeding and checkpointing them. On Windows those holders run from per-build copies of the helper (`server_dir::helper_copies`), so an old build's holder can run for days beside a new core. A core places pipe sessions, restarts included, only on a holder of its own build at H1 3 (`policy::placement`).

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
- Pulling the working row's tool call, or the copy actions' latest message, into the page (decision 13).

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Event::Window`, `Transcript::{page, fetch, prepend, window}`, the fold below the window, `PAGE_BYTES`, the new `FoldError`s, their property tests | pure | `brindle-transcript` (`transcript/src/lib.rs`, `fuzz.rs`) |
| `Attach.page`, `AGENT_FETCH` (23), `AGENT_PAGE` (43) with `pack_page`; transcripts and fuzzing | codec | `brindle-remote-protocol` `client.rs` |
| `CHECKPOINT_APPEND` (H1 16); transcripts and fuzzing | codec | `brindle-remote-protocol` `holder.rs` |
| Checkpoint format 2: a base and segments; `encode`, `encode_segment`, `decode` and `adopt` of formats 1 and 2 | pure | `brindle-remote-agent` `acp/checkpoint.rs` |
| The trim's low mark | pure | `acp/reduce.rs` |
| Whole or append (`Stored`, `CheckpointForm`, `agent_checkpoint_form`), the page budget's clamp, fetch refusals (`AgentRequest::Fetch`) | pure | `server/core/policy.rs` |
| Storing appended checkpoints: concatenate, length checks | shell (loop handler) | `server/holder.rs` |
| Paged replays (`PendingReplay.page`), answering `AGENT_FETCH`, keeping the unsaved events and what the holder stores | shell (loop handler) | `server/core/agents.rs`, `core/sessions.rs`, `core/clients.rs` |
| `AgentLink::fetch`; routing `AGENT_PAGE` to the session's sink (`SessionSink::page`); a fetch refused while the link is down | shell (connection thread) | `src/remote/mod.rs`, `src/remote/local/{session,connection,agent}.rs` |
| Folding a page into `AgentShared` | shell (reader thread) | `src/agent/sink.rs`, `shared.rs` |
| `Earlier`, `fetch_due`, `Sent::Fetch`, the prepend's renders and splice, the dump | pure (GPUI-free model) | `src/agent/model.rs` |
| Sending the fetch, wanting the motion clock while loading, the Earlier row | shell (entity, view) | `src/agent/mod.rs`, `view.rs` |
| The scroller's top row, its place in a page scroll, and keeping the view on an insert above | component | `brindle-ui` `components/message_scroller.rs` |

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

All six still fit the built stack. Decision 16 lists the choices made beyond them, the refresh's included.

### 2. Protocol 4 and H1 3 are amended, not succeeded (`process R4`, `R5`)

The agent changes ship in one release, so protocol 4 and H1 3 have never been released (v0.3.0 is still the latest tag): `process R5`'s "a format that has shipped is frozen" doesn't bind them yet. This change adds to them in place instead of opening protocol 5 and H1 4:

- **Client protocol 4** gains the message `AGENT_FETCH`, the reply `AGENT_PAGE`, the optional field `Attach.page`, and the event kind `Window`. The range stays 1–4. The `agent-sessions` requirement "Transcript event forms are fixed for each protocol version" holds for the released form of 4, which now includes `Window`.
- **H1 3** gains `CHECKPOINT_APPEND`. The range stays 1–3.
- **No fallback** for a peer built from these changes before this one: there is no capability flag, no "whole replay" mode, no per-holder probe. Section "What a pre-release build meets" in decision 10 says what such a peer sees anyway.
- **Released versions keep their rules.** Clients on 1–3 never see agent sessions; holders on H1 1–2 never hold pipe sessions, so they never receive an agent session's checkpoint, whole or appended. Nothing sent on a connection below 4 (client) or below 3 (H1) changes, and the existing fixtures for them stay byte for byte.
- **What would force a real bump:** a release cut after the agent stack merges but before this change. Then protocol 4 and H1 3 are frozen at that release, and this change needs protocol 5 (for `AGENT_FETCH`, `AGENT_PAGE` and `Window`) and H1 4 (for `CHECKPOINT_APPEND`), with the core sending a protocol 4 client today's whole replay. The release PR (release-please) must not be merged until this change is in (Migration Plan).
- **Message numbers.** `AGENT_RESTART` took client type 21 and `AGENT_LOGIN` 22, as `agents-login`'s design foresaw ("the next free one after recovery's `AGENT_RESTART`, and `agents-log-paging`'s `AGENT_FETCH` if that lands first"). So `AGENT_FETCH` is client → core type **23**, `AGENT_PAGE` core → client type **43** (the next after `TRANSCRIPT`, 42), and `CHECKPOINT_APPEND` H1 core → holder type **16** (the next after `SPAWN_PIPES`, 15). `agents-commands` plans `AGENT_SET_OPTION` "after `AGENT_RESTART`, `AGENT_LOGIN` and `AGENT_FETCH`", so it takes 24. Should another change land a client message first, this one takes the next free number; the transcripts pin whichever it gets.
- **Fixtures.** The fixtures of the amended versions are written only when missing and never edited, as today: every new message gets new fixtures; existing ones stay valid, since an attach whose whole log fits in a page encodes exactly as before (no `Window`, no `page` field), titled conversations included.

### 3. The page (pure, `brindle-transcript`; `fcis R1`, `R3`)

`PAGE_BYTES = 256 * 1024` is the most a page holds, counted in `Kept::size` (an entry's encoded length, which the bound counts too).

`Transcript::page(budget: usize) -> Vec<Event>` is `replay()` cut at a start entry `from`:
1. Walk the kept entries newest first, taking whole entries while their sizes add up to at most `budget`; the newest is always taken, however large.
2. Lower `from` to the oldest permission entry whose `outcome` is `None`, when that is older (the reducer keeps at most 16 waiting, and a trim cancels any it drops, so this is bounded).
3. Emit `Trimmed` (when entries were ever dropped), then `Window { from, earlier }` when `from` is past the first kept entry (`earlier` = how many kept entries precede `from`, which with contiguous ids is `from − first kept`), then each entry from `from` as `replay_entry` emits it, then what `replay()` emits after the entries, in its order: `Queue` (when not `Empty`), `Conversation` (when there is one, with its title), `Turn`, `Agent`.

Everything that isn't an entry (the slot, the conversation and its title, the turn, the agent's state with a login's methods) is in every page, so a page shows the zone's title, prompt state and login row exactly as a whole replay would. With a budget of `PAGE_BYTES` and a log that fits, `page` equals `replay()`, so short conversations replay exactly as before.

`Transcript::fetch(before: EntryId, budget: usize) -> Fetched { events: Vec<Event>, more: bool }` takes the newest whole kept entries with ids below `before`, the same way (at least one when any exists), and emits them as `replay_entry` does, in id order: each in its current state. `more` says whether kept entries remain below the first one returned. A `before` at or below the first kept entry gives no events and `more: false`. A fetch carries entries only: the state after them is already the client's.

`replay()` stays: the checkpoint's base is the whole log.

**The event.** `Event::Window { from: EntryId, earlier: u64 }` (JSON `{"event":"window","from":N,"earlier":M}`): the entries from `from` on follow; `earlier` kept entries before `from` exist on the server and can be fetched. It appears only in a page, after any `Trimmed` and before the first `Added`, never live. *Rejected:* reusing `Trimmed` with an optional `fetchable` field, which would give an existing event a second meaning (`process R5`), and leaving the client to infer the window from the first `Added`'s id, which would make "dropped" and "not sent" look alike.

### 4. The fold over a suffix (pure, `brindle-transcript`; `railway R1`, `R5`; `types R1`)

`Transcript` gains `window: Option<Window { from: EntryId, earlier: u64 }>` (private, read through `window()`): `None` means the transcript holds every kept entry, which is always the case on the server. The rules `apply` adds:

| Event | With a window |
| --- | --- |
| `Window { from, earlier }` | Valid only on a transcript with no entries and `from ≥ next` (after a `Trimmed`, `next` is past the dropped ids); sets `next = from` and the window. Otherwise `FoldError::WindowAfterEntries` or `WindowBehind { from, next }`. |
| `Appended`, `ToolChanged`, `Answered` on an id below `from` | Ignored: `Ok(())`, nothing changes. The entry isn't held; a later fetch brings its current state. |
| `Added` | Unchanged: it must be `next`, which is never below `from`. |
| `Queue`, `Conversation`, `Turn`, `Agent` | Unchanged: they aren't entries. |
| `Trimmed { through }`, `through < from` | Records the trim; `earlier` becomes `from − through − 1`, the window going when that is 0. |
| `Trimmed { through }`, `through ≥ from` | Drops held entries through `through` as today; the window goes. |

`Transcript::prepend(events: &[Event], more: bool) -> Result<(), FoldError>` folds a fetched page in front: the events are folded into a scratch transcript starting at their first `Added`'s id; its last entry must be `from − 1` (else `FoldError::PageGap { last, from }`); its entries go in front, `from` moves to the first, `earlier` drops by their count, and the window goes when `more` is false. `more: false` while `earlier` would stay above 0, or `more: true` with no entries, is `FoldError::WindowMismatch`. A page with entries while there is no window is `FoldError::NoWindow`. **An empty page with `more: false` while there is no window is accepted and changes nothing** (refresh, decision 16): it is the server's answer to a fetch it handled after a trim that took the window (below), and the zone has nothing to add. A refused page changes nothing.

**Why the ignore rule is safe.** The server sends every live event for every entry; the client drops the ones below its floor. A fetch reads the log at the moment the core handles it, and its reply goes out on the same ordered client queue as the live events, so every change before that moment is in the reply and every change after it arrives after the reply, when the entry is held. Pending permissions are always inside the page (decision 3), so an answer never names an entry the client lacks, and the reducer's chunk run always targets the newest entry, which the page always holds.

**A fetch racing a trim.** Only one fetch is in flight per zone (decision 11). If the core handles it before a trim, the page arrives first and the `Trimmed` after it, folding as today. If after a trim with `through < from`, the page holds the entries the trim kept, ending at `from − 1`. If after a trim with `through ≥ from` (the user had fetched back past the low mark), the client's window went with the `Trimmed`, and the fetch is answered empty with `more: false`, which the rule above accepts. None of these breaks the session.

**Invalid states ruled out.** A window exists only on a transcript that was paged; the server's transcript never has one, since it is never fed a `Window`. `earlier` can't be 0 inside a window (the window goes instead). A fetched page can't leave a gap: `prepend` refuses one.

**The laws**, as property tests in `brindle-transcript` over `fuzz::random_transcript` with the crate's own `XorShift` (no new dependency), across budgets from 0 to beyond the log, with titled conversations and login states among the generated logs:
- `fold(page(t, b)) == t.suffix(from)`, where the test-only `suffix` is `t` with the entries before `from` taken out into a window.
- Folding `fetch(from, b)` pages in front until `more` is false gives `t` (equal, window gone).
- For every valid live event `e`: `fold(page(t)) + e == (t + e).suffix(from')`, the floor moving only by a trim; and a fetch after `e` returns the entries as `t + e` holds them.
- The page holds every pending permission, the newest entry, and the slot, conversation, turn and agent's state; its entries' sizes add up to at most `b` unless one entry alone or the pending permissions push it past.

### 5. `AGENT_FETCH` and `AGENT_PAGE` (`process R4`–`R7`, `R9`; `railway R3`)

| type | name | direction | payload | reply |
| ---: | --- | --- | --- | --- |
| 23 | `AGENT_FETCH` | client → core | `req, session, before, max_bytes` (protocol 4) | `AGENT_PAGE`… or `ERROR` |
| 43 | `AGENT_PAGE` | core → client | `req, session, events: [object], last?: { more }` (protocol 4) | — |
| 6 | `ATTACH.page?` | client → core | the page budget in bytes; omitted when unset | — |

- **The reply** is one or more `AGENT_PAGE` frames for the `req`, each holding whole events under the frame limit (`client::pack_page`, as `pack_transcript` packs a replay; a 64 KiB `Appended` is the largest event); the last frame carries `last: { more }`, the others leave it out. An empty page is one frame with no events and `last`. In Rust `last` is `Option<PageEnd { more: bool }>`. The frame with `last` is the request's answer on Brindle's connection: it ends the request's wait as `OK` does for the other agent requests.
- **The budget.** `max_bytes` and `ATTACH.page` are clamped by the core to `PAGE_BYTES` (`policy::page_budget(asked: Option<u32>) -> usize`; absent means `PAGE_BYTES`). Brindle omits `page` and fetches with `PAGE_BYTES`. A reconnect's replays (no `ATTACH`) use `PAGE_BYTES`. A pending replay keeps its budget (`PendingReplay.page`), so an attach that waits for an adoption or a meta store gets the page it asked for.
- **Refusals** (`policy::agent_fetch_refusal`, exhaustive), each an `ERROR` with `req` and `session`: a client below protocol 4 (`agent_request_refusal`, with `AgentRequest::Fetch` beside the other agent requests); no such session; a terminal session ("session N isn't an agent session"); a client that doesn't have it attached ("session N isn't attached to this client", the other agent messages' text); a client still waiting for that session's replay ("session N's replay hasn't ended"), which also covers a session still being adopted, since its replay waits for the rebuild.
- **Who may fetch.** Only the attached client, as for prompts; a fetch changes nothing on the server, so it needs no checkpoint, no reducer step and no ordering with the holder.
- **Every phase.** A fetch reads the core's log, never the agent, so it is answered whatever the agent's state: starting, ready, needing or running a login, restarting after a terminal login, failed, exited, or lost with its holder. A session kept after its agent ended (`Life::AgentEnded`, `agents-session-recovery`) keeps its log in the core, so fetches work on it unchanged, before and after a Continue. A session adopted as "could not be restored" holds one notice and is never paged.
- **Queues (`process R9`).** A reply goes through the client's existing queue and marks (1 MiB / 256 KiB) like a replay. Its size is one budget, or one entry or the pending permissions when they alone pass it, so at most the 16 MiB log, as a replay is today. Brindle keeps at most one fetch in flight per zone.
- **The relay** copies frames by header and needs no change.
- **Transcripts** (`remote-protocol/tests/transcripts.rs`): `client-agent-attach-paged` (`ATTACH` with `page`, a replay with `Trimmed` and `Window`, then live), `client-agent-fetch` (two fetches, a multi-frame reply, the last with `more: false`, an empty page), `client-agent-fetch-refused` (each refusal). `every_message_variant_and_form_appears_in_a_transcript` gains `AgentFetch`, `AgentPage` and both `last` forms. The event form is pinned by `transcript/tests/fixtures/v4/window.json`. `fuzz.rs` covers the new decoders.

### 6. The bound keeps 16 MiB and trims to a low mark (`process R9`)

`LOG_LIMIT` stays 16 MiB. `Step::trim` now trims, when the log passes `LOG_LIMIT`, down to `LOG_TRIM_TO`, 12 MiB (`trim_to(LOG_TRIM_TO)`), instead of to `LOG_LIMIT`; what it does for the dropped entries (forgetting their tools and run, answering a dropped pending permission `cancelled`) is unchanged.

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

- **The base's state** is format 1's state object, unchanged: the log as `replay()` events (the conversation and its title among them) plus the ACP version, `cwd`, `version`, `phase` (starts, muted counts, logins and restarts included), `methods`, `rpc`, `run`, `tools`, `bad_lines`, `lines`.
- **A segment's state** has the same fields except `log`, which becomes `events`: the events the session applied since the previous record, in order. Every other field is whole and replaces the previous record's, so a segment written during a login or a muted load carries that phase as it stands. Consecutive `Appended`s for one entry are merged, up to `TEXT_CHUNK` each, before writing (refresh, decision 16): they fold to the same text, and a streamed reply's token-sized chunks would otherwise cost an event's framing apiece.
- **Reading** folds the base's log, then each segment's `events` in order onto it, and takes the other fields and the partial line from the last record. Then everything format 1 checks is checked on the result (entries the run, tools and waiting permissions name, `Rpc` parts, the partial).
- **New ways not to read** (`Unreadable`, each with its text after "the conversation could not be restored: "): no base first (`NoBase`), a record kind other than `B` or `S` (`RecordKind { kind }`), a sequence number out of order (`Sequence { expected, found }`), a record running past the end (`Truncated`, reused), a segment's events that don't decode or fold (`Log`, reused). The adoption is `not_restored`'s, as today.
- **Format 1 is still read**, so a holder storing a checkpoint from a build before this change (only pre-release builds wrote format 1) adopts. `encode` writes only format 2. `Unreadable::Format`'s text names both formats this core reads.
- **API:** `encode(session) -> Result<Vec<u8>, TooLarge>` writes the header and a base; `encode_segment(session, events: &[Event], seq: u32) -> Result<Vec<u8>, TooLarge>` writes one record with no header; `decode` and `adopt` read both formats. `TooLarge` keeps its meaning for the whole blob; a segment past the limit makes the core write a whole checkpoint (decision 9).
- *Rejected:* a segment holding the entries added since the last checkpoint (instead of the events), which can't express a change to an older entry (a tool call finishing, an answer) without restating it, and segments for the log alone with the rest kept in the base, which would rewrite the base for every `Rpc` change.

### 8. `CHECKPOINT_APPEND` in H1 3 (`process R1`, `R3`, `R7`)

| type | name | direction | payload |
| ---: | --- | --- | --- |
| 16 | `CHECKPOINT_APPEND` | core → holder | session, `len: u64 BE` (version 3) |

It opens a blob as `CHECKPOINT_BEGIN` does, of the same kind (so `MAX_OPEN_BLOBS` and "one open blob per session" cover both), followed by `CHECKPOINT_CHUNK`s and `CHECKPOINT_END { covers }`. On `END` the holder, instead of replacing the stored checkpoint, appends the bytes to it and covers the tail as for a whole one. It refuses, with an `ERROR` for the session, keeping the stored checkpoint and the tail as they were (as for a whole checkpoint it drops):
- an append to a session whose stored checkpoint is empty ("nothing to append to"), so no blob without a header can be stored;
- an append that would make the stored checkpoint pass `MAX_CHECKPOINT_LEN` (64 MiB).

Both are length checks: the holder still parses nothing and holds no policy (`process R1`). It is one message type, one flag on the open blob and one `extend`, handled under the holder's per-message `catch_unwind` as `CHECKPOINT_BEGIN` is (`Unit::Checkpoint`, `process R3`). On a connection below H1 3 it is refused as `SPAWN_PIPES` is. Adopting is unchanged: `WELCOME` is followed by the stored checkpoint, base and segments as one blob, and the tail. `ToHolder::checkpoint_append(id, bytes, covers)` builds the messages as `ToHolder::checkpoint` does.

**Why in the holder at all:** the alternative is for the core to keep sending whole checkpoints (decision 9's cost) or to make the holder understand records, which `process R1` forbids. Folding it into H1 3 now spares a later H1 4 and a release of holders that can't append. Transcripts: `holder-checkpoint-append` (a whole checkpoint, two appends, an adoption returning the concatenation), `holder-checkpoint-append-refused` (onto empty, past 64 MiB). `fuzz.rs` covers the decoder.

### 9. When the core appends and when it rewrites (pure policy, `fcis R1`; shell in `core/agents.rs`)

The core keeps, on each agent session's `Agent` beside `checkpointed`, what its holder stores and what changed since:

```rust
enum Stored {
    Unknown,                                        // nothing written to this holder session yet, adopted, or the holder refused one
    Known { base: u64, appended: u64, seq: u32 },   // bytes of the base, of the segments, the last seq
}
enum CheckpointForm { Whole, Append }
```

and `unsaved: Vec<Event>`, every event its steps applied since the last checkpoint (`Core::agent_apply` pushes each `Output`'s events, the trim's `Trimmed` among them; a reducer test pins that the log changes only through them). `policy::agent_checkpoint_form(stored, at, segment_len, trimmed, holder) -> CheckpointForm`, where `at` is the existing `AgentCheckpoint` and `holder` the holder's `policy::Adoption`, is `Whole` when:
- `stored` is `Unknown`: a new session, an adopted one, a restarted one (below), or after the holder sent an `ERROR` naming the session (any error: the core can't tell which message it answers, and a whole checkpoint is always safe);
- `at` is `StepDown` (the next core adopts one base, compact);
- `unsaved` holds a `Trimmed` (decision 6);
- `appended + segment_len > base` (the segments would outgrow the base);
- `base + appended + segment_len > MAX_CHECKPOINT_LEN`, or `encode_segment` returned `TooLarge`;
- the holder is `Adoption::Drain`, of another build (decision 10);

and `Append` otherwise. `agent_checkpoint_due` (when, with its seven moments, step-down and detach) is unchanged; this decides how. A whole checkpoint sets `Known { base: len, appended: 0, seq: 0 }`; an append adds its length and bumps `seq`; both clear `unsaved`.

**What starts again at `Unknown`** (refresh, decision 16). Every `Agent::restarted` (an `AGENT_RESTART` of an ended agent, and a login restart's start after its agent's exit) runs the agent in a new holder session, possibly on another holder, whose stored checkpoint is empty: `restarted` sets `Stored::Unknown` and clears `unsaved`, so the checkpoint written right after its `SPAWN_PIPES` is whole, as a continuing session's first one is. A session kept after its agent ended has no holder and is never checkpointed; its few events (the exit, a notice) are dropped from `unsaved` at the restart, which writes whole anyway. An `unstorable` session keeps no `unsaved` events, since it is never checkpointed again.

**Loads.** A kept log's muted load adds one notice, so its segment is small. A fresh log's shown load (`SPAWN_AGENT` with a `conversation`) can add a whole conversation before the agent is ready: its checkpoints past 512 KiB of stdout and at `Started` carry those entries as segments until they outgrow the base, then the next is whole. No rule for loads is needed beyond decision 9's.

**What it costs.** Between two whole checkpoints of base size B, appends write at most B, and the next whole one is at least as large as the base it replaces grew to. So bytes written per byte of log growth are bounded by a small constant (about 2 below the cap, about 4 at it with decision 6's low mark), against today's copy of the whole log at every moment, which at 16 MiB and several moments per turn is hundreds of times the turn's output. A pure test pins it: driving the policy and encoders through a synthetic 64 MiB conversation (moments and 512 KiB stdout steps), total checkpoint bytes stay under 4× the log bytes added, plus one base per trim.

**What stays in memory only (`process R2`).** `Stored` and `unsaved` live in the core. A core restart loses both, and loses nothing: the new core adopts the session from the holder's base, segments and tail, starts at `Unknown`, so its first checkpoint is whole, and starts `unsaved` empty, since everything before the adoption is in what the holder stores.

### 10. What survives what (`process R2`, `R3`, `R12`)

| Event | What happens |
| --- | --- |
| core crash or `SIGKILL` | The holder stores the last whole checkpoint plus the segments it accepted. The next core decodes them (decision 7), feeds the tail as today, and writes a whole checkpoint first. A segment the holder refused leaves a sequence gap only if the core sent another append before it read the `ERROR`: the decoder then refuses the checkpoint (`Sequence`) and the session is adopted as "could not be restored". Holder errors on checkpoints are bugs in the core, so this window is not expected to open. |
| upgrade (`STEP_DOWN`) | The old core writes every agent session's checkpoint whole, then hands over; the new core reads format 2. |
| an agent restarted (Continue, or after a terminal login) | A new holder session: its first checkpoint is whole (decision 9). The log, its ids and the conversation with its title carry on in the core as today. |
| Brindle crash or quit | Nothing changes on the server. The zone's fetched pages are gone; the next attach or restore sends the newest page again. |
| holder crash | Terminal sessions and agents that can't continue are lost, as today; an agent that can continue is kept as lost with its holder (`agents-session-recovery`), its log in the core and fetchable. |
| an older core adopting a newer holder | Only a pre-release core (format 1 only) could meet a format 2 checkpoint: it adopts the session as "could not be restored" (`Unreadable::Format`), and the agent keeps running. A released core (H1 ≤ 2) is never shown pipe sessions. |
| a holder of another build (draining, Windows per-build copies included) | Every checkpoint of its agent sessions is whole. A released holder (H1 2) never holds one; a pre-release holder at H1 3 may lack `CHECKPOINT_APPEND`, and the build rule means the core never needs to know. Such a holder runs from its own copy until its sessions end, as today. A restart never goes to it: restarts are placed on the core's own build. |
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

set from the transcript's window on every absorb, with `Loading` and `Failed` kept until the page or the reply arrives. The fetch is also a `Sent::Fetch` in the model's pending requests, so `replied` routes its outcome as it routes a prompt's. `fetch_due(row_in_view, was_in_view) -> Option<Fetch { before: EntryId, max_bytes: usize }>` decides from the model's own `Earlier` and link: `Available` asks whenever the row is in view; `Failed` asks only when the row comes into view again (`was_in_view` false) or on a press (`retry()`); nothing is asked while `Loading`, while the link is `Reconnecting`, or with no window. `before` is the window's `from`. The entity sends `AGENT_FETCH` through `AgentLink::fetch` and calls `fetching(request)`; `Err(why)` from the reply makes it `Failed`; the page itself arrives as a change.

**The fold.** The sink buffers a reply's `AGENT_PAGE` frames by `req` (`SessionSink::page(req, events, last)`) and, at `last`, calls `AgentShared::prepend`, which runs `Transcript::prepend` and records `Changes::prepended: Option<usize>` (entries added in front), then the connection sends the request's `Reply { Ok }`. A `FoldError` from `prepend` breaks the session as any fold error does (agent-zones "A session Brindle can no longer show"). `REPLAY_START` resets the fold, the window and `Earlier` (a reconnect replays the newest page again, dropping fetched pages, as a reset drops every block today), and drops a buffered page; a late reply for a request the model no longer waits for is dropped. A prepend's changes are marked `from_replay`: a fetched page is history, so it raises no notification and doesn't reset `seen` (agent-zones "Replays raise nothing").

**The prepend in `absorb`.** `prepended` copies the new entries to the front of `entries`, renders each agent message and thought once into the Markdown cache (`keep_render`, as a reset does, with its panic guard), draws their blocks with each thought's open state (`toggled`, else closed; `auto_open` only ever names the newest entry), and emits one `Splice { range: 0..0, count }`. Then `finish` recomputes the working row and the title as after any change: the conversation's title is in every page (decision 3), so the zone is titled correctly from the first page; a fetched tool call in progress can become the working row's text (decision 13).

**The splice and the view.** `MessageScroller` draws the Earlier row in its list's top padding rather than as an item, so the list's items stay the blocks then the working row while it shows (`working_splice` keeps adding and removing at the end), and GPUI's `ListState::splice` of `0..0` moves the scroll position past the inserted items (it shifts `item_ix` when the range ends at or before it): every block in view stays where it was, and the Earlier row scrolls up out of view with the new content. While the zone follows its end (`Follow::AtEnd`, bottom-aligned), inserting above changes nothing in view either. The scroller reports whether its top padding is in view through a callback the zone registers (`process R14`: computed from the layout it already did, no work in `render`). `scroll_page`'s "hidden above" counts the top row, so shift-pageup at the first block's top scrolls the row into view instead of doing nothing. When the window goes, the padding goes. **Spike first** (task 5.1): a GPUI `VisualTestContext` test that a block's bounds are unchanged after inserting items above it, with the padding row in view and out of it and with a working row as the last item. If GPUI moves the view, the fallback is the Earlier row as item 0 with the view re-anchored on the first block, which may shift by up to the row's height.

**The row** shows "Earlier messages" with a `brindle-ui` `Loader` (the stepped `loader-circle` spinner `agents-zone-attention` added) while `Loading`, and "Earlier messages couldn't be loaded: {why}" while `Failed`, pressable to retry. It is drawn inside the zone, so in the zone's token set (`agents-zone-theme-scope`) like the rest of the conversation. It takes no focus: the keyboard path is shift-pageup, which scrolls it into view, and that fetches. **Motion:** the `Agent` entity wants the window's motion clock while `Earlier` is `Loading`, as it does while its agent works, and stops wanting it when the page or the reply comes. That widens `agents-zone-attention`'s requirement "Motion only while an agent works" (decision 16; task 8.3).

**Failures shown where they happen (`railway R6`).** A refused fetch and a link that goes down mid-fetch (the connection fails a request written on a link that ended, with `Err`) show on the row; nothing else in the zone changes. `src/remote/local/session.rs` answers `AGENT_FETCH` with `Err("the connection to the terminal server is down")` while the link is down instead of letting the writer hold it as it holds prompts, since a page asked of a connection that then replays would not fit the new fold.

**Memory.** Fetched pages are kept until the zone closes or replays; the zone's two copies (fold and model) are bounded by the server's 16 MiB as before, and in the common case hold one page. Rendering a fetched page's Markdown costs one parse of at most a page's entries (or one oversized entry), once, on the UI thread inside `absorb`, far less than today's attach of a whole log, which renders every entry. A session continued into a fresh log with a shown load (`agents-session-recovery`) is followed live from its start, so its zone holds the whole loaded log with no window until it is next attached.

### 12. The dump (`testing R2`; `command-line`)

The `--- conversation` line becomes `--- conversation entries=N at=end|away earlier=N|loading|none[ fetch-error="…"]`. `earlier` goes after `at` so that `wait_text` (which drops `at=`) reads `entries=N earlier=…`, and every existing wait on `conversation entries=N` (`agent-working`, `agent-zone-theme`, `agent-split`) and dump check for `conversation entries=N at=end` or `at=away` (`agent-scroll`) keeps matching, since both are prefixes of the new line. `earlier=N` is `Available` or `Failed`'s count, `loading` is `Loading`, `none` is no window; `fetch-error` appears only while `Failed`, escaped as the other quoted fields are. `--- entry` lines (and their `--- block` and `--- link` lines) list only the entries the zone holds, so their ids show where the page starts. The `--- working` line (`agents-zone-attention`) is unchanged; it reads from the held entries (decision 13). CLAUDE.md's dump paragraph gets the new form (task 5.6).

### 13. The built agent stack

- **`agents-markdown`:** its parts split within an entry; pages split between whole entries, so they never meet. A message bigger than the page is sent whole and still cut into parts by its renderer. The render cache is keyed by entry id, so a prepend only adds to it (decision 11); a trim drops from it as today. Its `--- block` and `--- link` lines sit inside the entry lines, before `--- conversation`.
- **`agents-session-recovery`:** kept logs (`Life::AgentEnded`) stay in the core and are fetchable (decision 5); muted loads add one notice and shown loads are paged like any log, server-side (decision 9). A restart runs in a new holder session whose first checkpoint is whole. `AGENT_RESTART` (21) and `AGENT_FETCH` (23) are independent messages of protocol 4. The `continue=` word and Continue button read the transcript's conversation and agent state, which every page carries.
- **`agents-login`:** login methods and states are agent state, carried by every page's `Agent` event, so the login row shows from the first page. A fetch is answered in every login phase (decision 5). A login restart is a restart for the checkpoint (decision 9). `AGENT_LOGIN` (22) fixed the number before `AGENT_FETCH`.
- **`agents-session-title`:** `Conversation.title` is state, not log: it is in every page, so a zone and its tab are titled correctly however little of the log was sent; a title change is a live `Conversation` event, untouched by the window; a checkpoint's segment carries a retitle as an event.
- **`agents-zone-attention`:** the working row reads the zone's held entries for its tool call in progress. A turn whose in-progress tool call has more than a page of entries after it shows `Working…` until the call is fetched, then its title; the page isn't widened for it (*rejected*: lowering `from` to the running turn's newest in-progress tool call, which would tie the server's page to a view rule). Likewise `copy_last_agent_message` and `copy_last_code_block` act on the held entries, as the spec's "in the zone" already reads. The Earlier row's spinner is a `Loader` on the motion clock (decision 11). Fetched permissions that were answered long ago and finished turns raise no notification (decision 11).
- **`agents-zone-theme-scope`, `agents-markdown-layout-fixes`:** the Earlier row is drawn in the zone's tokens and at the conversation's width, as every other row; nothing else meets.
- **Archive order:** the stack archives `agents-session-title` and `agents-zone-attention` before this change. This change's command-line delta modifies "Debug hooks in an agent zone", which `agents-zone-attention` also modifies, so it carries that change's wording (the `--- working` line, the option order, its scenarios) beside its own. Task 8.3 re-checks each MODIFIED block against the synced main specs before archiving, and adds the MODIFIED "Motion only while an agent works" once that requirement is in the main spec.
- **Proposal-only changes that meet this one:** `agents-plans` sends its plans on every attach after the agent's state (decision 3's "everything that isn't an entry is in every page" covers it); `agents-commands` numbers `AGENT_SET_OPTION` after `AGENT_FETCH`; `agents-prompt-context` pages user entries whole; `agents-acp-v2` changes the reducer, not the log or the wire.

### 14. Tests (`testing R1`, `R7`, `R8`, `R13`, `R14`, `R16`, `R17`)

Each behaviour in the layer that answers it:
- **Unit and property (pure):** the page, fetch and prepend laws and the fold below the window, the empty page without a window (`brindle-transcript`); format 2 round trips (logins, muted loads and titles among them), every new `Unreadable`, format 1 still read, the merged `Appended`s, a pinned format 2 byte string (`acp/checkpoint.rs`); the low mark (`reduce.rs`); `agent_checkpoint_form` for each `Whole` reason, the page budget's clamp, the fetch refusals, and the write-cost bound over a synthetic 64 MiB conversation (`policy.rs`); `Earlier`, `fetch_due`, the prepend's renders, splice and `from_replay`, the working row with a tool call above the page, and the dump line (`src/agent/model.rs`); the sink's frame buffering and the shared fold's `prepended` (`sink.rs`, `shared.rs`); the client session's refusal while the link is down (`session.rs`).
- **Component:** the scroller keeps a block's bounds when items are inserted above, and a page up reaches the top row (`message_scroller.rs`, `VisualTestContext`).
- **Transcripts and fuzzing:** decisions 5 and 8.
- **Real role tests** (`remote-agent/tests/`, with the fake ACP agent): a paged attach of a long log, fetched back to the start equal to the whole log, a refused fetch from another client, a tool call updated outside the page then fetched current, and a fetch on an ended session kept for its conversation (`agents.rs`); appends then `SIGKILL` and reconnect, and a restart whose first checkpoint is whole (`agents.rs`); a step-down writing whole and the new core carrying on (`upgrade.rs`); the holder's append, its refusals, an append on an H1 2 connection, and adoption returning base plus segments, on the Linux and Windows CI jobs (`holder.rs`, `testing R17`); a draining holder of another build getting only whole checkpoints (`core.rs`).
- **e2e** (`tests/e2e/cases/agent.rs`, fake agent, `--wait-for`, gated on files, no sleeps): `agent-earlier-page`, `agent-earlier-scroll`, `agent-earlier-reattach`.
- **Perf:** a perf-suite case `agent-attach-long` (`testing R14`) in `tests/perf/agent.rs` beside `agent-markdown-stream`: time from launch to the first page of a 16 MiB conversation shown, against a baseline taken on the build before this change.
- **Manual:** wheel scrolling to the top of a long real conversation (`testing R16`).

### 15. Docs

`CLAUDE.md` (the dump paragraph; the manual check), `docs/code-map.md` (`brindle-transcript`'s page and window; the core's paged replays, fetch, `Stored` and appends; the holder's append; the zone's Earlier row; the scroller's top row in the `brindle-ui` entry), `docs/architecture/process-architecture.md` ("How it applies here": what the holder stores is a checkpoint and segments appended to it), `README.md`'s agent zone section, and the module docs of `remote-protocol/src/client.rs` and `holder.rs` (their tables and the protocol 4 and H1 3 paragraphs) and of `acp/checkpoint.rs` (the format table).

### 16. Choices made beyond the user's answers (flagged)

From the design (2026-11-25), each still fitting the built code:
1. **Trimming to a 12 MiB low mark** (decision 6), so "compact on a trim" doesn't rewrite every checkpoint of a full log.
2. **Appends only to a holder of the core's own build** (decision 10): stricter than "H1 3", so a pre-release H1 3 holder without `CHECKPOINT_APPEND` is never sent one, with no probe. Expressed through the existing `policy::Adoption` rather than a new type.
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

From the refresh (2026-11-26), against the built stack:

13. **The numbers are fixed:** `AGENT_FETCH` 23, `AGENT_PAGE` 43, `CHECKPOINT_APPEND` H1 16 (decision 2).
14. **An empty page with `more: false` on a transcript with no window is accepted** (decision 4). The first design refused it as `NoWindow`, which would have broken the session when a fetch races a trim that took the window. Recommend: accept, as written.
15. **Every page carries the conversation with its title** in `replay()`'s place, between `Queue` and `Turn` (decision 3). This follows from `replay()` as built, not a new choice; listed because the first design's page omitted `Conversation`.
16. **The working row and the copy actions read only the held entries** (decision 13): a tool call in progress above the page shows `Working…` until fetched. Recommend: accept; widening the page for it would tie the server to a view rule, and it needs more than a page of output inside one tool call.
17. **The Earlier row's spinner runs the window's motion clock while a fetch waits** (decision 11), which widens `agents-zone-attention`'s "Motion only while an agent works" to "or loading earlier messages". Recommend: accept, and add the MODIFIED requirement once that change is archived (task 8.3). The alternative, a still `loader-circle` icon, avoids the spec change but shows no progress.
18. **A fetched page raises nothing** (`from_replay`, decision 11), and agent-zones "Replays raise nothing" and session-restore "Restored agent zones get their sessions back" now say the newest page rather than the whole conversation. Recommend: accept; both requirements said "whole conversation", which paging changes.
19. **A restart resets `Stored` to `Unknown`** (decision 9), so its first checkpoint in the new holder session is whole; an unstorable or ended session keeps no unsaved events.
20. **A segment merges consecutive `Appended`s of one entry** (decision 7). Recommend: accept; it costs a loop in the encoder and keeps a streamed reply's segment near the reply's size.
21. **`AGENT_FETCH` joins `policy::AgentRequest`** for the protocol 4 refusal, and a session being adopted is refused as "replay hasn't ended" (decision 5).

From the implementation (2026-11-26):

22. **A `Window` with `earlier: 0` is refused** (`FoldError::WindowEmpty`, "a page's window at entry N has no earlier entries"), beside decision 4's errors. A window over nothing would move `next` with no record of why, and the replay round trip would then differ; the server never sends one (`replay_from` writes a window only when entries are earlier). Recommend: accept.
23. **The holder refuses `CHECKPOINT_APPEND` below H1 3 by an explicit version check** (`holder::policy::append_open_refusal`, "CHECKPOINT_APPEND needs H1 version 3"). Decision 8 said "as `SPAWN_PIPES` is", but the holder doesn't refuse `SPAWN_PIPES` by version: the core simply never sends it below 3 (`placement`). The append is a storage change, so the holder checks it itself. Recommend: accept.
24. **The holder transcripts run two connections**: `holder-checkpoint-append` ends with a `GOODBYE` and a second connection whose adoption returns the concatenation, as decision 8's "an adoption returning their concatenation" needs.
25. **`policy::agent_appendable` is split out of `agent_checkpoint_form`**, so the core encodes a segment only when it could be appended (a step-down, a trim, an unknown store or a draining holder skip the encoding); `agent_checkpoint_form` takes the segment's length as `Option<u64>`, `None` for none encoded or too large. Same rules as decision 9.
26. **A holder that hasn't welcomed the core counts as another build's** (`Holder::adoption` gives `Drain`), so its agent checkpoints are whole; the core places spawns only on welcomed holders, so this is unreachable in practice.
27. **`Content::Agent` is boxed** (`Box<Agent>`): `Stored` and `unsaved` made the agent variant large enough for clippy's `large_enum_variant`, and the terminal's `Screen` was boxed for the same reason.
28. **How the real roles observe paging's server side** (superseding the first implementation's "left to pure tests"): a checkpoint's form is read from what the holder stores, by killing (or stepping down) the core and adopting the holder with the test core, whose blob the test splits into records without decoding them (`support::checkpoint_records`); a fetch refused while the replay is pending is an `ATTACH` that changes the session's owner and an `AGENT_FETCH` written in one write, so the core reads both before the flush that lets the replay go (`TestClient::attach_agent_and_fetch`). The draining holder's case is in `upgrade.rs` rather than `core.rs`, beside the other-build copy it needs.

From the implementation of the zone (2026-11-26):

29. **The spike (task 5.1) took the fallback: the Earlier row is the scroller's first item.** GPUI 0.2.2's `list` draws its top padding above whichever item is at its scroll position (`layout_items` starts at `padding.top` from the scroll-top item, and `scroll_to(item 10)` leaves a padding-sized gap above item 10 with item 9 undrawn), so a row in the padding would show above any item scrolled to the top. The row is the list's item 0, owned by `MessageScroller` (`with_top_row`, `show_top_row`, `refresh_top_row`, `top_row_in_view`, `on_top_row_view`); the owner's indices and splices don't count it. Items spliced in at the owner's start keep the view exactly, following the end or away from it, unless the row itself is the first item in view: then the item that was first goes to the view's top, moving up by what showed of the row (at most its height). Tested in `message_scroller.rs` and the gallery's `message-scroller-page-up-reaches-the-top-row`.
30. **The Earlier row asks again from `sync` while it is in view, besides the scroller's callback.** The callback fires only when the row's visibility changes, so a page that leaves the row in view (a short page, or a reconnect's replay) would otherwise never ask for the next one; `fetch_due` with `was_in_view = true` asks only while `Available`, so a failure still waits for the row to come back or a press. A fetch that can't be sent (no link) fails at once with the unsent reason, like a prompt.
31. **The `agent-earlier-*` e2e cases attach in a second run.** A zone that ran the turn live holds every entry (the window comes only with an attach's replay), so each case first runs the turn live and leaves it detached (`keep_detached`, its tab recorded), then restores it. The long conversation's bytes are in its thoughts, which a closed thought draws as one row, and its messages are short, so a fixed number of shift-pageups (60, at a 0.2 s step interval) reaches the start; the reply needs no gate, since it follows the prompt the case sends. `agent-earlier-reattach` checks run two's count by waiting for `earlier=` the first attach's count before scrolling, rather than comparing two dumps of one run.
32. **`agent-attach-long` uses about 15 MiB, not 16.** The server trims an agent's log past 16 MiB (`LOG_LIMIT`), so a conversation of exactly 16 MiB would be trimmed before the attach; 1,900 thoughts of 8 KiB stay under it with their encoding.

## Risks / Trade-offs

- [A release is cut before this change lands, freezing protocol 4 and H1 3 without paging] → hold the release PR until this change merges; if it happens anyway, this change moves to protocol 5 and H1 4 (decision 2).
- [GPUI moves the view when items are inserted above] → the spike comes first (task 5.1); the fallback shifts by at most one row.
- [A fetch races a trim] → both go through the core's one ordered client queue; `prepend` refuses a gap, and a page after a trim that took the window is empty with `more: false`, which folds as nothing (decision 4); the property tests cover trims between fetches.
- [A huge single entry or many pending permissions make a page large] → bounded by the 16 MiB log, as a replay is today, and sent through the client queue's marks.
- [Dropping up to 4 MiB more history at a time] → decision 6; confirm or fall back to trims inside segments.
- [A segment lost between a holder's refusal and the core reading it] → the sequence number turns it into an honest "could not be restored" instead of a wrong log; holder refusals of checkpoints are core bugs.
- [Covers after a dropped checkpoint] → appends behave exactly as whole checkpoints do when the holder drops one (stored checkpoint and tail kept); the accounting after a dropped checkpoint is today's and out of scope here.
- [Pre-release builds in daily use] → decision 10 lists what they see; none loses a session, a long conversation in an old Brindle needs the newer one to show.
- [Two copies in the zone of a fully fetched long log] → still bounded by 16 MiB each, as before; paging only makes the common case smaller.
- [The working row says `Working…` for a tool call above the page] → decision 13; rare, and truthful.
- [The motion requirement lands in main after this change's delta is written] → task 8.3 adds it before the archive.

## Migration Plan

Additive for every released version: clients on 1–3 and holders on H1 1–2 see nothing new. Within the one release that brings agent sessions, protocol 4 and H1 3 carry paging and appends from the start. A newer helper upgrades the running core (`process R12`); the old core writes whole checkpoints at step-down; the new core appends only to its own build's holders and reads checkpoints of format 1 and 2. **The release PR must wait until this change is merged** (the other agent changes, and the open stack, merge before it). Rollback to a build before this change: its core reads this change's format 2 checkpoints as unreadable, so agent sessions adopted by it show "the conversation could not be restored" and keep running; terminal sessions are unaffected. No config or state file changes.

## Open Questions

None that change the specs, the approach or the tasks. Decision 16 lists the choices made beyond the user's answers for confirmation: the first (the 12 MiB low mark) is the one most worth a look, then 17 (the motion clock while loading) and 16 (the working row above the page).
