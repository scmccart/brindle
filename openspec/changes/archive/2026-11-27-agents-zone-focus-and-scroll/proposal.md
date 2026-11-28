# Proposal

## Why

The 2026-11-26 agent code review (`docs/architecture/audits/2026-11-26-agents.md`; batch report `docs/architecture/audits/2026-11-26-agents/e.md`; change 2 of `2026-11-26-agents-remediation.md`) found that an agent zone loses keyboard focus and its reading position in ordinary use. Both faults come from GPUI defaults that nothing overrides:
- **E-1 (high):** a pointer press on anything in an agent zone that takes no focus itself (message text, a thought's "Thought" trigger, a tool row's line, a message's copy button, the Earlier row, the scroll-to-end button) moves keyboard focus to the zone tree's own handle, off the prompt input. GPUI focuses the nearest tracked ancestor on any press nobody prevented, and the only one above the conversation is `ZoneTreeView`'s root. Typing, escape, tab and the page keys then do nothing until the user clicks the prompt. This was found by reading; the change's first test shows it.
- **E-2 (high):** while the user reads back, the view jumps to the start of the item at its top whenever that item is spliced. GPUI's `ListState::splice` zeroes the offset of a top item inside the spliced range. A streaming code fence, a streaming thought and a running command's open output all do it, and every config reload and settings preview does it to the whole conversation, because `AgentView::set_profile` splices every item.
- **E-3:** Tab onto a pending permission's button that is out of view leaves it undrawn, so the next keys dispatch from the window's root and do nothing.
- **E-4:** a zone whose Earlier row was in view when its tab was hidden keeps fetching pages until it holds the whole log, because only a drawn frame clears the scroller's `top_in_view`.
- **E-6:** the working row's splice is computed from the model's block count when the view handles the event, not when the model emitted it, so two absorbs in one paced chunk can leave empty items at the list's end.
- **E-7:** every modifier change re-renders the whole window while an agent zone has focus.
- **E-8:** a zoom remeasures no off-screen item, so heights go stale, while every reload remeasures them all for nothing.
- **E-9:** an agent's notification shares its tab's OSC bucket, so a terminal in the same tab that sent three notifications in two seconds silently drops the agent's "needs permission".

## What Changes

- **One pointer rule for the zone (E-1):** a press in an agent zone that no control inside took puts keyboard focus in the zone's prompt input, and goes no further, so the zone tree never takes it. Controls that take focus (the prompt, a permission's buttons, the login row's, the queue row's, stop) keep taking it.
- **Scroll anchoring lives in `MessageScroller` (E-2, E-4, E-8):** one stated set of rules for following the end, reading back, items inserted at the start, an item shrinking under the view's top, and a remeasure of every item. While away from the end, a splice that replaces the item at the view's top keeps that item and its offset. Any insert at the owner's start clears the "top row in view" fact until a frame shows the row again. A new `remeasure_all` remeasures every item and keeps the item at the top, scaling its offset by how much it grew or shrank.
- **Remeasure only when a measure changed (E-8):** the zone remembers the text size and the two font families it measured at, remeasures when one of them changes (a zoom, a profile font size, a font edit), and not otherwise (a theme or colour change).
- **A focused permission button is in view (E-3):** Tab onto a pending permission's button scrolls its row into view; a permission button that has focus and is scrolled out of view gives focus back to the prompt input.
- **The working row's splice comes from the model (E-6):** `ChatModel` emits the working row's insert or removal at the end of its own ordered splices, computed when it absorbs. `AgentUpdate::Working`, `view::working_splice` and the view's copy of the row go.
- **Redraw on the link gesture only (E-7):** the view redraws on a modifier change only when the link gesture (`links::wants_links`) starts or stops.
- **Agent notifications get their own per-tab bucket (E-9):** the limiter keys its buckets by a closed `LimitClass::{Bell, Program, Agent}` per tab, the agent's with the programs' numbers, all under the global cap.
- **Manual checks:** `CLAUDE.md`'s list gains the presses only a real pointer shows: message text, a thought's trigger, a tool row, a message's copy button, the Earlier row and the scroll button each leaving focus in the prompt, and reading back while a fence, a thought and a command's output stream, through a config reload and a zoom.

No wire, transcript, checkpoint, config or state file changes.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-zones`:
  - added "A pointer press keeps focus in the zone": a press on anything that takes no focus puts it in the prompt input. "Keyboard path through an agent zone" keeps its text, which this restores ("Keyboard focus SHALL go to the prompt input when the zone is focused").
  - added "A focused permission button is in view": Tab onto a button out of view scrolls it into view; a focused permission button scrolled out of view gives focus back to the prompt.
  - modified "Scrolling the conversation": scenarios for a streaming code block, thought and command output at the top of the view, a config reload or settings preview, and a zoom.
  - modified "Earlier messages": a scenario for a page that arrives while the zone's tab is hidden.
- `ui-components`:
  - modified "Message scroller": scenarios for the item at the view's top growing and shrinking, items inserted at the start, and every item measured again;
  - added "Revealing an item in a message scroller".
- `desktop-notifications`:
  - "Rate limits": agent notifications are limited per tab apart from program notifications, under the same global cap.
  - "Agent notifications": "subject to the focus rule and the agent rate limits" instead of "the program rate limits".

## Impact

- **Code:**
  - `ui/src/components/message_scroller.rs`: `splice`'s anchoring, `remeasure_all`, `reveal`, the drawn range for the owner, tests;
  - `ui/src/primitives/focus_scope.rs`: `FocusScope::on_step`, which tells the owner where Tab moved focus;
  - `src/agent/view.rs` (and `view/tests.rs`): the press rule at the root, the measure key, the reveal and the focus give-back, the modifier rule, the working row read from the model;
  - `src/agent/model.rs`: the working row's splice in `finish`, `ChatModel::item_count`, `ChatModel::permission_block`; `Absorbed.working` goes;
  - `src/agent/mod.rs`: `AgentUpdate::Working` goes;
  - `src/zone_view.rs`: the capture handler's comment, and the tests that press into an agent zone in a `ZoneTreeView`;
  - `src/notify/policy.rs`: `LimitClass` and the limiter's key.
- **Unchanged:** the dump's lines (`--- working`, `--- focus`, `--- conversation`), every e2e case's expectations, the transcript and its protocols.
- **Docs:** `docs/code-map.md` (the `brindle-ui` scroller entry, the agent view, `notify/`), `CLAUDE.md` (manual checks), `ui/README.md` (the scroller's and `FocusScope`'s additions).
- **Recorded decisions changed:** `agents-zone-attention` decision 2 (the view applied the working row's splice), `agents-profiles` 5.3 ("`set_profile` measures every block again") and 7.1 (agent notices take the programs' per-tab bucket).
- **Overlap:**
  - **`agents-commands` (PR #151, unmerged):** it changes `src/agent/model.rs`, `src/agent/view.rs` and `PromptInput` (a suggestion list and a footer of buttons that take no focus) before this change is implemented. A press on those buttons reaches the new root rule, which leaves focus in the prompt, as its design intends. The first task re-opens every cited site on `main`.
  - **`agents-plans`:** its plan block sits between the conversation and the prompt and takes no focus; the press rule covers its trigger and dismiss mark. Its "the conversation's last row is the working row" still holds.
  - **`agents-prompt-context`, `agents-prompt-images`:** their attachment rows in the conversation and the prompt input take no focus; a press on them leaves focus in the prompt. Both add list items whose height changes when opened, which the anchoring rules cover.
  - **`agents-chat-selection` (proposal only):** a selectable text element will handle its own press and prevent the default, so the root rule leaves its presses alone (design, decision 1).
  - **Sibling remediation changes:**
    - `agents-client-state-fixes` reworks `Earlier` and the fetch ledger (D-7); E-4's view test is written against whichever `Earlier` shape is on `main`. Both changes modify `agent-zones` "Earlier messages", each adding its own scenarios: whichever is archived second carries the other's text in its delta.
    - `agents-module-structure` splits `model.rs`; the working row's splice lands in whichever module holds `finish` then.
    - `agents-absorb-cost` makes absorb render once per frame (D-3) and keeps pending permissions as a set; the working row's splice stays the last of the splices it emits, and `permission_block` reads that set if it exists.
    - `agents-event-bounds`, `agents-process-lifecycle`, `agents-checkpoint-deltas`: no overlap.
