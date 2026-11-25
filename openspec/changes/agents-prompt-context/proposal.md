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
- **Attached text is kept beside the conversation, not in it.** The terminal server writes each text attachment to a file under the user's cache directory (`~/.cache/brindle`), private to the user, on the agent's host, so it is on disk and outlives a reboot. The log, a queued prompt and the checkpoint hold only a reference (a name, a size, an id), so attachments never grow them. Brindle reads a text back when the user opens its row.
- **The server sends them as ACP content.** A link becomes a resource link, which every agent must accept. Kept text of at most 64 KiB is embedded in the prompt for an agent that takes embedded context, so it is seen at once; larger text, and any text for an agent that doesn't, is sent as a link to the kept file. So no agent is left out.
- **Attachments stay with their prompt:** in the queue slot while it waits or is held, across a quit, a core restart and an upgrade, and in the conversation, where the user's message lists them.
- **Kept text goes when nothing refers to it:** with a discarded prompt and with a log entry the log's bound drops. After its session ends it stays 14 days by default, so a link the agent saved still works if the conversation is taken up again, and is then removed.
- **Where and how long are settings.** `[agent] attachments_keep_days` sets the time, for every host. `[agent] attachments_dir` sets the directory for agents on this machine, and `attachments_dir` in a profile's `remote` table sets it for that host, since a path belongs to one machine.
- **Bounds:** 8 attachments a prompt; 256 KiB a text attachment; 16 MiB kept a session. Longer command output keeps its end; a longer file keeps its start; the item says it was cut.
- **Client protocol 4 is amended**, as for the rest of the series: two client messages (attach a text, read one back) and one server message are added, and `AGENT_PROMPT`, the user entry and the queue slot gain an optional list.

Not in this change: images and audio (`agents-prompt-images`); completing a path after `@` (`agents-file-mentions`); attachments on a first prompt from `brindle agent -p`; turning a long paste into an attachment; attaching a directory; reading a file's contents from the agent's host; a prompt of attachments with no text; sending a prompt automatically when something is attached; attachment items as keyboard focus stops.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`: new requirements for a prompt's attachments, their bounds, how each kind is sent, and their life in the queue slot and the log.
- `agent-zones`: new requirements for attachments in the prompt input and the conversation, attaching a file, dropping files, and sending a command's output to an agent.
- `ui-components`: the prompt input's attachment row.
- `command-line`: the dump's lines for attachments, and driving them.
- `configuration`: `[agent]`'s `attachments_dir` and `attachments_keep_days`.
- `profiles`: `attachments_dir` on a profile's remote.

## Impact

- **`brindle-transcript`** (`transcript/`): `Attachment` (a link, or a reference to kept text); `Entry::User`'s and `Slot`'s optional attachments; new `tests/fixtures/v4/` files (none edited). No new event kind.
- **`brindle-remote-protocol`** (`remote-protocol/`): `AGENT_ATTACH`, `AGENT_READ_ATTACHMENT` and `ATTACHMENT`, and `AGENT_PROMPT`'s optional `attachments`, with fixtures and fuzzing. Their numbers are the next free ones when the stack is assembled.
- **The terminal server** (`remote-agent/src/server/`): a store of kept texts, one private directory a session under the cache directory (a new `BRINDLE_CACHE_DIR` names it for tests and throwaway runs), with its decisions in a pure module and its file work in `server/platform/`. The holder is untouched.
- **`brindle-remote-agent`** (`remote-agent/src/acp/`): the agent's prompt support read from `initialize` in each version's module; the reducer's checks and its choice of embedded text or a link; `acp/v1/encode.rs` writing resource links and embedded resources; a pure path-to-URI helper. The checkpoint's format 1 is amended (unreleased).
- **`brindle`**: `src/agent/` (the draft's attachments, the drop plan, the conversation's rows, the dump), `src/terminal_view.rs` and `src/workspace.rs` (the terminal action and choosing the agent zone), `src/actions.rs`.
- **`brindle-ui`** (`ui/`): `PromptInput` gains an attachment row. Its dependency list is unchanged.
- **Tests:** real-role tests through the fake ACP agent; new `agent-attach-*` and `terminal-to-agent-*` e2e cases; a manual check for dropping files.
- **Docs:** `docs/code-map.md`, `README.md`, `CLAUDE.md`.
