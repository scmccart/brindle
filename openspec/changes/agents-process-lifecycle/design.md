# Design: agents-process-lifecycle

## Context

This is change 4 of the remediation plan for the 2026-11-26 agent review (`docs/architecture/audits/2026-11-26-agents-remediation.md`, section 4). Its findings are in `docs/architecture/audits/2026-11-26-agents/a.md` (A-3, observation 6) and `c.md` (C-1, C-3, C-4, C-5, C-7, structural observations 1 and 3). The audit lists C-4 through C-7 as taken from the batch reports without a second reading (`2026-11-26-agents.md`).

Every finding here is a place where an agent process ends, restarts or fails, and the server's record then disagrees with what happened:
- the reducer's log and its `Rpc`;
- the log and what the client or the checkpoint holds;
- the core's `Life` and whether anything will ever place or end the session;
- the holder's pipe and what reached the core.

The sites, as read on `5f335dc`:
- **Reducer** (`remote-agent/src/acp/reduce.rs`):
  - `end` (about line 1067) drops the list and cancels the permissions.
  - `restart_for_login`, `respawn` and `refuse` do neither; `respawn` calls `Rpc::restart`, which clears the permissions silently.
  - `fail` drops the list but leaves the permissions waiting.
  - `finish` calls `fail` on a fold error after the step has already put its `stdin` in the `Output`.
  - `line` (`Malformed::InvalidRequest`) and `message` (`RequestKind::Unoffered`) write `-32600` and `-32601` with no `mark`.
- **Core** (`remote-agent/src/server/core/`):
  - `client_agent_restart` and `restart_after_login` push a `WaitingSpawn` with `SpawnRequest::Restart`, which `drop_client` (`clients.rs`) drops with the client's other spawns.
  - `session_lost` (`sessions.rs`, about lines 1037–1108) is a seven-branch decision written in the handler.
  - `guarded_step` (`agents.rs`, line 328) returns the fault step's `Output` as if the panicked step had never run.
  - `reattach` (`sessions.rs`, about line 1333) sends `EXIT` alone for `Life::ExitedAwaitingOwner`.
- **Holder** (`remote-agent/src/server/holder.rs`): `read_limit` (line 1828) reads nothing while paused or over the core's mark, and `tick` (line 1555) finishes a reaped session once `policy::drained` sees 250 ms with no output. While the reads are held back, that quiet means nothing.

### Layers

| Piece | Layer | Where |
| --- | --- | --- |
| The closing step, the fault rule in `finish`, `Moment::Answered`, the log/`Rpc` invariant | pure core (the reducer) | `acp/reduce.rs`, `acp/rpc.rs` |
| `Life::Restarting`, `RestartStage`, `Life`'s answers | pure model in the shell's module | `server/core/sessions.rs` |
| `lost_fate`, `after_loss` | pure policy | `server/core/policy.rs` |
| `read_limit` with `Reads`, `DRAIN_PAST_HOLD` | pure item in the holder's module, tested there | `server/holder.rs` |
| Placing restarts, the replay after a fault, the deferred replay, applying `LostFate` | shell (core loop handlers) | `server/core/{agents,sessions,holders,clients}.rs` |
| `BRINDLE_TEST_ACP_PANIC_LATE` | shell, debug builds only | `server/fault.rs`, `server/core/agents.rs` |
| Reading a draining pipe session | shell (holder loop) | `server/holder.rs` (`read_output`, `update_interests`) |
| UI | unchanged | — |

## Goals / Non-Goals

**Goals**
- One step closes what an agent process left open, and every path that takes a process away calls it.
- The log's pending permission requests are exactly `Rpc`'s, after every step.
- A pending restart can't be orphaned: it is a `Life` of its own that the close policies, `STOP` and a holder's loss all see.
- After a caught panic, the client and the checkpoint hold what the server's log holds.
- A returning owner sees the end of an exited agent's conversation.
- Nothing the server writes to an agent on its own is written twice across a core restart.
- A pipe session's last output reaches the core before its exit.

**Non-Goals**
- The `Process` struct, `ToolBook`, moving `lost_fate`, and splitting `reduce.rs`: change 5, `agents-module-structure`. This change shapes the closing step so that struct can absorb it (decision 1).
- Event and line bounds (A-1, A-2, B-2, B-7, D-8): change 3, `agents-event-bounds`. A client's requests under its queue's marks (C-6): change 8, `agents-client-request-marks`.
- The segment encoding: change 6, `agents-checkpoint-deltas`.
- The PTY drain (decision 9).
- Any wire change.

## Decisions

### 1. One step closes what an agent process left open (A-3; `fcis R1`, `types R1`)

The reducer gains one method:

```rust
/// Whether the agent process whose requests are closed still runs.
enum Closing {
    /// It is gone or hung up: nothing more can be written to it.
    Gone,
    /// It runs on, but this session can no longer serve it (`fail`).
    LeftRunning,
}

fn close_process(&mut self, why: String, closing: Closing)
```

Its effects, in this order:
1. **The list.** `drop_list(why)`: a list request still out gets `Output::listed` with `Err(why)`.
2. **Every awaited request is forgotten.** A new `Rpc::close() -> Vec<(EntryId, Vec<u8>)>` clears `outgoing` and takes `permissions` in entry order, each with its cancelled answer's bytes. `next_id` is kept, so ids keep counting. `Rpc::close` replaces `Rpc::restart`, and `end`'s `rpc.forget(Purpose::Authenticate)` becomes part of it.
3. **Each waiting permission is closed.** For each one it emits `Event::Answered { id: entry, outcome: Cancelled }`. With `Closing::LeftRunning` it also appends the answer's bytes to `out.stdin`.

It does not call `unblock`. In `end`, that would emit a turn change ("working" or "idle") just before the "turn failed" that `end` emits, so each caller decides about the turn itself.

| Caller | `Closing` | `why` | What follows, and the event order | Checkpoint |
| --- | --- | --- | --- | --- |
| `end` | `Gone` | `Refused::Lost` or `Refused::Exited`, as today | Unchanged: `answered`…, then `held`, `turn failed`, `agent exited` or `lost`. Existing shape tests keep their shapes. | `Stopped`, as today |
| `restart_for_login` | `Gone` | new `Refused::Restarted` ("the agent is started again after its login") | Called after the `CannotContinue` check, so a refused restart changes nothing. Events: `answered`…, the `RESTARTED_AFTER_LOGIN` notice, `agent starting`. The turn is already failed (`prompt_needs_login`) or there is none (a start's demand), so it calls no `unblock`. | None, as today (`agents-login` decision 8: only the hang-up step's state reaches a checkpoint, and a hung-up session isn't checkpointed) |
| `refuse` | `Gone` | its message, as today | `answered`…, then `unblock` (a permission asked during a muted load left the turn blocked), then `refuse`'s own events as today | `Stopped`, as today |
| `fail` | `LeftRunning` | "the terminal server's ACP handling failed", as today | `answered`…, then `fail`'s own events as today (`turn failed` if a turn runs, `agent failed`). The cancelled answers go to the agent. | `Stopped`, as today; the core writes the checkpoint before `stdin` |
| `respawn` | `Gone` | `Refused::Restarted` | Normally a no-op, because `end` or `restart_for_login` closed the process first. It stands in for `Rpc::restart` so that no path resets `Rpc` without closing it. | — |

**The agent left running by `fail` is answered.** After a fault the session's `answer` is refused (the phase is `Broken`), so a waiting request could never get a person's choice, and the agent's turn would hang on it until the session is closed. Answering `cancelled` is ACP's answer for "no choice will come", and the log records the same outcome. This adds a case to `process R19`'s list of when a permission is answered `cancelled`: when the session's ACP handling has failed. Task 9 amends `R19`.

**Shaped for change 5.** `close_process` reads and writes only `Rpc`, the list slot and the permission entries' outcomes. Change 5 gathers the per-process state that a new process starts without (`lines`, `run`, `tools`, `outputs`, `bad`) into a `Process` struct, which `respawn` replaces whole right after this step. `Rpc` stays on the session, since it keeps `next_id` across processes (change 5, decision 10), so the closing step stays a `Step` method there too, with the same inputs and effects.

### 2. A faulted step writes nothing it asked for (observation 6; `process R3`)

`finish` handles `fault` first:
1. it clears `out.stdin`;
2. it calls `fail` (decision 1's `close_process(LeftRunning)` forgets every awaited request, those the faulted step took ids for included);
3. then `trim` and `wake` run as today.

`out.events` is kept: every event in it was applied to the transcript before the fold error, and `emit` applies nothing after one, so the events and the log agree. `out.listed` is kept: it is an answer to a client, not a write to the agent. A list the step answered is already forgotten by `Rpc`, so `fail`'s `drop_list` doesn't overwrite it.

So the only bytes a failed session's agent receives are `fail`'s cancelled answers. A prompt with attachments (`agents-prompt-context`, `agents-prompt-images`) is one step's `stdin`, so it is dropped whole, never in part.

### 3. The invariant, and where it is checked (A-3; `testing R1`)

**Invariant:** the ids of the log's `Entry::Permission { outcome: None }` entries equal `Rpc`'s waiting permission entries, after every step.
- A `#[cfg(test)] fn waiting_permissions(&self) -> Vec<EntryId>` on `Rpc` and a test helper that filters `Transcript::entries()` give both sides.
- The reducer's test `Agent::step` (`reduce.rs` tests) asserts it after every step, beside its fold check, so every existing reducer test checks it.
- `random_steps_checkpointed_whole_or_appended_decode_to_the_live_session` (`acp/checkpoint.rs`) asserts it after every step. Its choices gain:
  - a prompt answered `-32000` (a login demand);
  - `Input::Restart` while a login is needed;
  - the agent's exit, and `Input::Restart` after it;
  - `Input::Fault`;
  - a client's answer to a waiting permission;
  - the agent's `$/cancel_request` for one;
  - a list request and its answer.

  A run whose agent ended, failed or restarted carries on through a helper that answers `initialize` and the resume, or stops there when the agent can't continue. The test keeps its seeds and run count, and stays deterministic.

**A checkpoint written before this change** can hold a pending entry that `Rpc` doesn't, or permissions left pending in `Broken`. Format 2 hasn't shipped (v0.3.0, 2026-11-04, predates agents), so there is no repair at adoption, and such a checkpoint is refused (decided 2026-11-27: checkpoints of development builds need not survive these changes). `checkpoint::decode` checks the invariant: a log with a pending permission that `Rpc` doesn't await is `Unreadable::Entry { name: "permissions", entry }`, the converse of today's check, and the session is adopted as not restored, its agent left running.

### 4. A pending restart is a `Life` of its own (C-1; `types R1`, `fcis R1`, `process R2`)

Today a restart is `Life::Starting { spawn: SpawnRequest::Restart { .. } }` plus a `WaitingSpawn` in `Core.waiting`. Nothing ties the two together, and `drop_client` (`self.waiting.retain(|w| w.client != client)`) separates them.

```rust
/// A restart (a continue, or a login's restart), with what it needs.
struct Restart { restart: client::AgentRestart, cwd: String }

enum RestartStage {
    /// Not placed yet: `place_waiting` places it.
    Waiting,
    /// Sent to `Session::holder` with `SPAWN_PIPES`.
    Placed { hung_up: bool },
}

enum Life {
    Starting { spawn: SpawnRequest, hung_up: bool },
    Restarting { restart: Restart, stage: RestartStage },
    Running { pid: u32 },
    HungUp { pid: u32 },
    ExitedAwaitingOwner { code: Option<i32>, pid: u32 },
    AgentEnded { code: Option<i32> },
}
```

**Deviation from the plan.** The plan's shape was `Waiting { client }` and `Placed { holder }`. Neither field is kept:
- `Ownership` already says which client has the session, and a second copy could disagree with it (`types R1`).
- `Session::holder` is the holder a placed restart was sent to, as it is for every placed session.

`SpawnRequest` loses `Restart`, so `Core.waiting` holds only new sessions' spawns. `Session::spawning` returns its request with no `Option`, and the `send_spawn` → `send_restart` detour goes.

`Life`'s answers for `Restarting`:

| Question | `Waiting` | `Placed { hung_up }` |
| --- | --- | --- |
| `pid` | `None` | `None` |
| `is_starting` | true | true |
| `is_listed` | false | false |
| `is_live` | false | false |
| `is_held` | false: it is in no holder | true |
| `is_hung_up` | false | `hung_up` |
| `hang_up()` (tell the holder?) | false (the caller abandons it) | true once, setting `hung_up` |

The sites that change:

| Site | Today | After |
| --- | --- | --- |
| `start_again` | `life = Life::starting(SpawnRequest::Restart { .. })` | `life = Life::Restarting { restart, stage: Waiting }` |
| `client_agent_restart`, `restart_after_login` | `waiting.push(WaitingSpawn { .. })`, then `place_waiting` | `place_waiting` only |
| `place_waiting` (`holders.rs`) | Places `waiting`; the mode is pipes if any is. | Also places every `Restarting { stage: Waiting }` session, counted in the pipes mode and in `starting`, through `send_restart`. |
| `send_restart` | Needs `is_starting`. | Needs `Restarting { stage: Waiting }`; sets `stage: Placed { hung_up: false }` and `Session::holder`, then the whole checkpoint as today. |
| `session_spawned` | `Starting { hung_up }` → `Running` or `HungUp` | Also `Restarting { stage: Placed { hung_up } }` → the same. |
| `session_spawn_failed` | `restarting(life)` → `restart_failed` | `Restarting { .. }` → `restart_failed` |
| `hang_up` | `waiting.any(id) && restarting` → `abandon_restart` | `Restarting { stage: Waiting }` → `abandon_restart`; `Placed` → `Life::hang_up`, `HANGUP` |
| `abandon_restart` | `waiting.retain`, `life.hang_up()`, `restart_failed(id, "")` | `restart_failed(id, "")` (no list to edit) |
| `fail_waiting` (`holders.rs`; a holder start failed, or a step-down) | Restarts in `waiting` → `restart_failed` | Also every `Restarting { stage: Waiting }` session → `restart_failed(id, message)` |
| `client_stop` | Keeps restarts in `waiting`, refuses the rest. | Refuses every waiting spawn; restarts end through `end_session` with the other sessions. |
| `take_over` (`clients.rs`) | Partitions on `client == old && !is_restart()`. | Partitions on `client == old`. |
| `drop_client` | Drops the client's waiting spawns, restarts included. | Unchanged code, now correct: only new spawns are in `waiting`. |
| `restarting()` (`sessions.rs`, line 752) | — | Deleted. |
| `grace_fate`, `end_lingering`, `restart_refusal`, `detach_ends`, `agent_stderr`, `reattach`, `replay`, `Seen::busy` | Match `Starting` | Match `Restarting` with `Starting`'s answers: `NotEnded` for a restart, no detach ending, replayed on attach, counted busy. |

**What follows for C-1's case.**
- **A `kill` client gone with its restart `Waiting`:** `client_gone` → `gone_fate(Kill, starting: true, …)` → `HangUp` → `hang_up` → `abandon_restart`. The session ends, and `STOP` gets its `OK`.
- **A `detach` client gone:** `Keep`. The restart stays `Waiting`, is placed when a holder is ready, and runs detached.
- **An explicit `DETACH`:** `detach_ends` is false for it, so it is kept the same way.

### 5. `policy::lost_fate`: what a holder's loss does to a session (C-1, structural observation 1; `fcis R1`)

`session_lost`'s tree becomes a pure function. The handler gathers its inputs, calls it, and applies the outcome.

```rust
pub(super) enum LostFate {
    /// Not in a holder, or already queued: nothing to do.
    Unheld,
    /// A placed restart for an attached client: back to `Waiting`, placed again.
    PlaceAgain,
    /// A placed restart otherwise: the agent ends as lost.
    EndRestart,
    /// A new spawn for an attached client: removed and queued again.
    Requeue,
    /// A new spawn with no client: removed.
    Forget,
    /// A running or exited agent: step `Input::Lost`, then `after_loss`.
    AgentLost,
    /// A terminal with its client attached: replay, remove, `EXIT { lost }`.
    ReportLost,
    /// A terminal with no client: as its exit.
    Exited,
}

pub(super) fn lost_fate(life: &Life, kind: SessionKind, attached: bool, queued: bool) -> LostFate
```

| `life` | `kind` | `attached` | `queued` | Outcome |
| --- | --- | --- | --- | --- |
| any | any | any | true | `Unheld` |
| `AgentEnded`, `Restarting { Waiting }` | — | — | false | `Unheld` |
| `Restarting { Placed { hung_up: false } }` | — | true | false | `PlaceAgain` |
| `Restarting { Placed { .. } }` | — | false, or hung up | false | `EndRestart` |
| `Starting { .. }` | — | true | false | `Requeue` (hung up or not, as today) |
| `Starting { .. }` | — | false | false | `Forget` |
| `Running`, `HungUp`, `ExitedAwaitingOwner` | `Agent` | — | false | `AgentLost` |
| the same | `Terminal` | true | false | `ReportLost` |
| the same | `Terminal` | false | false | `Exited` |

`after_loss(ownership, life, continuable) -> AfterLoss::{Keep, ReportLost, Exited}` is the second half of the agent row:
- `end_fate`'s `Keep` gives `Keep` (life becomes `AgentEnded { code: None }`);
- otherwise an attached client gives `ReportLost`;
- otherwise `Exited`.

Each row is today's branch for the same case, except the restart rows, which today depend on `waiting`. The table changes no other outcome. Each row gets a unit test in `policy.rs`'s tests, named for its case.

**A core restart and a pending restart** (`agents-login` decision 8, `agents-session-recovery` decision 6), unchanged:
- A `Waiting` restart lives only in the core's memory, as an ended session does. A new core never hears of it: the session is gone, and the client reconnects to a lost zone that offers to continue in a new session.
- A `Placed` restart is in its holder, with the whole checkpoint `send_restart` wrote after `SPAWN_PIPES`, so the new core adopts it and the resume or load goes out from there.
- A login restart kept in `Agent::login_restart` is still not checkpointed. A new core steps the agent's exit as any exit.

### 6. After a caught panic: a whole checkpoint and a fresh replay (C-3, observation 6's neighbour; `process R3`, `R20`)

`guarded_step` returns which kind of step ran:

```rust
enum Stepped {
    Clean(Output),
    /// The step panicked; this is the `Input::Fault` step's output (or an
    /// empty one if that panicked too). The session's log may hold events
    /// that no client and no checkpoint has.
    Faulted(Output),
}
```

On `Faulted`, `agent_step` (and the adoption path in `agent_adopted`, which also runs `guarded_step`) does the following:
1. **Forget the stored checkpoint:** `agent.forget_stored()` sets `Stored::Unknown` and clears `unsaved`, so the checkpoint the fault step's `Moment::Stopped` asks for is whole (`agent_appendable` is false for `Unknown`). An empty fault output asks for none, and the next due checkpoint is whole.
2. **Apply the output without its events:** `agent_apply` gains `delivery: Delivery::{Events, Replay}`. With `Replay` it sends `listed`, writes the checkpoint, the `stdin` and the hang-up as today, but sends no `TRANSCRIPT` and doesn't extend `unsaved`.
3. **Send a fresh replay:** if a client is attached and no replay is pending, `self.replay(client, id, PendingReplay::reconnect())`: an unrequested `REPLAY_START`, the newest page of the server's log, and `REPLAY_END`. It is deferred, as any replay is, while the session's checkpoint is being stored. The client's sink resets its fold at a `REPLAY_START` it didn't ask for, so the client then holds what the server holds. This is the same path a reconnect takes.

**The new switch.** `BRINDLE_TEST_ACP_PANIC_LATE` (`fault.rs`, `process R20`) holds a marker, like `BRINDLE_TEST_ACP_PANIC`.
- It is honoured only in debug builds, and it joins the `--list-fault-switches` listing and its test `the_helper_lists_its_switches_only_in_debug_builds`.
- Inside `guarded_step`'s `catch_unwind`, after `reduce::step` returns, it panics when the input held the marker and the returned `Output` holds at least one event.
- To the shell this is a panic after the first emit: the events are in the session's log, and the `Output` is lost.
- The reducer is pure and reads no environment (`fcis R2`), so the switch can't live inside `step`.

`tests/e2e/switches.rs` lists only the switches e2e cases use, and none uses this one.

### 7. A returning owner's exited agent session is replayed (C-4)

In `reattach`, `Life::ExitedAwaitingOwner` depends on the session's kind:
- an agent session goes through `self.replay(client, id, PendingReplay::reconnect())`, which already ends with `REPLAY_END`, removes the session and sends `EXIT` for `ExitedAwaitingOwner`;
- a terminal keeps `EXIT` alone (its screen is gone with its emulator, and no finding covers it).

**Race.** A core that adopts a holder holding a finished pipe session learns of the exit at `WELCOME`, gets the session's tail, and gets `STDERR_TAIL` last (`holder.rs` `take_over`). The core steps the agent's exit only at `STDERR_TAIL` (`agent_stderr`). A replay sent before that would end without the exit. So:
- `replay` also defers while an `ExitedAwaitingOwner` agent's exit isn't stepped yet. This uses a new pure accessor, `AgentSession::has_ended()` (phase `Exited` or `Lost`).
- `agent_stderr`, after `agent_exited`, sends the pending replay, through the helper `session_tail` already uses for its deferred one.

**Client.** `ReplayStart` with no `req` calls `sink.replay(true)` (the fold resets). The page folds, then `replay(false)`, then `EXIT` calls `sink.exited(code)` and `model.ended()` (`src/remote/local/connection.rs`, `src/agent/sink.rs`). The zone shows the turn's end, the exit and its standard error, then the exit. No client change.

### 8. The server's own answers are checkpointed before they are written (C-5; `process R2`)

`line` (`-32600`) and `message` (`-32601`) call `self.mark(Moment::Answered)`. `Moment` gains:

```rust
/// The server answered one of the agent's requests itself: the line it
/// read reaches the holder, as consumed, ahead of the answer's bytes.
Answered,
```

`Answered` gets an arm of its own, not `BeforeRequest`: `BeforeRequest` means "a request id of ours is about to go out", and the reason a checkpoint is due belongs in the moment's name. Neither `mark`'s rule (`BeforeRequest` wins over every other moment) nor `agent_checkpoint_due` (any moment is due) changes beyond the new arm, and every `match` on `Moment` names it (`railway R3`).

**Cost.** One checkpoint per such request:
- with a known stored checkpoint, an appended segment holding the step's events (the bad-line notice for `-32600`, none for `-32601`) and the state as segments carry it today;
- `agent_checkpoint_form`'s base-size rule turns a run of them into a whole checkpoint once the segments outgrow the base.

A well-behaved agent sends none of these: the server offers no `fs/*`, `terminal/*` or elicitation. An agent that floods them costs one H1 `CHECKPOINT` frame per request, as a flood of permission requests already does through `Moment::Permission`. That is bounded by the agent's own rate and by the holder's checkpoint limits (`process R9`). Change 6 makes the segment a delta, which makes this cheaper.

### 9. The holder drains a pipe session past a pause (C-7, choice C3; `process R1`, `R8`, `R9`)

**The rule.** A reaped pipe session is *draining*. While draining, the core's pause and the core's queue mark don't stop its reads, up to `DRAIN_PAST_HOLD` (1 MiB) read while they would have. `read_limit` takes the session's reads:

```rust
/// Whether a session's reads follow the core's pause and marks.
enum Reads {
    /// Running, or a PTY session at any time.
    Following,
    /// A reaped pipe session; `held` bytes were read past a pause or mark.
    Draining { held: usize },
}

fn read_limit(dest: Dest, paused: bool, reads: Reads, chunk: usize) -> usize
```

| `dest` | `paused` or `over` | `reads` | Limit |
| --- | --- | --- | --- |
| `Tail { room }` | — | any | `room.min(chunk)`, unchanged |
| `Core` | false | any | `chunk` |
| `Core` | true | `Following` | 0, unchanged |
| `Core` | true | `Draining { held }` | `(DRAIN_PAST_HOLD - held).min(chunk)` |

- `Life::Reaped` gains `held: usize`. `read_output` adds a read to it when the read happened past a pause or mark. `update_interests` passes the same `Reads`, so the loop watches a draining session's stdout again.
- `policy::drained` is unchanged. With reads allowed, its 250 ms of quiet now means the pipe is empty (or the budget is spent), and `OUTPUT_DRAIN_LIMIT` (2 s) still ends a session whose stdout a process it started holds open.

**The bound.** 1 MiB is the most an exited program can leave in its pipe:
- on Linux, an unprivileged process can grow a pipe to at most `pipe-max-size`, 1 MiB by default (the default capacity is 64 KiB);
- on Windows, the holder creates the pipe with a 64 KiB buffer (`PIPE_BUFFER`, `platform/windows/pty.rs`).

So the whole of what the program wrote is read. A process it left behind adds at most 1 MiB past the marks before the drain stops reading and the session finishes. The extra reaches the core's queue to that client once per session end, so memory stays bounded (`process R9`).

**PTY sessions are unchanged** (`Reads::Following` always):
- For a pipe session, every byte of stdout is a protocol message, and a lost one leaves the log wrong: a turn that never ends, a tool call that never completes, a missing exit reason.
- No finding shows a loss for terminals, and a PTY's end has its own behaviour (background jobs holding the PTY, `EIO` on Linux).
- Each holder change is one that can kill every session (`process R1`), so the change stays as small as the finding.

This is an open question below.

**Windows.** The holder's pipe ends are overlapped, and the loop itself does the IO (`process R8`, `platform/windows/piped.rs`). `Pipe::read` returns what a completed read buffered, `WouldBlock`, or `Ok(0)` once the program and every inheritor closed the pipe. While `read_limit` is 0, `update_interests` doesn't post reads. With the session draining, the interest is back, the loop posts a read, and its completion or `ERROR_BROKEN_PIPE` wakes the loop. No Windows-specific code changes; the paired holder tests (`#[cfg(windows)]` with PowerShell programs) get the same case.

**What is not covered.** With no core, `Dest::Tail { room }` still limits the drain to the tail's room. A full tail with no core and an exited program loses the pipe's remainder, as today. That is the tail's bound (`TAIL_LIMIT`), and the next core shows the output as lost. It is outside C-7.

**`process R1`.** The holder already decides when a reaped session is drained (`policy::drained`). This extends that decision with one counter in `Life::Reaped`, parses nothing, and keeps no policy about what the bytes mean.

### 10. The remediation's settled decisions that touch this change

- **C3 (the holder's drain) is this change's choice:** decision 9. Draining past a pause, bounded, was chosen over the alternatives:
  - *Hold the drain while paused:* a stalled client would keep a finished session in its holder for as long as it stalls.
  - *Read to the end at finish:* a process left holding the pipe makes "the end" unbounded.
- **C4 (unreleased protocols are amended in place, not bumped):** this change makes no wire change, and nothing in client protocol 4, H1 3 or checkpoint format 2 changes. `Moment` and `Life` are core memory, never encoded. If implementation finds a field it must store, it amends format 2 in place under C4 and adds a transcript test, as `process R6` requires.
- **Change 5 absorbs the closing step:** decision 1 keeps it to the state change 5 gathers.
- **`lost_fate` is written in `policy.rs` now:** change 5 moves it with the rest of the agent policy.

### 11. Failures and where they show (`railway R1`, `R6`)

| Failure | Shown |
| --- | --- |
| A restart dropped because its `kill` client is gone | No client to tell; the session ends, and the core logs `session N: restart dropped, its client is gone`. |
| A `Waiting` restart that can't be placed (holder start failed, step-down) | `restart_failed`: the session returns to ended with the reason as a notice in the zone, as `SPAWN_FAILED` does today. |
| A list out when its process is closed | An `ERROR` to the client saying why (exited, lost, started again, handling failed). |
| A permission closed with its process | Its entry shows cancelled; a later answer gets `AlreadyAnswered` naming the entry. |
| A caught panic | `AgentState::Failed` with the reason (unchanged), now in a fresh replay. |
| A drain that reached `DRAIN_PAST_HOLD` | The holder logs `session N: output past the drain bound left unread`; the session ends as today. |

### 12. What a core restart, a Brindle crash and an upgrade do (`process R2`, `R12`)

| Change | Core restart | Brindle crash | Upgrade |
| --- | --- | --- | --- |
| Closing step, invariant (1–3) | The checkpoint carries the closed entries and `Rpc`'s parts, which agree; adoption keeps them. | Nothing new: the agent's state is server-side. | As a core restart. |
| `Life::Restarting` (4, 5) | `Waiting` is lost with the core's memory, as ended sessions are (`agents-session-recovery` decision 6). `Placed` is adopted from `send_restart`'s whole checkpoint. | The client is gone: `kill` drops a `Waiting` restart and hangs a `Placed` one up; `detach` keeps it, placed later. | Step-down's `fail_waiting` returns `Waiting` restarts to ended, and ended sessions aren't carried across (unchanged). A `Placed` one is adopted. |
| Whole checkpoint and replay after a panic (6) | The whole checkpoint makes the adopted log equal the faulted session's log. | The client's next attach replays as today. | As a core restart. |
| Replay to a returning owner (7) | A new core that adopted an exited agent waits for `STDERR_TAIL` before it replays. | That is this change's case: the owner returns after a crash. | As a core restart. |
| `Moment::Answered` (8) | The new core resumes after the answered line and never answers it again. | Unaffected. | As a core restart. |
| Drain (9) | The pause goes with the core (`paused` resets for a new core); with no core the tail rule applies, unchanged. | Unaffected (the core, not Brindle, pauses). | Holders of the older build keep today's rule until they drain and exit (`process R12`). A holder is never replaced while it has sessions. |

### 13. Archived decisions this change changes

- **`2026-11-25-agents-login`, decision 8 ("As built").** "Steps `Input::Restart` again (`Restarting` → `Spawning { start }`, resetting the RPC, lines and tool ids …)". The RPC reset now closes the process first (decision 1): waiting permissions are recorded cancelled and a list is refused, where before they were cleared silently. "Queues it as an ended agent's restart (`start_again`, `SpawnRequest::Restart`)" becomes `Life::Restarting { stage: Waiting }`.
- **`2026-11-25-agents-session-recovery`, decision 5, step 3.** `Life::Starting { spawn: SpawnRequest::Restart { restart, cwd } }` becomes `Life::Restarting { restart, stage }` (decision 4).
- **`2026-11-25-agents-session-recovery`, decision 6's table.**
  - The "Brindle crash" cell for a continue in progress ("the agent starts; the returning owner gets the replay") holds only for `detach`. With `kill`, the continue is dropped and the session ends, as any starting session of a gone `kill` client.
  - The "holder crash" cell ("before `SPAWNED`: re-placed as any `Starting` spawn") holds only while the client is attached (`PlaceAgain`); otherwise the agent ends as lost (`EndRestart`), as the code already did.
- **`process R19`** gains "or when the session's ACP handling has failed" in its list of when a permission is answered `cancelled` (decision 1).

### 14. When a fault can't be shown

Each finding's first task is a test that fails on `main`. If that test passes instead:
- the finding is recorded as not reproduced in this design's "As built" note, with the test's name;
- the test is kept as a regression guard;
- the fix for that finding is not made.

For each finding:
- **C-4, C-5, C-7:** the spec scenario stays, since it states behaviour that then already holds.
- **C-1:** `Life::Restarting` is still made, because it removes a state the types allow (`types R1`) whether or not today's handlers reach it. Its outcome tests then pass before and after.
- **C-3:** the switch and the whole-checkpoint rule are still added. Without the switch, no test can reach the path, and a panic in the reducer is the case `process R3` guards whatever its point.

### 15. Tests (`testing R1`, `R7`, `R9`, `R10`)

- **Pure** (`reduce.rs`, `rpc.rs`, `checkpoint.rs`, `policy.rs`, `holder.rs`'s own tests): the closing step per caller with its event shapes, the fault rule, `Moment::Answered`, the invariant (decision 3), `Life`'s answers for `Restarting`, `lost_fate` and `after_loss` per row, and `read_limit` per row.
- **Real processes** (`remote-agent/tests/agents.rs`, `holder.rs`): one per finding, each gated on files the test creates (the fake agent's `wait-file`), never on time. The C-1 case runs in the core's in-process harness (`server/core.rs` tests) when placement can't be held from outside (task 4.1).
- No sleep, retry or longer timeout. A test that needs "before placement" or "right after the answer" holds that point with a gate.

## Risks / Trade-offs

- **[A replay after every caught panic costs a whole absorb on the client]** → Panics are bugs, and rare. A replay is the existing reconnect path, so nothing new runs on the client.
- **[Answering a failed session's permissions `cancelled` acts for the person]** → The person can no longer answer (`Broken` refuses), and the agent would otherwise wait forever. `cancelled` asserts no choice, and the log shows it.
- **[The drain sends up to 1 MiB past a pause]** → Once per session end, bounded, and only for pipe sessions.
- **[`Life` gains a variant every `match` must name]** → That is the point (`railway R3`): each site decides, and the compiler lists them.
- **[Merge order with #151 and changes 3 and 5]** → This change rebases after #151 and change 3, and before change 5. The closing step and `lost_fate` are written so that change 5 moves them unchanged.

## Migration Plan

No data migration and no wire change. Holders of the older build keep today's drain until they exit, and cores of either build read the same checkpoints. Rollback is a revert.

## Open Questions

1. **A hung-up `Starting` spawn is requeued when its holder is lost** (`lost_fate`'s `Requeue` row keeps today's behaviour). Recommendation: refuse it with an error instead, since the client already asked for it to end. That is behaviour outside these findings, so it is left as today.
2. **Drain PTY sessions too?** Recommendation: not without a finding (decision 9). Revisit if a terminal shows lost last output under a pause.
