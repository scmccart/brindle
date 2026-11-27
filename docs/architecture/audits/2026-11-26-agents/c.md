# Batch C: the terminal server's imperative shell around agent sessions

## Coverage

Read in full:
- `remote-agent/src/server/core/agents.rs`
- `remote-agent/src/server/core/sessions.rs`: lines 1-2040 (everything except the unit tests).
- `remote-agent/src/server/platform/unix/piped.rs`, `remote-agent/src/server/platform/windows/piped.rs`
- `remote-agent/src/server/holder/policy.rs`: the spawn, append, drain and `shown` policies (lines 1-200).

Read in part:
- `core/policy.rs`: 50-935. That covers the lifecycle, queue marks, placement, adoption, every fate, restart, checkpoint, page, fetch, the protocol-4 refusals, attach and returning.
- `core/clients.rs`: 160-540, which covers welcome, take_over, the read path, `handle_client` dispatch, `send_client`/`flush_client` marks and `drop_client`.
- `core/holders.rs`: 260-760, which covers the read path, `handle_holder`, adopt, blob, `drop_holder` and placement.
- `core.rs`: 340-636, which covers the loop turn, tick, the exit decision and shutdown/close_peers.
- `core/manage.rs`: LIST, STATUS and STEP_DOWN.
- `holder.rs`:
  - `Tail` and `StderrRing` (200-500)
  - `take_over`/WELCOME and the core frame handling (680-1100)
  - `spawn_pipes`, `input`, `hang_up` and its steps, `read_output`, `write_input`, `read_errors`, `reap`, `tick` (1168-1650)
  - `read_limit`/`core_backlog` (1805-1850)
- `platform/piped.rs`: the item list and the `HangUp` steps only.
- `platform/pipe_state.rs`: the head only. It is Windows overlapped-pipe bookkeeping with no agent-specific decisions.
- `remote-protocol/src/holder.rs`: the header, the message table, the constants, and the `checkpoint`/`checkpoint_append`/`split` builders.
- `remote-protocol/src/client.rs`: the header and protocol-4 notes, plus `pack_transcript`, `pack_page` and `pack_events`.
- To follow paths in other batches' files:
  - `acp/checkpoint.rs`: format, `encode`, `encode_segment`, `merged`, `state_fields`, `push_record`
  - `acp/reduce.rs`: `LOG_LIMIT`/`LOG_TRIM_TO`, `Output`, `step`'s head, `emit`/`mark`, `restart_for_login`, `refuse`, `fail`, `line`/`message`
  - `transcript/src/lib.rs`: `apply` for `Added`, `page`, `fetch`, `newest_within`, `replay_from`, `trim_to`
- Designs checked before calling anything wrong: `agents-sessions` (decisions 11, 12, as built), `agents-log-paging` (decision 9, the as-built notes, the step-down compaction), `agents-login` (decisions 8, 15) and `agents-session-recovery` (decision 6).

Skipped: the Windows overlapped pipe internals beyond `pipe_state`'s head, because they contain no agent-session decision. Fuzz and test modules were used only as evidence.

## Findings

### C-1: A client that leaves while its `AGENT_RESTART` waits for a holder leaves a zombie session that blocks `STOP` and idle exit
- Severity: medium
- Confidence: confirmed
- Category: correctness / memory
- Where:
  - `remote-agent/src/server/core/clients.rs:440` (`drop_client`: `self.waiting.retain(|waiting| waiting.client != client)`)
  - `core/sessions.rs:1409-1442` (`client_gone`)
  - `core/sessions.rs:1558-1580` (`hang_up`: `waiting && restarting(..)`)
  - `core/sessions.rs:752` (`restarting`)
  - `core/policy.rs:176-183` (`should_exit`)
- Scenario:
  1. An agent's holder is lost. The session is kept as `Life::AgentEnded`. This is the case `AGENT_RESTART` exists for.
  2. The user presses Continue. `client_agent_restart` runs `start_again` (`Life::Starting { spawn: Restart, hung_up: false }`) and pushes a `WaitingSpawn`. Placement says `Start`/`Wait` because no adopted holder of the core's build is there yet.
  3. Within the holder's start time, the client's connection ends (Brindle quits, crashes or loses its link).
  4. `drop_client` drops every waiting spawn of that client, the restart included. Compare `take_over` (`clients.rs:256-258`) and `client_stop` (`sessions.rs:1520-1524`), which both keep restarts in `waiting` so that `hang_up` reaches `abandon_restart`.
  5. `client_gone` then runs one of two ways:
     - **`kill` policy:** `gone_fate(.., starting = true, ..)` is `HangUp`, and `hang_up` finds no waiting entry. So `life.hang_up()` sets `hung_up` and sends `HANGUP` to the old, dead holder token, and `send_holder` returns `false`.
     - **`detach` policy, or after a `DETACH` (which Brindle's quit sends first):** the result is `Fate::Keep`, or the session isn't even among `sessions_of(client)`.

  Either way the session stays in `Life::Starting` forever:
  - Nothing places it, and `returning` and `LIST` skip starting sessions.
  - No `EXITED` can come for it.
  - `session_lost` frees it only if the token in `session.holder` is dropped later, and that holder is usually gone already.

  Consequences:
  - The core is never idle (`Seen::busy` counts `sessions`), so it never exits on its own.
  - A later `STOP` hangs it up again. `life.hang_up()` re-sends `HANGUP` to nothing, `Ending::Stopping { sessions_left: true }` never clears, and `STOP` never gets its `OK`.
  - Up to 16 MiB of log stays in core memory.
- Fix: keep restarts in `drop_client`, as `take_over` does: `retain(|w| w.client != client || w.spawn.is_restart())`. `client_gone`'s hang-up then takes `abandon_restart`, and a `detach` one is placed later and runs detached. Better is the structural fix in observation 1: one place that knows a restart is pending (`types R1`). Test gap: no case drops a client between `AGENT_RESTART` and its placement.

### C-2: Each checkpoint during a long line re-copies the whole partial line, and the non-log state makes segments grow with the conversation
- Severity: medium
- Confidence: confirmed
- Category: performance
- Where:
  - `remote-agent/src/server/core/agents.rs:811-823` (`since` counts `Lines::covers`, which includes the partial line)
  - `core/policy.rs:577` (`Stepped(None) => since > CHECKPOINT_AFTER`)
  - `acp/checkpoint.rs:246-264`, `309-340` (`encode_segment` writes `state_fields`: the whole `Collecting` partial, the whole `tools` map, `rpc` and `outputs`)
  - `core/policy.rs:682-684` (whole once `appended + segment > base`)
- Scenario:
  1. An agent writes one large JSON line, for example a `tool_call_update` carrying a big file's diff, or a `session/load` replay line. `MAX_LINE` allows 8 MiB.
  2. No events come until the newline. Each 512 KiB of the line fed makes `since > CHECKPOINT_AFTER`, so a checkpoint is due.
  3. Each segment carries the whole partial so far: 0.5, 1, 1.5 … 8 MiB, about 68 MiB written for an 8 MiB line.
  4. Segments soon outgrow the base. That is at once for a young conversation, and after about 7 segments for a 16 MiB one. Each time, the next checkpoint is a whole re-encode of the log plus the partial.

  With a full log that is two or three 16-20 MiB JSON encodes on the loop thread, which blocks every session's output meanwhile. The holder also stores and replaces each one.

  The same structure costs at scale in a tool-heavy conversation:
  - Every segment re-encodes the full `tools` map. That is roughly 45 bytes per tool call kept, so a few thousand calls is 100-300 KiB.
  - A segment is written at every `BeforeRequest`, at every `Permission` (twice per tool call that asks) and at every `TurnEnded`.
  - So appends stop being proportional to what changed, and compaction to a whole 16 MiB checkpoint comes every few dozen checkpoints.

  `agents-sessions` decision 11 accepted a 32× copy before paging. `agents-log-paging` set out to make checkpoints proportional to the delta, and its design doesn't mention the partial line or the tools map.
- Fix, in format 2, which hasn't shipped:
  - Make the partial line a delta: record the partial's new bytes since the previous record, and have `decode` concatenate. A `Collecting` partial only grows until its newline, and any other partial resets it.
  - Write `tools` and `outputs` as changes (added and removed ids) in segments, or leave them out of segments when unchanged.

  Both keep segments O(change) (`process R9`'s spirit: bounded work per event).

### C-3: A reducer panic after an `emit` leaves the client's fold and the next appended checkpoint without that step's events
- Severity: low
- Confidence: confirmed for the divergence. That it surfaces assumes a later `Added` in the session, for example the exit's or a lost-agent notice.
- Category: correctness / errors
- Where:
  - `remote-agent/src/server/core/agents.rs:328-348` (`guarded_step` drops the panicking step's `Output`)
  - `acp/reduce.rs:786-794` (`emit` applies to `transcript` before pushing to `out.events`)
  - `core/agents.rs:808-823` (`unsaved` and `TRANSCRIPT` take only the `Fault` step's events)
  - `transcript/src/lib.rs:1050-1056` (`Added` with `id != next` is `OutOfOrder`)
- Scenario:
  1. A reducer bug panics partway through a step that has already emitted events, such as an `Added` message entry or `Appended`s. Those events are in `acp.transcript` but not in any `Output`.
  2. `guarded_step` steps `Input::Fault` on the same session. Its `Agent`/`Turn`/`Queue` events and its `Moment::Stopped` checkpoint go out. That checkpoint is an append, because `stored` is still `Known`, so the segment lacks the lost events.
  3. The next `Added`, for example when the agent exits, has an id past what the client and the stored base+segments hold:
     - The attached zone's fold fails with `OutOfOrder`, and it shows broken instead of the fault's reason.
     - The next adoption reads the checkpoint as `Unreadable::Log`, "the conversation could not be restored".

  The `BRINDLE_TEST_ACP_PANIC` fault switch (`inject_fault`) panics before `reduce::step` runs, so no test covers a mid-step panic.
- Fix: on a caught panic, call `agent.forget_stored()`, so that the next checkpoint is a whole one of the transcript as it stands. For an attached client, send a fresh replay (`REPLAY_START`…`REPLAY_END`, which clients already take on reconnect) instead of the fault step's increments. This restores `process R3`: the fault stays confined to ACP handling, and the stored state and the client's copy stay coherent. Add a fault-switch point after the first `emit`.

### C-4: A returning owner gets only `EXIT` for an agent that ended while it was away, so its exit and stderr reason are never shown
- Severity: low
- Confidence: confirmed
- Category: correctness
- Where:
  - `remote-agent/src/server/core/sessions.rs:1356-1360` (`reattach`: `ExitedAwaitingOwner` means remove and `EXIT`, no replay)
  - `core/agents.rs:934-972` (`agent_stderr`/`agent_exited` step the exit into a log nobody is sent)
  - `core/sessions.rs:1312` (`EndFate::KeepForOwner`)
  - compare `core/sessions.rs:1943-1993` (`replay` sends the log and then `EXIT`)
- Scenario: either of two paths:
  - An agent that can't be continued exits while its `kill` owner lingers, or while an adopted owner hasn't come back. `end_fate` gives `KeepForOwner`, which is `ExitedAwaitingOwner`.
  - An agent exits while no core runs. It is adopted as exited, and its `STDERR_TAIL` steps the exit.

  Either way the core puts the exit (with the 16 KiB stderr tail explaining it) into the session's log. On the owner's `HELLO`, `reattach` sends only `EXIT`. The zone keeps its stale pre-disconnect copy and an exit code, without the reason the core holds.

  An explicit `ATTACH` of the same session goes through `replay`, which does send the log and then `EXIT`. So the two paths give different results.
- Fix: in `reattach`, send agent sessions in `ExitedAwaitingOwner` through `replay(client, id, PendingReplay::reconnect())`, which already ends with the `EXIT` and the removal. Terminal sessions keep the decided `EXIT`-only behaviour.

### C-5: Answers the core writes without a checkpoint are written again after an adoption
- Severity: low
- Confidence: confirmed
- Category: correctness / protocol
- Where:
  - `remote-agent/src/acp/reduce.rs:1289-1296`, `1311-1321` (`-32600` for an invalid request and `-32601` for unoffered methods: stdin with no `mark`)
  - `core/policy.rs:577`
  - `core/agents.rs:820-833`
  - `server/core.rs:304-325` with `core/holders.rs:304-325` (`closing_holder_frames` still steps holders' `OUTPUT` during step-down)
- Scenario:
  1. The agent sends a request Brindle didn't offer, such as `fs/read_text_file`, `terminal/create`, `elicitation/create` or an extension.
  2. The core answers `-32601` at once, but the step has no `Moment`, so no checkpoint covers that line until 512 KiB more stdout or the next moment.
  3. The core is then killed, or steps down and processes the line in `close_peers` after its step-down checkpoint.
  4. The next core adopts and re-feeds the request line from the tail, and answers again. The agent gets two responses with one id.

  Most JSON-RPC peers log and ignore a response to an unknown id, so the harm is small. The window is "until the next checkpoint", which in a quiet session can be long.
- Fix: `mark(Moment::BeforeRequest)` (or a new `Moment::Answered`) on these answers, so that `agent_apply`'s checkpoint-before-stdin order covers them as it covers permission answers. The cost is one small segment per such request.

### C-6: `AGENT_FETCH` answers bypass the client's queue marks
- Severity: low
- Confidence: plausible. This assumes a client that sends fetches faster than it reads, which Brindle's one-fetch-at-a-time zone shouldn't.
- Category: memory
- Where:
  - `remote-agent/src/server/core/agents.rs:695-733`
  - `core/clients.rs:365-393` (marks only pause holder reads)
  - `core/clients.rs:279-310` (frames are read and handled whatever `over` is)
- Scenario: each `AGENT_FETCH` (about 40 bytes) produces up to 256 KiB of `AGENT_PAGE`. The queue marks pause holder reads, but they never stop the core from reading and answering a client's own requests. A client stuck in a fetch loop, or a hostile same-user client, grows its connection's queue without bound.

  `process R9` asks every producer-to-slower-consumer queue for a mark. The same holds for `LIST` and `STATUS`, but those answers are small.
- Fix: while `entry.over` is set, either stop reading that client's frames (`update_interests` drops `readable` for an over client), or refuse `AGENT_FETCH` with "try again". The first bounds every request kind at once.

### C-7: An agent's last stdout is dropped when it exits while its session is paused or the core's queue is over
- Severity: low
- Confidence: confirmed
- Category: correctness
- Where:
  - `remote-agent/src/server/holder.rs:1828-1835` (`read_limit` returns 0 when paused or over)
  - `holder.rs:1555-1571` (`tick`: `drained` after `OUTPUT_DRAIN_IDLE`, 250 ms, then `close_io`)
  - `holder/policy.rs` (`drained`)
  - `pty.rs:37`
- Scenario:
  1. The attached client is slow, so the core sends `PAUSE`. Alternatively the holder's queue to the core is past 8 MiB, or there is no core and the tail is full.
  2. The agent writes its last ACP lines, for example an error response or a final message chunk, and exits.
  3. The holder doesn't read stdout. stderr is still read, so `last_output` moves only with it. 250 ms after the reap the session is finished, and `close_io` drops what is still in the pipe (up to 64 KiB on Linux).
  4. The core then gets `EXITED` with a log missing those lines.

  PTY sessions behave the same way. For an agent the loss is protocol messages rather than screen text.
- Fix: for a pipe session, read stdout to its end at finish, into the tail and the core's queue (it is bounded by the pipe buffer), before `close_io`. Alternatively, have `drained` wait while `paused`/over, bounded by `OUTPUT_DRAIN_LIMIT`. Both keep the holder policy-free (`process R1`): a drain rule in `holder/policy.rs`.

## Structural observations

1. **A pending restart is two facts in two places.**
   - It is `Life::Starting { spawn: SpawnRequest::Restart }` on the session, plus an entry in `Core.waiting` that holds a second clone of the same `SpawnRequest`.
   - `hang_up`, `session_lost` and `restarting` derive "waiting for a holder" from `self.waiting.iter().any(|w| w.id == id)`.
   - `drop_client`, `take_over`, `client_stop` and `fail_waiting` each edit `waiting` by their own rule. C-1 comes from that.

   I'd give the session one `Life::Restarting(RestartStage::{Waiting { client }, Placed { holder }})` (`types R1`). `waiting` would then carry only spawns that have no session yet, so a client leaving can't strand a restart.
2. **`sessions.rs` (2451 lines) carries the agent lifecycle inline.** It holds:
   - `Life::AgentEnded`, which can be represented on a `Content::Terminal`
   - `SpawnRequest::{Agent, Restart}`, with `Session::spawning` returning `None` to mean "restart"
   - `send_restart`, `start_again`, `restart_failed`, `abandon_restart`, `end_ended`
   - agent branches in `session_lost` (five), `ended`, `hang_up`, `reattach`, `replay`, `client_detach`, `client_gone`, `checkpoint` and `checkpoint_at`

   I'd split it into two parts:
   - The ownership/attach/replay machinery stays shared in `sessions.rs`.
   - The restart and ended-session lifecycle moves to `agents.rs`, behind two calls (`Content::on_end`, `Content::on_lost`) that return a pure fate.
3. **`session_lost` is a 70-line decision tree in a handler** (`sessions.rs:1037-1108`): re-place a restart, end a restart, keep a continuable agent, re-queue a starting spawn, report lost to an attached client, or exit. That is `fcis R1`. I'd add `policy::lost_fate(kind, &life, &ownership, continuable, waiting) -> LostFate`, with unit tests per row, and keep only the applying in the handler. `ended` (`sessions.rs:1290-1327`) is half-way there already through `end_fate`/`exit_fate`.
4. **The protocol-4 gate is one function called from four handlers.** Those are `client_spawn_agent`, `client_agent_restart`, `client_agent_command` and, through `agent_fetch_refusal`, `client_agent_fetch`. Nothing ties `AgentRequest` to the `ToCore` variants, so a new agent message that forgets the call is accepted below 4. I'd gate it once in `handle_client` with an exhaustive `fn agent_request(&ToCore) -> Option<AgentRequest>`, so the compiler forces every new variant to say whether it is an agent message. `SET_LAYOUT`'s `protocol < 3` check (`sessions.rs:1832`) is the same pattern, written inline.
5. **`AgentCheckpoint` leaks into terminal checkpoints.** `checkpoint_due` calls `self.checkpoint(id, AgentCheckpoint::Stepped(None))` for terminals, with the comment "A terminal's checkpoint is always whole". I'd split `checkpoint` into `checkpoint_screen(id)` and `agent_checkpoint(id, at)` at its one `match` on `Content`, and drop the parameter from the terminal path.
6. **A whole agent checkpoint makes about six copies of the log on the loop thread.** The copies are:
   - `replay()` events, with the text cloned
   - a `Value` tree
   - `state.to_string()`
   - `out`
   - `split`'s chunk `Vec`s
   - the queued frames

   That is roughly 100 MiB transient for a 16 MiB log, at every trim, every compaction, every restart or adoption, and N times at step-down. Each `TRANSCRIPT` event is also serialized twice, once to measure in `pack_events` and once to encode. The fix is serializing with `serde` straight into one buffer and chunking frames from slices. This is not a correctness issue, but it is the dominant per-checkpoint cost once C-2 is fixed.

## Answers to the batch's questions (brief)

**Loop time per handler.** None of these scales with the conversation per event:
- A stdout step costs the reducer plus encoding that step's events.
- A replay page is O(entries) for the pending-permission scan plus about 256 KiB encoded.
- A fetch is O(budget).
- `LIST` and tick are O(sessions).
- The core never reads the login environment: only the relay does.

What does scale is:
- a whole checkpoint, up to 16 MiB of JSON plus the copies in observation 6, at trims (once per 4 MiB thanks to `LOG_TRIM_TO`), compactions, restarts, adoptions and step-down
- segments during long lines or tool-heavy logs (C-2)

**Backpressure.**
- Agent to client: client queue over 1 MiB, then `PAUSE`, then the holder stops reading, then the agent blocks. This is bounded.
- Agent to core: the holder stops reading past `CORE_QUEUE_HIGH`. This is bounded, but the 1 MiB tail truncates while the core lags more than about 0.5-1 MiB (the as-built note in `agents-sessions` line 443 says so; the risk line 485 understates it).
- Core to agent stdin: one prompt plus small answers. Past the holder's 4 MiB cap, whole `INPUT`s are dropped with an `ERROR`, and the reducer isn't told. That is reachable only by an agent flooding unoffered requests while not reading stdin.
- Fetch answers aren't bounded (C-6).

**Attach and detach.**
- Events between `ATTACH` and the replay are held back by `pending_replay` and folded into the replay. This is correct.
- Two clients can't attach one session: `Ownership::Attached` is single and a takeover moves it, so a permission can't be shown in two places.
- A detach with a permission pending leaves it pending (`R19`), and the next page reaches back to it.
- A fetch overtaken by a trim gets an empty page with `more: false`. The client's fold of that is batch A/B's.

**Lifecycle.**
- An `AGENT_RESTART` is taken only after the exit was stepped, or in `LoginRequired`, so it can't race an exit report.
- A login restart is lost on a core restart or a client detach. This is decided in `agents-login` decision 8.
- The holder owns the hang-up (close stdin, 1 s, end the group, 1 s, kill), so a core dying mid-way leaks nothing. On Windows the job handle closes with the holder.
- Ended sessions are freed on `DETACH`, `CLOSE`, `KILL` or `STOP`, when the client goes without lingering, or when the linger expires. There is no count bound, but each needs an attached or lingering owner (one per zone) and keeps its whole log (up to 16 MiB) in core memory.

**Checkpoints.** The core checkpoints:
- at every reducer `Moment`, in the order checkpoint, then `INPUT`, so a prompt or permission answer is stored before its bytes
- every 512 KiB of stdout
- at detach and at step-down, where it is whole

What goes wrong:
- A crash between the two `INPUT` writes is the recorded "known gap" (a prompt the agent never got).
- Answers with no moment are sent twice after an adoption (C-5).

The holder keeps up to about 2× base (≤ 64 MiB) per agent session in memory.

**Protocol.**
- Protocol 4 and H1 3, `CHECKPOINT_APPEND` included, are unreleased (v0.3.0 predates them), so amending them is in line with the "unreleased protocols" practice.
- Messages are bounded by the frame limit:
  - `STDERR_TAIL` ≤ 16 KiB
  - pages and transcripts are packed into whole-event frames
  - an unencodable answer is replaced by an `ERROR` for its request
- The protocol-4 guard is one function with four call sites (observation 4).
