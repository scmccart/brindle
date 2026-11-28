# Proposal: agents-process-lifecycle

## Why

The 2026-11-26 agent review (`docs/architecture/audits/2026-11-26-agents.md`, batch reports in `docs/architecture/audits/2026-11-26-agents/`, remediation plan in `docs/architecture/audits/2026-11-26-agents-remediation.md`, section 4) found several places where an agent's process ends, restarts or fails, and the server's record of it then disagrees with what happened:

- **A-3** (`a.md`): three paths take an agent process away (`end`, `restart_for_login`, `respawn`), and only `end` closes what the process left open. A terminal login's restart drops the waiting permission requests from `Rpc` without recording them, so their entries stay pending in the log forever: the client's buttons are refused as "already answered", every attach's page is pulled back to the oldest of them, and a list request out at the time is never answered. A handling fault (`fail`) leaves the same requests pending in a session that can no longer answer them.
- **Observation 6** (`a.md`): a step that hits a fold error still writes what it asked for to the agent, such as a `session/prompt`, though the session has failed.
- **C-1** (`c.md`): a continue or login restart waiting for a holder is in `Core.waiting`, and `drop_client` drops every waiting spawn of a gone client. The session is left `Starting` for good: never placed, never hung up, and a `STOP` waits on it until the core is killed.
- **C-3** (`c.md`): a panic inside the reducer after it applied an event to the log loses the step's `Output`. The client and the stored checkpoint then miss events the server's log has, and the next appended checkpoint or event no longer folds.
- **C-4** (`c.md`): an owner returning to an agent session that exited while it was away, and can't be continued, gets only `EXIT`: the turn's end, the exit and the standard error it missed never reach it.
- **C-5** (`c.md`): the server's own `-32600` and `-32601` answers are written with no checkpoint first, so a core restarted before the next checkpoint reads the same request from the tail and answers it again.
- **C-7** (`c.md`): a holder finishes a pipe session 250 ms after its program exits even when the core paused its reads, and drops the agent's last lines of output unread.

C-4, C-5 and C-7 were taken from the batch reports without a second reading, so each one's first task shows the fault with a failing test.

## What Changes

- The reducer gets one step for "this agent process is gone or can no longer be served": it answers the list out, forgets every request the session awaited, and records each waiting permission as cancelled, answering it too when the agent is left running. `end`, `restart_for_login`, `refuse`, `fail` and `respawn` all call it. The log's pending permission requests are then always exactly the ones `Rpc` waits on, checked after every step of the randomized test.
- A step that faults writes nothing it asked for to the agent; the only bytes a failed session's agent gets are the cancelled answers to its waiting permissions.
- A pending restart becomes a state of its own in the core, `Life::Restarting`, instead of a spawn in `Core.waiting`, so a gone client can no longer orphan it: it follows the close policy as a starting session does. What a holder's loss does to a session becomes the pure `policy::lost_fate`, with a test per row.
- After a caught panic in ACP handling, the core forgets the stored checkpoint (the next one is whole) and sends the attached client a fresh replay instead of the fault step's events. A new debug-only fault switch makes that panic happen after the reducer applied events.
- A returning owner's exited agent session is replayed, ending with the exit, before its `EXIT`.
- The server's own error answers to the agent are checkpointed before they are written.
- A holder reads a pipe session's remaining standard output after its program exits even while paused, up to a bound, before it finishes the session.

No wire change: client protocol 4, holder protocol H1 3 and checkpoint format 2 are unchanged.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`:
  - "The server answers the agent's requests" (its own answers survive a core restart, answered once).
  - "Permission requests wait for the client's choice" (a request whose agent process is gone or restarted, or whose session's handling failed, is recorded cancelled).
  - "The agent's exit is reported" (a returning owner's exited session is replayed before its exit).
  - "A faulty agent is confined to its session" (a fault keeps the client's log and the checkpoint whole, and writes nothing the faulted step asked for).
  - "Continuing a conversation" (a continue whose client is gone before it is placed follows the close policy).
  - "An agent that needs a login can be restarted" (waiting permissions and a list are closed at the restart).
  - "Refusing a list" (a list out when the agent process stops is refused saying why).
- `terminal-server`:
  - "Session I/O" (a pipe session's output written before its exit reaches the core even while paused).

## Impact

- **`remote-agent/src/acp/reduce.rs`, `acp/rpc.rs`:** the closing step, `Rpc::close` in place of `Rpc::restart`, the fault rule in `finish`, a new `Moment::Answered`, and tests, including the invariant in the test `Agent::step`.
- **`remote-agent/src/acp/checkpoint.rs`:** the randomized test gains the steps that end, restart, fail and log in, and checks the invariant after each step.
- **`remote-agent/src/server/core/`:** `sessions.rs` (`Life::Restarting`, `SpawnRequest` without `Restart`, `session_lost` through `lost_fate`, `reattach`), `holders.rs` (`place_waiting`, `fail_waiting`), `clients.rs` (`take_over`), `agents.rs` (`guarded_step` returns `Stepped`, a replay after a fault, a deferred replay until an adopted exit is stepped), `policy.rs` (`lost_fate`, `after_loss`).
- **`remote-agent/src/server/fault.rs`:** the switch `BRINDLE_TEST_ACP_PANIC_LATE`, listed by `--list-fault-switches` in debug builds only.
- **`remote-agent/src/server/holder.rs`:** `read_limit` takes the session's flow, so a reaped pipe session drains past a pause, bounded.
- **Tests:** `remote-agent/tests/agents.rs` (a restart whose client drops, a late panic, a returning owner's exited agent, a refused request across a core kill) and `remote-agent/tests/holder.rs` (a paused pipe session's last output).
- **Docs:** `docs/code-map.md` (the server core's restart state, the holder's drain), `docs/architecture/process-architecture.md` `R19` (a failed session's waiting permission requests are answered `cancelled`), and `R9`'s pipe paragraph (the drain's bound).
- **Client:** unchanged. A replay in the middle of a session and a replay followed by `EXIT` are handled by today's sink (`src/agent/sink.rs`).

### Overlap with other changes

- **`agents-commands` (PR #151) lands first.** It touches `acp/reduce.rs`, `src/agent/model.rs` and the agent view. Its commands and options last one run of the agent, so `respawn` resets them; this change rebases onto it and leaves that reset where #151 put it. The closing step touches only `Rpc`, the list and the permission entries.
- **`agents-prompt-context` and `agents-prompt-images`:** a prompt with attachments is still one step's `stdin`. A faulted step drops all of it, so "A prompt with its images always reaches the agent whole" still holds: it reaches the agent whole or not at all.
- **`agents-plans`:** adds entries and events to `reduce.rs`; no shared behaviour. Only merge order matters.
- **`agents-acp-v2`:** `Rpc::close` replaces `Rpc::restart`, and the `-32600`/`-32601` answers are built through the `Dialect`. A v2 implementation names `close`.
- **`agents-client-terminals`:** once the server offers `terminal/*`, those requests stop getting `-32601`. `Moment::Answered` covers whatever the server answers on its own.
- **Remediation siblings:**
  - `agents-client-state-fixes` (1) and `agents-zone-focus-and-scroll` (2) are client-side; C-4's replay relies on today's sink behaviour, which change 1 must keep.
  - `agents-event-bounds` (3) lands before this change and touches `reduce.rs` and `core/agents.rs`. Both changes modify `agent-sessions` "Permission requests wait for the client's choice", each adding its own text: this change is archived second, so its delta carries that change's text once it has been archived.
  - `agents-module-structure` (5) follows this change: it moves `lost_fate` and gathers the per-process state (the lines, the run, the bad-line run and the tool indexes, not `Rpc`, which keeps its request ids across processes) into a `Process` struct that `respawn` replaces whole after the closing step defined here.
  - `agents-checkpoint-deltas` (6) changes how segments are encoded; the moments and the forced whole checkpoint here keep their meaning.
  - `agents-absorb-cost` (7): the replay after a fault is one more whole absorb on the client, at a rare moment.
  - `agents-client-request-marks` (8) lands after this change and touches `clients.rs` (`process_client_frames`, `flush_client`), not `take_over`. Both changes modify `terminal-server` "Session I/O", each adding its own text (this change the pipe session's drain, that one a client's requests under its queue's marks): whichever is archived second carries the other's text in its delta; this change is expected to be archived first.
