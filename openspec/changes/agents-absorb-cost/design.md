# Design

## Context

See proposal.md for why. The code as it is at `5f335dc`, which this design was checked against:

- **The lock.** `Agent::apply` (`src/agent/mod.rs:700-711`) takes `self.shared.lock()` (a `parking_lot::Mutex<AgentShared>`), takes the changes (`AgentShared::take_changes`), and calls `self.model.absorb(shared.transcript(), changes)` with the guard alive, so everything `absorb` does runs under it: cloning entries, `render_entry` (the whole message through `markdown::render`), the part comparison, `keep_texts` (each changed tool text item rendered), `draw`, `refresh_rows`, `update_thoughts`. The reader thread takes the same lock in `Sink::transcript` and `Sink::page` (`src/agent/sink.rs:178-221`) for every `TRANSCRIPT` frame and every finished page, and it does so inside `Connection::deliver` (`src/remote/local/connection.rs:2096-2103`), holding the connection's per-session sink mutex. One reader thread serves every session of an endpoint, so the UI's render blocks every tab's output there. `shared.rs`'s module doc already calls the lock a leaf ("nothing is taken under it"), and that holds today: nothing is nested under it on either side.
- **What one absorb costs** (`src/agent/model.rs:1687-1772`). A reset clones every transcript entry and renders every message and thought. A prepend (`prepend_entries`) clones the fetched page's entries and renders them. Added entries are found by `transcript.entries().filter(|(id, _)| *id >= from)`, a scan from the first held entry. `update_entry` copies an appended tail, but any other dirt (`ToolItems`, `Output`, `Replaced`) clones the entry whole (a tool entry up to `CALL_LIMIT`, 256 KiB, plus two `RAW_LIMIT` raw values); then it renders an agent message or thought whole again and compares the fresh parts with the cached ones from the start (deep `PartialEq`). `redraw` of a tool calls `tool::detail` for the whole entry, which copies every kept output run and diff line into new `SharedString`s. Every absorb then runs `update_thoughts` and `refresh_rows`, and `refresh_rows` calls `refresh_facts`, which walks every held entry (a `HashMap` of calls and a `BTreeMap` of permission tools), then checks every held entry's row (two binary searches each, `entry_blocks`).
- **Pacing.** `Agent::drain` (`mod.rs:668-697`) reads the sink's channel as `ready_chunks(256)`, waits `Pace::wait`, then in one `update` calls `pace::drain`, which keeps taking ready chunks for `BUDGET` (16 ms). The sink's `EventGate` holds at most one merged `Changed`; `Taker::took` re-arms it as soon as the zone takes it, so a frame that arrives while the zone absorbs queues a new `Changed`, which the same `pace::drain` call takes and absorbs again. Across wakes, `pace::step` allows `BURST` (4) quick updates and then one per `PACE` (4 ms), up to about 250 a second. So renders are bounded per wake, not per frame. How many there are depends on how the core spreads an agent's output over `TRANSCRIPT` frames, which the review didn't measure.
- **`agents-markdown` decision 6** says "a stream costs at most one parse per paced wake, about one per frame while text arrives", and its as-built notes say the perf case's fake agent "writes its chunks back to back, so Brindle's reader merges them into a few absorbs; the case measures that burst, not a reply trickling in". Its fallback ("render on the reader thread after `apply`, outside `AgentShared`'s lock") assumed rendering was already outside the lock; it isn't (D-2).
- **The view's per-absorb work** (`src/agent/view.rs:560-606`). `AgentView::sync` runs on every notify of the `Agent`: `stops()` twice, `refresh` (which calls `ChatModel::pending_permissions`, a scan of every block, `model.rs:2740-2756`), `prompt_state` (`Agent::offer`, which looks the profile up in the config), and `follow_earlier`.
- **What a zone holds.** `AgentShared.transcript` (the reader's fold); `ChatModel.entries` (a full copy); `rendered` (every agent message's and thought's `Part`s); `blocks` (user text copied into one `SharedString` per part by `user_blocks`, `model.rs:348`; an open tool row's detail parts; a pending permission's copy of its tool's detail parts); `texts` (`BTreeMap<(EntryId, usize), (SharedString, Rich)>`, each tool text item's source copied again beside its render, `model.rs:432` and `:1890`). `DetailPart::Text { item, source }` (`tool.rs:201`) copies a text item's source a third time while its row is open, and nothing reads that `source` outside tests.
- **Tool output.** `Event::ToolOutput { id, item, text, exit }` folds through the transcript's private `Output::append`, which keeps the longest suffix within `ITEM_LIMIT` (64 KiB) in whole lines and counts the dropped bytes in `omitted`; `fuzz.rs` checks that appending `a` then `b` equals appending `a + b`. `shared.rs` records it as `Dirty::Output { item }`, merged by `Dirty::and`.
- **The sibling changes this one builds on.** `agents-module-structure` (5) splits `model.rs` (this change's code lands in its `src/agent/model/blocks.rs`); `agents-zone-focus-and-scroll` (2) moves the working row's splice into `ChatModel::finish` (its decision 5) and scroll anchoring into `MessageScroller` (its decision 2); `agents-client-state-fixes` (1) turns `Changes::from_replay` into `status_from: StatusFrom` (its decision 8) and reconciles prompts in the reset; `agents-checkpoint-deltas` (6) turns `Output.text` into `OutputText` (its decision 10); `agents-event-bounds` (3) bounds a text entry at 1 MiB (its C1).

## Goals / Non-Goals

**Goals:**
- Under `AgentShared`'s lock, only taking the changes and copying what changed, with a stated worst case per kind.
- One render of a changed message per draw, and a measurement that says whether draws need a frame gate on top.
- A tool output delta costs its new bytes and one output's last part.
- Every copy of the conversation a zone holds is listed, the avoidable ones gone, and the recorded bound true.
- Per-absorb work in the model and the view that doesn't grow with the conversation.

**Non-Goals:**
- A fold owned by the UI thread (decision 1).
- Long top-level tables and lists in continuation parts, and a code block's shown text computed once (E-5): `agents-markdown-long-blocks`.
- Changing what the core sends or how it batches frames.

## Layers

| Part | Layer | Crate |
| --- | --- | --- |
| `Transcript::entries_from`; `Output::append` made public | shared, pure | `brindle-transcript` (`transcript/src/lib.rs`) |
| `ChangeCopy`, `Copied`, `ChatModel::copy_changes` (the first half) | GPUI-free model, reads `&self` and the transcript only | `brindle` (`src/agent/model.rs`; `src/agent/model/blocks.rs` after change 5) |
| `ChatModel::absorb(ChangeCopy)`, `draw(Due)`, `Undrawn`, `Facts`, `Rows`, `render_stats`, the dump's grouping | GPUI-free model | `brindle` (same) |
| `tool::output_parts` per item; `DetailPart` without the text item's source | pure module | `brindle` (`src/agent/tool.rs`) |
| interned link targets | pure module | `brindle` (`src/agent/markdown.rs`) |
| `pace::draw_wait` (only with the frame gate) | pure | `brindle` (`src/terminal/pace.rs`) |
| the two halves around the lock, the draw at the end of a drained update, the frame gate's timer, the render-count log line | shell (`Agent` entity) | `brindle` (`src/agent/mod.rs`) |
| reading the pending set | view | `brindle` (`src/agent/view.rs`) |
| `pace <ms>` | pure parser and the stand-in's player | `brindle-remote-agent` (`remote-agent/src/fake_acp/script.rs`, `main.rs`) |
| the trickled run, render counts, the latency cases | perf harness | `tests/perf/` |

No decision moves into a view, a thread or a loop handler; no crate reference changes (`ports R1`, `R3`). `brindle-transcript` stays pure (`fcis R4`).

## Decisions

### 1. The reader keeps its fold; the zone shrinks its work under the lock (D1)

This is the remediation plan's decision D1, answered by the user on 2026-11-26: the reader thread keeps folding into `AgentShared`'s transcript, and the zone takes the changes and copies the changed tails under the lock and renders after releasing it. Two copies of the log stay (the fold and the zone's), and decision 8 corrects the recorded bound.

*Rejected:*
- **A fold the UI thread owns** (`d.md` observation 2, D-9's first fix): the reader would only decode, hand a `Vec<Event>` over a leaf lock, and the zone would fold into the only `Transcript`. It would drop a copy of the log, but a `FoldError` would be found on the UI thread and have to break the session through `Requests`, and a burst would be folded on the UI thread instead of the reader's. The user chose against it (D1).
- **`ChatModel` drops `entries` and reads the shared transcript under the lock in its handlers** (D-9's second fix): every handler (a toggle, a copy, the dump, `answer`) would take the lock the reader needs, and rendering would still need the text outside it.
- **Render on the reader thread** (`agents-markdown` decision 6's fallback): the render would then hold up the reader itself, which is the stall D-2 describes, for every tab on the endpoint.

### 2. The first half: take the changes and copy what changed (`process R14`; `fcis R1`, `R9`)

`Agent::apply(AgentEvent::Changed)` becomes:

```rust
let taken = {
    let mut shared = self.shared.lock();
    let changes = shared.take_changes();
    if changes.is_empty() { return; }
    self.model.copy_changes(shared.transcript(), changes)
}; // the lock is released here
let absorbed = self.model.absorb(taken);
```

`ChatModel::copy_changes(&self, transcript: &Transcript, changes: Changes) -> ChangeCopy` is pure and reads the model only to size the copies. (It is not named `Taken`: `agents-client-state-fixes` decision 5 gives that name to its prompt-reconciliation enum in the same model.) `ChangeCopy` holds the `Changes` as taken (with `agents-client-state-fixes`' `status_from` in it) and:

```rust
pub struct ChangeCopy {
    changes: Changes,
    /// Copies of the entries that changed, in id order.
    entries: Vec<(EntryId, Copied)>,
    /// The turn, the agent, the conversation and the activity, when
    /// `reset`, `turn_or_agent` or `added_from` says they may have moved.
    session: Option<SessionCopy>,
    /// The slot, when `reset` or `slot_changed`.
    slot: Option<Slot>,
    window: Option<Window>,
}

pub enum Copied {
    /// An entry the zone doesn't hold: added, on a fetched page, or any entry on a reset.
    New(Entry),
    /// A message's text from `from_byte` on (`Dirty::Appended`, when the zone's text is that long).
    Tail { from_byte: usize, text: String },
    /// Output item `item`'s new bytes and how its command ended (`Dirty::Output`, decision 6).
    Output { item: usize, text: String, exit: Option<Exit> },
    /// A tool call's items from `from` on (`Dirty::ToolItems`).
    Items { from: usize, items: Vec<ToolItem> },
    /// Anything else (`Dirty::Replaced`, or a tail or delta the zone's copy can't take): the entry again.
    Whole(Entry),
}
```

`Copied` is a closed enum: a copy is exactly one of these, and the second half matches it without a wildcard (`types R1`, `railway R3`). A `Tail` whose `from_byte` doesn't match the zone's text, or an `Output` whose delta can't be derived (decision 6), is never built: the first half makes a `Whole` instead, as `update_entry`'s `own.len() == from_byte` check does today. Added entries are read from `Transcript::entries_from(added_from)`, a new pure method in `brindle-transcript` that starts at the id by binary search (the private `index` it already has) instead of scanning from the first entry.

**What is copied under the lock, at most:**

| Change | Copied | Worst case |
| --- | --- | --- |
| Reset (a replay, a new session's first change) | every entry the transcript holds | the newest page: `PAGE_BYTES` (256 KiB) of encoded entries, or one larger entry when the newest is; `agents-event-bounds` bounds a text entry at 1 MiB, a tool entry is at most `CALL_LIMIT` plus two `RAW_LIMIT` (288 KiB) |
| Prepended page | the page's entries | the same as a reset |
| Added entries | each new entry | what arrived since the zone last took changes: one paced wake's frames |
| `Appended` | the new tail | what arrived since the last take |
| `Output` | the new bytes | at most `ITEM_LIMIT` (64 KiB) plus the command and directory, when the delta can't be derived |
| `ToolItems` | the items from `from` | `CALL_LIMIT` |
| `Replaced` | the entry | 288 KiB for a tool, a few KiB for a permission |
| Session state, slot, window | small values | a prompt in the slot: 256 KiB (`MAX_PROMPT`) |

These are memory copies and small allocations, about a millisecond for the largest; today the same lock covers renders of up to 256 KiB at about 12 ms each in a debug build (`agents-markdown` as built), several per absorb.

**Lock order.** `AgentShared`'s lock stays a leaf: the reader takes it under the connection's per-session sink mutex (`Connection::deliver` → `Sink::transcript`), and the UI takes it alone, never the sink mutex, so there is no cycle. Nothing is taken under it on the UI side: no gate lock (the `Taker::took` call happens before `apply`), no log call, no IO. The reader's `send` to the gate happens after its block, as today. `render` and `prepaint` never take it: `AgentView` and `render_block` read only `ChatModel`, and the dump reads only `ChatModel` (`process R14`). `shared.rs`'s module doc and `docs/code-map.md` say this.

### 3. The second half: apply the copy, decide, mark (`fcis R1`, `R5`; `types R10`)

`ChatModel::absorb(&mut self, taken: ChangeCopy) -> Absorbed` takes no `&Transcript`, so it can't read the shared fold: the type keeps the rule (`process R14`). It:
- applies each `Copied` to the zone's entry (`New` inserted in id order, `Tail` pushed, `Output` appended with `Output::append`, `Items` spliced into `content`, `Whole` replacing), updating the kept facts (decision 7) as it goes;
- on a reset, replaces `entries` and clears `rendered`, `texts` and `auto_open` as today; `agents-client-state-fixes`' reconciliation of an uncertain prompt and its `toggled`/`by_hand` clearing run here, on the copy;
- on a trim, drops the entries, renders, texts, facts and blocks through the id, and returns that splice at once (it renders nothing);
- copies the session state, slot and window from the `ChangeCopy` as today's absorb reads them from the transcript;
- marks every entry it changed in `undrawn` (decision 4), and renders nothing;
- runs `finish` as today: the status, notice, title, queue, first prompt and choosing are decided here, from the entries, phase and turn, none of which needs a render.

Expected failures are unchanged: an event that doesn't decode or fold breaks the session on the reader thread (`Broken`, `railway R6`), and a renderer panic still shows its text as one plain paragraph (`render_entry`, under `catch_unwind`). A copy the zone can't apply, because its entry is missing (it was trimmed, or the first half's id is below the window), is skipped, as `update_entry` returns nothing today.

### 4. Draw marked entries once, at the end of the update (D-3; `fcis R1`, `R9`; `types R1`, `R10`)

`ChatModel` gains what it hasn't drawn yet, named for what refreshes it (`types R10`: `absorb` marks, `draw` clears):

```rust
struct Undrawn {
    /// A reset: every block is drawn again, as one splice of the whole list.
    all: bool,
    entries: BTreeMap<EntryId, Mark>,
}

enum Mark {
    /// Held, with no blocks yet (added or prepended).
    New,
    /// Drawn, and changed since: its text grew, its output grew, its items changed (deferrable by the frame gate, decision 5).
    Grown,
    /// Drawn, and replaced or otherwise changed (drawn with the next draw, never deferred).
    Changed,
}
```

`ChatModel::draw(&mut self, due: Due) -> Vec<Splice>` with `enum Due { All, Settled }` (today's private `draw(id, entry, thought_open)`, which builds one entry's blocks, is renamed `draw_entry` to free the name): `All` draws every mark; `Settled` (only with the frame gate) leaves `Grown` marks for the next draw. It renders each marked message or thought once (`render_entry`), each tool text item whose source changed (decision 8), and draws and splices in id order:
- a reset gives one splice `0..old → new`, as today, so `MessageScroller`'s anchoring sees what it sees now;
- consecutive `New` entries give one insert at their place, so a prepended page is one `0..0` insert (the scroller's "insert at the owner's start", `agents-zone-focus-and-scroll` decision 2, rule 6) and an added run one insert at the end;
- a `Grown` or `Changed` entry is spliced as `update_entry` and `redraw` splice today: from its first part that differs, or the least range for a tool;
- then `update_thoughts` and `refresh_rows` (decision 7), whose splices follow;
- last, the working row's insert or removal. `agents-zone-focus-and-scroll` decision 5 puts that splice at the end of `finish`; this change moves it to the end of `draw`, computed at `self.blocks.len()` as it is then, so it is still the last of the splices and still computed when the model applies them, which is what that decision needs.

**Who draws.** `Agent::drain`'s update calls `self.draw(cx)` once after `pace::drain` returns, before its `cx.notify()`: it calls `model.draw(Due::All)` and emits one `AgentUpdate::Splices`. Every other model method that splices (`toggle_thought`, `toggle_thoughts`, `toggle_tool`, `toggle_tool_details`, `set_policy`, the life events through `life_event`) calls `draw(Due::All)` first and returns its splices after those, so no splice is computed over stale blocks. `ChatModel::render_stats() -> RenderStats { renders: u64, bytes: u64 }` counts every `markdown::render` call and the bytes it was given; the `Agent` logs them at `debug` once, when the zone is closed or quit (`close`, `quit_with`), which the perf cases read.

**Order after an absorb.** Within one update, each event is still applied in the order the channel gave it, and each absorb's `Absorbed` (status, notice, title, first prompt, choosing) is still emitted when it is applied, before the next event; a `Reply`'s draft change likewise. What moves is the list's splices: they are emitted once, after the update's last event, instead of after each absorb. Nothing that the earlier events emit reads the list: the tab status and notification read `shown_status` and `notice` (from the entries), the first prompt's send reads the model's state, a draft change goes to the prompt input, and the scroller receives every splice before the next render (gpui delivers emitted events before it draws). `ChatModel::answer` and the pending set read entries and facts, never blocks. The view's `refresh` reads the pending set (decision 7), which `absorb` updates, so a permission's focus handles exist from the same update as its block. The dump (`describe`) reads `rendered` for its `--- block` lines; it is taken outside updates, so it sees the drawn state unless the frame gate holds a `Grown` entry, for at most one frame (decision 5).

*Rejected:* rendering in `AgentView::render` or `prepaint` (`process R14`: no heavy work there); `cx.on_next_frame` from the view (only a window can schedule it, a zone in a hidden tab draws no frame, so its marks would wait until it is shown and its first frame would show stale blocks, and the `Agent` would need its view to finish its own work).

### 5. Measure first; a frame gate only if the numbers ask for it (D-3 is plausible; `testing R8`, `R10`, `R14`)

D-3's multiplier depends on how the core spreads an agent's chunks over frames, so the change measures it before deciding:
- **The unit guard** (`model.rs` tests): `a_batch_of_appends_absorbed_in_one_update_renders_the_message_once` feeds three `Appended` changes to one message through `copy_changes` and `absorb`, then `draw(Due::All)`, and asserts `render_stats().renders` rose by one. Written first against today's `absorb` (whose renders count is the number of absorbs), it fails with 3.
- **The trickle mode** (`agent-markdown-stream`): a third run, `trickled`, plays the same 2,048 chunks of 32 bytes with the fake ACP agent's new `pace 10` step, so they arrive about 10 ms apart, about 20.5 s in all, the `agents-markdown` decision 6 example's length. `pace <ms>` makes each later `send` wait until `ms` after the previous one, against a deadline on a monotonic clock so the spacing doesn't drift. It is the stand-in agent's behaviour, a real agent's pace, not a test's wait: the perf suite reports numbers and asserts nothing (`testing R14`), and no e2e case or real-role test uses `pace`, since a test that asserts gates on files (`testing R7`, `R8`). The run reports its time, and from Brindle's log (decision 4's line, the case setting `RUST_LOG=brindle=debug,warn` for that child) the renders and rendered bytes of the streamed and trickled runs.
- **The criterion.** Over a trickle of `t` seconds at about 60 frames a second, one parse per frame is about `60·t` renders. The run is taken on `main` before the change (with decision 4's counter and log line landed first) and after decisions 2–4.
  - If the trickled run after decisions 2–4 shows at most about `1.2 × 60·t` renders (about 1,500 for 20.5 s), "one render per draw" is already about one per frame: the frame gate isn't built, its tasks are dropped, and the as-built notes record the numbers and that D-3's "several times" wasn't shown beyond the per-update coalescing.
  - If it shows more, the gate below is built and the run taken again.
- **The frame gate** (built only on that condition): `pace::draw_wait(last_draw: Option<Instant>, now: Instant) -> Option<Duration>` (pure, `fcis R2`), `None` when a frame (`FRAME`, equal to `BUDGET`, 16 ms) has passed since the last draw, else the rest of it. At the end of an update the `Agent` asks it: `None` draws `Due::All`; `Some(wait)` draws `Due::Settled` (new, changed, reset and prepended entries, so nothing appears late and a permission's block comes with its handles) and, unless one is armed, arms one timer (`background_executor().timer(wait)`) whose update draws `Due::All`. So a growing message is rendered at most once per frame, and a burst of chunks within a frame costs one render. On Windows the timer is one more posted wake per frame at most, within what `pace.rs` allows.

*Rejected:* pacing the whole drain loop to frames (replies, life events and status would wait a frame too, and a terminal's pace would differ from an agent's for no reason); a fixed sleep (`testing R9` binds tests, and in the product it would add latency to every change).

### 6. Tool output by its delta (D-3; `agents-tool-content` decision 10)

The first half derives an output's new bytes from the zone's copy, without the event's text (which `Changes` doesn't keep): with the zone's item `(omitted₀, len₀)` and the transcript's `(omitted₁, len₁)`, the bytes appended since are `d = (omitted₁ + len₁) − (omitted₀ + len₀)`. When `0 ≤ d ≤ len₁` and `len₁ − d` is a char boundary, they are the last `d` bytes of the transcript's text, since none of them was dropped; the copy is `Copied::Output { item, text, exit }`. Otherwise (the zone's item isn't an output, or the new bytes pushed some of themselves out) the first half copies that one item, or the entry when `Dirty::and` merged the output with other dirt.

The second half appends with `Output::append`, made public in `brindle-transcript` (today it is private; `agents-checkpoint-deltas` decision 10 changes its return type, which this doesn't read). The transcript's own property, appending `a` then `b` equals appending `a + b` (`fuzz.rs`), makes the zone's item equal the transcript's, `omitted` and `partial` included; `an_output_delta_appended_to_the_zones_copy_equals_the_transcripts` pins it over every split point and over drops past `ITEM_LIMIT`.

The draw redraws only that output: `tool::output_parts` already builds one output's parts; the model splices that item's parts from the first that differs (the last part, while nothing was dropped from the front; all of that item's parts after a drop, at most `ITEM_LIMIT` of text) and leaves the entry's other items' parts alone. `tool::detail` (whole) is still used for a new or replaced entry. A pending permission showing that tool's detail is redrawn the same way. `an_output_delta_redraws_only_its_last_part` and `the_detail_drawn_from_deltas_equals_the_detail_drawn_whole` pin it.

### 7. Facts kept on change, rows checked by cause (`e.md` observation; `types R10`; `fcis R1`)

`refresh_facts` (every entry, every absorb) is replaced by `Facts`, kept by `absorb` as it applies copies:

```rust
struct Facts {
    /// The latest user message held.
    last_user: Option<EntryId>,
    /// The newest tool entry held for each call id.
    newest_tool: HashMap<String, EntryId>,
    /// The pending permission requests in id order: their options' ids, and the tool entry each shows.
    pending: BTreeMap<EntryId, Pending>,
    /// Tool entries whose status is pending or in progress: the rows a turn's or a user message's change can move.
    unfinished: BTreeSet<EntryId>,
}
```

A new entry updates them in id order (a permission takes the newest tool before it, as `refresh_facts` does), a replaced one updates its own, a trim drops ids through `through` (and `newest_tool` values among them), and a reset or prepend reads them again once from the entries (`Facts::read`, also what the tests compare with). `refresh_rows(rows: Rows)` with `enum Rows { All, Only(BTreeSet<EntryId>) }` checks the rows given: `All` for `set_policy` and a reset; otherwise the `unfinished` set when the turn or `last_user` changed, the tool a permission names when it is added or answered, and the entries the absorb copied. A finished tool's row depends only on the user's choice, the policy and a pending permission, so the rest can't change. `facts_kept_on_change_equal_facts_read_again` and `rows_refreshed_by_cause_equal_rows_refreshed_whole` check both against the whole computation over generated event sequences.

`ChatModel::pending_permissions()` returns the set (at most 16: the helper's `MAX_PENDING_PERMISSIONS` in `remote-agent/src/acp/rpc.rs`, which the app doesn't name) instead of scanning blocks; `pending_body` and `answer` read it too; `agents-zone-focus-and-scroll`'s `permission_block` reads it with `entry_blocks`. `AgentView::refresh` rebuilds its handle list from the set (16 at most), so `sync` no longer scans blocks. `tool_in_progress` still walks back to the latest user message, which is bounded by the current turn. `Agent::offer`'s profile lookup per `sync` stays: it is a lookup by name among the configured profiles, independent of the conversation.

### 8. The copies a zone holds, and the bound (D-9, traced)

D-9 holds as the report says; the copies, read from the code at `5f335dc`, with `L` the log the zone holds (at most the core's `LOG_LIMIT`, 16 MiB encoded, once fetched back to its start or grown to it live):

| Copy | Holds | Size | After this change |
| --- | --- | --- | --- |
| `AgentShared.transcript` | every held entry | ≤ `L` | stays (decision 1) |
| `ChatModel.entries` | every held entry | ≤ `L` | stays (decision 1) |
| `rendered` | agent messages' and thoughts' parts: their shown text, runs and links | about their text; more with repeated reference links and images | stays; link targets shared (below) |
| `blocks`, user parts | user text, one `SharedString` per part (the parts partition it) | ≤ the user text | stays (below) |
| `texts` | each tool text item's source again, and its render | 2 × the tool text | the source goes: ≤ 1 × |
| `DetailPart::Text { source }` in an open row's or a pending permission's parts | the text item's source a third time | ≤ the open rows' text items | goes |
| open rows' other detail parts | diff lines and output runs as `SharedString`s; raw values | ≤ `CALL_LIMIT` + 2 × `RAW_LIMIT` per open row and per pending permission's detail | stays: it is the drawn form, and a row is open by the user's choice or the policy |
| `CodeBlock::render`'s `shown_code` | a fence's code, per render | transient, per frame | goes in `agents-markdown-long-blocks`: `Code::new` computes the shown text once (its decision 7) |

What goes and why:
- **`texts` keeps only renders** (`BTreeMap<(EntryId, usize), Rich>`). Whether a text item changed is decided when the copy is applied: the zone's old item is in hand then, beside the new one, so the comparison needs no stored source. A `New` entry renders all its text items; `Items { from, .. }` and `Whole` re-render the items whose text differs from the old ones.
- **`DetailPart::Text { item }`** drops `source`: the view draws the rendered `ShownPart::text`, and the dump reads the entry.
- **Link targets shared.** `Walk` keeps a `SharedString` per target it has made during one render, so `[a][d]` used a thousand times holds one target, and the tooltip is the same `SharedString`, not a second allocation.
- **User text stays.** The parts' strings are one copy of the user text, not one per part. Removing it would need the zone's entries to hold `SharedString`s, which every reader of the transcript's `Entry` would see. User messages are bounded by `MAX_PROMPT` each.

**The corrected bound**, replacing `agents-profiles`' "at most about twice the core's 16 MiB": a zone holds its log twice (the shared fold and its own copy), the rendered text of its agent messages and thoughts once more (about their text), its tool text items' renders, and the detail of its open rows and pending permissions (at most `CALL_LIMIT` plus two `RAW_LIMIT`, 288 KiB, each). With every row open that is at most about four times `L`, about 64 MiB of text, plus the render's structure. `held_text_counts_the_copies` (a `#[cfg(test)]` `ChatModel::held_text()` summing each table row) pins the table on a fixed conversation, and fails first on today's `texts` (the source held twice). `docs/code-map.md`'s agent entry says: "A zone holds its log twice (`AgentShared`'s fold, `ChatModel.entries`), plus the rendered Markdown of its agent messages and thoughts, its tool text renders and the detail of its open rows: at most about four times the core's 16 MiB `LOG_LIMIT` with every row open."

The archived `agents-profiles` design stays as written; this decision is the record that corrects it, and `agents-markdown` decision 6's "about one more log's worth" for renders is part of this bound.

### 9. The measurements for D-2 (`testing R10`, `R14`)

Two new perf cases in `tests/perf/agent.rs`, each a `Measured::Latency` (no new report table), run on `main` before the change and after it:
- `agent-latency-quiet`: a tab of the perf suite's `Shell` with the agent profile `Fake` split beside it (`--action split_agent_right`, a short reply, its turn ended), focus back in the shell (`--action focus_zone_left`), then `--latency-bench N`: key echo in a shell beside an idle agent zone, the baseline.
- `agent-latency-trickle`: the same, with the 64 KiB reply trickling (`pace 10`) while the bench types. This is D-2's effect (the reader thread, shared by the shell's and the agent's sessions, waiting on the lock) together with D-3's. The trickle must outlast the bench; the case reports the bench's own time beside the trickle's length and marks a run whose stream ended first.

The numbers go in the as-built notes. `agents-markdown-long-blocks` (E-5) takes `agent-latency-quiet` as the baseline of its `agent-latency-table`.

### 10. The wire, the state and the server (`process R1`–`R6`, `R12`)

No message, protocol version, checkpoint, config key or state file changes. `Transcript::entries_from` and a public `Output::append` are pure additions to `brindle-transcript` that change no encoding, so no transcript fixture changes and both versions of every protocol meet as before. The server is untouched: a core restart, a Brindle crash and an upgrade do what they do today. The client's undrawn marks, facts and copies live in a zone's memory and are rebuilt by the next attach's reset.

### 11. Recorded decisions this changes

- `agents-markdown` decision 6: its cost model ("one parse per paced wake, about one per frame") becomes decision 4's (one render per draw, and per frame with the gate), and its fallback (render on the reader thread) is rejected (decision 1).
- `agents-profiles`' memory line: replaced by decision 8's bound.
- `agents-tool-content` decision 10: "the zone draws that output's last part again" now holds without cloning the entry or rebuilding its other parts (decision 6).
- `agents-zone-focus-and-scroll` decision 5 (unarchived sibling): its working-row splice moves from the end of `finish` to the end of `draw` (decision 4), keeping its order.

The remediation plan's D1 is decision 1. No row of its choices table (C1–C5) touches this change.

### 12. Tests (`testing R1`, `R13`, `R15`)

- **Pure, in `model.rs`'s tests** (through the test `Zone`, whose `absorb` becomes `copy_changes`, then `absorb`, then `draw(Due::All)`, returning the splices of both in order, so every existing test keeps its meaning): the guard of decision 5; `the_copy_taken_under_the_lock_holds_only_what_changed` (a tail, an output delta, the items from `from`, a reset's entries); `absorb_renders_nothing_and_draw_renders_each_marked_message_once`; `a_reset_is_drawn_as_one_splice_of_the_whole_list`; `a_prepended_page_is_drawn_as_one_insert_at_the_start`; `a_toggle_draws_marked_entries_first`; decision 6's and 7's tests; `held_text_counts_the_copies`; `a_tool_text_item_is_rendered_again_only_when_its_text_changed`. The existing `streaming_at_every_split_point_ends_with_the_whole_texts_parts` and `streaming_a_long_message_re_renders_only_what_changed` keep passing through the new helper.
- **Pure, elsewhere:** `markdown.rs` (`a_reference_used_many_times_shares_one_target`); `tool.rs` (`an_outputs_parts_are_built_on_their_own`); `transcript/src/lib.rs` (`entries_from_starts_at_the_id_or_the_next_held_one`); `fake_acp/script.rs` (`pace_takes_milliseconds_and_refuses_anything_else`); `pace.rs` (`a_draw_waits_out_the_rest_of_a_frame`, `a_draw_after_a_quiet_frame_goes_at_once`, only with the gate).
- **Entity, `src/agent/tests.rs`:** `an_update_emits_its_statuses_in_order_and_its_splices_once_after_them`.
- **e2e:** no new case; the existing `agent-markdown-*`, `agent-tool-*`, `agent-thoughts`, `agent-scroll`, `agent-working`, `agent-earlier-*`, `agent-permission-*` and `agent-continue-*` cases run unchanged.

## Findings traced

- **D-2:** confirmed by reading (`mod.rs:700-711`, `sink.rs:178-221`, `connection.rs:2096-2103`). `agent-latency-trickle` gives the numbers.
- **D-3:** the code path is confirmed (Context); the multiplier is not. Decision 5's guard shows the per-update multiplier (several absorbs, several renders) with a failing test; the trickle mode shows whether more than one render per frame remains, and decides the frame gate.
- **D-9:** confirmed by reading; the table in decision 8 is the evidence, and `held_text_counts_the_copies` fails first on the tool text source held twice. One copy the report lists is shrunk rather than removed: user text is held once in its parts, not per part, and stays (decision 8).

## Risks / Trade-offs

- **[A dump taken while the frame gate holds a growing entry shows its entry line ahead of its block lines]** → for at most one frame, and only with the gate. `--wait-for` polls, so it sees the drawn state on its next look; no case takes a dump mid-stream and compares the two.
- **[The splices of an update come after its statuses]** → decision 4 lists every reader of what comes earlier; none reads the list. The entity test pins the order.
- **[The first half is new code on the reader's critical path]** → it only allocates and copies; its tests pin each kind's size, and a delta it can't derive falls back to a whole copy, never to a wrong one.
- **[Facts kept by hand can drift from the entries]** → the property tests compare them with `Facts::read` and the whole row refresh over generated sequences, and a reset or prepend reads them again.
- **[Rendered text can still exceed its source]** → images (`[image: ALT]` and up to 120 characters of target for a few bytes of `![d]`) and runs (a structure per few bytes of `*a*`) amplify; link targets are now shared. Open Question 1.

## Migration Plan

Nothing to migrate: no stored state changes. The change lands after `agents-module-structure` (5) and `agents-zone-focus-and-scroll` (2) on `main`, and after `agents-commands` (PR #151). The counter and log line (decision 4) and the perf cases land first in the same PR, so the "before" numbers are taken on the code as it was. Rolling back is reverting the PR.

## Open Questions

1. **Should rendered text be capped?** An agent can make the render larger than its text with images and short emphasized runs. *Recommendation:* count it in `held_text` on an adversarial message in this change's tests, and propose a cap only if it passes a few times the source; a cap would be a visible change (an image shown as `[image: ALT]` alone past it) and needs its own spec.
2. **`FRAME` is 16 ms whatever the display's rate.** *Recommendation:* keep it equal to `BUDGET`; a 120 Hz display then draws a growing message every other frame, which reads the same.
