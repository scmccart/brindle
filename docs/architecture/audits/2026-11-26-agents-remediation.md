# Remediation plan for the 2026-11-26 agent code review

- **Report:** [`2026-11-26-agents.md`](2026-11-26-agents.md), of commit `5f335dc`. Batch reports in [`2026-11-26-agents/`](2026-11-26-agents/); "D-1" below means finding 1 of `d.md`.
- **Planned against:** `main` at `5f335dc`. `agents-commands` (PR #151) has since merged as `ba18a14`; it changes `acp/reduce.rs`, `src/agent/model.rs` and the view, so the line numbers the changes cite are from before it, and each change's first task re-opens the cited sites.
- **Output:** one OpenSpec change per group below, in this order, each sized for one PR. The order puts what a user meets first, then the server's safety and correctness, then the split that the last two changes build on.

## Decisions

| # | Question | Decision | Lands in |
|---|---|---|---|
| D1 | The client's fold: have the UI thread own the only transcript, or keep the reader thread's fold and shrink what the UI does under its lock? | **Keep the reader thread's fold and shrink the lock** (answered 2026-11-26). Under `AgentShared`'s lock the zone takes the changes and copies the changed entries' tails; it renders after releasing it. Two copies of the log stay, and the recorded bound is corrected. | `agents-absorb-cost` |

| D2 | Must agent checkpoints written by development builds before these changes still be read? | **No** (answered 2026-11-27). Formats 1 and 2 are unreleased, and losing once the conversations that are running at that reinstall is acceptable. A change that alters what a checkpoint holds refuses the earlier form: the session is adopted as not restored, its agent left running. | `agents-checkpoint-deltas` (no reader for the earlier shape or format 1), `agents-process-lifecycle` (a checkpoint that breaks the permission invariant is refused), `agents-event-bounds` (no repair beyond the fold's clamps) |

Choices the changes make and flag in their designs, for review with each change:

| # | Choice | Lands in |
|---|---|---|
| C1 | A text entry past 1 MiB ends and its text continues in a new entry of the same kind, so nothing is cut and the trim works on whole entries. The zone shows two bubbles for such a reply. | `agents-event-bounds` |
| C2 | One oversized event fails its session through the existing fault path, and the failure names the event's kind, its entry, its size and the limit. The bounds make this unreachable; the guard makes a future mistake loud. (Simplified on 2026-11-27 from "replaced by a notice entry, and the rest of its batch is sent", which needed recovery logic at four call sites.) | `agents-event-bounds` |
| C3 | The holder reads a pipe session's stdout to its end before it finishes the session. This adds a drain rule to `holder/policy.rs` and no parsing (`process R1`). | `agents-process-lifecycle` |
| C4 | Checkpoint format 2 and client protocol 4 are amended in place; neither has shipped (v0.3.0 predates both). | `agents-checkpoint-deltas`, `agents-event-bounds` |
| C5 | D-12 disputes `agents-login` decision 9's list of commands that re-parse their arguments. The change adds `pwsh`/`powershell -Command` and `wsl` to it. | `agents-client-state-fixes` |

## The changes

### 1. `agents-client-state-fixes`

Findings: **D-1** (high), D-4, D-5, D-6, D-7, D-10, D-12, and D-11's silently dropped choice.

- **D-1:** `Requests` returns `Result<RequestId, NotSent>` with `NotSent::{Starting, Gone, LinkDown}` in place of `Option<RequestId>`. The session tells its sink when `Phase::Running` is set (a life event, so the channel orders it after everything the reader sent before it), and the model keeps the first prompt pending, and `prompt_status` at connecting, until it has seen that event. A send that meets `Starting` is held, never refused. The e2e flake is reproduced first with a fault switch that delays the session thread, and the fix is shown by numbers (`testing R9`, `R10`).
- **D-4:** a prompt whose reply failed with the link is `Sent::Uncertain`, not refused. The reconnect's replay decides: if the replayed log or the slot holds the text, the draft is cleared; otherwise the refusal shows then.
- **D-5:** `State::link_lost` also fails queued, unwritten agent fetches, so no fetch crosses a link.
- **D-7, D-6:** the ledger is the only record of a fetch in flight (`Earlier::Loading` carries no request id); `first` and `first_shown` become one enum; a new log clears the thought and tool-row toggles.
- **D-10:** `Changes` records where the replay ended, and notices and the first prompt are decided on the live part only.
- **D-11 (part):** `choose` returns `Choose::{Send, Refused(text)}` and the zone shows the refusal.
- **D-12:** the withheld list (C5).

Specs: `agent-zones` (first prompt, a prompt across a reconnect, paging across a reconnect). No wire change.

### 2. `agents-zone-focus-and-scroll`

Findings: **E-1**, **E-2** (high), E-3, E-4, E-6, E-7, E-8, E-9.

- **E-1:** one rule at `AgentView`'s root: a left press nobody took focuses the prompt and prevents the default, so the zone tree never takes it. A `VisualTestContext` test in `zone_view.rs` shows the fault first.
- **E-2, E-4, E-8:** scroll anchoring moves wholly into `MessageScroller`. `splice` keeps the top item and its offset while away; any insert at the start clears `top_in_view` until a frame says the row shows; `remeasure_all` does both. `AgentView::set_profile` remeasures only when the text size changed, and a zoom does remeasure.
- **E-3:** focus landing on a permission or login stop reveals its block (`MessageScroller::reveal`).
- **E-6:** the working row becomes the model's last block, so the view applies one ordered list of splices.
- **E-7:** the view notifies on a modifier change only when `control` flips.
- **E-9:** `Source::Agent` gets its own notification bucket.

Specs: `agent-zones` (focus, reading back), `ui-components` (`MessageScroller`), `desktop-notifications`. New manual checks for the pointer presses.

### 3. `agents-event-bounds`

Findings: A-1, A-2 (B-1), B-2, B-7, C-6, D-8.

- **One invariant, stated once and tested:** every event the reducer emits, and every event of a replay or page, encodes within a frame; a full log and a full partial line fit a checkpoint. A reducer property test asserts the first over adversarial updates and permissions, and a test derives both from the constants.
- **A-1:** a per-entry limit for text entries (C1).
- **A-2:** limits count encoded bytes (a diff line as its text plus its framing, an output with its command and directory); the diff fallback stops allocating once full; options are capped in count and an option whose id is over 1 KiB is left out; location paths and terminal ids are capped. One oversized event is replaced by a notice (C2).
- **B-2:** raw input and result are formatted through a writer that stops at `RAW_LIMIT`, and cleaned after the cut.
- **B-7:** the fold enforces the tool content bounds it defines (clamping into `more`), so the core and the client share one definition.
- **D-8:** hunk line numbers use checked arithmetic on the client, and the decoder bounds them.
- **C-6:** the core stops reading a client's frames while that client's queue is over its mark.

Specs: `agent-sessions` (the log bound, tool content bounds, permission options). Amends unreleased protocol 4's bounds (C4).

### 4. `agents-process-lifecycle`

Findings: A-3, C-1, C-3, C-4, C-5, C-7, and `a.md`'s observation 6.

- **A-3:** one "this agent process is gone" step, used by `end`, `restart_for_login` and `respawn`: it answers the awaited list and records each waiting permission cancelled. `fail` records them too. The invariant, tested as a property over random steps: the log's pending permissions are exactly `Rpc`'s.
- **C-1:** a pending restart is one fact, `Life::Restarting(RestartStage::{Waiting { client }, Placed { holder }})`; `Core.waiting` holds only spawns with no session yet. `session_lost`'s decision becomes `policy::lost_fate`, pure and tested per row.
- **C-3:** after a caught reducer panic the stored checkpoint is forgotten (the next one is whole) and an attached client gets a fresh replay. The fault switch gains a point after the first `emit`.
- **Observation 6:** a step that faulted writes nothing more to the agent.
- **C-4:** a returning owner's ended agent session goes through `replay`, which ends with its `EXIT`.
- **C-5:** the answers to requests Brindle did not offer are checkpointed before they are written.
- **C-7:** the holder drains a pipe session's stdout before finishing it (C3).

Specs: `agent-sessions`, `terminal-server` (the drain). No wire change.

### 5. `agents-module-structure`

No behaviour change. Structural observations of all five reports, plus B-6 and the rest of D-11.

- The splits listed under "Structure" in the report: `reduce.rs`, `checkpoint.rs`, `transcript/src/lib.rs`, `model.rs`, the agent lifecycle out of `sessions.rs`, `workspace/conversations.rs`, `zone_view/login.rs`. Each is a move with its tests, one commit per file, and `git diff --color-moved` shows nothing else.
- Types that follow from the cuts: a `Process` value the respawn replaces whole (built on change 4's step), a `ToolBook` that owns the tool and output indexes, `chose`/`lists`/`methods` moved into the phase and the greeting, the `Agent` entity's `Held` removed by taking its requests on leave.
- **B-6:** `Dialect::failure(&RpcError) -> forms::Failure`, so no error code is named outside `v1/` (`ports R14`).
- **D-11:** the entity's remaining decisions become model methods (`leave_on_quit`, `replay_ended`), and the `_ =>` arm in `conversations.rs` is spelled out (`railway R3`).
- One exhaustive protocol-4 gate in `handle_client`; `checkpoint_screen` and `agent_checkpoint` split; `ZoneId`, `TabId` and `split_target` move to a pure module the agent's pure code may name.
- `tests/architecture.rs` gains what the cuts make checkable (the pure-module list, no ACP error code outside `v1/`), and `docs/code-map.md` follows.

Specs: none. The change says so, as `keep-pure-modules-pure` and `extract-decisions-into-models` did.

### 6. `agents-checkpoint-deltas`

Findings: B-3 (C-2), B-4, A-4, A-5, B-5, and `c.md`'s observation 6.

- **B-3, B-4:** a segment carries deltas: tool calls added since the previous record, the partial line's new bytes, and the login methods only when they changed. The decoder folds them as it folds events. A pinned synthetic conversation with thousands of permission-gated tool calls and one 8 MiB line asserts the bound that `agents-log-paging` decision 9 states.
- **Observation 6:** a whole checkpoint is serialized once into one buffer and framed from slices; a transcript event is serialized once.
- **A-4:** an output keeps its text with a start offset and compacts when the dead prefix passes the limit.
- **A-5:** a diff item is reused when its path and texts match the entry's last; a permission's call is taken by value.
- **B-5:** the `Dialect` methods take their values by value, and the decoders move fields out.

Specs: `agent-sessions` (checkpoint cost). Amends unreleased checkpoint format 2 (C4). The perf suite's `agent-attach-long` and a new tool-heavy case report the numbers before and after.

### 7. `agents-absorb-cost`

Findings: D-2, D-3, D-9, E-5, and `e.md`'s observation on per-absorb view work.

- **D-2:** absorb in two halves (D1): under the lock, take the changes and copy the changed tails; after it, render, draw and splice from the zone's copy.
- **D-3:** absorb marks entries dirty and renders them once per frame; a tool output delta appends to the zone's copy and redraws its last part. A unit test pins renders per frame of streamed input, and the perf case gains a trickle mode (chunks spaced over seconds) to measure the real case before and after.
- **D-9:** the avoidable copies go (the tool text cache's second copy of its source, user text copied into new strings), and the designs' "about twice" is corrected to what is held.
- **E-5:** `markdown::render` splits a large table or list into continuation parts, as code blocks and diffs already join; a code block's shown text is computed once.
- The model keeps pending permissions as a set updated on change, so the view's per-absorb work stops scanning every block.

Specs: `agent-zones` (reading back while streaming stays as specified; no visible change intended). No wire change.

### 8. `agents-client-request-marks`

Finding: C-6, split out of change 3 on 2026-11-27. It lives in the core's loop and shares no code with the bounds.

- The core stops reading a client's frames, and handling those already read, while that client's queue is over its high mark, and takes them up in order once it is back under the low mark. A client whose write failed still has what it sent handled (`process R10`).
- The finding is *plausible*: Brindle has one fetch out at a time. The first task shows the fault with a client that asks without reading; if it can't be shown, the change is closed with the test kept.

Specs: `terminal-server` ("Session I/O"). No wire change. Lands after change 5.

### 9. `agents-markdown-long-blocks`

Finding: E-5, split out of change 7 on 2026-11-27. It lives in the Markdown renderer and the UI components and shares no code with the absorb path.

- `markdown::render` splits a top-level table or list of more than 50 rows or items into continuation parts, so the list draws only the parts in view. The dump and copying still see one block.
- A code block's shown text is computed once. This part is made whatever the numbers show.
- The finding is *plausible*: a perf case measures a 1,000-row table in view first, and the split is made only if it is slower than the quiet baseline by more than the runs' spread.

Specs: `agent-zones` (tables, lists), `command-line` (the dump's block lines). No wire change. Lands after change 7.

## As written (2026-11-27)

All seven changes are in `openspec/changes/`, each with a proposal, design and tasks, and delta specs where behaviour changes, and each passes `openspec validate --strict`. Writing them meant re-reading every cited site, and that changed the plan above in these places; each change's design has the reasoning.

| Change | What differs from the plan |
|---|---|
| `agents-client-state-fixes` | D-1 has a second instance: a `-c` launch's list of conversations can be refused in the same window, and is gated on the same `SessionRunning` event. D-10 records where the newest turn or agent-state event came from (`StatusFrom`), not an entry id. D-4 moves `clean` into `brindle-transcript`, so the client and the core match a prompt by the same text. |
| `agents-zone-focus-and-scroll` | E-1's rule covers every mouse button. E-3 reveals the button on Tab and gives focus back to the prompt when a focused button leaves the view. E-6's working row is the model's last list item, not a block kind. |
| `agents-event-bounds` | Three more unbounded fields join A-2: the ACP session id, a stop reason's name, and `CALL_LIMIT` checked only when items are listed. C2's guard is in the reducer's `emit`, before the log applies the event, so the log, the checkpoint and every client agree. The bounds live in a new pure module, `acp/limits.rs`, with `const` assertions. |
| `agents-process-lifecycle` | C-1 is `Life::Restarting { restart, stage }` (`Ownership` and `Session::holder` already hold the client and the holder). C-5 uses a new `Moment::Answered`. A-3's `fail` answers the running agent's waiting permissions cancelled, which adds a case to `process R19`. |
| `agents-module-structure` | The model's modules go under `src/agent/model/`. The serde mirror for the checkpoint's state is rejected: `serde_json`'s key order depends on feature unification, so the bytes could change. `chose`, `lists` and `methods` stay where they are (see "Not planned"). |
| `agents-checkpoint-deltas` | B-3 is fixed by dropping the tool index from the checkpoint and rebuilding it from the log and a `tool_epoch`, not by a per-segment delta: a respawn clears the index with no event a delta could carry. Checkpoints written by today's development builds are not read (D2; the change first kept a reader for them). `agents-log-paging` decision 9's bound gains a per-record term. |
| `agents-absorb-cost` | D-3's frame gate is built only if a trickle run shows more than about 1.2 times one render a frame. The corrected D-9 bound is about four times `LOG_LIMIT` with every row open. E-5 splits top-level tables and lists into 50-row parts. |

More differences, found by the review of 2026-11-27 (below):

| Change | What differs from the plan |
|---|---|
| `agents-zone-focus-and-scroll` | E-3 also gives focus back to the prompt when a focused permission button is scrolled out of view, because gpui doesn't draw an unseen item. E-7 redraws on the link gesture (ctrl without alt), not on `control` alone. |
| `agents-event-bounds` | An output counts as its full 64 KiB towards a call's 256 KiB, so a call keeps at most three outputs (32 today). The log's measure adds 64 bytes an entry, and a replay groups a tool entry's items into pieces, both to make the checkpoint bound provable. The emit guard, first written with four callers handling a withheld entry, is now one check that faults the step (C2). |
| `agents-process-lifecycle` | C3's drain reads at most 1 MiB past a pause, not "to its end": a process left holding the pipe would make the end unbounded. `refuse` calls the closing step too. |
| `agents-module-structure` | `TabId` stays in `workspace/tabs.rs`, already a pure module. `Held` becomes a three-variant `Hold`, not "requests taken on leave". The model's modules share `ChatModel`'s fields; none owns its own. `Process` doesn't hold `Rpc`. |
| `agents-checkpoint-deltas` | `outputs` is written as a patch as well as `methods`. A-5's reuse is a digest memo, not a comparison with the entry's last texts. |
| `agents-absorb-cost` | D-9's user text copy stays (one copy in the blocks, not one per part). Splices are emitted once per update, after its statuses. Three perf cases, an e2e case and a `pace` step in the fake agent are added. |

## Review of the written changes (2026-11-27)

Read after `agents-commands` (PR #151) merged as `ba18a14`: all seven designs and proposals, the delta specs by requirement and scenario, and the tasks by section. The tasks were not read line by line, and nothing was built or run.

**Checked against the code at `ba18a14`:**
- No delta drops a scenario of the requirement it modifies, and #151 changed the text of none of those requirements.
- Change 1: a respawn builds a new sink and channel (`Agent::renew`), so an old session's late `SessionRunning` can't reach the new one.
- Change 3: hunk starts are always 1 or more (`diff.rs`), so the decoder's refusal of 0 breaks nothing; the partial line is stored as raw bytes, so I2's sum holds.
- Change 4: `Core::replay` already ends an `ExitedAwaitingOwner` session's replay with its `EXIT` and removes it.
- Change 6: the tool index has one insert (`add_tool`, keyed by the entry's own call id), one clear (`respawn`) and one retain (`trim`), as its invariant needs.

**Corrected in the changes:** the model module's path in change 7 (`src/agent/model/blocks.rs`); change 7's `Taken` renamed `ChangeCopy` (change 1 uses `Taken` in the same model); change 5's references to change 4's `Life::Restarting` and to a sentence change 6 doesn't contain; change 4's description of change 5's `Process`; change 1's `pwsh` rule (`-co` names `-Command`); change 3's spec, which didn't state the three-output limit its design makes.

**Open, for a decision before the change it touches is applied:**
1. **Three outputs a tool call** (change 3): **accepted 2026-11-27.** It follows from counting an output at its full size, which the fold's clamp needs. What is lost is fidelity in the log and the zone: outputs past the third aren't kept or shown, and the row counts them. The agent's own commands are unaffected.
2. **Checkpoints from development builds: settled as D2.** Change 6 no longer reads the earlier shape or format 1, and change 4 refuses a checkpoint that breaks its invariant (its open question 1 is now its decision 3).
3. **Change 3's guard: settled 2026-11-27.** Every oversized event, of any kind, fails the session through the existing fault path, with the cause in the failure message and the core's log (C2 above). The replacement logic and its four call sites are gone.
4. **Change 3 and change 7 were the largest: split 2026-11-27.** C-6 (the core's loop) is now change 8, `agents-client-request-marks`, and E-5 (tables and lists in parts) is change 9, `agents-markdown-long-blocks`. Sections 3 and 7 above describe those changes as first planned.
5. **Deferred work: settled 2026-11-27.** The `chose`/`lists`/`methods` move and model modules that own their fields are each a proposal-only change (`agents-reducer-phase-flags`, `agents-model-state-ownership`). The checkpoint's serde mirror, `sh -c` withholding and code fences in parts each wait on something showing up first, and stay as notes in the designs that name them.

**Requirements two changes modify.** Whichever of a pair is archived second carries the other's text in its delta; the four proposals say so.

| Requirement | Changes |
|---|---|
| `agent-zones` "Earlier messages" | `agents-client-state-fixes`, `agents-zone-focus-and-scroll` |
| `agent-sessions` "Permission requests wait for the client's choice" | `agents-event-bounds`, `agents-process-lifecycle` |
| `terminal-server` "Session I/O" | `agents-process-lifecycle`, `agents-client-request-marks` |

**Open questions.** Each of the nine designs ends with its own (some with none), about 15 in all since the review's decisions settled three, each with a recommendation; none blocks the change it is in.

## Not planned
- **`chose`, `lists` and `methods` moved into the phase and the greeting** (`a.md`, observation 5). It changes what a checkpoint records in three phases, so it is not a move, and no written change takes it. It is kept as the proposal-only change `agents-reducer-phase-flags`.
- **Model modules that own their fields** (`d.md`, observation 1). `agents-module-structure` moves the model's methods into modules and leaves the fields on `ChatModel`. The rest is kept as the proposal-only change `agents-model-state-ownership`.

- **`agents-commands` (PR #151)** was not reviewed. Its reducer and model additions join the modules of change 5 when that change is implemented.
- **A client-chosen prompt id the core deduplicates** would close D-4 at the protocol. It needs a new negotiated message; change 1's reconciliation on replay covers the case without one.
- **The agent tool-call ids cleared on a respawn** (`a.md`, "Ids"): after a resume, an update for a call from before the restart adds a new entry titled with its id. Recorded; no agent in the captures does it.
