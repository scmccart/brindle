# Proposal

**Depends on:** `agents-profiles` (the agent zone, its GUI-free chat model and its message blocks, and the `Message` component in `brindle-ui`), implemented and archived.
**Next:** `agents-login`, in-zone login through ACP's `authenticate`. The five follow-ups of `agents-profiles` are independent of one another, so that order is a suggestion.

**Status: proposal stage.** This change has no design, specs or tasks yet. It is taken further once `agents-profiles` is archived (and with it `agents-launch-commands` and `agents-sessions`, which that change needs).

## Why

In step 1 an agent zone shows every message as plain text. ACP agents write Markdown: ACP says text content may be plain text or Markdown and that clients SHOULD render it as Markdown (v2 names CommonMark, with GitHub Flavored Markdown extensions such as tables allowed). So a reply shows its code as backtick fences in the proportional font, headings as `#` lines, emphasis as asterisks and tables as rows of pipes. Code, which is most of what a coding agent shows, is the hardest part to read.

## What Changes

- Agent messages in an agent zone are rendered as Markdown: at least paragraphs, emphasis, inline code, fenced code blocks in the terminal's monospace font, lists and headings. The stored text doesn't change: the server keeps plain text and Brindle renders it.
- Fields ACP defines as plain text stay plain (tool call titles, permission titles, notices, error messages); ACP v2 says clients SHOULD NOT render those as Markdown.
- Raw HTML is never rendered (ACP: clients SHOULD disable or sanitize it).

Open questions, to answer before a design:
1. **Where the parser comes from.** `brindle-ui`'s closed dependency list is `gpui` and `log` (`ports R3`), so the choices are a parser dependency added to that list (with `tests/architecture.rs`, `ports R3` and the `Cargo.lock` rule in `CLAUDE.md`: additions only, no regenerated lock or broad update), a parser in the `brindle` crate handing `brindle-ui` plain styled runs so its list stays closed, or a hand-written subset. Zed's `markdown` crate is GPL-3.0 and can't be copied from.
2. **Which subset:** CommonMark only, or GFM's tables, task lists and strikethrough as well; what happens to images (shown as their alt text or link?).
3. **Which entries:** agent messages only, or thoughts, the user's own messages and plan entries too.
4. **Streaming.** How an unfinished construct (an unclosed fence) looks while it streams, and how the message blocks split. Step 1 splits a message at blank lines and past 4 KiB so a streaming message re-lays out only its last block, which would cut a fenced code block or a list in two.
5. **Links:** shown only, or clickable through Brindle's existing link handling (`links`), with its refusals and opener.
6. **Copying:** whether the copy button and Copy Last Agent Message copy the Markdown source or the rendered text; whether a code block gets its own copy button.
7. **The dump:** how `--dump-screen-after`'s `--- entry` lines describe a rendered message, so e2e cases can check it.
8. **Tokens:** whether code blocks and inline code need tokens the theme set doesn't have yet.

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
- `agent-zones` (added by `agents-profiles`): "The conversation" renders agent messages as Markdown; possibly "Copying from the conversation".
- `ui-components`: "Messages" gains rendered Markdown content, or a new Markdown component beside it.
- `links`: possibly, if links in messages become clickable.

## Impact

- `ui/` (`brindle-ui`): the message component, and possibly a new dependency on its closed list, with `docs/architecture/ports-and-adapters.md` (`ports R3`) and `tests/architecture.rs` changed in the same change.
- `src/agent/`: the chat model's blocks and how a message splits.
- `Cargo.lock`, if a parser is added.
- The `--dump-screen-after` output for an agent zone, and e2e cases against the fake ACP agent.
- No change to the terminal server, the transcript or the client protocol.
