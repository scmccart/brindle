# Proposal

**Depends on:** `agents-tool-content` (the `Output` content item, the `Dialect` seam and its captures of which agents call `terminal/create`), implemented and archived.
**Next:** nothing depends on it.

**Status: proposal stage.** This change has no design, specs or tasks yet. It is taken further once `agents-tool-content` is archived and its capture task has shown which ACP v1 agents use the capability.

## Why

An ACP agent that runs a command either runs it itself and describes the output, or, in ACP v1, asks its client to run it through the `terminal` client capability. Brindle offers no such capability, so every agent runs its own commands and Brindle shows what the agent chooses to report, as plain text (`agents-tool-content`). Brindle is a terminal emulator whose server already owns PTYs that outlive Brindle, core restarts and upgrades: it could run an agent's command in a real terminal, drawn by the real emulator, that the user can watch, scroll and stop.

## What Changes

- The terminal server, as the ACP client, offers `clientCapabilities.terminal` to v1 agents and answers `terminal/create` (a command, arguments, environment, directory and an output byte limit), `terminal/output`, `terminal/wait_for_exit`, `terminal/kill` and `terminal/release`.
- A command the agent starts this way runs in the terminal server, on the agent's host, and its output is shown with the tool call that embeds its terminal.
- ACP v2 has no such methods: a v2 agent owns its terminals and the client only displays them. So this is a v1-only feature, inside the v1 module behind `Dialect`.

Open questions, to answer before a design:
1. **Whether to offer it at all.** `agents-sessions` bounded what an agent can make the server do: "it can't make it read files or run commands (no `fs`, no `terminal`)". Offering the capability removes half of that bound, and the command runs outside whatever sandbox the agent applies to commands it runs itself.
2. **Who uses it.** Which agents call `terminal/create` when it is offered, from `agents-tool-content`'s captures. If none of the agents we can run does, there is little to gain.
3. **Permission.** Whether the server runs a `terminal/create` at once, as ACP has it (the agent is expected to have asked permission through `session/request_permission` first), or asks the user itself.
4. **Where it runs and how it is shown:** on a PTY or on pipes; in the tool row as plain text, in the tool row through the emulator, or as a terminal zone of its own that the user can type into.
5. **Lifetime:** what a core restart, an upgrade, the agent's exit and closing the zone do to a command the server started for the agent, and whether such a session is listed.
6. **Limits:** how many terminals an agent may hold, and the output byte limit when the agent names none.
7. **The `fs` capability,** which ACP v1 pairs with this one and v2 also drops: in scope here, a change of its own, or never.

## What the full changes already say

- `agents-sessions`, design decision 4: `initialize` sends `clientCapabilities: {}`; its `agent-sessions` requirement "The server answers the agent's requests" answers `terminal/*` with `-32601`. This change modifies both.
- `agents-sessions`, Risks: an agent can't make the core read files or run commands (question 1).
- `agents-tool-content`, design decision 6: the capability stays unoffered there, and its task 1.1 records which agents call `terminal/create` when offered (question 2). Its `Output` item and plain-text filter are what a terminal's output would fill.
- `agents-acp-v2`: v2 has no client terminal methods, so nothing here applies to a v2 agent.

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `agent-sessions` (added by `agents-sessions`): "The server is the agent's ACP client" and "The server answers the agent's requests", and new requirements for the commands the server runs for an agent.
- `agent-zones` (added by `agents-profiles`): how such a command is shown, if it is more than `agents-tool-content`'s output block.
- `terminal-server`: if the commands are sessions of their own.

## Impact

- `remote-agent/src/acp/v1/`: the capability and the five requests.
- `remote-agent/src/server/`: starting, reading, ending and freeing a command for an agent session, and its checkpoint.
- `transcript/` and `src/agent/`: only if the output is shown as more than an `Output` item.
- The fake ACP agent gains `terminal/*` requests, for server tests and e2e cases.
