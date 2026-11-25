# Proposal

**Depends on:** `agents-profiles` (the agent zone's exit and lost states, and through it `agents-sessions`' agent sessions, their checkpoint and their exit report), implemented and archived.
**Next:** `agents-log-paging`, paging the conversation log. The five follow-ups of `agents-profiles` are independent of one another, so that order is a suggestion.

**Status: designed.** Specs, design.md and tasks.md are written; the open questions below are answered (2026-11-25).

## Why

In step 1 an ACP agent that exits (a crash, an out-of-memory kill, an update that replaced its binary) ends its agent session. The agent zone stays open with the exit status and the end of the agent's standard error, but its prompt input is unavailable and the conversation can't be continued: the user closes the zone and starts a new conversation that knows nothing of the old one. The same happens when the terminal server loses the session, for example when its holder crashes. Worse, the server keeps nothing after the session ends: a prompt the user had queued is held only in the zone's copy, an exited zone isn't recorded for session restore, and an agent that exits while no Brindle is attached leaves only its exit status. Many ACP agents keep their conversations and can take them up again; Brindle should let them.

## What Changes

- After an agent session's agent exits, or the terminal server loses the session, the conversation can be continued in the same agent zone: the server starts the agent again in the same directory and asks it to take up the same ACP session, then the zone carries on with its log, its held prompt and its prompt input.
- ACP v1 offers two ways, each gated by a capability the agent advertises in `initialize`:
  - **`session/load`** (`agentCapabilities.loadSession`): the agent replays the whole conversation as `session/update` notifications, then answers.
  - **`session/resume`** (`sessionCapabilities.resume`): the agent restores the session's context without replaying it.
  Both take the ACP session id, the working directory and the MCP servers. An agent that offers neither can't be recovered; what the zone says then is part of this change.
- ACP v1's `session/list` (`sessionCapabilities.list`) lists the sessions an agent knows, which could also let a user pick up a conversation Brindle no longer has a record of.

Open questions, to answer before a design:
1. **Who starts it:** a recovery the user asks for in the exited zone, or an automatic one, and if automatic, the limit that stops a crash loop.
2. **Load or resume.** Brindle's server already has the log, so `session/resume` fits when the log survived; `session/load` fits when it didn't, but replays history Brindle already holds. How a replay is reconciled with the kept log (duplicates, entry ids), and which method wins when both are offered.
3. **What the server keeps after an exit,** and where: the ACP session id, the log, the held prompt, the negotiated ACP version, and the agent's command, arguments, environment and directory to start it again; and for how long an ended agent session is kept. A holder crash loses the checkpoint with the session, so something outside the holder (the core, or Brindle's session state) would have to hold what recovery needs.
4. **Session restore:** whether an exited, recoverable zone is recorded and recovered on the next launch, and whether a detached session whose agent exited is kept for its owner with its conversation rather than only its exit.
5. **A turn in flight at the exit:** how it is shown after recovery, and whether its prompt may ever be resubmitted (ACP warns that resubmitting an inserted prompt can repeat the agent's tool calls). Whether a held prompt's send-again becomes available again once the recovered agent is ready.
6. **Pending permission requests** at the exit: how their rows end, since the agent that asked is gone.
7. **`session/list`:** in scope here or not, and how it would show.
8. **ACP v2:** v2 drops `session/load` and gives `session/resume` an optional `replayFrom` cursor that does both, with a `replay_complete` marker. This change speaks v1 (`agents-acp-v2` adds v2); its design should keep the v2 mapping in view so that change only swaps the requests.

Answers (user, 2026-11-25; design.md has the detail):
1. The user: a Continue conversation button in an exited or lost zone, and a palette action. No automatic restarts.
2. `session/resume` when the log is kept and it is offered; `session/load` with the replay muted and a notice when only load is offered (Copilot); `session/load` folded into a fresh log when the server lost the session. Entry ids continue from the kept log.
3. The core keeps an ended session in memory (log, slot, ACP session id, version, directory, what the agent offered); the client re-sends command, arguments and environment in a new `AGENT_RESTART`, so no secrets are stored and a profile edit applies; a deleted profile means no recovery. Kept until the zone closes, or as exits are kept for an absent owner today. A lost holder is recoverable too. Client protocol 4 is amended in place, since the `agents-*` changes ship in one release.
4. Exited zones aren't recorded for session restore, and the state file gains no field. `AgentState::Ready` gains no session id: the conversation is its own transcript event.
5. A turn in flight ends failed ("the agent exited"), stays in the log and is never resubmitted or offered for it; a held prompt can be sent again once the continued agent is ready.
6. As built: the exit cancels pending permissions, and a continued agent asks anew.
7. Out of scope: the proposal-only change `agents-session-list`.
8. v1 only: `session/load` and `session/resume` are called directly, and `agents-acp-v2` reshapes them later.

## What the full changes already say

- `agents-sessions`, design decision 17: an exit gives `AgentState::Exited { code, stderr }`, then the session's `EXIT`; no restart; `session/load`/`session/resume` recovery is this change.
- `agents-sessions`, design decision 10 ("What survives"): an exit holds a waiting prompt (`AgentStopped`) and ends the session; the attached client keeps the conversation, held prompt included, "but the server keeps nothing after the session's `EXIT` until `agents-session-recovery`". Risks: "A held prompt lost when the agent exits" names this change as where the server would keep it.
- `agents-sessions`, design decision 11: the checkpoint holds the transcript, the ACP session id and the negotiated ACP version; a holder crash loses its sessions, agent sessions included (`EXIT` with `lost`) (question 3).
- `agents-sessions`, `agent-sessions` requirement "The agent's exit is reported": the client receives the exit status and the last 16 KiB of standard error, then the session's exit, after which the session is no longer listed except as a detached session's exit kept for its owner (question 4).
- `agents-profiles`, design decision 13: an exited zone stays until closed and "isn't recorded (`agents-session-recovery` brings it back)"; a session that can't be got back is `Lost`, shown until closed. The `agent-zones` requirement "An agent that exits" says the zone SHALL NOT be recorded for session restore; this change modifies it.
- `agents-profiles`, design decision 8 and the requirement "Sending a held prompt again, or discarding it": send-again is unavailable once the session has ended, and discard then clears the row locally without a request (question 5).
- `agents-profiles`, design decision 4: splits aren't started from a listed session's command, because the listing doesn't carry its environment (question 3).

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `agent-sessions` (added by `agents-sessions`): "The agent's exit is reported", "Transcript event forms are fixed for each protocol version", and new requirements for recovering a session.
- `agent-zones` (added by `agents-profiles`): "An agent that exits", "A disconnected endpoint" (a lost session), "Sending a held prompt again, or discarding it", and new requirements for continuing a conversation.
- `session-restore`: a new requirement that agent zones whose agent ended aren't restored.
- `terminal-server` is unchanged: `AGENT_RESTART` joins client protocol 4, which "Protocol versions" already says adds agent sessions.

## Impact

- `remote-agent/src/acp/`: the reducer's start sequence (`session/load` or `session/resume` in place of `session/new`), the agent's capabilities from `initialize`, and reconciling a replay with the log.
- `remote-agent/src/server/`: keeping an ended agent session's state, and starting its agent again.
- `transcript/` and `remote-protocol/`: possibly new agent states and a new client message, with transcript tests.
- `src/agent/`, `src/session.rs`: the exited zone's recover action, and restore.
- The fake ACP agent plays `loadSession`, `resume` and exits with its existing steps (`respond`, `send`, `exit`), for server tests and e2e cases.
