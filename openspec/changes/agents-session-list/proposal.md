# Proposal

**Depends on:** `agents-session-recovery` (continuing a conversation by its ACP session id: `SPAWN_AGENT`'s `conversation`, the transcript's `Conversation`, the choice of `session/resume` or `session/load`), `agents-login`, `agents-session-title`, `agents-log-paging` and `agents-tool-content` (the ACP seam: a version-free reducer, one module per version behind `Dialect`), implemented and archived; and `zones-mixed-tabs` (a pane of any kind split with an agent zone), implemented and archived (PR #149).
**Next:** none named.

## Why

`agents-session-recovery` continues a conversation Brindle still shows: one in a zone whose agent exited or was lost. A conversation Brindle has no zone for can't be picked up: one whose zone was closed, one that ended while Brindle was closed, one from before a relaunch, or one the user had with the same agent in its own command line or another editor. Every captured ACP agent keeps its conversations and lists them: Claude, Codex, Copilot and Antigravity all offer `sessionCapabilities.list`.

The agents' own tools have two habits for this, `-c` to carry on with the last conversation and `-r` to choose one, and the user wants both.

## What Changes

- **A list of an agent's earlier conversations in this directory**, in a palette: each with its title and when it was last active. It lists the conversations for the directory the pane is in, and no others.
- **Two ways in.** Agent Conversations… in an agent zone lists that zone's profile. Continue Agent Conversation… anywhere asks which agent profile first when the config has more than one.
- **Who lists.** A zone whose agent is running asks its own agent. Otherwise the terminal server starts the profile's agent to list, and that same agent then continues the conversation chosen, so one start serves both; if nothing is chosen it is hung up.
- **Where the chosen conversation opens.** In the pane itself when the pane has no agent running (a zone whose agent stopped). From a pane with a running agent or a terminal, the palette asks: split right, split down, or a new tab. A conversation already open in a zone is marked, and choosing it shows that zone.
- **A zone whose agent stopped offers the choice too:** beside Continue conversation it shows Conversations… and New conversation.
- **`brindle agent PROFILE -c`** (`--continue`) opens a tab that carries on with the last conversation in the directory. **`-r`** (`--resume`) opens a tab on the list. With no earlier conversation, `-c` opens the same choosing screen with the reason, where a new conversation can be started.
- **Client protocol 4 is amended**: two client messages (list, 31; choose, 64), one reply (45), one more way to start an agent session, and one more agent state. The ACP checkpoint's format 2 is amended with the choosing phase and the list capability.

Not in this change: conversations of other directories; deleting, archiving or renaming a conversation; forking one; searching the list; `-r` or `-c` with a conversation's id; previews of a conversation before it is opened; listing in the sessions palette; remembering the list between uses.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`: an agent session that starts by choosing; listing an agent's conversations; choosing one or a new one.
- `agent-zones`: the conversations palette, where a choice opens, the choosing screen, and a stopped zone's choices.
- `command-palette`: the two actions and the palette's rows.
- `command-line`: `-c` and `-r` under `agent`, and the dump's lines.
- `session-restore`: a zone that is still choosing isn't recorded.

## Impact

- **`brindle-transcript`** (`transcript/`): `AgentState::Choosing`; a new `tests/fixtures/v4/` file, none edited.
- **`brindle-remote-protocol`** (`remote-protocol/`): `AGENT_LIST`, `AGENT_CHOOSE` and `AGENT_CONVERSATIONS`; `SPAWN_AGENT`'s optional `start`; fixtures and fuzzing.
- **`brindle-remote-agent`** (`remote-agent/src/acp/`): the forms (`forms.rs`) and `v1/` for `session/list`, behind `Dialect`; the reducer's `Phase::Choosing` and `Start::Choose`; the core's handlers (`server/core/agents.rs`, `policy.rs`). The checkpoint's format 2 is amended (unreleased).
- **`brindle`** (`src/agent/`, `src/workspace.rs`, `src/zone_view.rs`): the pure plans (who lists, which directory, where a choice opens, `-c`'s choice), the palette, the choosing screen, the stopped zone's buttons; two actions in `src/actions.rs`; `-c` and `-r` in `src/main.rs` and the hand-off (`src/instance.rs`, a new request kind `agent-choose`).
- **`brindle-ui`**: nothing new; the palette and the prompt input's reason line are used as they are.
- **Tests:** a capture of each agent answering `session/list`, which no fixture holds yet; real-role tests through the fake ACP agent; new `agent-conversations-*` e2e cases.
- **Docs:** `README.md`, `docs/code-map.md`, `CLAUDE.md`.
