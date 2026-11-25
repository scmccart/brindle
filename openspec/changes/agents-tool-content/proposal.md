# Proposal

**Depends on:** `agents-profiles` (the agent zone, its tool and permission rows, and through it `agents-sessions`' reducer, transcript and client protocol 4) and `agents-markdown` (the renderer and code block that draw a tool call's text), implemented and archived. In the PR stack it sits on top of `agents-log-paging`, so it also meets `agents-session-recovery`, `agents-login` and `agents-log-paging`; none of them changes a tool or permission requirement.
**Next:** `agents-plans`, `agents-commands` and `agents-prompt-context`, each still to be proposed.

## Why

In step 1 a tool call is one line (an icon, a title, a status) and a permission request is a title with buttons. The agent's edit, the command it wants to run and the output it got never reach the user: the terminal server drops a tool call's content before it enters the conversation log. So the user allows or rejects `Edit main.rs` without seeing the edit, which undercuts the rule that nothing runs unseen, and afterwards can't see what a tool did without asking the agent.

Brindle will speak ACP v1 and v2 side by side for good, so that no agent is left out (decided 2026-11-25). The two versions describe a tool call's content differently: v1 sends a diff as a path with the old and new text and replaces the content list whole; v2 sends file changes with an optional Git patch, appends content items, and streams an agent-owned terminal's output. This change is the first to add a feature both versions carry, so it also fixes how a version difference is written: once, in that version's own implementation, never as version checks spread through the reducer.

## What Changes

- **Tool calls keep their content.** The terminal server keeps, for each tool call, its text, its file changes as diff hunks, its command output, the file locations it names and its raw input and result, as Brindle's own transcript forms. They are in the log, the checkpoint and every replay, so they survive a detach, a quit, a core restart and an upgrade.
- **Everything kept is bounded** (`railway R5`): 64 KiB for each content item, 32 items and 256 KiB for a tool call, 16 KiB for each of the raw input and result, with what was left out counted and shown. Command output keeps its end, the rest keeps its start. Escape sequences are removed from command output, which is shown as plain text.
- **Diffs are computed on the server.** For ACP v1 the server compares the old and new text and keeps only the changed lines with their context, so a one-line edit of a large file costs a few lines of log, and Brindle never sees which ACP version the agent speaks.
- **A tool row opens.** A tool call with content shows a summary on its line (lines added and removed, a command's exit status) and opens, on a pointer press or the new `toggle_agent_tool_details` action, to show its diffs, output, text and locations. Rows start closed.
- **A permission row shows what it asks about.** While a request is pending, the content of the tool call it names is shown with it, above its buttons, so an edit is read before it is allowed. An agent that puts the content only in the permission request (Antigravity does) is covered.
- **Raw input and result are the fallback.** A tool call with no content shows its raw input and result instead, which is all some agents send (Copilot's edit is a patch in its raw input).
- **Each diff and each output has a copy button.**
- **One implementation per ACP version.** The server's ACP code is split into a version-free reducer and one module per ACP version, chosen once per session by the negotiated version through a closed `Dialect` enum that is matched in a single file. This change moves the existing v1 code behind that seam and adds its tool content mapping there; `agents-acp-v2` adds the v2 module beside it. A new architecture rule and a check in `tests/architecture.rs` hold it.
- **Client protocol 4 is amended, not followed by a 5**, since no release carries it yet (decided 2026-11-25 for the whole agent series): two transcript event kinds are added and two forms gain optional fields. No client message is added.

Not in this change: syntax highlighting; word-level or side-by-side diffs; drawing command output through the terminal emulator (colours, cursor movement); opening a location or a diff's file; applying, reverting or editing a diff; images in tool content; text selection and find in a detail; a tool call's v2-only statuses (`paused`, `cancelled`) and subagent session references, which `agents-acp-v2` adds; ACP's `fs` and `terminal` client capabilities, which stay unoffered (running an agent's commands in Brindle's own terminals is the proposal-only change `agents-client-terminals`).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`: new requirements for a tool call's content, diffs, command output, locations, raw input and result, their bounds, and a permission request's tool call.
- `agent-zones`: "Tool call rows" and "Permission rows" change; new requirements for a tool call's detail, diffs, command output, copying and the toggle action.
- `ui-components`: "Tool rows" gains a trigger and a summary; a new diff block.
- `ui-theme-tokens`: new diff colours, with their legibility pairs.
- `command-line`: the dump's lines for a tool call's content.

## Impact

- **`brindle-transcript`** (`transcript/`): `ToolItem`, `FileChange`, `Hunk`, `DiffLine`, `Output`, `Location`; `Entry::Tool`'s optional fields; `Event::ToolItems` and `Event::ToolOutput`; the fold's bounds; new `tests/fixtures/v4/` files (none edited); the fuzz corpus.
- **`brindle-remote-agent`** (`remote-agent/src/acp/`): `wire.rs` and `rpc.rs` split into the version-free envelope and forms, `dialect.rs` and `v1/`; a pure line diff (`diff.rs`) and escape-sequence filter (`plain.rs`); the reducer's tool handling. No new dependency. The checkpoint's format number stays 1, amended (unreleased).
- **`brindle`** (`src/agent/`): the chat model's tool detail, its parts and open state, the dump; the view's drawing and copy; a new action in `src/actions.rs`.
- **`brindle-ui`** (`ui/`): `Tool` gains a trigger and a summary; a new `Diff` component; two tokens. Its dependency list is unchanged.
- **Tests:** fixtures captured from real agents running an allowed edit and command, real-role tests through the fake ACP agent, and new `agent-tool-*` e2e cases. Existing agent e2e expectations change where the dump's tool line gains fields.
- **Docs:** `docs/architecture/ports-and-adapters.md` (a new rule for ACP versions) and `README.md`'s table, `tests/architecture.rs`, `docs/code-map.md`, `README.md`, `CLAUDE.md`, `.claude/rules/remote-agent.md` and `transcript.md`.
- **Other changes:** `agents-acp-v2` builds its v2 module on this change's seam and maps v2's tool content onto these forms; its proposal should say so when it is taken up.
