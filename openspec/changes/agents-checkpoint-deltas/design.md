# Design

## Context

See proposal.md for why. The state this design starts from, at `5f335dc`:

- **Checkpoint format 2** (`remote-agent/src/acp/checkpoint.rs`, `agents-log-paging` decision 7) is a header, a base record and segment records. `encode_within` calls `state_fields` (every field but the log, built with `json!`), adds `log` as `transcript.replay().iter().map(Event::to_value)`, and `push_record` turns the `Value` into a `String` and copies it into the record buffer. `encode_segment(session, events, seq)` writes `merged(events)` as `events` beside the same `state_fields`, which is "whole and replaces the previous record's". `decode_2` folds the base's `log` and each segment's `events`, and `restore` takes every other field from the last record: `tools`, `outputs`, `methods`, `rpc`, `run`, `lines` and the partial line's bytes.
- **The core** (`server/core/agents.rs`) keeps `Stored` (`policy.rs`: `Unknown`, or `Known { base, appended, seq }`) and `unsaved: Vec<Event>` per `Agent`. `agent_checkpoint` asks `policy::agent_appendable`, encodes a segment, asks `policy::agent_checkpoint_form`, and sends `ToHolder::checkpoint` or `checkpoint_append`. `agent_checkpoint_due` checkpoints at every reducer `Moment`, at detach and step-down, and past `CHECKPOINT_AFTER` (512 KiB) of stdout. `TAIL_LIMIT` (1 MiB) in the holder makes that last rule necessary.
- **The reducer** (`acp/reduce.rs`) inserts into `tools` only in `add_tool`, together with an `Entry::Tool { call, .. }` whose `call` is the same key. It removes from `tools` only in `trim` (`retain(entry > through)`, with a `Trimmed` event) and in `respawn` (`clear()`, with no event).
- **`Lines`** (`acp/lines.rs`) holds `Partial::Collecting(bytes)`, `TooLong { len }` or `Resyncing { len }`, and `covers`, every byte fed. A `Collecting` partial only grows until its newline, and its line started at stream offset `covers - len`.
- **The decision 9 test** `checkpoints_of_a_64_mib_conversation_write_a_small_multiple_of_what_it_added` (`policy.rs`) drives text-only turns. Each checkpoint there adds 64 KiB or more of events, so the fixed per-record state never shows.

### Findings, traced

- **B-3 / C-2 (tools, methods, outputs):** confirmed. `state_fields` writes `tools` (every kept tool entry), `methods` and `outputs` into every segment. The estimate in proposal.md (about 80·T² bytes written) comes from a base of about 440·T bytes (log and `tools`) and segments of about 40·T bytes each, with a whole rewrite once the segments outgrow the base, so about 160·T bytes are written per tool call. Task 1.2 measures the exact figure before any fix.
- **B-4 / C-2 (partial line):** confirmed. `state_fields` returns the whole `Collecting` bytes for every record. 0.5 + 1 + … + 8 MiB is 68 MiB for one 8 MiB line, before the rewrites the size rule forces.
- **Observation 6:** confirmed, and one copy more than counted. A whole checkpoint passes through `replay()`'s cloned events, `Event::to_value`'s `Value` tree, `state.to_string()` and the record buffer `out`. After that come `holder::split`'s per-chunk `Data { bytes: chunk.to_vec() }`, each frame's payload (`wire::session_bytes` copies again), and `Conn::queue`'s copy into the connection's output. `pack_events` serializes each event with `to_vec` to measure it, and `ToClient::encode` serializes it again.
- **A-4:** confirmed. `Output::append` does `push_str` then `drain(..cut).collect::<String>()`, which memmoves the kept text and allocates the dropped prefix on every append past `ITEM_LIMIT`.
- **A-5:** confirmed. `tool_content` runs `diff::hunks` for every `ToolItemForm::RawDiff` before comparing with the entry's items. `permission` clones `request.call` to apply it.
- **B-5:** confirmed. `split`, `chunk`, `tool_item`, `initialize_result`, `new_session_result`, `resume_result`, `load_result` and the location decode each clone a `Value` to reach `serde_json::from_value`. The reducer owns every `Message` it passes by reference.

## Goals / Non-Goals

**Goals:**
- A segment's size is bounded by its own events, plus the partial line's new bytes, plus a constant. Nothing in it grows with the conversation.
- `agents-log-paging` decision 9's bound holds for tool-heavy conversations and long lines, with the per-record constant stated, and a pure test asserts it.
- A whole checkpoint makes two copies of the log (its buffer and the connection's queue), not seven. A transcript event is serialized once.
- Output appends and repeated diffs cost amortized O(delta).
- No wire encoding changes. Format 2 is amended in place, and a checkpoint written before this change is not read (decision 1).

**Non-Goals:**
- The client's absorb cost and its copies of the log (change 7, `agents-absorb-cost`).
- B-2's bounded raw formatting and B-7's fold bounds (change 3, `agents-event-bounds`).
- Replacing the hand-written `restore_*` decoders with a serde mirror. This change adds a serialize mirror only where streaming needs it: the log and the events. The rest of `b.md`'s observation 1 stays with change 5's split.
- Salvaging part of a checkpoint that doesn't read (Open Questions, 1).

## Layer map

| Part | Kind | Crate, file |
| --- | --- | --- |
| Record shapes, `Written`, `encode`, `encode_segment`, `decode` and the delta fold | pure (`PURE_MODULES`) | `brindle-remote-agent`, `acp/checkpoint.rs` (after change 5: `checkpoint/{format,state}.rs`) |
| `tool_epoch`, the derived tool index, the diff memo, the permission's call moved | pure reducer | `acp/reduce.rs` (after change 5: `reduce/tools.rs`, `ToolBook`) |
| Owned `Dialect` arguments | pure codec | `acp/dialect.rs`, `acp/v1/decode.rs` |
| `Stored::Known` carrying `Written`, and the appendability rules | pure policy | `server/core/policy.rs` |
| Encoding events once, queueing sliced frames | shell (loop handlers) | `server/core/agents.rs`, `sessions.rs`, `holders.rs`, `clients.rs`, `server/conn.rs` |
| `checkpoint_frames` and `SlicedFrame`; `JsonObject`, `transcript_frames` and `page_frames` | shared wire, pure | `brindle-remote-protocol`, `holder.rs`, `client.rs` |
| `OutputText`, `Output::append`, `Transcript::replay_ref` | shared, pure | `brindle-transcript`, `lib.rs` (after change 5: `output.rs`, `page.rs`) |
| Readers of `Output::text` | app model and view helper | `src/agent/tool.rs`, `src/agent/model.rs` |

Every decision lives in a pure function or type (`fcis R1`). The handlers gather, call `checkpoint::encode*` and `policy::*`, and queue what comes back. No crate reference changes (`ports R1`, `R3`): `brindle-remote-protocol` learns nothing about transcripts. It packs opaque JSON objects, as it already carries them as opaque `Value`s.

## Decisions

### 1. Format 2 is amended in place (choice C4; `process R5`, `R15`)

The plan's choice C4 settles this. Format 2 hasn't shipped (v0.3.0 predates it), so `FORMAT` stays 2 and the header and record framing are unchanged. The record's JSON state changes to the **delta shape**: no `tools` field, a `tool_epoch` field in every record, and in segments the rules of decisions 2 to 5.

A record without `tool_epoch` is `Unreadable::Field { name: "tool_epoch" }`. A checkpoint written before this change is such a one: its records hold `tools` and every non-log field whole. A core appends only after it wrote the base itself: `Stored` starts `Unknown` on every adoption, so the first checkpoint after one is whole, and a holder of another build gets only whole checkpoints (`Adoption::Drain`).
**Checkpoints from before this change are not read** (decided 2026-11-27). No release wrote format 2's earlier shape or format 1 (v0.3.0 predates agents), so only holders of development builds hold them. A core with this change adopts such a session as not restored ("the conversation could not be restored: …"), its agent left running, as it does any unreadable checkpoint. The loss happens once, at the first reinstall across this change, to the agent conversations running then. `decode_1` and `FORMAT_1` go too: format 1's state is the earlier shape, so a format-1 checkpoint is refused as a format this server doesn't read.

*Rejected:*
- Format 3 with format 2 kept as is. It is clearer to read, but choice C4 is settled, and amending unreleased formats in place is the house practice.
- Reading the earlier shape: a branch on `tools`' presence, an epoch recovered from the stored index, and two more pinned byte strings. It would save the development holders' running conversations once, and then stay as code for a shape no release wrote.

### 2. The tool index is rebuilt from the log (`types R10`, `fcis R1`)

`AgentSession` gains `tool_epoch: EntryId`: the first entry id that belongs to the running agent process. `AgentSession::starting` sets it to `EntryId::FIRST`. `respawn` sets it to `self.s.transcript.next_id()` where it clears `tools`. After change 4 that happens in the one "agent gone" step, and after change 5 in `ToolBook::clear`, which takes the next id.

The invariant: `tools` equals `{ call → id | the log keeps Entry::Tool { call, .. } at id, id ≥ tool_epoch }`. Three facts make it hold:
- `add_tool` is the only insert, and it adds that entry.
- `tool()` finds an existing call before `add_tool` is reached, so within one epoch a call has at most one kept entry.
- `trim` and `respawn` are the only removals, and they remove exactly the dropped entries and everything before the epoch.

A new pure function `reduce::tool_index(transcript, epoch) -> BTreeMap<String, EntryId>` computes the right-hand side. A property test over random reducer steps (task 2.1) asserts the invariant after every step, so a later insert path can't break it silently.

So `tools` is never written. A delta-shaped record writes `tool_epoch` (a number), and `restore` sets `tools = tool_index(&transcript, epoch)` once, on the final log. One scan of the entries from `epoch` on takes the place of reading T map entries from every record. The "each `tools` value names a tool entry" check in `restore` holds by construction and stays as an assertion of the rebuilt map.

*Rejected:*
- **`tools_added` per segment, as B-3 proposed.** The additions follow from the segment's own `Added` events, so writing them repeats the events. A respawn clears the map with no event, so a delta would also need a "cleared" marker ordered among the events. The epoch says the same thing in one number, and the base needs no `tools` either.
- **Dropping the epoch and indexing every tool entry.** After a restart, a load's replayed `tool_call c1` would then update the earlier agent's entry `c1` instead of adding one. The spec scenario "Tool calls carried on after a core restart" pins today's behaviour.

### 3. The partial line is a delta (`process R9`'s spirit: bounded work per step)

A delta-shaped record's `lines` object gains an optional `from: u64`. The record's trailing bytes are the partial line's bytes from offset `from`, and `from` absent means 0.

- **Encoding.** `Written.line` (decision 6) holds `LineMark { epoch, start, len }` for the `Collecting` partial the last record left: the tool epoch, the stream offset where its line started (`covers - len`), and how many of its bytes are stored. `encode_segment` writes `from = last.len` and only `bytes[last.len..]` when four things hold: the session's partial is `Collecting`, its epoch equals `last.epoch`, `covers - bytes.len() == last.start`, and `bytes.len() >= last.len`. That is the same line in the same agent process, grown. Otherwise it writes `from` absent and the whole partial. A partial that ended (its newline came) and a new one started, a `TooLong` or `Resyncing` partial (no bytes), and a new process all reset it. A base always writes the whole partial.
- **Decoding.** `decode_2` keeps the partial's bytes and the `LineMark` of the record before. A record with `from = n > 0` must follow a collecting record that left exactly `n` bytes of a line starting at the same offset: `prev.start == covers - (n + new_len)`. The decoder then appends; anything else is `Unreadable::PartialGap { from: n, held: prev_len }` ("its checkpoint's partial line continues from byte {from}, and the record before it held {held}"). A record with no `from` replaces. The bytes are restored at the end through `restore_lines`, unchanged, which already turns a partial past `MAX_LINE` + 1 into `TooLong`.
- **Cost.** Each byte of a line is written once per record chain. Rewrites write it again only when the size rule forces a whole checkpoint, and decision 9's bound covers those. An 8 MiB line in 64 KiB reads is about 8 MiB of segments, against 68 MiB today.
- **The epoch in `LineMark`.** It stops a false continuation across a respawn, where `covers` starts again from 0. The core already makes a restart's first checkpoint whole (`Agent::restarted` sets `Stored::Unknown`), so this is a second guard; Risks names the one case it can't tell apart.

*Rejected:*
- **Skipping `Stepped(None)` checkpoints mid-line.** `TAIL_LIMIT` (1 MiB) then cuts the holder's tail during a long line, as B-4 says.
- **Keeping a copy of the written partial in the core to compare bytes.** That costs up to 8 MiB per session for what an offset and a length already prove.

### 4. Which fields a segment writes, by size

| Field | Bound | In a delta-shaped segment |
| --- | --- | --- |
| `events` | the step's events | always, merged as today |
| `lines` (`partial`, `covers`, `len`, `from`) and the partial's bytes | header < 100 B; bytes per decision 3 | always; bytes as a delta |
| `tool_epoch` | 8 B | always |
| `acp`, `version`, `bad_lines`, `run`, `lists`, `chose` | < 200 B together | always |
| `cwd` | a path, < 4 KiB | always |
| `phase` | < 1 KiB typical; a login's message is bounded at 64 KiB | always: it changes at most a few times a turn, and a login is short |
| `rpc` (`next_id`, `outgoing`, `permissions`) | ≤ 16 permissions with ids ≤ 1 KiB each, so ≤ 17 KiB; < 200 B typical | always: in a permission-gated session it changes at nearly every checkpoint, so a delta saves nothing |
| `methods` | ≤ `MAX_LOGIN_METHODS_LEN` (256 KiB) | only when it differs from `Written.methods`; `[]` when it became empty |
| `outputs` | ≤ `MAX_OPEN_OUTPUTS` (64) × a line of ≤ `ITEM_LIMIT` (64 KiB), so ≤ 4 MiB | a patch: `{ terminal: open-output }` for each output added or changed since `Written.outputs`, and `{ terminal: null }` for each closed one; left out when none changed |
| `tools` | — | never (decision 2) |

The rule for a future field: a field bounded under 1 KiB in the typical case, and under 64 KiB at worst, is written whole. A larger one is written when it changed, or as a patch. This change records the rule in the module doc's format table. `agents-commands`' option keys, `Busy` mark and kept modes (at most 32 options) are small and stay whole. `agents-plans`' `dismissed` (up to 4 MiB) must be written when it changed.

The base writes `methods` and `outputs` whole, left out when empty, as today. Decoding a delta-shaped chain: `methods` absent in a segment keeps the previous value, and present replaces it; `outputs` patch entries set or remove keys in order. A `null` for a terminal the chain doesn't hold is `Unreadable::Field { name: "outputs" }`. `restore` then checks the final map as today: each entry names an `Output` item of a kept tool entry.

### 5. `Written`: what the last record holds, kept with `Stored` (`types R1`, `R10`)

```rust
// acp/checkpoint.rs
pub struct Written {          // what the stored records leave, as the next segment's base line
    line: Option<LineMark>,   // the collected partial's line, if the last record was collecting
    methods: Vec<LoginMethod>,
    outputs: BTreeMap<String, OpenOutput>,
}
struct LineMark { epoch: EntryId, start: u64, len: u64 }

pub struct Encoded { pub bytes: Vec<u8>, pub written: Written }       // a whole checkpoint
pub struct Segment { pub bytes: Vec<u8>, changes: WrittenChanges }    // one record
impl Written { pub fn apply(&mut self, changes: WrittenChanges); }

pub fn encode(session: &AgentSession) -> Result<Encoded, TooLarge>;
pub fn encode_segment(session: &AgentSession, events: Vec<Event>, seq: u32, last: &Written)
    -> Result<Segment, TooLarge>;
```

`WrittenChanges` holds what the segment wrote: the new `LineMark` (or none), the new methods when they changed, and the patched outputs. `Written::apply` folds it in, so `methods` and an output are cloned only when they changed. `encode_segment` borrows `last` and doesn't change it. The core applies a segment's changes only when it actually appends that segment. A segment the size rule discards is followed by a whole `encode`, whose `Encoded.written` replaces everything.

`policy::Stored::Known` gains `written: Written`, so what the holder stores and what its last record says are one fact. `Unknown` has none, and `encode_segment` can't be reached without one. `Stored` loses `Copy`. `agent_appendable` and `agent_checkpoint_form` take `&Stored`, `Stored::whole(len, written)` and `Stored::appended(self, len, changes)` build it, and `next_seq(&self)` is unchanged. `Agent::forget_stored` (and change 4's forgetting after a caught panic) drops `Written` with it.

`events` is taken by value: the core hands over `std::mem::take(&mut agent.unsaved)`, so `merged` moves events instead of cloning them. The events were cleared after a checkpoint either way.

*Rejected:*
- **A per-field digest instead of clones.** A collision would silently drop a `methods` change from the checkpoint. The clones are small and are made only when the value changed.
- **A change counter on `AgentSession`.** It adds reducer state whose only reader is the checkpoint, and every round-trip test would have to exclude it from equality.

### 6. How `decode` folds a chain, and what a bad record loses (`railway R5`, `R6`)

`decode_2` reads the base and then folds each segment in sequence:
- the log, by folding `events` (as today);
- `lines`, the partial's bytes and the `LineMark`, by decision 3;
- `methods` and `outputs`, by decision 4;
- `tool_epoch` and the always-whole fields, by the last record that has them.

`restore` then rebuilds `tools` (decision 2) and runs every existing cross-check: run, outputs, permissions, `Rpc` parts and stray bytes.

What a corrupt or truncated segment loses is what it loses today: the whole checkpoint is unreadable, and the session is adopted as `not_restored` ("the conversation could not be restored: …"). The agent keeps running and its lines are ignored. That failure shows on the session's own zone, nowhere else (`railway R6`). The holder stores only records the core sent and completed with `CHECKPOINT_END`, and the core sends a segment only onto a base it wrote, so a bad record is a bug, not an expected state. The new failure kinds are:
- `PartialGap`;
- a record with no epoch, which every record written before this change is (`Field { name: "tool_epoch" }`);
- a malformed `from` (`Field { name: "lines" }`);
- an `outputs` patch removing a terminal the chain doesn't hold (`Field { name: "outputs" }`).

Each has a test (task 1.4), and the fuzz test runs over delta-shaped chains.

### 7. The cost bound, pinned (`testing R1`, `R8`, `R13`)

A new pure test in `policy.rs`, `checkpoints_of_a_tool_heavy_conversation_and_a_long_line_write_a_small_multiple_of_what_they_added`, drives the reducer through the existing `Checkpointing` helper (extended to carry `Written` in `Stored`). Its pinned conversation:
- `initialize`, then `session/new`;
- 500 turns of 10 tool calls each, 5,000 calls in all. Each call has a 30-character id (`toolu_` and 24 characters, as Claude's), a `tool_call` (pending), a `session/request_permission` naming it with three options, the client's answer (`Command::Answer`), and a `tool_call_update` (completed) with a 200-byte text item. The turn ends with `end_turn`.
- one `tool_call_update` line of 8 MiB (a `diff` item whose `newText` fills it), fed in 64 KiB reads (128 `Input::Stdout` steps), then its newline.

It asserts three things:
1. **No segment grows with the conversation.** Every appended segment is at most the encoded length of its merged events, plus the partial's new bytes, plus `RECORD_ALLOWANCE` (1 KiB, a constant in the test that names the always-whole fields of decision 4 at this conversation's sizes).
2. **The total.** `written < 4 × added + RECORD_ALLOWANCE × records + wholes_after_trims × LOG_LIMIT`. `added` is the log's encoded growth plus the line's 8 MiB, and `records` is the checkpoint count. Decision 9 of `agents-log-paging` states the bound per byte of growth. A permission-gated step adds tens of bytes of events but always writes a record, so the bound needs the per-record term, and this corrects that decision's claim.
3. **The session round-trips.** Decoding what the holder would store equals the live session.

**Numbers.** Today's code fails assertion 1 at the first few hundred calls; task 1.2 runs the total and records it here as "As built (before)". The estimate is about 2 GB written for about 10 MB added, about 200×. That is 80·T² for the tool calls and 68 MiB plus rewrites for the line. The target from the same arithmetic is about 4× added plus about 1 KiB per record, under 60 MB. The test prints `written`, `added` and `records` in its assertion message, so the before and after numbers come from the same test.

The existing 64 MiB text-conversation test keeps its bound unchanged.

### 8. A whole checkpoint is serialized once into one buffer (`c.md` observation 6)

- **The log streams.** `brindle-transcript` gains `Transcript::replay_ref(&self) -> ReplayRef<'_>`, which implements `Serialize` as the JSON array `replay()` would encode. Each event is written from borrowed data through a private `EventRef<'a>` mirror of `Event`'s replay kinds: `Trimmed`, `Window`, `Added { id, entry: &'a Entry }`, `Queue`, `Conversation`, `Turn` and `Agent`. It has the same `#[serde(tag = "event", rename_all = "snake_case")]` and field names. A test over `fuzz::random_transcript` asserts `serde_json::to_vec(&t.replay_ref()) == serde_json::to_vec(&t.replay())` byte for byte, so the mirror can't drift. No IO: `Serialize` writes into whatever serializer the caller gives it (`fcis R4`).
- **The record streams into `out`.** `push_record` writes the record header with a zero state length and serializes the state straight into `out`. The state is a `#[derive(Serialize)] struct StateOut<'a>`: the small fields as today's `Value`s from `phase_value`, `rpc_value` and the rest, which are small, and `log: ReplayRef<'a>` (base) or `events: &'a [Event]` (segment, already merged). It writes through `serde_json::to_writer` over a `Limited<'_>` adapter that returns an error once `out` would pass the limit, which becomes `TooLarge`. It then patches the length in place and appends the partial's length and bytes. No `Value` tree of the log, no `String`, no second buffer. A side effect: the derived struct's field order is fixed whether or not serde_json's `preserve_order` is on. Today the helper-only build (`-p brindle-remote-agent`, no gpui) sorts the `json!` keys and the workspace build doesn't; both decode the same.
- **Frames from slices.** `brindle-remote-protocol::holder` gains `checkpoint_frames(session, bytes: &[u8], covers, form: BlobForm) -> impl Iterator<Item = SlicedFrame<'_>>`, with `BlobForm::{Whole, Append}` choosing `CHECKPOINT_BEGIN` or `CHECKPOINT_APPEND`. A `SlicedFrame { msg_type, head: [u8; 16], head_len, body: &[u8] }` is one frame whose payload is `head[..head_len]` followed by `body`: the session id, plus the length or `covers`, then a `CHUNK_LEN` slice. `server/conn.rs` gains `Conn::queue_sliced(&SlicedFrame)`, which writes the type, the length, the head and the body straight into the output queue. `Core::send_holder_sliced` in `holders.rs` queues them; an over-long frame can't be built, since the chunks are `CHUNK_LEN`. `ToHolder::checkpoint` and `checkpoint_append` stay for the holder's own tests and transcripts. A transcript test asserts that each `SlicedFrame`'s bytes equal `ToHolder::encode()` of the matching message, so H1's encoding is unchanged and pinned by the existing `holder-checkpoint-append` fixtures.
- **The copies after:** the record buffer and the connection's queue. The transcript itself is not a copy. A 16 MiB log's whole checkpoint goes from about 100 MiB of transient allocation to about 16 MiB of buffer plus what the queue holds.

### 9. A transcript event is serialized once (`pack_events`; `ports R3`)

`brindle-remote-protocol::client` gains `JsonObject`: the bytes of one encoded JSON object, built by `JsonObject::new(bytes) -> Result<Self, NotAnObject>`, which checks the first byte is `{` and the last is `}`. It also gains:
- `transcript_frames(session, events: &[JsonObject]) -> io::Result<Vec<Frame>>`;
- `page_frames(req, session, events: &[JsonObject], more) -> io::Result<Vec<Frame>>`.

They group as `pack_events` does, using `events[i].len()` with no serialization, and write each frame's payload as the envelope's bytes with the objects joined by commas. For `TRANSCRIPT` that is `{"session":N,"events":[…]}`; for `AGENT_PAGE` it is `{"req":R,"session":N,"events":[…],"last":{"more":M}}` on the last frame and the same without `last` on the others. An event too large for a frame is the same `InvalidInput` error as today.

The crate stays free of transcript knowledge: it handles objects it doesn't read, as `Transcript.events: Vec<Value>` already does. The core encodes each `Event` once with `serde_json::to_vec(&event)` (the derived `Serialize`, no `Value`), wraps it, and queues the frames through a new `Core::send_client_frame`. `pack_transcript` and `pack_page` stay for tests and the client side. Transcript tests assert that each new frame equals `ToClient::Transcript(…)` or `ToClient::AgentPage(…)`'s `encode()` for the same events: equal bytes in a workspace build, and equal canonical JSON in a helper-only build (the `preserve_order` note in decision 8). The fuzz test feeds random object bytes to `JsonObject::new` and the packers.

In `agent_apply` the events are encoded for the client first, then moved into `unsaved`, so the clone (`events.iter().cloned()`) goes too. The replay (`sessions.rs`' attach page) and `client_agent_fetch` use the same path.

*Rejected:* `serde_json::value::RawValue`. It needs serde_json's `raw_value` feature, which only gpui enables, so a helper-only build would fail. Turning it on is a dependency change for one type.

### 10. Output text keeps a start offset (A-4; `brindle-transcript`)

`Output.text` becomes `OutputText`: `{ buf: String, start: usize }`, with `as_str() -> &str` (`&buf[start..]`) and `Deref<Target = str>`. So `output.text.len()`, `output.text == "…"` (through `PartialEq<str>` and `PartialEq<&str>`) and slicing read as before. It has `From<String>` and `From<&str>`, so struct literals write `text: "…".into()`. `Serialize` writes the visible `str`, `Deserialize` reads a `String` with `start: 0`, and `Debug` prints the visible `str`. `PartialEq`/`Eq` compare the visible text. `Clone` copies only the visible text. `Default` is empty.

`Output::append(&mut self, text) -> usize` pushes, works out `cut` on the visible text as today, and advances `start` by `cut` instead of draining. It returns the escaped length of the dropped bytes (`escaped_len` of that slice, computed before advancing) instead of a `String`. When `start` passes `ITEM_LIMIT` it compacts once (`buf.drain(..start)`, `start = 0`). That is a 64 KiB memmove at most once per 64 KiB appended, so an append is amortized O(delta).

The fold's `Event::ToolOutput` arm uses the returned length in place of `escaped_len(&dropped)`, and its size arithmetic is otherwise unchanged. `encoded_len`, equality, serialization and every fold result are unchanged. The existing `output_appended_in_pieces_folds_as_appended_whole` and `output_appends_and_keeps_whole_lines_within_the_limit` hold, as does `fuzz.rs`' "appending `a` then `b` equals appending `a + b`" property. A new test appends 1 MiB in 32-byte pieces and checks that the buffer never exceeds 2 × `ITEM_LIMIT` plus a piece.

Readers: `src/agent/tool.rs` (`output.text.as_str()`, and `output.text.clone()` becomes `output.text.to_string()`), `src/agent/model.rs`'s dump (`{:?}` prints the same), `reduce.rs` and `checkpoint.rs` tests, and `fuzz.rs`. The open-output filter state in `reduce.rs` (`plain::State`) is unrelated.

*Rejected:* a `VecDeque<u8>` or ring buffer. It can't hand out a contiguous `&str` without a copy, and every reader wants one.

### 11. Repeated diffs are reused (A-5; pure reducer, `types R10`)

`AgentSession` gains `diffs: DiffMemo`, a map from `(EntryId, item index)` to a `u64` digest of the `(path, old, new)` that produced the `ToolItem::Diff` at that index. The digest comes from `std::hash::DefaultHasher::new()`, which has fixed keys, so it reads nothing ambient (`fcis R2`, `R3`).

In `tool_content`, a `RawDiff` about to take index `i` computes its digest first. When `diffs` holds that digest for `(entry, i)` and the entry's old item `i` is a `Diff`, it takes `old[i].clone()` and skips `diff::hunks`. The lookup is a pure method, `DiffMemo::reused(entry, index, digest, old) -> Option<ToolItem>`, tested on its own. Afterwards the entry's memo is replaced by the digests of the diffs now kept. A repeat of a 200,000-line file then costs one hash pass (about a millisecond per MiB) and a clone of at most `ITEM_LIMIT` of lines, instead of Myers and the allocation of every line.

- **Lifetime:** `DiffMemo` names what refreshes it. It is pruned with the tool index: `trim` drops the entries at or before `through`, and `respawn` clears it. After change 5 it is a field of `ToolBook`. It is bounded by kept tool entries × `MAX_ITEMS` × 16 bytes.
- **Not in the log or the checkpoint.** It is a cache of a pure function's result. A restored session starts with an empty memo: its first repeat of a diff recomputes the same items (`diff::hunks` is deterministic), finds them equal to the entry's, and emits nothing, exactly as today.
- **Equality:** `DiffMemo`'s `PartialEq` is always true, with a doc comment saying why. Two sessions that differ only in it behave the same, and the checkpoint round-trip tests compare decoded sessions, which have an empty memo, with live ones.
- **Collisions:** a 64-bit collision shows the entry's earlier diff for the same item slot. The agent sends both texts, so it controls the content either way, and nothing outside the zone is affected.
- **The permission's call is moved.** `permission` reads the call id and title it needs first, then passes `request.call` by value to `permission_tool`, and takes the entry-title fallback afterwards as today. The `toolCall` clone (both texts) goes.

*Rejected:*
- **Keeping the last `(path, old, new)` texts.** That is up to 2 × 8 MiB per item to save a hash.
- **Comparing the computed diff only.** That still pays Myers.

### 12. The decoders take owned values (B-5; `ports R14` unchanged)

`Dialect`'s methods that reach `serde_json::from_value` take their value by value:
- `initialize_result(self, result: Value)`;
- `new_session_result(self, result: Value)`;
- `continued_result(self, how, result: Value)`;
- `session_update(self, params: Value)`;
- `permission_request(self, params: Value)`.

Those that read by hand and clone nothing keep `&Value`: `auth_methods`, `list_result`, `prompt_result`, `cancel_request`, `supported_versions` and `dialect::spoken`.

In `v1/decode.rs`:
- `split(update: Value)` removes `BY_HAND`'s fields from the owned map;
- `tool_detail(taken: Map<String, Value>)` takes it by value;
- `tool_item(item: Value)` and each location move into `from_value`;
- `chunk(update: Value)` moves;
- `raw(value: Option<Value>)` moves a string, and change 3's bounded writer keeps its own signature for the rest.

In `reduce.rs`, `message` passes `params` by value, and `response(purpose, Result<Value, &RpcError>)` and the per-purpose handlers take the owned result. `speaking` reads `auth_methods(&result)` before handing `result` to `initialize_result`.

Only `dialect.rs` matches on `Dialect`, and only `v1/` names v1's fields and the schema crate. `tests/architecture.rs`' `acp_versions_stay_in_their_modules` holds unchanged. `agents-acp-v2`'s decoder adopts the same signatures.

### 13. What a core restart, a Brindle crash and an upgrade do (`process R1`–`R3`, `R12`)

| Event | What happens |
| --- | --- |
| core crash or SIGKILL | The holder stores the last base and the segments it accepted. The next core decodes the chain (decision 6): it folds the events, rebuilds the partial line from its deltas, applies the `methods` and `outputs` changes, and rebuilds `tools` from the log and `tool_epoch`. It feeds the tail and writes a whole checkpoint first (`Stored::Unknown`). `Written` and the diff memo lived in the dead core and are not needed: the next checkpoint is whole, and the memo is a cache. |
| Brindle crash or quit | Nothing changes on the server. The client's `OutputText` offsets were in its memory; the next attach replays the outputs as text. |
| upgrade (`STEP_DOWN`) | The old core writes every agent checkpoint whole, as today. A new core adopting a checkpoint that a build before this change wrote adopts the session as not restored (decision 1): once, on development builds. Downgrading to such a build does the same to a delta-shaped checkpoint (`Unreadable::Field { name: "tools" }`), as decision 10 of `agents-log-paging` accepted for format 1 builds meeting format 2. |
| an agent restarted | `Agent::restarted` sets `Stored::Unknown`, so the first checkpoint is whole, with the new `tool_epoch` and no `LineMark`. |
| the holder | Unchanged. It appends opaque bytes and parses nothing (`process R1`). |

### 14. Versions and the tests that pin them (`process R4`–`R6`, `R15`)

- **Checkpoint format 2, amended in place.** `PINNED` is written again once from the new encoder and pins a small session's base, and a new pinned chain of a base and two segments pins the segments' deltas. Today's `PINNED` bytes move to `a_checkpoint_from_before_the_delta_shape_is_unreadable`, which also refuses a format-1 header.
- **H1 3.** No change. `holder-checkpoint-append`, `holder-checkpoint-append-refused` and the new sliced-frame equality test pin it.
- **Client protocol 4.** No change. The `TRANSCRIPT` and `AGENT_PAGE` transcripts, the `transcript/tests/fixtures/v4/` fixtures (compared in canonical form), and decision 9's frame equality tests pin it. No fixture is edited.

### 15. Numbers (`testing R14`)

Before and after numbers, from a release build, go into "As built" below:
- **Pure:** decision 7's test prints written and added bytes, before and after.
- **`agent-attach-long`:** the attach time. The whole checkpoints at quit and detach, and the replay's packing, run once-only serialization.
- **`agent-tool-output`:** gains a third run, `small`: 1 MiB of output in 32,768 deltas of 32 bytes (A-4). `summary.toml`'s `[stream]` table gains `small_ms`.
- **New `agent-tool-heavy`** (`tests/perf/agent.rs`, registered in `tests/perf/main.rs`). The fake agent waits on the gate, then sends 3,000 tool calls. Each is a `tool_call`, a `session/request_permission` (a `Moment::Permission` checkpoint), a `$/cancel_request` withdrawing it (another `Moment::Permission`, which needs no user answer), and a completing `tool_call_update`, then a last message. Then comes one 8 MiB `tool_call_update` line written in 64 KiB pieces. It reports the time from the gate to Brindle's exit on a `--wait-for` of the last message, as `agent-tool-output` does, in a `[tool_heavy]` table. It skips without the fake agent.

They report and never pass or fail.

## Risks / Trade-offs

- **A false partial continuation after a respawn with no new entries.** If a respawn added no entry, the epoch can't move. A checkpoint could then fall between the reducer's `Input::Restart` step and `Agent::restarted` while the new process's partial starts at the same offset and is longer, and the line would be misread. → `Agent::restarted` runs in the same handler as the restart's spawn (`sessions.rs`), before any further step, so no checkpoint falls between. Task 3.3 adds a core test that a restart's first checkpoint is whole even when the earlier process left a partial line. After change 4's single "agent gone" step, that step can also return a flag the core uses to forget `Stored`, which closes the window structurally (Open Questions, 2).
- **`DiffMemo`'s always-true equality could hide a real difference in a test.** → It holds only digests, the reducer reads it only to skip recomputing an equal value, and a test asserts that a repeated diff after a restore emits no event.
- **`OutputText` is a public type change in `brindle-transcript`.** Readers in the app and the helper change in the same commit. → The wire form is a JSON string as before, and the v4 fixtures pin it.
- **The 1 KiB per-record allowance in decision 7 is a test constant, not a production bound.** A pathological agent with 16 waiting permissions and 1 KiB ids makes each record about 17 KiB. → The allowance documents the typical case. The no-growth assertion (1) is what guards the regression, and decision 4's table states the worst case.
- **The agent conversations running at the first reinstall across this change are not restored.** → Accepted (decision 1): once, on development builds only. Their agents keep running and their zones say why. Closing agent zones, or letting their turns end, before that reinstall avoids it.

## Migration Plan

Nothing to migrate by hand, and nothing is converted: a core with this change adopts an agent session whose checkpoint an earlier build wrote as "could not be restored" (decision 1). Rollback to a build from before this change does the same to a delta-shaped checkpoint. Only development builds can meet either.

## Open Questions

1. **Salvage on a bad segment?** A checkpoint whose later segment doesn't read could be adopted at its last good record, with a notice, rather than not restored at all. Recommendation: no, not in this change. A bad segment is a core bug, the tail after the good prefix would be fed against the wrong `covers`, and `not_restored` is the recorded behaviour. Propose it as its own change if a holder ever shows one.
2. **A respawn flag from change 4's "agent gone" step.** Recommendation: if change 4's step lands with an `Output` field that says the process changed, `agent_apply` forgets `Stored` on it, and the epoch check in `LineMark` becomes a second guard. Decide when implementing, after re-reading `main`.

## As built

To be filled in by tasks 1.2, 7.3 and 7.4: the decision 7 test's written and added bytes before and after, and the three perf cases' numbers before and after against a baseline from the build before this change.
