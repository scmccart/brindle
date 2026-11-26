# Design

## Context

See proposal.md for why. What this design rests on, from `main` at 725ed28 and the investigation of 2026-11-25 (`~/.cache/claude-tasks/brindle/manual-findings/investigation.md`, question 9, with a `TestAppContext` probe of the real components; gpui's test text system gives each character 0.6 em):

- **The code block's label** (`ui/src/components/code_block.rs:116-126`) is `div().min_w_0().text_size(rems(0.75))…children(label)` in a `flex items_center justify_between gap_2` row beside the copy button. Inside an assistant `Message` (a `max_w(80%) min_w_0` column) in a 600 px host, the probe measured it **0 px wide and 77.5 px tall**: four lines of one letter each. Without `min_w_0`, with `min_w_0` plus `flex_shrink_0`, or with `flex_none`, it measured 39 px, one line.
- **The cause.** A text flex item with `min_w_0` (minimum width 0) and an automatic basis takes its basis from its own measure. gpui 0.2.2's text measure (`gpui-0.2.2/src/elements/text.rs:346-378`) treats `MinContent` and `MaxContent` as "no wrap width" and returns the size it **cached** on any later unconstrained measure in the same layout. So a narrow measure that taffy makes first, while it sizes the nested percentage-width columns, sticks: the basis collapses, and the text is then wrapped at that width. It is the same quirk as the code map's ellipsis pitfall ("the text keeps its first, natural-width measure"). The exact taffy call wasn't traced, but the probe reproduces it every time.
- **The jagged bubble.** `Message` (`ui/src/components/message.rs:205-222`) is a `flex w_full` row holding a `flex flex_col max_w(relative(0.8)) min_w_0` column with no width, so the column is as wide as its content's longest line, capped at 80%. The app gives each top-level Markdown block of an agent message its own list item and its own `Message` (`BubblePart::{First, Middle, Last}`, `src/agent/view.rs:711-716`). So each part's bubble is as wide as its own text: the probe measured 42 px for `short` and 427 px for a long paragraph, drawn edge to edge.
- **The "connecting" line** is the prompt input's reason (`DisabledWhy::Connecting`, `src/agent/model.rs:504, 1551`), drawn by `PromptInput`'s footer (`ui/src/components/prompt_input.rs:515-535`): [`flex_1 min_w_0` row of [reason `div().min_w_0()…`, action], submit button]. It is the label's pattern, a `min_w_0` text item with an automatic basis in nested flex rows. In isolation (a plain column 160–600 px wide) the probe did **not** reproduce it. The real tree adds `RemScope`, `FocusScope`, the prompt's `flex_none flex_col px_3` container, the card's `relative flex_1 w_full` content and the split's `flex_1 size_full` children (`src/zone_view.rs:1547-1556`), and a split changes the zone's width between two frames. The cause is inferred from (a), not observed (`testing R11`).
- **What tests can see.** The dump prints no element bounds. `TestAppContext` tests read `debug_bounds` of a `debug_selector` (`message.rs`'s tests already do), the gallery's `-- --check` records painted bounds with real fonts (`ui/examples/gallery.rs:309`), and e2e cases sample window captures against the `e2e` theme's colours (`tests/e2e/capture.rs`, `cases/cards.rs`).

## Goals / Non-Goals

**Goals:**
- A width rule for messages that makes a message one bubble, stated in the spec.
- One rule for text in flex rows, applied at every site that breaks it and written down.
- A test for each fault in the layer that can see it.

**Non-Goals:** changing how messages are split into list items, or the `MessageScroller`; colours (`agents-zone-theme-scope`); patching gpui (its cached measure is worked around, not changed); a lint for the rule (a text search can't tell a text leaf from a container reliably).

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Message`'s column per side | component | `brindle-ui`, `ui/src/components/message.rs` |
| The code block's label | component | `brindle-ui`, `ui/src/components/code_block.rs` |
| The reason line | component (entity) | `brindle-ui`, `ui/src/components/prompt_input.rs` |
| A confirmation's stacked actions (audit) | component | `brindle-ui`, `ui/src/components/confirmation.rs` |
| Layout tests | `TestAppContext` tests | each component's file; `src/agent/view/tests.rs`; `src/zone_view.rs`'s tests |
| Gallery checks | gallery | `ui/examples/gallery.rs` |
| `agent-markdown-layout` | e2e case | `tests/e2e/cases/agent.rs` |

Nothing here decides anything from state: the changes are styles on existing elements, so there is no pure code to add (`fcis R1` is untouched). `brindle-ui` keeps knowing nothing of agents: `Message` takes its side, as today, and the width rule follows from the side (`ports R2`). No crate reference changes (`ports R1`, `R3`).

## Decisions

### 1. The width rule (`ui-components` "Messages")

- **The other side's message fills its row; the person's fits its text, up to four fifths.** The assistant column becomes `w_full` (a definite width) with no cap; the user column keeps `max_w(relative(0.8))` and shrinks to its text. The bubble inside is a `flex_col` child and stretches to the column.
- Every part of an agent message then has the same definite width, so the parts' edges line up and the `BubblePart` edges already drawn make one rectangle. The definite width also removes the intrinsic-sizing context fault (a) sat in.
- A user message is one part (`UserText`), so shrink-to-fit can't be jagged there.
- **Why this over the alternatives.** A fixed 80% for both sides would line the parts up too, but a short agent reply would sit in a wide empty box, and code blocks and tables would lose a fifth of the zone. Shrinking each part to the widest part of the message needs the widest measure across list items, which `gpui::list` lays out one at a time. AI Elements gives the assistant's content the full width and caps only the user's bubble, so this keeps close to it; the remaining difference (Brindle keeps a bubble on the card behind the assistant's content, where AI Elements draws none) is listed in `ui/README.md`.
- **FLAG F1 (the user didn't choose this).** Recommendation: full width for agent messages, shrink-to-fit up to 80% for user messages. The alternative is a definite 80% for agent messages.

### 2. Parts stay list items

A message stays one list item per top-level Markdown block, as `agents-markdown` decided (design decisions 5 and 6 there): `absorb` splices from the first part that differs, so a streamed reply re-measures one part, not the whole message. "One bubble" is the drawing: equal widths plus the parts' edges (decision 1). Merging parts into one item would re-lay out the whole message on every chunk.

### 3. The code block's label: the one-line label helper

- The label becomes `trigger_label(label)` (`ui/src/components/tabs.rs:767-777`), in the label's size and muted colour: a line clamp of 1 laid over an invisible unwrapped copy, as tab triggers, zone header titles, tool rows and the queue row already do. Its width comes from the row (`flex_1`, basis 0), never from its own measure, so the cached measure can't collapse it, and a label longer than the room left is cut with an ellipsis while the copy button keeps its size (`flex_none` on the copy button's wrapper).
- **Why not `flex_shrink_0`**, the investigation's first suggestion. It fixes the collapse, but a label is the first word of a fence's info string with no bound on its length (`src/agent/markdown.rs`, "A fence's info string as written gives the label's first word"), and a long one would then push the copy button out of the block.
- **FLAG F2.** Recommendation: `trigger_label`. The alternative, `flex_shrink_0`, is one line shorter and overflows on long labels.

### 4. The reason line takes its width from the footer

- The reason becomes `div().flex_1().min_w_0()…`: basis 0, growing into the room the footer gives it, so its wrap width is the row's, never its own measure's. It wraps over as many lines as it needs.
- The action button (`set_action`, Continue conversation) stays after it with `flex_none`, so it now sits at the end of the reason's row, just before the submit button, rather than right after the reason's last word. The spec's "after its reason line's text" still holds.
- **FLAG F3.** Recommendation: as above. The alternative keeps the button right after the text by giving the reason `flex_initial` (an automatic basis), which is the pattern that fails.

### 5. The rule for text in flex rows, and the audit

**Rule.** In a flex row, a text item that may wrap MUST take its width from the row (`flex_1().min_w_0()`: grow from a zero basis) or have a definite width (`w_full`, `w(…)`). A one-line label MUST use `trigger_label`, or `flex_none` when it must never shrink. A bare `min_w_0()` on a text item with an automatic basis is the defect. The rule goes into `docs/code-map.md`'s `brindle-ui` entry as the "Wrapping text pitfall", next to the ellipsis pitfall, and into `ui/README.md`'s notes for component authors.

**Audit** of every `min_w_0` in `ui/src` and `src` (`grep -rn min_w_0`, 2026-11-25), and of the text leaves in flex rows near them:

| Site | What it is | Verdict |
| --- | --- | --- |
| `ui/src/components/code_block.rs:121` | the label, a text leaf in a row | **defect**: decision 3 |
| `ui/src/components/prompt_input.rs:530` | the reason, a text leaf in a row | **defect**: decision 4 |
| `ui/src/components/message.rs:217` | the message's column, no width | **defect** (shrink-to-fit per part): decision 1 |
| `ui/src/components/confirmation.rs:186` | a stacked action's column (button and wrapping description) in a `flex_col items_start` column, so sized to fit its content | **suspect**: the same automatic sizing of a wrapping text. It becomes `w_full` with `items_start`, so the description wraps at the confirmation's width and the button keeps its own. The login row's layout test (task 2.4) shows whether it failed before |
| `code_block.rs:105`, `message.rs:122`, `prompt_input.rs:466, 503, 520, 524`, `tool.rs:72`, `alert.rs:107, 123`, `confirmation.rs:218`, `reasoning.rs:92`, `response.rs:579, 620, 631, 645, 723`, `card.rs:246`, `input.rs:63`, `textarea.rs:67`, `text_input.rs:872`, `text_area.rs:641` | containers with `w_full` or `flex_1`, grid cells (equal tracks), or editors that measure themselves | fine |
| `badge.rs:96`, `tabs.rs:771`, `menu.rs:606`, `command.rs:645, 732`, `combobox.rs:442`, `select.rs:634, 868` | one-line labels through `trigger_label` or `flex_1` with `truncate` | fine |
| `src/zone_view.rs:1553-1594`, `src/workspace.rs:2854, 2986, 3566`, `src/find_bar.rs:435`, `src/settings_dialog.rs` and `settings_dialog/{clipboard,fonts,profiles}.rs` | `flex_1` columns and fields, the strip's shrinking list, the config line | fine |
| `src/agent/view.rs` (banner, note, notices, login note, alert detail) | text in `flex_col` columns, stretched to the column's width | fine |

### 6. Tests (`testing R1`, `R6`, `R11`, `R13`, `R15`)

Each fault gets a test in the lowest layer that sees it, written first against today's code so it fails, then passing after the fix.

- **`TestAppContext` layout tests in `brindle-ui`** (as `message.rs`'s tests and the probe do; each component file's own `mod tests`):
  - `code_block.rs`: inside an assistant `Message` inside a 600 px column, a `rust` label is one line high and as wide as its text; a label wider than the row is one line high and the copy button keeps its width.
  - `message.rs`: an assistant message in two parts (`short`, then a long paragraph) gives both bubbles the row's width and the same x; an assistant `ok` fills the row; a user `ok` is narrower than half the row and ends at its end.
  - `prompt_input.rs`: the reason at 240 px is at least as wide as `connecting` and at most two lines high; drawn at 600 px and then, after the host's width changes and a notify, at 240 px, its bounds equal a fresh draw at 240 px.
  - `confirmation.rs`: stacked actions with a long description at 240 px wrap the description at the confirmation's width.
- **App view tests.** `src/agent/view/tests.rs`: an `AgentView` connecting (the `Zone` fixture), drawn at a full zone's width and then at a split's half, with the reason's bounds checked as above (a `debug_selector` on the reason, `prompt-reason`). If that passes on today's code, so doesn't reproduce the user's fault, the same check moves up to `src/zone_view.rs`'s `TestAppContext` fixture, splitting an agent zone (`split_zone`) and reading the new zone's reason. The task records which one reproduced it; if neither does, it says so, and the fix ships on the inferred cause with the e2e case below as the check that the zone reads right (`testing R11`).
- **Gallery checks** (`-- --check`, real fonts, painted bounds): `message-parts-line-up`, `code-block-label-one-line` and `prompt-reason-wraps`, driving the scenarios of `ui-components` this change modifies.
- **e2e case `agent-markdown-layout`** (`needs_fake_agent`, a window capture): the fake agent replies `Hi.`, a blank line, a paragraph longer than the zone, a blank line and a fenced `rust` block. In the capture, the bubble's right border (the `border` token, `CHROME_BORDER` in the `e2e` theme) is at the same x on every row from the bubble's top to its bottom, one `px_3` inside the card's content edge; the code block's header (from the block's top border to the start of its `muted` surface) is at most twice the label's line height. The `muted` token's colour joins `tests/e2e/brindle.rs`'s `theme` constants with its derivation, as `CHROME_BORDER` is. `agent-split` gains nothing: its "connecting" lasts too short a time to capture reliably, and a fault switch to hold it would test the switch more than the layout.
- **Manual check.** The user's own: split an agent zone and read the connecting line; a Markdown reply with a code block on Windows and Linux. Added to `CLAUDE.md`'s manual list beside the existing Markdown checks.

## Risks / Trade-offs

- [The reason fault doesn't reproduce in any test] → The fix follows the rule that fixed (a) in the probe; the task records which test, if any, reproduced it, and the manual check stays until one does (`testing R11`).
- [Full-width agent bubbles change every reply's look] → A short reply sits in a wide bubble. Flag F1 lets the user pick the 80% variant before implementation.
- [Pixel checks depend on token colours that `agents-zone-theme-scope` changes] → That change's tasks update the case's expected colours; the checks read the border's position, not a shade.
- [A future text item breaks the rule again] → The pitfall is documented where the ellipsis one is; the gallery checks and layout tests cover the components agents use.

## Migration Plan

None: a layout fix with no state, config or wire change. A rollback is a revert.

## As built

**Groups 1–4 (2026-11-25).** Flags F1–F3 were taken as recommended.
- **The reason fault (c) reproduced in the app's view test, not in the component.** `prompt_input.rs`'s own tests (240 px, and 600 px then 240 px) passed on `main`'s code, as the investigation found. `agent::view`'s `the_connecting_line_reads_across_a_split_zone` (task 2.1), run on `main`'s components with only the `prompt-reason` selector added, failed: drawn alone in a zone card 800 px wide the reason was one line, and in the next frame, as the first of a split's two zones, it measured **11 × 560.5 px**, a letter a line. With decision 4 it passes. So the `zone_view.rs` fallback test (task 2.2) wasn't added. The cause is the one decision 4 inferred: a text item with a bare `min_w_0` keeping a narrow measure.
- **The label's collapse doesn't depend on the bubble's width.** With decision 1's definite width alone the label still measured 0 × 77.5 px in `TestAppContext`, so the investigation's hope that the definite width removes (a)'s context didn't hold; decision 3 is what fixes it. With real fonts (the gallery) the short `rust` label didn't collapse on `main`, but a label too long for its column wrapped (a block 87 px tall against 73 for the same block unlabelled).
- **The code block's label fills the room beside the copy button** (`trigger_label` is `flex_1`), so the tests check it is one line and at least as wide as its text, not exactly as wide.
- **The confirmation's suspect site didn't wrap badly; its button stretched.** On `main` a stacked description already wrapped at the confirmation's width, but the described action's button was stretched to it (206 px, against 178 for the undescribed one beside it). `w_full` and `items_start` go on the action's column only when the actions stack, since in the unstacked row a full-width column would put each action on its own line. The description gets `w_full` too, per the rule's column case.
- **`ui/README.md` had no notes for component authors.** It gains a "Notes for component authors" section holding both pitfalls, and the bubble's difference from AI Elements joins its AI elements list.
- **The gallery checks compare heights**, since the gallery can't read an element inside a component: a labelled block against an unlabelled one of the same width beside it (within a pixel, which two boxes at different places can differ by once snapped), and a 240 px prompt input's reason against a one-letter reason's input plus one line. They add a "Width rules" section to the gallery and two disabled prompt inputs.
- **`agent-markdown-layout` measures in the zone's rems.** An agent zone draws its components in a rem of its font size (14 px by default), so `px_3` is 10.5 px there, not 12. The case's measure (`measure_layout`, with a `--self-test` on painted captures through a new `Image::painted`) finds the card's borders on the middle row, the bubble's left border down the column `px_3` inside the card, and requires its right border to be one run over the same rows `px_3` inside the card's right edge (give or take a pixel) with nothing of the border's colour right of it; the header is measured down a column 6 px inside the block's left border. On `main`'s components it fails at the right border; with the fixes it passes. The header bound is two lines of the label at gpui's default line height (0.75 × 14 × 1.618 px each).
