# Proposal

**Depends on:** `agents-profiles` (the agent zone, its prompt input and queue row, and through it `agents-sessions`' prompts, queue slot, transcript and client protocol 4), `agents-markdown` (the code block that draws an attachment's text) and `agents-tool-content` (the `Dialect` seam), implemented and archived.
**Next:** `agents-prompt-images` (images in a prompt, with the chunked upload they need) and `agents-file-mentions` (completing a path as the user types `@`), both proposal-only; then `agents-plans`.

## Why

In step 1 a prompt is plain text. To show an agent a compiler error the user copies it out of a terminal tab and pastes it into the agent's prompt, and to point it at a file they type the path into a sentence and hope the agent reads it. ACP has two better ways that every agent we captured accepts: a link to a file, which the agent reads itself, and text embedded in the prompt as a named piece of context. Brindle is a terminal, so the most useful context is already on its screen: the text the user selected, and, where the shell has integration loaded, the exact output of the last command. Nothing connects the two today.

## What Changes

- **A prompt can carry attachments** beside its text, of two kinds: a **link** to a file on the agent's host, and **text** with a name (a command's output, a file's contents). Images are the next change.
- **Sending a command's output to an agent.** In a terminal zone, the new `send_last_output_to_agent` action adds the last command's output, named by its command line, to an agent zone's prompt as a text attachment, and shows that zone. It is the same text Copy Last Command Output copies. With several agent zones open, a palette asks which. Nothing is sent until the user sends the prompt. A selection needs no action: it is copied and pasted into the prompt.
- **Attaching a file.** The new `agent_attach_file` action asks for a path on the agent's host and attaches it as a link; dropping files on an agent zone attaches them. A dropped file is a link when the agent runs on this machine; for an agent on another host or in a container, where the path means nothing, a dropped text file is embedded as text instead and anything else is refused, saying why.
- **Attachments show in the prompt input** as a row of items above the text, each with its kind, name and size. Backspace at the start of the text removes the last one; `agent_clear_attachments` removes them all. No keyboard stop is added.
- **The server sends them as ACP content.** A link becomes a resource link, which every agent must accept. Text becomes an embedded resource when the agent says it takes embedded context, and is otherwise written into the prompt's text under its name, so no agent is left out.
- **Attachments stay with their prompt:** in the queue slot while it waits or is held, across a quit, a core restart and an upgrade, and in the conversation, where the user's message lists them and a text attachment opens to show what was sent.
- **Bounds:** 8 attachments a prompt; 64 KiB a text attachment and 128 KiB together. Longer command output keeps its end; other text keeps its start; the item says it was cut.
- **Client protocol 4 is amended**, as for the rest of the series: `AGENT_PROMPT`, the user entry and the queue slot gain an optional list, and one transcript event kind is added. No client message is added.

Not in this change: images and audio (`agents-prompt-images`); completing a path after `@` (`agents-file-mentions`); attachments on a first prompt from `brindle agent -p`; turning a long paste into an attachment; attaching a directory; reading a file's contents from the agent's host; a prompt of attachments with no text; sending a prompt automatically when something is attached; attachment items as keyboard focus stops.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`: new requirements for a prompt's attachments, their bounds, how each kind is sent, and their life in the queue slot and the log.
- `agent-zones`: new requirements for attachments in the prompt input and the conversation, attaching a file, dropping files, and sending a command's output to an agent.
- `ui-components`: the prompt input's attachment row.
- `command-line`: the dump's lines for attachments, and driving them.

## Impact

- **`brindle-transcript`** (`transcript/`): `Attachment`; `Entry::User`'s and `Slot`'s optional attachments; `Event::Attached`; the fold and replay; new `tests/fixtures/v4/` files (none edited).
- **`brindle-remote-protocol`** (`remote-protocol/`): `AGENT_PROMPT`'s optional `attachments`, additive, with fixtures and fuzzing.
- **`brindle-remote-agent`** (`remote-agent/src/acp/`): the agent's prompt support read from `initialize` in each version's module; the reducer's checks, bounds and the fallback for text; `acp/v1/encode.rs` writing resource links and embedded resources; a pure path-to-URI helper. The checkpoint's format 1 is amended (unreleased).
- **`brindle`**: `src/agent/` (the draft's attachments, the drop plan, the conversation's rows, the dump), `src/terminal_view.rs` and `src/workspace.rs` (the terminal action and choosing the agent zone), `src/actions.rs`.
- **`brindle-ui`** (`ui/`): `PromptInput` gains an attachment row. Its dependency list is unchanged.
- **Tests:** real-role tests through the fake ACP agent; new `agent-attach-*` and `terminal-to-agent-*` e2e cases; a manual check for dropping files.
- **Docs:** `docs/code-map.md`, `README.md`, `CLAUDE.md`.
