# Proposal

**Depends on:** `agents-prompt-context` (link attachments) and `agents-commands` (the suggestion list above the prompt input), implemented and archived.
**Next:** nothing depends on it.

**Status: proposal stage.** This change has no design, specs or tasks yet. It was left out of `agents-prompt-context` on 2026-11-25.

## Why

With `agents-prompt-context` a file is attached by typing its whole path into a prompt (Attach File…) or by dropping it. Most agent clients let the user type `@` and a few letters and pick the file from a list, which is faster and catches a wrong path before it is sent. Brindle can't do that yet because the files are on the agent's host, which may be another machine or a container, and nothing lets Brindle list a directory there.

## What Changes

- Typing `@` in an agent zone's prompt input lists files under the session's working directory that match what follows, in the suggestion list `agents-commands` adds; choosing one attaches it as a link.
- The terminal server answers a request to list or search the files of a session's working directory, on the host where the agent runs.

Open questions, to answer before a design:
1. **The listing request:** a new client message, what it returns (one directory's entries, or matches for a query under the whole tree), its bounds, and whether it runs on the core's thread or elsewhere, since a walk of a large tree mustn't stall the server.
2. **What is listed:** whether ignore files are honoured (which needs a parser), hidden files, symbolic links, and how deep.
3. **What an agent may learn.** The server would read directory names for a client, not for the agent, so `agents-sessions`' bound on what an agent can make the server do still holds; whether listing outside the working directory is allowed at all.
4. **In the text or as an item:** whether a chosen file stays in the text as `@path` (and what is then sent to the agent) or becomes an attachment item and leaves the text.
5. **`@` that isn't a mention:** an e-mail address, a decorator, a package scope. When the list shows, and that nothing is ever attached without a choice.
6. **Keys,** which must agree with the slash command list's: up, down, enter, tab, escape.
7. **Older servers:** what the zone does for an endpoint whose server can't list.

## What the full changes already say

- `agents-prompt-context`, Non-Goals: completing a path after `@` and listing an agent host's files are this change's. Its design decision 4: the server never reads or checks a linked file (question 3).
- `agents-commands`, design decisions 6 and 7: the suggestion list, its keys and `PromptInput::set_suggestions`, which this change reuses (question 6).
- `agents-sessions`, Risks: an agent can't make the core read files (question 3).

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `agent-zones` (added by `agents-profiles`): the `@` list.
- `terminal-server`: the listing request.
- `agent-sessions` (added by `agents-sessions`): if the listing is per agent session.

## Impact

- `remote-protocol/`: a listing request and its answer, with fixtures and fuzzing.
- `remote-agent/src/server/`: listing or searching a directory within bounds, on every platform the server runs on.
- `src/agent/`: the `@` completion and its requests.
