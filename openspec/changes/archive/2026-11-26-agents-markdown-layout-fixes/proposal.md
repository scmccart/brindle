# Proposal

**Depends on:** `agents-markdown` (the `Response`, `CodeBlock` and message parts it draws), `agents-profiles` (the agent zone, `Message`, `PromptInput`) and `agents-session-recovery` (the reason line's action), implemented and archived.
**Next:** `agents-zone-theme-scope`. The two are independent; this one is first because it is a bug fix and both touch `ui/src/components/message.rs`.

## Why

The user's manual checks on Windows (2026-11-25) found three layout faults in agent zones. A code block's language label wraps one letter per line ("e" above "x"). An agent message's Markdown parts are each as wide as their own longest line, so a message's bubble is jagged. And after a zone split, the "connecting to the terminal server" line under the prompt wraps in a very thin column. An investigation reproduced the first two in gpui's `TestAppContext`. The label and the reason line share one cause: a wrapping text item in a flex row whose width comes from its own measure. gpui 0.2.2's cached text measure can make that measure collapse.

## What Changes

- **A code block's label is one line,** cut with an ellipsis when it doesn't fit beside the copy button, and never wrapped.
- **An agent message is one bubble.** Every part of an agent message fills the conversation's width, so the parts line up into one rectangle. A user message stays as wide as its text, up to four fifths of the row.
- **The prompt input's reason line wraps at the input's width,** in a zone of any width and in the first frame after a split.
- **One rule for text in flex rows across `brindle-ui` and the app:** a wrapping text item takes its width from its row (grow from a zero basis) or has a definite width, and a one-line label uses the existing one-line label helper or doesn't shrink. Every site that breaks the rule is fixed, and the rule is written down beside the existing ellipsis pitfall.
- **Tests that see each fault:** layout tests in `TestAppContext` for each component, gallery checks that measure with real fonts, a view test of a connecting zone at a split's width, and an e2e case that checks a message's bubble edges and a code block's header in a window capture.

Not in this change: the zone's colours (`agents-zone-theme-scope`); text selection (`agents-chat-selection`); merging a message's parts into one list item.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `ui-components`: "Messages" (the width rule for each side and parts drawn as one bubble), "Response code blocks" (the label on one line), and a new requirement that text in a component wraps only at the width its container gives it.
- `agent-zones`: a new requirement that an agent message is drawn as one bubble across the conversation's width, and one for the prompt's reason line in a narrow or newly split zone.

## Impact

- **`brindle-ui`** (`ui/`): `components/code_block.rs` (the label), `components/message.rs` (`Message`'s column for each side), `components/prompt_input.rs` (the reason line), any other site the audit in design.md lists, their `TestAppContext` tests, the gallery's checks (`ui/examples/gallery.rs`) and `ui/README.md`'s list of differences.
- **The app:** `src/agent/view/tests.rs` (a connecting zone at a split's width). `src/agent/view.rs` only if the audit finds a site there.
- **e2e:** a new case, `agent-markdown-layout`, in `tests/e2e/cases/agent.rs`.
- **Docs:** `docs/code-map.md`'s `brindle-ui` entry gains the wrapping-text pitfall beside the ellipsis pitfall.
- No wire, config, state-file or dependency change.
