# Proposal

**Depends on:** these changes, all implemented and archived:
- `agents-sessions` and `agents-profiles`: the reducer, the transcript, client protocol 4, and the agent zone with its tool and permission rows.
- `agents-markdown`: the renderer and code block that draw a tool call's text.
- `agents-session-recovery`, `agents-login` and `agents-session-title`: the v1 requests and decoders that move behind the `Dialect` seam.
- `agents-markdown-layout-fixes`, `agents-zone-theme-scope` and `agents-zone-attention`: the bubble layout, the zone token sets, the dangerous permission row and the working row.
- `agents-log-paging`: the page, the fetch, the window and checkpoint format 2.

`agents-log-paging` is archived on `feat/agents-log-paging` (PR #144, not yet merged), and this change is branched from it. `remote-login-environment` is archived too, and doesn't touch what this change does.

**Next:** `agents-commands`, `agents-plans` and `agents-prompt-context`. Each is designed and builds on this change's `Dialect` seam, `acp/forms.rs` and `[agent]` table. `agents-acp-v2` and `agents-client-terminals` are proposal-only.

**Status:** designed, and refreshed on 2026-11-26 against the built agent stack (`feat/agents-log-paging` at 20e9858). The refresh did these things:
- It rewrote design.md's Context for the code as built.
- It found an existing capture that settles design decision 6 for one agent. Codex sends a command's output to a client that offered no terminal capability, through `_meta.terminal_output_delta`.
- It moved the checkpoint onto paging's format 2.
- It made a page reach the tool call that a waiting permission names.
- It recorded its choices as flags (design decision 17).

No implementation has started.

## Why

In step 1 a tool call is one line (an icon, a title, a status) and a permission request is a title with buttons. The agent's edit, the command it wants to run and the output it got never reach the user: the terminal server drops a tool call's content before it enters the conversation log. So the user allows or rejects `Edit main.rs` without seeing the edit, which undercuts the rule that nothing runs unseen, and afterwards can't see what a tool did without asking the agent.

Brindle will speak ACP v1 and v2 side by side for good, so that no agent is left out (decided 2026-11-25). The two versions describe a tool call's content differently: v1 sends a diff as a path with the old and new text and replaces the content list whole; v2 sends file changes with an optional Git patch, appends content items, and streams an agent-owned terminal's output. This change is the first to add a feature both versions carry, so it also fixes how a version difference is written: once, in that version's own implementation, never as version checks spread through the reducer.

## What Changes

- **Tool calls keep their content.** The terminal server keeps, for each tool call, its text, its file changes as diff hunks, its command output, the file locations it names and its raw input and result, as Brindle's own transcript forms. They are in the log, the checkpoint and every replay, so they survive a detach, a quit, a core restart and an upgrade.
- **Everything kept is bounded** (`railway R5`): 64 KiB for each content item, 32 items and 256 KiB for a tool call, 16 KiB for each of the raw input and result, with what was left out counted and shown. Command output keeps its end, the rest keeps its start. Escape sequences are removed from command output, which is shown as plain text.
- **Command output from a v1 agent comes through the `_meta` terminal convention**. Codex sends `terminal_info`, `terminal_output_delta` and `terminal_exit` beside a `terminal` content item, even to a client that offered no terminal capability (`codex-acp-2.12.0-reject.log`). The v1 module reads those three keys and nothing else outside the ACP spec.
- **A page reaches what a waiting permission asks about.** The attach page already reaches the oldest waiting permission (`agents-log-paging`). It now also reaches the tool call that permission names, so the edit shows with the permission row however long the turn was.
- **Diffs are computed on the server.** For ACP v1 the server compares the old and new text and keeps only the changed lines with their context, so a one-line edit of a large file costs a few lines of log, and Brindle never sees which ACP version the agent speaks.
- **A tool row opens.** A tool call with content shows a summary on its line (lines added and removed, a command's exit status) and opens to show its diffs, output, text and locations. A row is open while its tool call runs and closes when the call finishes; a pointer press or the new `toggle_agent_tool_details` action opens or closes it by hand, and a row set by hand stays that way.
- **Both states are configurable.** A new `[agent]` config table has `tool_details` (`"open"` or `"closed"`, for a running call) and `tool_details_when_done` (`"closed"` or `"open"`, for a finished one), applied on reload.
- **A permission row shows what it asks about.** While a request is pending, the content of the tool call it names is shown with it, above its buttons, whatever the settings, so an edit is read before it is allowed. An agent that puts the content only in the permission request (Antigravity does) is covered.
- **Raw input and result are the fallback.** A tool call with no content shows its raw input and result instead, which is all some agents send (Copilot's edit is a patch in its raw input).
- **Each diff and each output has a copy button.**
- **One implementation per ACP version.** The server's ACP code is split into a version-free reducer and one module per ACP version, chosen once per session by the negotiated version through a closed `Dialect` enum that is matched in a single file. This change moves the existing v1 code behind that seam and adds its tool content mapping there; `agents-acp-v2` adds the v2 module beside it. A new architecture rule and a check in `tests/architecture.rs` hold it.
- **Client protocol 4 and checkpoint format 2 are amended, not succeeded**, since no release carries either (decided 2026-11-25 for the whole agent series; v0.3.0, the latest release, speaks client protocols 1–3). Two transcript event kinds are added (`tool_items`, `tool_output`) and two forms gain optional fields. No client message and no holder message is added, so the numbers `agents-log-paging` fixed (`AGENT_FETCH` 23, `AGENT_PAGE` 43, `CHECKPOINT_APPEND` H1 16) and the next free ones (`agents-commands`' `AGENT_SET_OPTION` 24) are untouched.

Not in this change: syntax highlighting; word-level or side-by-side diffs; drawing command output through the terminal emulator (colours, cursor movement); opening a location or a diff's file; applying, reverting or editing a diff; images in tool content; text selection and find in a detail; a tool call's v2-only statuses (`paused`, `cancelled`) and subagent session references, which `agents-acp-v2` adds; ACP's `fs` and `terminal` client capabilities, which stay unoffered (running an agent's commands in Brindle's own terminals is the proposal-only change `agents-client-terminals`).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`: new requirements for a tool call's content, diffs, command output, locations, raw input and result, their bounds, and a permission request's tool call; "Attach replays the transcript" (the page reaches a waiting permission's tool call), "Entries a client hasn't fetched stay current" (tool content outside the page) and "Transcript event forms are fixed for each protocol version" (a tool call without content encodes as before) change.
- `agent-zones`: "Tool call rows", "Permission rows" and "Agent commands in the palette" change; new requirements for a tool call's detail, its open state, diffs, command output, copying and the toggle action.
- `ui-components`: "Tool rows" gains a trigger and a summary; a new diff block.
- `ui-theme-tokens`: new diff colours in the window and zone token sets, with their legibility.
- `command-line`: the dump's lines for a tool call's content.
- `configuration`: a new `[agent]` table with two tool detail settings.

## Impact

- **`brindle-transcript`** (`transcript/`): `ToolItem`, `FileChange`, `FileOp`, `Hunk`, `DiffLine`, `Output`, `Exit`, `Location`; `Entry::Tool`'s optional fields; `Event::ToolItems` and `Event::ToolOutput`, which a paged copy ignores below its window as it does `Appended`; the fold's bounds; `Transcript::page` reaching a waiting permission's tool entry; new `tests/fixtures/v4/` files (none edited); the fuzz corpus and the paging laws over tool content.
- **`brindle-remote-agent`** (`remote-agent/src/acp/`): `wire.rs` and `rpc.rs` split into the version-free `envelope.rs` and `forms.rs`, `dialect.rs` and `v1/`; a line diff (`diff.rs`, over the `similar` crate, which joins the crate's closed dependency list and adds one package to `Cargo.lock`) and a pure escape-sequence filter (`plain.rs`); the reducer's tool handling and its open outputs; the checkpoint, format 2 amended (unreleased). The static helper copied to remote hosts grows by that crate.
- **`brindle`** (`src/agent/`): the chat model's tool detail, its parts and open state, the permission row's detail, the dump; the view's drawing and copy; a new action in `src/actions.rs`; the `[agent]` table in `src/config.rs` and `assets/default-config.toml`.
- **`brindle-ui`** (`ui/`): `Tool` gains a trigger and a summary; a new `Diff` component; two tokens in `Tokens` (window and zone sets). Its dependency list is unchanged.
- **Tests:** fixtures captured from real agents running an allowed edit and command (Codex's command output is already captured), real-role tests through the fake ACP agent, new `agent-tool-*` e2e cases and a `windows-agent-tool-diff` case, and a perf case `agent-tool-output`. The dump's tool line gains fields; no existing e2e expectation quotes a tool line, and a permission line changes only when it shows a detail.
- **Docs:** `docs/architecture/ports-and-adapters.md` (a new rule, `ports R14`, for ACP versions) and `README.md`'s table, `tests/architecture.rs`, `docs/code-map.md`, `README.md`, `CLAUDE.md`, `.claude/rules/remote-agent.md` and `transcript.md`.
- **Other changes:** `agents-acp-v2` builds its v2 module on this change's seam and maps v2's tool content onto these forms; its proposal should say so when it is taken up. `agents-commands`, `agents-plans` and `agents-prompt-context` already name `acp/forms.rs`, `Dialect` and the `[agent]` table; `agents-client-terminals` reads task 1.1's record of which agents call `terminal/create`.
