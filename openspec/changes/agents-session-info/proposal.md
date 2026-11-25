# Proposal

**Depends on:** `agents-commands` (the options shown under the prompt input, which this would sit beside, and through it `agents-tool-content`'s `Dialect` seam), implemented and archived.
**Next:** nothing depends on it.

**Status: proposal stage.** This change has no design, specs or tasks yet. It is taken further once `agents-commands` is designed, since both add to the same row under the prompt input.

## Why

Most ACP agents we captured tell their client two more things that Brindle throws away. `session_info_update` gives the conversation a title ("Hello.txt file creation"), and `usage_update` gives how full the context window is (`used` and `size` in tokens) and sometimes the session's cost. In step 1 an agent tab is titled with the agent's name, so three conversations with one agent are three tabs called the same thing, and the sessions palette can't tell them apart. And nothing shows that a conversation is near its context limit until the agent compacts or fails.

## What Changes

- The terminal server keeps the conversation's title and the latest usage as session state, in the replay and the checkpoint, and lists the title with the session.
- An agent zone's title, its tab and its row in the sessions palette show the conversation's title.
- An agent zone shows how full the context window is.

Open questions, to answer before a design:
1. **The title's place.** `agent-zones`' "Agent zone title" says the title is the name the agent gave, else the profile's name. Whether the conversation's title replaces that, follows it ("Claude: Hello.txt file creation"), or shows only in the tab's tooltip and the sessions palette; and what a manual tab title does to it.
2. **Bad titles.** Copilot's title is the whole first prompt. What bounds and cuts it.
3. **Usage:** where it shows (beside the options under the prompt input, or in the zone header), as what (tokens, a percentage, a bar), and whether it warns near the limit and at what share.
4. **Cost:** whether the session's cost is shown at all, given its unit may be money or credits.
5. **Naming a conversation from Brindle.** ACP has `session/set_title`, gated by a capability the Claude and Codex adapters advertise. Whether renaming a tab should tell the agent.
6. **The session list:** whether the title is in `SessionInfo` (an additive field) so the sessions palette and `brindle-remote list` show it for detached sessions.
7. **Notifications:** whether an agent notification names the conversation.

## What the full changes already say

- `agents-sessions`: the reducer ignores `session_info_update` and `usage_update` (`Update::Ignored`); the agent's name comes from `initialize`'s `agentInfo` and is in `AgentState::Ready`.
- `agents-profiles`, `agent-zones` requirement "Agent zone title": the name the agent gave, else the profile's name; a tab with a manual title keeps it. This change modifies it (question 1).
- `agents-commands`: the row under the prompt input and its `FooterItem`s, which usage could join (question 3).
- The captures: Claude, Codex, Copilot and Antigravity send both updates; Codex's `session_info_update` sometimes carries only `_meta`, with no title.
- ACP v2 keeps both updates in the same shape, so the v2 module maps them as the v1 one does.

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `agent-sessions` (added by `agents-sessions`): new requirements for the title and usage.
- `agent-zones` (added by `agents-profiles`): "Agent zone title", and a new requirement for usage.
- `terminal-server`: if the session list gains the title.

## Impact

- `transcript/` and `remote-agent/src/acp/`: two session-state events and their decoding in each version's module.
- `remote-protocol/`: an additive `SessionInfo` field, if question 6 says so.
- `src/agent/`, `src/workspace.rs`, the sessions palette: the title; the usage display.
