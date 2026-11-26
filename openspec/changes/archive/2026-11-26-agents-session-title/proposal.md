# Proposal

**Depends on:** `agents-sessions` (the reducer, the transcript and client protocol 4), `agents-profiles` (the zone's title and notifications), `agents-session-recovery` and `agents-login` (the restarts that keep a conversation), implemented and archived.
**Next:** `agents-zone-attention` (archived after this one: its notification text builds on this change's zone title) and `agents-session-info`, narrowed by this change to the context window's usage, the cost, naming a conversation from Brindle and the title in the session list.

## Why

In the user's manual checks (2026-11-25) the Codex and Claude tabs were titled `@agentclientprotocol/codex-acp` and `@agentclientprotocol/claude-agent-acp`. An agent zone takes its title from `initialize`'s `agentInfo.name`, which the npm adapters fill with their package name; the readable `agentInfo.title` (`Codex`, `Claude Agent`) is decoded and dropped. Three conversations with one agent are also three tabs with the same name, because ACP's `session_info_update`, which Claude, Copilot and Antigravity send to name the conversation (`Hello.txt file creation`), is ignored. Notifications repeat the problem: their title is the agent's name and then the tab's, which is the same name again (`@pkg — @pkg`).

The user wants the profile's name until the agent explicitly names the session, or the user renames the tab.

## What Changes

- **The zone's title** is the conversation's title once the agent has given one, else the profile's name. The agent's own name no longer titles the zone or its tab. A manual tab title still wins over both.
- **The server reads `session_info_update`.** A `title` sets the conversation's title; `null` or a blank one clears it; an update without `title` (Codex sends `_meta` only) changes nothing. The title is cleaned, its whitespace made single spaces, and cut at 256 characters, so Copilot's whole-prompt title stays one line.
- **The title is part of the conversation** the transcript already names (`Event::Conversation`), as an optional field. It is in every replay and checkpoint, so it survives a detach, a quit, a core restart and an upgrade, and a restart of the agent that continues the same ACP session keeps it. A new ACP session starts without one.
- **The agent's name is its readable one:** `agentInfo.title` when given and not blank, else `agentInfo.name`. That is the name the session list, the dump's `name=` and the zone's failure texts (`Claude Agent needs you to log in.`) use.
- **Notifications use the zone's title:** `Claude` or `Hello.txt file creation`, followed by the tab's title only when the user renamed the tab to something else.
- **Client protocol 4 is amended in place** (it has not been released): `Conversation` gains an optional `title`, omitted when unset, so every existing fixture still encodes byte for byte.

Not in this change, left to `agents-session-info`: the title in the session list and the sessions palette for detached sessions, telling the agent when the user renames the tab (`session/set_title`), and the context window's usage and cost.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`: "The transcript names the agent's conversation" (its title), "Transcript event forms are fixed for each protocol version" (an untitled conversation encodes as before); new requirements for `session_info_update`, the title across restarts, and the agent's readable name.
- `agent-zones`: "Agent zone title".
- `desktop-notifications`: "Agent notifications" (the title's names).

## Impact

- **`brindle-transcript`** (`transcript/`): `Conversation.title: Option<String>`; one new `tests/fixtures/v4/` file (`conversation-titled.json`), none edited; the replay property test and the fuzz corpus.
- **`brindle-remote-agent`** (`remote-agent/src/acp/`): `wire.rs` decodes `session_info_update` and prefers `agentInfo.title`; `reduce.rs` keeps the title (a pure `session_title` clean-up, the same-session carry in `ready`); its tests and the capture replays. The checkpoint's format number stays 1 (the log is replayed with the field).
- **`brindle`** (`src/`): `ChatModel::title()` and a separate agent name for failure texts; `policy::layout`'s agent arm takes whether the tab's title is manual; `Workspace::on_attention` passes it.
- **No new message, event kind, config key or state file.**
- **Tests:** reducer and wire unit tests over the captures, a real-role test across a core restart (`remote-agent/tests/agents.rs`), model and policy unit tests, and an e2e case `agent-session-title`; the agent e2e cases that expected the tab to read `Fake Agent` now expect the profile's name.
- **Other changes:** `agents-session-info`'s proposal is narrowed (this change edits it). `agents-tool-content` moves `wire.rs`'s v1 decoding behind its `Dialect` seam; whichever lands second moves or writes this decoding there.
