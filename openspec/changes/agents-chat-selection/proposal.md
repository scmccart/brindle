# Proposal

**Depends on:** `agents-markdown` (the `Response` and `CodeBlock` that would draw selectable text, and the message parts with their source ranges), implemented and archived. Best taken after `agents-markdown-layout-fixes` (one bubble per message, with the same width for every part) and `agents-zone-theme-scope` (the selection's colour from the zone's tokens).
**Next:** nothing depends on it.

**Status: proposal stage.** This change has no design, specs or tasks yet. The user deferred it on 2026-11-25 as too complex for now; the copy buttons stay. It is taken further when the user asks for it. (`.openspec.yaml` sets `skip_specs` only so that the proposal validates on its own. The design stage removes it and writes the specs.)

## Why

In the manual checks of 2026-11-25 the user wanted to select text anywhere in an agent zone's conversation (in chat bubbles, in thoughts, any text) and copy it. Today nothing in a conversation can be selected. A user can copy a whole message (its hover copy button, or `copy_last_agent_message`), a code block (its copy button, or `copy_last_code_block`) and the queued prompt, but not a sentence, a path in a paragraph, or part of a tool's title.

## What Changes

- Text in the conversation can be selected with the pointer and copied, starting with one block of text and, in later steps, across a message's blocks and across messages.
- The copy buttons and the copy actions stay as they are.

## What the investigation found (2026-11-25)

From `~/.cache/claude-tasks/brindle/manual-findings/investigation.md`, question 10, read against gpui 0.2.2's sources:

- **gpui 0.2.2 has no selectable text.** `StyledText` and `InteractiveText` (`gpui-0.2.2/src/elements/text.rs:148-700`) offer `on_click(ranges)`, `on_hover` and `tooltip`, with no drag and no selection.
- **It has the pieces.** `TextLayout::index_for_position(point) -> Result<usize, usize>` (`text.rs:483`) maps a point to a byte index and handles wrapped lines. `position_for_index` (`:517`) and `line_layout_for_index` (`:548`) go the other way, and `bounds`, `line_height` and `text` complete it. `StyledText::with_highlights` can paint a selection's background, `Window::on_mouse_event` and the capture handlers can follow a drag, and `paint_quad` paints rectangles.
- **Zed's selectable Markdown** is one custom element in its `markdown` crate. This was recalled from memory and not checked here, since no Zed source is on this machine. The element keeps its own rendered lines with their source offsets, tracks the selection in its own state across mouse down, move and up, maps points through each line's `TextLayout::index_for_position`, paints the selection's rectangles, and copies the source range. Zed's crates are GPL-3.0: they may be read for the shape of the API only, never copied (`ports R12`).
- **Precedent in this repo.** `brindle-ui`'s `TextArea` and `EditState` (`ui/src/text_area.rs`, `ui/src/text_input.rs`) already do selection, hit testing over `shape_text`'s wrap maps, and copying, pure and tested. But each is an editor entity per instance, with no styled runs.
- **What each step would take:**
  1. **Within one block of text** (a paragraph, a user bubble, a thought's paragraph), about **3–5 days**, with a pure selection model and its tests and a gallery check. This is a `SelectableText` element in `brindle-ui` over `StyledText`: press, drag and release mapped through `index_for_position`, the selection drawn with `with_highlights`, double-click for a word and triple-click for a paragraph, a copy action, and the selection held by the owner (as `Reasoning`'s `open` is) so that the component stays stateless. It must live with `InteractiveText`'s link presses (links act only with ctrl, so a plain drag can select) and with `Response`'s nested blocks.
  2. **Across the blocks of one message**, about **1–1.5 weeks more**. This is one selection model per `Response`, its anchor and head each a block path and a byte offset, with each text leaf told which slice to highlight. Hit testing walks the leaves' recorded bounds. Copying joins the covered blocks' shown text, or better, maps back to the Markdown source range that each top-level `Part { range }` already carries. Code blocks and table cells need leaves of their own.
  3. **Across messages, thoughts and tool rows in the scroller**, about **1.5–2 weeks more**. The selection lives in `AgentView` and `ChatModel` as pairs of entry id, part, leaf and offset: a stable identity, not a list index (`types R3`). `gpui::list` lays out only the items in view, so a drag must keep working while the list scrolls at its edges, and items scrolled out of view still count as selected by identity. Every item draws its slice from the model, and copying builds the text from the entries' source, not from what was painted. Streamed appends and a trimmed log (`trimmed_through`) must keep or clip the selection. Extending it from the keyboard (shift and the arrows) is optional.
  - **In all, selecting any text across the conversation is about 3–5 weeks.** The pragmatic first step is step 1, beside the existing "copy message" and "copy last code block", which covers most needs in under a week.

## Open questions, to answer before a design

1. **How far the first change goes:** step 1 alone, steps 1 and 2, or all three; and whether thoughts, tool titles, permission titles, notices and alerts are selectable or only messages.
2. **What a copy gives:** the text as shown (list markers, table cells joined by tabs, a code block's text), or the Markdown source of the covered range. For step 1 they differ only in inline markup.
3. **Gestures:** press and drag, double-click for a word, triple-click for a paragraph, shift-click to extend; whether a press outside the text clears the selection; what the I-beam and the pointing hand show over a link (ctrl held) and over plain text.
4. **Keys and focus:** focus usually sits in the prompt input, whose ctrl-c copies its own selection. Which key copies a conversation selection (ctrl-c when the input has none, ctrl-shift-c as in terminals, a palette action), whether a selection in the conversation takes focus from the prompt, and how it reaches the `FocusStops`.
5. **The clipboard settings:** whether `copy_on_select` applies to a selection in the conversation, and whether a Linux selection becomes the primary selection (`primary.rs`), as in terminals.
6. **Its look:** the selection's colour (a token, such as `accent` at 40%, from the zone's tokens once `agents-zone-theme-scope` lands), and whether a selection over a code block or a bubble's edge is drawn round the whole block.
7. **Streaming and the log:** what happens to a selection inside an entry that is still streaming, one whose part is re-rendered (`absorb` splices from the first part that differs), and one whose entry is trimmed or replaced by a replay's reset.
8. **Scrolling:** whether a drag past the scroller's edge scrolls it (step 3), and whether a selection keeps `Follow::AtEnd` from following the end.
9. **Tests:** the e2e suite drives a terminal's pointer with `--press`, `--hover` and `--release` over grid cells, and an agent zone has no grid. Step 1 needs a debug hook that places a press in an agent zone (by entry, part and offset, or in logical pixels), and the dump needs a line for the selection and what a copy would give. Pointer drags themselves stay on the manual list (`testing R16`).
10. **Where it lives:** a `SelectableText` element and a pure selection model in `brindle-ui` (`fcis R1`, `R5`; `ports R2`); the conversation-wide model (step 3) in `src/agent/model.rs`; and whether `Response` keeps `StyledText` or draws its own lines.

## What the full changes already say

- `agents-markdown`: a message is one list item per top-level block, each a `Part { range, block }` whose range is its slice of the Markdown source; code blocks have a pointer-only copy button that gives the owner the block's text; `link_modifiers` makes links act only with ctrl held, so a plain press on a link does nothing today.
- `agent-zones`, requirement "Copying from the conversation": copying a whole message, the last code block and the queued prompt. This change would add to it, not replace it.
- `agents-profiles`: `Reasoning`'s `open` is held by the owner, the pattern a selection would follow; `FocusStops` lists the zone's keyboard stops.
- The terminals' clipboard behaviour: `copy_on_select`, the primary selection on Linux, and right-click paste, which question 5 asks about.

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `agent-zones`: "Copying from the conversation", and a new requirement for selecting text in the conversation.
- `ui-components`: a new requirement for selectable text in a response and a message.
- `command-line`: the dump's line for a selection and the debug hook that makes one (question 9).

## Impact

- `brindle-ui` (`ui/`): a `SelectableText` element and its pure selection model; `Response`, `Message` and `Reasoning` drawing through it; the gallery.
- `src/agent/`: the view's copy action and its key, the model's selection for steps 2 and 3, and the dump's line.
- `src/main.rs` and the e2e harness: the debug hook for a press in an agent zone.
