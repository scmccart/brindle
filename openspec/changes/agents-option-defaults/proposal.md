# Proposal

**Depends on:** `agents-commands` (the session options the server keeps, and the request that sets one), implemented and archived.
**Next:** nothing depends on it.

**Status: proposal stage.** This change has no design, specs or tasks yet. The user left it out of `agents-commands` on 2026-11-25, to be taken further once that change is archived.

## Why

With `agents-commands` a user can see and change an agent's mode, model and other options, but only by hand and only for the conversation in front of them. Every new conversation starts with whatever the agent picks. Someone who always wants one agent to start in its plan mode, or on a particular model, sets it again each time, and a conversation started from a script with `brindle agent -p` runs its first prompt before anyone can.

## What Changes

- An agent profile can name starting values for its agent's options, which the terminal server sets once, when the session is ready and before the first prompt is sent.

Open questions, to answer before a design:
1. **The key's shape:** a table on the profile such as `options = { mode = "plan", model = "opus" }`, and whether it names an option by the agent's id, by its name, or by category (`mode`, `model`), given that the same setting has different ids and names with different agents.
2. **What a value matches:** a choice's value, its name, or either, and with or without regard to case.
3. **A value the agent doesn't offer,** or an option it doesn't have: where that is said (the zone's conversation, the config line), and whether the first prompt still goes.
4. **Order with the first prompt:** a `-p` prompt must wait for the options to be set, or for the agent to refuse them; how long, and what a slow agent does to the launch.
5. **Who applies them:** the server (a field on the message that starts an agent session, so they apply with no Brindle attached) or Brindle (after the session is ready, so nothing new is on the wire).
6. **What they apply to:** new conversations only, or also a conversation continued by `agents-session-recovery`, whose agent may have restored its own values.
7. **Remembering the last choice** per profile, instead of or beside fixed values, and where that would be stored.
8. **The command line:** whether `brindle agent` takes starting values too (`--mode plan`).
9. **The settings dialog,** which doesn't edit agent profiles today.

## What the full changes already say

- `agents-commands`, Non-Goals: starting values in a profile and remembering a choice across sessions are left to this change. Its design decision 4 gives the set request, its checks (`UnknownOption`, `NotOffered`, `Busy`) and "one change at a time", which several starting values would have to queue behind.
- `agents-commands`, design decision 6: a first prompt from `brindle agent -p` is always sent as text, so `-p "/mode plan"` is not a way to start in a mode.
- `agents-profiles`: an agent profile's keys are `command`, `args`, `cwd`, `env`, `theme`, `font_size` and `remote`; the settings dialog lists agent profiles and doesn't edit them.
- `agents-sessions`: `SpawnAgent` carries the command, arguments, directory and environment (question 5).

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `profiles`: the new key on an agent profile.
- `agent-sessions` (added by `agents-sessions`): applying starting values, if the server does it.
- `agent-zones` (added by `agents-profiles`): "A launch's first prompt", for the order with the first prompt, and what a value the agent doesn't offer shows.
- `command-line`: only if `brindle agent` takes starting values.

## Impact

- `src/config.rs` and `assets/default-config.toml`: the profile key.
- `remote-protocol/` and `remote-agent/src/acp/`: an additive field on `SpawnAgent` and the reducer's start sequence, if the server applies them.
- `src/agent/`: sending the values and holding the first prompt, if Brindle applies them.
