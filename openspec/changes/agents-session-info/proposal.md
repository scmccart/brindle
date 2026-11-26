# Proposal

**Depends on:** `agents-commands` (the options shown under the prompt input, which usage would sit beside, and through it `agents-tool-content`'s `Dialect` seam) and `agents-session-title` (the conversation's title, which this change lists), implemented and archived.
**Next:** nothing depends on it.

**Status: proposal stage.** This change has no design, specs or tasks yet. It is taken further once `agents-commands` is designed, since both add to the same row under the prompt input.

**Narrowed on 2026-11-25** by `agents-session-title`, which took the conversation's title in the zone, its tab and its notifications: the server reads `session_info_update`'s `title` and keeps it as an optional `title` of the transcript's `Conversation`, and the zone is titled with it, else with the profile's name. What is left here is the title outside an attached zone, naming a conversation from Brindle, and the context window's usage.

## Why

Most ACP agents we captured also send `usage_update`, which gives how full the context window is (`used` and `size` in tokens) and sometimes the session's cost; Brindle throws it away, so nothing shows that a conversation is near its context limit until the agent compacts or fails. And once `agents-session-title` is in, a conversation's title shows on its tab but not where detached sessions are listed: the sessions palette and `brindle-remote list` still can't tell three conversations with one agent apart.

## What Changes

- The terminal server keeps the latest usage as session state, in the replay and the checkpoint.
- An agent zone shows how full the context window is.
- The session list carries the conversation's title, so the sessions palette and `brindle-remote list` show it for detached sessions.
- Possibly: renaming an agent tab tells the agent (`session/set_title`).

Open questions, to answer before a design:
1. **Usage:** where it shows (beside the options under the prompt input, or in the zone header), as what (tokens, a percentage, a bar), and whether it warns near the limit and at what share.
2. **Cost:** whether the session's cost is shown at all, given its unit may be money or credits.
3. **Naming a conversation from Brindle.** ACP has `session/set_title`, gated by a capability the Claude and Codex adapters advertise. Whether renaming a tab should tell the agent, and whether the agent's later title then still loses to the manual one (it does in `agents-session-title`).
4. **The session list:** the title as an additive `SessionInfo` field, and how the sessions palette shows it beside the profile.

Answered by `agents-session-title` (the former questions 1, 2 and 7): the conversation's title replaces the zone's title and a manual tab title wins over it; it is made one line and cut at 256 characters; notifications name it.

## What the full changes already say

- `agents-sessions`: the reducer ignores `usage_update` (`Update::Ignored`).
- `agents-session-title`: `Conversation.title`, set and cleared from `session_info_update`, kept across core restarts, upgrades and restarts that continue the same ACP session.
- `agents-commands`: the row under the prompt input and its `FooterItem`s, which usage could join (question 1).
- The captures: Claude, Codex, Copilot and Antigravity send `usage_update`.
- ACP v2 keeps `usage_update` in the same shape, so the v2 module maps it as the v1 one does.

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `agent-sessions` (added by `agents-sessions`): new requirements for usage.
- `agent-zones` (added by `agents-profiles`): a new requirement for usage.
- `terminal-server`: the session list's title.
- `command-palette`: the sessions palette's title.

## Impact

- `transcript/` and `remote-agent/src/acp/`: a usage event and its decoding in each version's module.
- `remote-protocol/`: an additive `SessionInfo` field for the title.
- `src/agent/`, the sessions palette: the usage display and the listed title.
