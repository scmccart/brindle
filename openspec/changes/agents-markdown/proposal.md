# Proposal

**Depends on:** `agents-profiles` (the agent zone, its GUI-free chat model and its message blocks, and the `Message` component in `brindle-ui`), implemented and archived.
**Next:** `agents-login`, in-zone login through ACP's `authenticate`. The five follow-ups of `agents-profiles` are independent of one another, so that order is a suggestion.

**Status: designed.** `agents-profiles` is archived; the open questions below were answered by the user on 2026-11-25, and design.md, the delta specs and tasks.md follow from those answers.

## Why

In step 1 an agent zone shows every message as plain text. ACP agents write Markdown: ACP says text content may be plain text or Markdown and that clients SHOULD render it as Markdown (v2 names CommonMark, with GitHub Flavored Markdown extensions such as tables allowed). So a reply shows its code as backtick fences in the proportional font, headings as `#` lines, emphasis as asterisks and tables as rows of pipes. Code, which is most of what a coding agent shows, is the hardest part to read.

## What Changes

- Agent messages and thoughts in an agent zone are rendered as Markdown: paragraphs, emphasis, inline code, code blocks in the terminal's monospace font with a copy button, lists, headings, quotes, tables and links. The stored text doesn't change: the server keeps plain text and Brindle renders it.
- Fields ACP defines as plain text stay plain (tool call titles, permission titles, notices, error messages); ACP v2 says clients SHOULD NOT render those as Markdown.
- Raw HTML is never rendered (ACP: clients SHOULD disable or sanitize it).

Open questions, answered by the user on 2026-11-25 (design.md decision 1 says where each is worked out):
1. **Where the parser comes from:** `pulldown-cmark` (`default-features = false`) in the app crate, behind a pure function in `src/agent/` that hands `brindle-ui` plain blocks of styled runs; `brindle-ui`'s list stays `gpui`, `log`; `Cargo.lock` gains one package.
2. **Which subset:** CommonMark plus GFM strikethrough, task lists, tables (a grid) and autolinks; no footnotes, math or smart punctuation; raw HTML as literal text; images never fetched, shown as a muted `[image: alt]` with the target, clickable as a link.
3. **Which entries:** agent messages and thoughts. User messages and tool, permission and notice titles stay plain; there are no plan entries.
4. **Streaming:** the whole message is parsed again as it grows; its parts follow the parser's top-level blocks; an unclosed fence is code to the end. A perf guard pins the cost.
5. **Links:** clickable through `terminal/links.rs` with the terminal's gesture, the target shown on hover when it differs, schemes other than http and https confirmed first; no focus stops.
6. **Copying:** the copy button and Copy Last Agent Message copy the source; each code block has a pointer-only copy button; a palette action Copy Last Code Block.
7. **The dump:** `--- entry` keeps the source; `--- block` lines describe each rendered part.
8. **Tokens:** reuse `muted`, `border`, `muted_foreground` and the accent, adding a token only if the legibility test fails on a built-in. It does for links on `brindle-light`, so a `link` token is added.

## What the full changes already say

- `agents-profiles`, design decision 1 ("Plain text for messages"): `ui/`'s closed list (`gpui`, `log`) can't take a Markdown parser without this change's decision. Its proposal and Non-Goals defer Markdown here.
- `agents-profiles`, `agent-zones` requirement "The conversation": user messages, agent messages and notices are shown as plain text, and control characters are never interpreted. This change modifies that requirement; the second half stays.
- `agents-profiles`, design decision 8: a message is split into blocks at blank lines and past 4 KiB (`blocks_of`), each block holding plain `SharedString` data, so a long streaming message re-lays out only its last block (question 4).
- `agents-profiles`, design decision 15: the dump's entry lines print each entry's text escaped onto one line (question 7).
- `agents-profiles`, `agent-zones` requirement "Copying from the conversation": the copy button and `copy_last_agent_message` copy a message's text (question 6).
- `agents-sessions`, Non-Goals: Markdown is a client matter; the server stores plain text. Its reducer replaces control characters other than `\n` and `\t` before text enters the log (design decision 8), so this change needs nothing from the terminal server or the client protocol.

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `agent-zones`: "The conversation", "Copying from the conversation", "Agent zone appearance" and "Agent commands in the palette" change; Markdown, code blocks, Copy Last Code Block, streaming, tables, images and HTML are added.
- `ui-components`: responses, their code blocks, links and tables are added beside "Messages".
- `links`: links in agent messages, their target on hover, and confirming schemes other than http and https.
- `command-line`: the dump's `--- block`, `--- link` and `--- confirm link` lines.
- `ui-theme-tokens`: the `link` token and more legible pairs.

## Impact

- `ui/` (`brindle-ui`): `Response` and `CodeBlock` components, the rich-text data types and the `link` token; its closed list is unchanged.
- `src/agent/`: a pure Markdown renderer, the chat model's parts for agent messages and thoughts, link handling, Copy Last Code Block; `src/link_confirm.rs` and the workspace overlay; `terminal/links.rs` gains `plan_here` and the shared gesture.
- `Cargo.lock`: one added package, `pulldown-cmark`.
- The `--dump-screen-after` output for an agent zone, e2e cases against the fake ACP agent, and a perf case.
- No change to the terminal server, the transcript, the client protocol, config keys or state files.
