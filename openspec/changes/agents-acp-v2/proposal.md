# Proposal

**Depends on:** `agents-profiles` (and through it `agents-sessions`, whose ACP reducer and transcript shaped for v2 this change builds on), implemented and archived; best started once ACP v2 leaves Preview.
**Next:** nothing: the last of the series.

**Status: proposal stage.** This change has no design, specs or tasks yet. It is taken further once `agents-profiles` is archived (and with it `agents-launch-commands` and `agents-sessions`), and preferably once ACP v2 leaves Preview.

**Decided (user, 2026-11-25):** this change starts only after ACP v2 leaves Preview, and then Brindle always offers v2 (question 1). Its other open questions are deferred until then. `agents-session-recovery` is v1-only (it calls `session/load` and `session/resume` directly), and `agents-login` sends v1's `authenticate` and `$/cancel_request`, so this change reshapes the recovery and login requests (`session/resume { replayFrom }`, `auth/login`, v2's required method `type` and `capabilities.auth.terminal: {}`).

## Why

In step 1 the terminal server speaks ACP v1 only. ACP v2 went to Preview on 2026-11-20, with Zed, JetBrains and the Claude and Codex adapters building on it, and the ACP project says to support v1 and v2 side by side rather than drop v1. Agents will move to v2 over time; some features (background work, subagents, structured diffs, flexible permission requests) exist only there, and a v2-only agent can't be used from Brindle at all: it rejects a v1 `initialize`.

## What Changes

- The terminal server, as the ACP client, offers v2 in `initialize` and speaks whichever version each agent selects, keeping v1 for agents that answer with it. Brindle and its client protocol stay unaware of the version as far as possible: the transcript events are Brindle's own.
- The main v1 → v2 differences the server's reducer would absorb, per ACP's migration guide:
  - the `session/prompt` response only acknowledges that the user message was inserted (with its `messageId`); the turn ends with an idle `state_update` carrying the stop reason, `error` included;
  - `state_update` reports `running`, `requires_action`, `idle` and `unknown`, with a `background` list of work still running;
  - messages and tool calls are upserts by id; tool calls gain a `paused` status and per-item cancel;
  - a v2 agent must accept a prompt sent during active work, inserting it at once or holding it;
  - almost every enum accepts unknown values;
  - `session/load` is gone and `session/resume` takes a `replayFrom` cursor; `authenticate` and `logout` become `auth/login` and `auth/logout`; the client file system, terminal and session modes APIs are gone (step 1 offers none of them).

Open questions, to answer before a design:
1. **When v2 is offered:** always, per profile, or as an opt-in while v2 is in Preview (ACP suggests keeping it opt-in for users during Preview).
2. **The queue of one.** Brindle's server holds a second prompt until a turn ends normally; a v2 agent takes prompts at any time and decides whether one steers the running work or waits. Whether the server keeps its own slot for v2 agents or hands prompts to the agent, and what the held states then mean.
3. **What is new to show:** background work, subagents, paused tool calls, the `unknown` state, structured diffs. Each new event kind or value needs a new client protocol version, and the server must say what a protocol 4 client gets instead.
4. **The status mapping** for `requires_action` that isn't a permission request (an elicitation, which step 1 refuses) and for `unknown`.
5. **Prompt retries:** whether to send v2's `idempotencyKey`, which could close step 1's known gap where a core killed between checkpointing a prompt and writing it leaves the turn stuck.
6. **The schema crate:** whether the pinned `agent-client-protocol-schema` carries a v2 that matches the spec at the time, and what moving the pin brings into `Cargo.lock`.
7. **The follow-ups' v2 halves:** if `agents-login` and `agents-session-recovery` have landed, their v1 requests map onto `auth/login` and `session/resume { replayFrom }` here. From `agents-session-list` it inherits listing and choosing: `session/list` (with `cwd` and `cursor`, answered with `sessions` and `nextCursor`) is unchanged in v2, so the list, its bounds and `AGENT_LIST`/`AGENT_CONVERSATIONS` carry over as they are, and a chosen conversation continues through v2's `session/resume`, as the recovery requests do, since `session/load` is gone. From `agents-commands` (its design decision 3) it inherits commands and session options, behind the version-neutral forms in `acp/forms.rs` (`OfferedOption { option, key }`, `OptionKey`, `Offered`, `Update::{Commands, Options, Current}`, `Request::SetOption`): a v2 module maps `configOptions` (in the `session/new` and continue answers, `config_option_update` and the answer to `session/set_config_option`) to options keyed `OptionKey::Config { id }`, as v1 does, and `available_commands_update` to `Update::Commands`. v2 has no session modes API and no `models` list, so `OptionKey::Modes` and `OptionKey::Models` stay v1's (nothing outside `acp/v1/` builds or matches them) and a v2 agent's mode is a config option of category `mode`; neither Brindle nor the reducer changes.

## What the full changes already say

- `agents-sessions`, design decision 7: updates are by id and the end of a turn is a state event (`Turn`), never inferred from a response, "that is the shape ACP v2 has", so `agents-acp-v2` changes only the reducer (confirmed: ACP v1 only, shaped so v2 fits). `StopReason::Other { name }` keeps a stop reason a newer agent sends.
- `agents-sessions`, design decision 4: `initialize` asks for version 1; an answer with another version fails the session's agent ("the agent speaks ACP version N; Brindle speaks 1") and hangs it up. The `agent-sessions` scenario "Other ACP version" says the same, ending the session. This change replaces that rule.
- `agents-sessions`, design decision 11: the checkpoint records the negotiated ACP version, so a core can tell a session's version after a restart or upgrade.
- `agents-sessions`, design decision 6 and the requirement "Transcript event forms are fixed for each protocol version": within client protocol 4 an event may only gain optional fields; a new kind or enum value comes with a new protocol version (question 3).
- `agents-sessions`, design decision 11's known gap: the prompt/checkpoint write window (question 5).
- `agents-profiles`, design decision 1: nothing in Brindle keys on ACP's method names, so `agents-acp-v2` changes nothing in that change's code (true unless question 3 adds something to show).
- The brief behind both changes notes that the schema crate carries both v1 and v2.
- `agents-tool-content` split the helper's ACP client into one module per protocol version behind `Dialect` (`remote-agent/src/acp/v1/`, with the version-neutral forms in `acp/forms.rs`), so v2 is a new `acp/v2/` module and `Dialect::V2`'s arms, with no version checks elsewhere. `agents-prompt-context` (built 2026-11-28) adds a prompt's parts to those forms: `acp/forms.rs`'s `PromptPart::{Text, Link { uri, name }, Embedded { uri, name, text }}`, which `acp/v1/encode.rs` writes as `text`, `resource_link` and a `resource` of `text/plain`, and `PromptSupport { embedded, images }`, which `acp/v1/decode.rs` reads from `initialize`'s `promptCapabilities` and the checkpoint keeps. A v2 module writes the same three parts and reads the same capability (both are the same in v2); the reducer's choice of embedded or link (`acp/reduce/attach.rs`, `EMBED_LIMIT`) and the session's attachments directory in `additionalDirectories` at `session/new` and the continues (`reduce/lifecycle.rs`'s `directories`, for an agent that offers `sessionCapabilities.additionalDirectories`) carry over, with `session/resume` the only continue. Its design decision 9 maps v2's tool content onto the forms it added: `tool_call_update` upserts as `Patch::{Keep, Clear, Set}` and a list replacing the items, `tool_call_content_chunk` appending one, `diff { changes, patch }` as one `FileChange` per change with hunks from a pure Git-patch parser, `terminal` content, `terminal_update` and `terminal_output_chunk` as the open outputs' `Opened`, `Appended` and `Exited` (an output stays open until its exit), and `paused`, `cancelled` and unknown statuses as new `ToolStatus` values this change adds. Nothing in `brindle-transcript`, the reducer or Brindle changes for them.

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `agent-sessions` (added by `agents-sessions`): "The server is the agent's ACP client" (version negotiation), "Prompts", "A queue of one prompt", "Cancelling a turn", and the transcript events for anything new.
- `agent-zones` (added by `agents-profiles`): only if new rows or states are shown (question 3).
- `terminal-server`: "Protocol versions", if a new client protocol version is needed.
- `profiles`: only if v2 is chosen per profile (question 1).

## Impact

- `remote-agent/src/acp/`: a v2 reducer beside the v1 one, chosen per session by the negotiated version, and the checkpoint's version handling.
- `remote-agent/Cargo.toml` and `Cargo.lock`: the schema crate's pin, under the `Cargo.lock` rule in `CLAUDE.md`.
- `transcript/` and `remote-protocol/`: only for new event kinds, with a new client protocol version.
- The fake ACP agent gains a v2 mode, for server tests and e2e cases.
