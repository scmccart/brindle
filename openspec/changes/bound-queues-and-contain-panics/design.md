# Design

## Context

See proposal.md, "Why". The audit is `docs/architecture/audits/2026-11-16.md`: finding F8 and the design smells, with the entries in `2026-11-16/b24.md` #2–#3, `b28.md` #9, `b25.md` #19, `b10.md` #14, `b12.md` (its `process R9`, `R10` and `fcis R1` entries and its smells), `b23.md` (`config.rs:434`), and the `b01` and `b26` smells. The plan is `2026-11-16-remediation.md`, change 9. Line numbers below are from the audited tree, which is unchanged.

This change depends on change 2 (`carry-session-identity-and-host`) for the client's session identity. It depends on change 5 (`enforce-platform-seams`) for term-core's `PathStyle`, which decision 10 uses to settle the Open Question change 5 left.

### Today

- **Terminal events** (`src/terminal/mod.rs`, `server.rs`):
  - `Listener(UnboundedSender<ChannelEvent>, Arc<OutputMarker>)` (`mod.rs:103-114`) queues every alacritty event the `Term` emits while parsing.
  - `Sink::send` (`server.rs:58`) queues each OSC found, the replay markers (`:107`) and the exit (`:113`).
  - `Sink::output` queues a `Wakeup` for every read (`:96`).
  - The channels are made in `start_session`, `attach` and `message` (`mod.rs:413, 443, 481`).
  - `spawn_events` (`:287`) drains them with `ready_chunks(1024)` and `pace::drain` (16 ms a turn). `Batch::coalesce` merges titles and wakeups, but only after they were queued.
- **Parsing on the reader thread:**
  - `Connection::handle`'s `DATA` arm calls `sink.lock().output(&bytes)` (`connection.rs:1110-1113`).
  - `Sink::output` and `feed_replay` call `parser.advance` (`server.rs:72, 92`). Nothing catches a panic.
  - The reader is one thread per endpoint, so a panic there ends the reading for every tab on the endpoint.
- **Outgoing input** (`src/remote/local/`):
  - `Connection.outgoing` is an unbounded `mpsc::Sender<ToCore>` (`connection.rs:442`, made at `:510`, fed by `send` at `:690-693`).
  - `write_loop` (`:1536-1568`) and `write` (`:1592-1657`) hold each message while the link is down.
  - `Running::input` cuts input into 64 KiB `DATA` frames (`:1884-1888`).
  - Before that, `Phase::Starting.input: Vec<Vec<u8>>` (`session.rs:33-34`) holds input for as long as connecting and bootstrapping take.
- **Holder** (`remote-agent/src/server/holder.rs`):
  - `read_output` (`:875`) and `update_interests` (`:1093`) each test `queued < CORE_QUEUE_HIGH` (8 MiB) and `paused` in their own way, with no low mark.
  - `input` (`:818-833`) drops input past `INPUT_QUEUE_LIMIT` (4 MiB) with an `ERROR`.
  - `process_core_frames` (`:601-619`) calls `handle` (`:634`) with no guard. A panic there ends the holder, and with it every program it holds. The core's emulator is guarded already (`core/screen.rs`, `Screen::guard`).
- **Refusals:**
  - the core: `read_pending` (`core/clients.rs:130`) and `welcome` (`:155`) call `flush_for(REFUSAL_FLUSH_LIMIT)`, then drop the source;
  - the holder: `read_pending` (`:456`) calls an unbounded `flush`, and `take_over` (`:477`) calls `flush_for(1 s)`, each then dropping the source.
  - `Conn::close_for` (`conn.rs:199`) exists and is what the holder's `shutdown` uses.
- **Latency:** `samples` and `all_samples` (`src/latency.rs:232-239`) grow for the life of the run.
- **Config writes:**
  - `write_default` (`config.rs:430-436`) is a plain `fs::write`.
  - `config_edit::save` (`config_edit.rs:20-41`) renames a temp file into place, but with one fixed `.tmp` name and no `sync_all`.

### What the plan missed or got wrong

1. **The `process R15` entry is b23's (`config.rs:434`), not b26's.** `session::write_atomic` (`session.rs:321`) is specific to the state file (its name, the generation lock and private permissions), so it can't be reused. The config gets its own small helper (decision 9).
2. **"Drops only redundant Wakeups" bounds nothing else.** An OSC notice, a status report, a bell or a query reply is one entry each, and a program can send them at any rate. Merging at the source takes care of the events whose latest value is all that counts. Marks govern the rest (decision 2).
3. **b12 lists three holder entries on the lines this change edits:**
   - `process R9`: no low mark;
   - `process R10`: `:456` and `:477`;
   - `fcis R1`: the two copies of the read policy.
   Leaving them would mean editing those lines twice, so they are in (decisions 6 and 7).
4. **The b26 smell, `Phase::Starting.input`, is the same queue one phase earlier.** It is bounded by the same budget (decision 3).
5. **A holder fault must not end a session.**
   - An `EXIT` closes the zone or tab without a word (`zone_view.rs:526`, `workspace.rs:977`).
   - So a holder that hung up a session on a fault would make its tab vanish. The holder answers with `SPAWN_FAILED` or `ERROR` instead, and the program keeps running (decision 5).
6. **A tab whose emulator panicked can't keep its screen.**
   - A panic inside a `Term` update can leave its grid inconsistent, and the UI thread paints that grid. A panic there ends Brindle.
   - So the tab's `Term` is replaced, unlike a lost tab's, which keeps its last screen (decision 4). This is why the spec's requirement for it is its own, not more text in "A session lost for good is reported in the tab".

## Goals / Non-Goals

**Goals:**
- Every queue F8 names either merges down to one waiting entry or has a high and a low mark, and says what it drops or refuses past the high one (`process R9`).
- A panic in per-session code on a shared thread ends at most that session's view in Brindle, or that one message in the holder (`process R3`).
- A refused peer reads its `ERROR`, then the end (`process R10`).
- The default config is never left half-written (`process R15`).
- A Windows core keeps the directory a program reports with OSC 9;9, so its replays carry it (decision 10). This depends on change 5's `PathStyle`.

**Non-Goals:**
- **Showing a server `ERROR` about a session in its tab.** That covers the holder's fault reports and its "dropped some" input refusal. It is change 12's (`railway R6`), which depends on this change. Until then they are logged, as today.
- **The core's client-queue hysteresis and its `Pending` flags.** These are b10 #7 and change 13.
- **Making the holder's own input cap lossless.** The holder has no policy (`process R1`). Brindle's budget sits in front of it with the same 4 MiB, so the holder's cap is only reached by input Brindle already counted. The cap stays, documented as lossy under `process R9`.
- **`Terminal::feed`, the read-only message tabs.** It parses Brindle's own text on the UI thread and stays as it is.
- **Any wire, config or state-file change.**
- **Ever blocking a producer on the UI.** A reader thread that waited would stall every tab on its endpoint.

## Layers

| Part | Layer | Where |
|---|---|---|
| `QueueMarks` (the hysteresis rule) | pure | `src/queue_marks.rs` (new) |
| `EventGate`, `Kind`, `Merge`, `Admit`, `Recovered`, `kind_of` | pure model | `src/terminal/events.rs` (new) |
| `InputBudget`, `Offer`, `Refusal`, `Delivery` | pure model | `src/remote/budget.rs` (new; compiled and tested on every platform, `fcis R7`) |
| `Listener` through the gate; `spawn_events`; `Terminal`'s `Broken` and `InputRefused` arms | shell (reader thread, entity) | `src/terminal/mod.rs`, `server.rs` |
| `Connection::deliver` (the sink guard); `Outgoing`; the writer's `handled` | shell (reader and writer threads) | `src/remote/local/connection.rs` |
| `Session::input` across phases | shell | `src/remote/local/session.rs` |
| `Window` (bounded samples) | pure | `src/latency.rs` |
| `write_replacing` | shell (file IO, no decision) | `src/config.rs` |
| `core_backlog`, `read_limit`, `unit_of`, `Unit` | pure functions with tests | `remote-agent/src/server/holder.rs` |
| `Holder::guarded`, `Holder::fault` | shell (loop handler) | `holder.rs` |
| `fault::describe`, `fault::marker` | helpers for both roles | `remote-agent/src/server/fault.rs` (new; from `core/screen.rs`) |
| `Assembler::abandon` | codec helper, no encoding change | `remote-protocol/src/holder.rs` |
| `Emulator::new(.., style: PathStyle)` | pure | `term-core/src/emulator.rs` |
| `PATH_STYLE` (`Posix` / `Windows`) | platform seam constant | `remote-agent/src/server/platform/{unix,windows}/pty.rs` |

- The only platform seam item is `PATH_STYLE`, one constant per platform module under the same name (`ports R5`). `catch_unwind` and `Conn::close_for` are platform-neutral, and no `cfg` is added outside the seam (`ports R4`).
- No crate reference changes, so `tests/architecture.rs` holds as is.
- The hysteresis rule is two lines, so the holder keeps its own copy: the app can't depend on the agent, and the rule is neither wire nor terminal state, so it fits no shared crate (`ports R3`).

## Decisions

### 1. One hysteresis rule

```rust
pub struct QueueMarks { high: usize, low: usize }   // const fn new(high, low): low < high
impl QueueMarks {
    /// Whether a queue at `level` is over, given whether it was.
    pub fn over(self, was_over: bool, level: usize) -> bool  // was_over ? level >= low : level >= high
}
```

The rule returns a new state from the old one and the level, so a caller can't skip the low mark. The event gate and the input budget both use it, and the holder's `core_backlog` is the same two lines.

### 2. Terminal events: merge at the source, mark the rest

Every event gets a kind before it is queued:

| Kind | Events | Past the high mark |
|---|---|---|
| `Merged(Wakeup)` | `Wakeup` | at most one waiting, never dropped |
| `Merged(Title)` | `Title`, `ResetTitle` (one slot, `Option<String>`) | at most one waiting, latest value wins |
| `Merged(Foreground)` | `ChannelEvent::Foreground` | at most one waiting |
| `Merged(Clipboard(ty))` | `ClipboardStore` per `ClipboardType` | at most one per selection, latest text wins (each replaces the last) |
| `Life` | `Replay(bool)`, `Lost`, `Failed`, `Broken(String)` (new), `InputRefused` (new), `Exit`, `ChildExit` | always queued |
| `Side { status: false }` | `Bell`, `PtyWrite`, `ColorRequest`, `TextAreaSizeRequest`, `ClipboardLoad`, `Osc(Notice)` | dropped |
| `Side { status: true }` | `Osc(Status)`, `Osc(PromptStart)`, `Osc(Reset)` | dropped, and noted |
| `Ignored` | `MouseCursorDirty`, `CursorBlinkingChange` | never queued: the consumer has no arm for them (`handle_event`'s `_ => {}`) |

`kind_of(&ChannelEvent) -> Kind` names every variant of `ChannelEvent`, `Found` and alacritty's `Event`.

**The model.** `EventGate` (pure, `src/terminal/events.rs`):
- It holds:
  - `counted`: the number of `Life` and `Side` events waiting;
  - `over`;
  - the dropped count and whether a status report was among them;
  - one waiting flag per `Merge`;
  - the title and clipboard slots.
- `admit(kind) -> Admit` returns one of:
  - `Queue`;
  - `Merged`: an entry of that kind is already waiting, so the event only updates its slot;
  - `Dropped`;
  - `Ignored`.
- `took(kinds) -> Option<Recovered>` is called once per chunk the consumer takes. It lowers the count and clears the merge flags.
  - The first time the count falls under the low mark after events were dropped, it returns `Recovered { dropped: u64, statuses: bool }` and resets those counters.
- `take_title()` and `take_clipboard(ty)` hand the consumer the latest values.
- The marks are `QueueMarks::new(2048, 512)`:
  - The consumer takes up to 1024 events a chunk, and as many chunks as are ready within 16 ms. So a UI that keeps up never has more than one chunk waiting, and 2048 is reached only when the UI thread is more than two chunks behind, or a program floods side events faster than the UI can apply them.
  - 512, half a chunk, lets the next update clear the state once the UI catches up.
  - Memory: side events carry at most a 4 KiB OSC payload (term-core's `MAX_PAYLOAD`) or a short reply. So the queue holds at most about 8 MiB, and kilobytes normally. Merged events add one slot each.

**The shell:**
- `Listener` becomes `{ tx, output: Arc<OutputMarker>, gate: Arc<Mutex<EventGate>> }`, with `Listener::send(ChannelEvent)`.
  - A merged kind puts its value in the gate's slot, then queues a `ChannelEvent::Merged(Merge)` marker only if `admit` says `Queue`.
  - `Sink::send` and the `EventListener::send_event` impl both go through it.
  - `send_event` still advances the `OutputMarker` on a `Wakeup` first, on the parsing thread.
- The gate's mutex is taken while the producer holds the parser and term locks (alacritty calls `send_event` mid-parse). It is a leaf: nothing else is locked while it is held, and `unbounded_send` doesn't block. `process R14`'s lock-order note says so.
- The channel type stays unbounded, since the gate is what bounds it.
- `spawn_events` reads each chunk's kinds, calls `took` once, and applies the events in order:
  - a `Merged` marker takes its slot's latest value;
  - `Batch::coalesce` goes;
  - the match names every `ChannelEvent` variant, with no `other =>` arm (`railway R3`; b28 #15's arm goes with it).
- On `Recovered`, the terminal logs the count at `warn`. If a status report was dropped, it calls `Records::reset` and emits `TerminalEvent::Status(None)`, so the tab's dot is recomputed instead of left stale.

**Alternatives rejected:**
- *A bounded channel* blocks the reader thread, which stalls every tab on the endpoint, or else drops by position, which can drop a `Lost` or a `Replay(false)`.
- *Merging only*, the plan's wording, leaves OSC and bell floods unbounded.
- *Dropping output* is never needed: output goes straight into the grid under the term lock, and only the `Wakeup` that announces it is queued.

### 3. Input: a per-session budget

**The model** (`src/remote/budget.rs`, pure):

```rust
pub const INPUT_MARKS: QueueMarks = QueueMarks::new(4 << 20, 1 << 20);
pub struct InputBudget { waiting: usize, over: bool }
pub enum Offer { Take, Refuse(Refusal) }
pub enum Refusal { First, Again }
pub enum Delivery { Queued, Refused(Refusal), Discarded }
impl InputBudget {
    pub fn offer(&mut self, len: usize) -> Offer;  // under the high mark: Take and count it; going over: First; over: Again
    pub fn handled(&mut self, len: usize);         // written or skipped; back under when waiting < low
}
```

- **Marks: 4 MiB high, 1 MiB low.**
  - 4 MiB matches the holder's `INPUT_QUEUE_LIMIT`, so the server never drops input Brindle took for a live session.
  - 4:1 is the ratio of the core's client marks (`process R9`).
  - Typing during even a long outage is kilobytes, so only pastes and runaway replies reach it.
- **A call is taken whole while less than the high mark is waiting.**
  - So a 6 MiB paste into a connected tab goes through, and the bound on waiting input is the high mark plus one call.
  - Cutting a paste in two would deliver half of it, which is worse than all or nothing.
- **`First` is the refusal that finds the budget going over.** `Again` covers the rest until it is back under. That gives one line in the tab per crossing.

**The shell:**
- **Ownership.** `Session` owns one `Arc<Mutex<InputBudget>>` across its phases.
  - In `Starting`, `Session::input` offers the bytes and pushes them on `Take`.
  - At the handoff, `run()` gives the budget to the `Running` it made. The held input goes out through `Running` without a second offer, since it was counted already.
- **`Running::input`.**
  - It checks `State::shows(id, key)` first (change 2) and returns `Delivery::Discarded` if that fails. A lost tab's input is neither counted nor sent.
  - Otherwise it offers the bytes, and on `Take` queues its 64 KiB `DATA` frames as `Outgoing { message, backlog: Some((budget, len)) }`.
- **The writer** calls `handled(len)` once a frame is written, or once it skips one (a withdrawn request, a given-up close).
- **Other messages are never refused, and carry no backlog:** `RESIZE`, `CLOSE`, `DETACH`, `SET_LAYOUT`, `REPARENT`, `SPAWN`, `ATTACH`, `LIST` and `KILL`.
  - Each is small and made by the user or by the once-a-second layout tick, which sends only changes.
  - The quit path must never lose a `DETACH` or `CLOSE` to a budget (`process R16`).
- **Replies count too.** `Terminal::write` is the one way into `Session::input`, so the emulator's replies count against the same budget: they are input to the program.
- **The line.**
  - On `Refused(First)`, `Terminal::write`, which has no context, sends `ChannelEvent::InputRefused` (a life event) through a clone of its `Listener`, which `Terminal` now keeps.
  - The event task writes `\r\n\x1b[2minput not sent: 4 MiB of earlier input is still waiting to reach this session\x1b[22m\r\n` through parser → term → marks, as `feed` does, and notifies.
  - `Again` adds nothing.

**Alternative rejected:** *a bound on the connection's whole queue*. It would let one tab's paste refuse every other tab's typing, and the queue has no per-session view to name the tab that caused it.

### 4. A panic in a session's sink detaches that session

**Where.** `Connection` runs every call into a session's sink through one guard: `deliver(id, key, &sink, |sink| …) -> Delivered`, with `enum Delivered { Done, Panicked(String) }`. That covers:
- `output` (`DATA`);
- `replay` (`REPLAY_START`, `REPLAY_END`);
- `exited` and `lost` (`EXIT`, the listing's prune, `ended`);
- `foreground` (`PROCESS`);
- `replay_size` (`SESSIONS`);
- the lost line in `lost_link`.

**On `Panicked(reason)` for a session still shown:**
1. `State::forget(id, key)` (change 2).
2. `send(ToCore::Detach(Detach { session: id, layout: None }))`. If the link is down, the writer holds it like any other message.
3. Outside the state lock, a second guarded call: `sink.broken(&reason)`. A panic in that call is only logged.

A sink no longer shown (`ended`) gets only the `broken` call.

**Why `DETACH`:**
- The program is fine; only Brindle's view of it failed, so the session must outlive the view. `DETACH` turns a `kill` session (`-e`) into `detach`: Brindle never ends a program because of its own fault.
- Re-attaching on its own could loop, if the panic comes from bytes the replay carries again. So attaching again is the user's choice, from "Attach Session…" or at the next launch.

**`SessionSink::broken(&mut self, reason: &str)`** is a new trait method. `Sink::broken` sends `ChannelEvent::Broken(reason)` and parses nothing.

**`Terminal` on `Broken(reason)`:**
- It replaces its emulator state under parser → term → marks: `Term::new(term_config(config), &size, listener)`, a new `Parser` and default `Marks`. It also resets its `Records`.
- It writes `\x1b[31mBrindle can't show this session any more:\x1b[0m {reason}\r\nIt keeps running, detached: attach it again from Attach Session…\r\n`.
- It calls `mark_lost`. The tab stays open and `exited` is set, so the layout tick and the state file leave it out, and quitting sends nothing for it.
- Its `Running` no longer passes `State::shows`, so a close, a resize or input reaches nothing (the MODIFIED "A session lost for good" requirement).

**The fault switch.** In debug builds, `BRINDLE_TEST_SINK_PANIC=<marker>` makes `Sink::output` panic when the bytes it is given contain the marker.
- `terminal/server.rs` reads it once through a `OnceLock`, as `core/screen.rs`'s `inject_fault` does (`process R20`).
- Tests print the marker with one `printf`, so it arrives in one read.
- `connection.rs` gets its own payload-to-text helper, since the app can't use the agent's.

**Unwinding.** Neither Brindle nor the helper sets a panic hook, and no Cargo profile sets `panic = "abort"`. The guards here and the core's `Screen::guard` rely on both. `tests/architecture.rs` gains a check that no profile sets `panic = "abort"`.

### 5. The holder guards each message

**Where.** `process_core_frames` calls `self.guarded(message)` in place of `self.handle(message)`:
1. It computes `let unit = unit_of(&message)`.
2. It runs `catch_unwind(AssertUnwindSafe(|| self.handle(message)))`.
3. On `Err(payload)` it calls `self.fault(unit, fault::describe(&*payload))`.

**`unit_of` is pure** and names every `ToHolder` variant (`railway R3`):

```rust
enum Unit { Spawn(SessionId), Session(SessionId), Checkpoint(SessionId), Holder }
// Spawn → Spawn; Input, Resize, Hangup, Reparent, Pause, Resume, SetMeta → Session;
// CheckpointBegin, CheckpointChunk, CheckpointEnd → Checkpoint; Hello, Drain, Goodbye → Holder
```

**`fault(unit, what)`** logs one line naming the session, the message kind and `what`, then:
- `Spawn(id)`:
  - If the session was already inserted, it is hung up with `hang_up(id)`, as a failed spawn's would be. Before that point, the unwind has dropped whatever `spawn` had made, including a started program's PTY.
  - Either way, it answers `SPAWN_FAILED { session: id, message: "the holder failed while starting it: {what}" }`. The core turns that into the client's spawn error, which Brindle already shows as "Brindle could not start…".
- `Session(id)`: `ERROR { session: Some(id), message: "the holder failed handling {kind}: {what}" }`. The session keeps running.
- `Checkpoint(id)`: `core.checkpoints.abandon(id)`, then the same `ERROR`. The stored checkpoint and tail stay, so a later core rebuilds from the last whole checkpoint (`process R2`).
- `Holder`: `ERROR { session: None, … }`. The core connection stays: dropping it would make the core count every one of the holder's sessions lost.

**Why the session keeps running.**
- The holder's job is to keep programs alive (`process R1`).
- Its per-session state is the PTY, the input queue, the tail and the stored meta. These are byte buffers with no cross-field invariant that a half-run handler could break in a way worse than ending the user's program.
- And ending it would close its tab with no message (Context, item 5).

**The fault switch.** In debug builds, `BRINDLE_TEST_HOLDER_PANIC=<marker>` makes `spawn` panic when an argument of the request contains the marker, and `input` when its bytes do.
- `server/fault.rs` holds `describe`, the payload-to-text code moved from `core/screen.rs`, and `marker(var)`, the debug-only `OnceLock` read, used by both roles.

**`Assembler::abandon(&mut self, session) -> bool`** drops a session's open blob. It is a codec helper in `remote-protocol/src/holder.rs` with a unit test, and the encoding is unchanged.

### 6. The holder's read policy: a low mark, one function

- **`CORE_QUEUE_LOW` is 2 MiB**, a quarter of `CORE_QUEUE_HIGH`, the ratio `process R9`'s own example uses. Reading resumes while the core still has well over a megabyte queued, so it doesn't starve between turns.
- **`Core` gains `over: bool`.** The queue belongs to one core connection, and a new core's connection starts empty and under the mark.
- **Two pure functions** replace the two inline copies:
  - `core_backlog(over, queued) -> bool` (decision 1's rule);
  - `read_limit(dest: Dest, paused: bool, chunk: usize) -> usize`, with `enum Dest { Core { over: bool }, Tail { room: usize } }`. `Core` gives 0 when over or paused, else `chunk`; `Tail` gives `room.min(chunk)`, ignoring `paused`, which only a core sets.
- **Callers:**
  - `read_output` updates `over` before each read and breaks on a limit of 0.
  - `update_interests` updates it after `flush_core`, and sets `readable` from `read_limit(.., 1) > 0`.
- **Past the high mark nothing is dropped.** The programs block on their output, as today; nothing is shown, since that is ordinary backpressure.

This closes b12's `process R9` and `fcis R1` entries.

### 7. Refusals close gracefully

- The core's `read_pending` (`clients.rs:130`) and `welcome` (`:155`) call `conn.close_for(REFUSAL_CLOSE_LIMIT)`, the renamed `REFUSAL_FLUSH_LIMIT` (1 s), before `drop_source`.
- The holder's `read_pending` (`:456`, unbounded today) and `take_over` (`:477`) do the same with a holder `REFUSAL_CLOSE_LIMIT` of 1 s.
- `close_for` flushes, shuts the write side once empty, then reads and discards until the peer's end or the limit. So the peer reads its `ERROR`, then the end, even with bytes of its own still unread (`process R10`).
- A refusal still stalls the loop for at most 1 s, the same bound as today's `flush_for`. Peers are same-user processes, checked before anything is read.

### 8. Latency samples: a window

- `Window` (pure, in `src/latency.rs`) is a `VecDeque<Sample>` of at most `SAMPLE_WINDOW = 10_000` samples, plus a `dropped: u64`. Pushing past the window drops the oldest.
- `samples` and `all_samples` are both windows.
- The header stays `--- latency samples=N timeouts=T`, with ` dropped=D` added only when D > 0. `tests/perf/report.rs`'s `parse_latency` reads fields by name and ignores others, and a perf run takes far fewer samples, so the suite is unaffected. The JSON log still gets every sample.
- 10,000 samples are about 1 MB per list. A summary of a long session then describes its recent keys, which is what a reading of "now" wants.

### 9. The default config is written atomically

- `config::write_replacing(path, text) -> io::Result<()>`:
  1. writes `<name>.<pid>.tmp` beside the target;
  2. calls `sync_all`;
  3. renames it over the target;
  4. removes the temp file if the rename fails.
- `write_default` and `config_edit::save` both use it. `save` keeps its own canonicalization, so a symlinked config is written through.
- The pid in the name keeps two processes' saves from sharing one temp file.
- The `!path.exists()` check in `load_or_default` stays. Two Brindles racing past it with no config both write the same `DEFAULT_CONFIG`, and each rename is atomic.
- The file's format and keys are unchanged (`process R15`).

### 10. The core's emulator reads its programs' platform paths

This was added at the user's request. It resolves change 5's Open Question, "A Windows core's replays drop a 9;9 directory".

**Today.**
- Once change 5 lands, `Emulator::new` builds `Parser::new(PathStyle::Posix)` on every platform: today's `Parser::posix()`, made explicit.
- `reported_dir(body, style)` (`term-core/src/shell_integration.rs:277`) checks absoluteness in that style, so a Windows core drops `9;9;C:\src`.
- `replay::render` re-emits only what `Marks::reported_dir` holds, as `ESC ] 7 ; file:///C:/src ST` (`dir_uri`, `replay.rs:727`).
- So a Windows session's replay, and a new core's emulator rebuilt from a checkpoint, have no directory until the shell reports one again.
- OSC 7's `/C:/…` survives either way: it is absolute in POSIX terms, and the client's Windows-style parser decodes it.

**The change.**
- **The constructor.** `Emulator::new(cols, rows, scrollback, style: PathStyle)` passes `style` to `Parser::new`. term-core still knows no platform (change 5's rule).
- **The constant.** Each platform's `pty` module defines `pub const PATH_STYLE: PathStyle`: `Posix` in `server/platform/unix/pty.rs`, `Windows` in `server/platform/windows/pty.rs`. The constant sits in `pty` because the paths are the ones the PTY's programs report, a concern that module already owns.
- **The caller.** `core/screen.rs`'s `Screen::new` passes `platform::pty::PATH_STYLE`. The core names no platform type and adds no `cfg`. The choice lives in the seam (`ports R4`, `R5`), and the decision itself stays a pure function of a parameter, tested on every platform (`ports R7`, `fcis R7`).
- **Replays.** A Windows core then keeps `C:\src` from either report. `render` writes it as `file:///C:/src`, which Brindle's Windows client already reads back as `C:\src`.
- **Linux** passes `Posix`, so nothing changes there.
- **Remote endpoints** run the Linux core, so they are unaffected too.

**What the types rule out.** An emulator whose path style nobody chose: `Emulator::new` has no default, so the core, the tests' `support` emulator (`remote-agent/tests/support/mod.rs:1093`) and term-core's own tests each name one.

**Restart, crash, upgrade (`process R1`–`R3`, `R12`):**
- **The holder** is untouched: it stores checkpoints and tails as opaque bytes (`process R1`).
- **A core restart.**
  - The new core feeds a fresh `Emulator` the stored checkpoint, then the tail.
  - A checkpoint rendered by a new-build Windows core carries the directory as OSC 7, which the next core reads back in any style, so the directory survives restarts (`process R2`).
  - Nothing new is stored, and `SET_META` is unchanged.
- **The first upgrade.**
  - The new core adopts checkpoints rendered by an old core, which hold no 9;9 directory.
  - It does read any 9;9 report in the tail since that checkpoint. Otherwise the directory comes back with the shell's next report, as today.
  - Nothing breaks either way.
- **Fault confinement (`process R3`).** Parsing in the Windows style runs inside the same per-session `Screen::guard` as before.
- **A Brindle crash** changes nothing here: the state is the core's.

**On the wire.** Nothing changes. The replay already carries OSC 7, and now carries it in one more case (`process R4`–`R6`). No transcript changes.

### 11. Failures and where they show (`railway R6`)

| Failure | Shown on | How |
|---|---|---|
| Side events dropped | that terminal | a `warn` log line with the count; status records reset and the tab's dot recomputed when a status report was dropped; output and the screen are whole |
| Input refused | that tab | one dim line per crossing; earlier input still delivered in order |
| Brindle's emulator panics | that tab | a red line in place of the screen; the session detached, running and listed; the endpoint's other tabs unaffected |
| Holder fault on `SPAWN` | the new tab | "Brindle could not start…" with the cause, through `SPAWN_FAILED` |
| Holder fault on another message | holder and core logs; an `ERROR` for the session reaches Brindle, which logs it | in the tab with change 12 |
| A refused connection | the peer | it reads the `ERROR`; Brindle shows refusals as today |
| Writing the default config fails | the log | as today; the defaults are used |

### 12. Wire, versions, files

- Nothing on the wire changes: no message, field or version in the client protocol or H1, and no transcript changes (`process R4`–`R6`).
- `SPAWN_FAILED` and a session's `ERROR` are H1 messages every core version handles (`core/holders.rs`, `handle_holder`).
- `Assembler::abandon` is a helper on the decoder side, and its encoding is unchanged.
- `config.toml` keeps its format. Writing it through a temp file is what `process R15` asks of a file read across versions.

### 13. Core restart, Brindle crash, upgrade (`process R1`–`R3`, `R12`)

- **Holder changes** (the guard, the low mark, the graceful refusals) run only in holders of the new build.
  - A holder is never replaced while it has sessions, so sessions started before an upgrade stay in their old holders, unguarded, until those drain empty.
  - New sessions go to new-build holders.
- **A core restart:**
  - `Core.over` belongs to the connection. The next core's connection starts empty, so it starts under the mark.
  - Open checkpoint blobs are already per connection (`core.checkpoints`).
  - Nothing new needs checkpointing or `SET_META`.
- **The core's graceful refusal** takes effect from the next core start: an upgrade's `STEP_DOWN`, or the next idle exit.
- **A Brindle crash** loses the event gate, the budgets, the waiting input and any `DETACH` not yet written. These are process memory, lost as queued input is today.
  - A session whose sink-panic `DETACH` never went out stays owned by the dead client.
  - Profile tabs already run with the `detach` policy and stay. An `-e` tab's session is hung up once its owner is gone (`RECONNECT_GRACE`, or the link's linger), as on any crash.
- **Mixed versions** meet as today, since nothing on the wire changed. A new Brindle with an old holder gets no holder guard. An old Brindle with a new holder gets `SPAWN_FAILED` or `ERROR`, both handled already.

### 14. Tests, by layer (`testing`)

- **Unit, in each file:**
  - `queue_marks`: crossing up at the high mark, staying over until under the low one, and the boundaries.
  - `terminal::events`:
    - one waiting entry per merge kind, the latest title and clipboard text winning;
    - life events never dropped;
    - side events dropped past 2048 until under 512;
    - one `Recovered`, with the count and the status flag;
    - ignored events never queued.
  - `kind_of` over every variant.
  - `remote::budget`:
    - `Take` under the high mark, then `First` then `Again`;
    - `Take` again under the low mark;
    - one call over the high mark taken whole;
    - `handled` saturating.
  - `holder.rs`: `core_backlog`, `read_limit` (both `Dest`s, `paused`) and `unit_of`.
  - `latency::Window`, `config::write_replacing` (in a temp dir: content written, no temp left, an existing file replaced) and `Assembler::abandon`.
- **Fake link** (`connection.rs`, `fake::link`):
  - A sink that panics on `DATA` gets `broken`. A `DETACH` for it is written, and its entry is gone. The other session's `DATA` still reaches its sink.
  - With the link down: input is taken up to 4 MiB, then `First` once and `Again`. After the reconnect the held `DATA` is written in order, `handled` brings the budget under 1 MiB, and input is taken again.
- **Session** (`session.rs`): input held in `Starting` counts, and the handoff doesn't count it twice.
- **Sink** (`terminal/server.rs`):
  - the `sink()` helper builds a gated `Listener`;
  - many reads queue one `Wakeup` until it is taken;
  - `broken` sends `Broken`.
- **Real processes** (`remote-agent/tests`):
  - `holder.rs`, with a new `start_holder_with(dir, env)` in `tests/support/mod.rs`, and with the platform programs the existing holder tests use, so they run on both platforms:
    - a `SPAWN` with the marker gets `SPAWN_FAILED` and the holder's other session keeps its pid;
    - `INPUT` with the marker gets an `ERROR` for that session, and both sessions keep echoing input;
    - a connection whose first frame isn't `HELLO`, and one whose `HELLO` shares no version, each followed by 64 KiB the holder never reads, read their `ERROR` and then the end (EOF, not a reset).
  - `core.rs`: the same two refusals for a client.
  - `core.rs`'s `on_windows` module: a detached session whose program printed `ESC ] 9 ; 9 ; C:\src ST` replays `ESC ] 7 ; file:///C:/src ST`, both on attach and after the core is killed and a new one adopts the holder.
- **Unit, term-core** (`emulator.rs`, run on every platform): an emulator built with `PathStyle::Windows` keeps a 9;9 `C:\src` report and its replay re-emits it. One built with `PathStyle::Posix` keeps `/home/me` from OSC 7 and drops `9;9;C:\src`.
- **End to end:** `local-server-sink-panic` (task 7.1) checks the spec's "Emulator fails on one zone's output".
- **Not end to end:**
  - The input bound needs more than 4 MiB through `--send`, past argv limits.
  - The event marks need a UI stall that can't be produced on purpose.
  - The unit and fake-link tests above cover both (`testing R1`).
- **Manual:** the refusal line after a real paste into a remote tab whose host was paused (tasks, "Manual checks").

## Risks / Trade-offs

- [A dropped query reply can leave a program waiting for an answer] → Only past 2048 waiting events. Programs that query (DA, OSC 11, OSC 7501 `?`) time out, and the drop is logged with its count.
- [The refusal line lands in a live program's screen] → The same as today's "connection … lost" line. It comes once per crossing, and a reconnect's replay redraws the screen.
- [A panic mid-update leaves the `Term` half-changed] → It is replaced and never painted again. The last screen is lost, but the core still has it, and attaching again shows it.
- [The test marker in the core's replay would panic a re-attached tab again in tests] → No test re-attaches a broken session. The switch is debug-only.
- [A holder handler that panics on every message for one session sends an `ERROR` per message] → Bounded by the core's message rate. It is one log line each, and change 12 shows them.
- [Unwinding is assumed] → The `tests/architecture.rs` check on `panic = "abort"`.
- [`close_for` stalls the loop for up to 1 s per refused connection] → The same bound as `flush_for` today, and only for a same-user peer.
- [Lock order] → The gate's and the budget's mutexes are leaves, taken last and holding nothing. Noted under `process R14`.

## Migration Plan

Nothing to migrate: no wire, config or state-file change. Holders of older builds drain as they always do. A revert restores today's behaviour.
