# Proposal

**Depends on:** `agents-session-recovery` (continuing a conversation by its ACP session id in a new agent session, `SPAWN_AGENT`'s `conversation`, and the transcript's `Conversation`), implemented and archived.
**Next:** none named. Like the other `agents-*` follow-ups it is independent of `agents-log-paging` and `agents-acp-v2`, so any order works.

**Status: proposal stage.** This change has no design, specs or tasks yet. It was split out of `agents-session-recovery` (its open question 7), which left `session/list` out of scope.

## Why

`agents-session-recovery` continues a conversation Brindle still knows: one in an agent zone that is open, whose agent exited or whose terminal server lost the session. A conversation Brindle no longer has a zone for can't be picked up: one whose zone was closed, one that ended while Brindle was closed (an ended session that is detached is dropped), one from before a relaunch (exited zones aren't restored), or one the user had with the same agent outside Brindle, in its own command line or another editor. Every captured ACP agent keeps its conversations and lists them: `sessionCapabilities.list` is offered by Copilot 1.0.115, Codex ACP 2.12.0, claude-code-acp 0.16.2 and claude-agent-acp 0.112.0.

## What Changes

- A way to list an agent profile's conversations through ACP v1's `session/list` (`sessionCapabilities.list`), which answers with each session's id, working directory, title and last update, paged by a cursor, optionally filtered by directory.
- Picking one opens an agent zone that continues it, through `agents-session-recovery`'s continuing start (`session/load` folding the replay into a fresh log, or `session/resume` when only that is offered).

Open questions, to answer before a design:
1. **Where it shows:** a picker per agent profile (from the profile menu, the palette, or the sessions palette beside the terminal server's own sessions), or a list inside an exited or empty agent zone.
2. **Who runs `session/list`:** the core in a short-lived agent session (start the profile's agent, `initialize`, `session/list`, hang up), which needs a new client message and its replies, or a list kept from a running zone's agent. Starting an agent only to list can be slow (`npx` agents) and may demand a login.
3. **Which conversations:** all of the agent's, or those for the profile's directory (`cwd` filter), and whether conversations Brindle already has an open zone for are marked or left out.
4. **Paging and bounds:** how many pages are fetched, and how a long list is shown (`nextCursor`).
5. **Titles and times:** the agent's `title` and `updatedAt` are agent-provided text; how they are cleaned and bounded, as transcript text is.
6. **Deleting:** whether `session/delete` (`sessionCapabilities.delete`, offered by claude-agent-acp) is offered from the list, or left out as account-wide like logout.
7. **The command line:** whether `brindle agent PROFILE --continue ID` (or a listing subcommand) joins it.
8. **ACP v2:** v2 keeps `session/list`; whether its shape changes there, so `agents-acp-v2` only swaps the request.

## Capabilities

### New Capabilities
- Possibly none: the list may fit in `agent-zones` and `agent-sessions`.

### Modified Capabilities
- `agent-sessions`: listing an agent's conversations.
- `agent-zones`: picking one to continue.
- `command-line`, if a launch option joins.

## Impact

- `remote-agent/src/acp/`: `session/list` and its paged answers, possibly a listing-only agent session.
- `remote-protocol/` and `transcript/`: a request and its replies on client protocol 4 if released together with the `agents-*` changes, else a new client protocol version.
- `src/agent/`, the palette or the profile menu: the picker.
- The fake ACP agent plays `session/list` with the existing `respond` step.
