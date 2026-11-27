# Design

## Context

See proposal.md (Why). Every site below was read at `5f335dc`; `agents-commands` (PR #151) changes `src/agent/model.rs` and `src/agent/view.rs` before this is implemented, so task 1.1 re-opens them on `main`.

**How a press reaches focus today (E-1).**
- gpui 0.2.2 registers, for every element that tracks a focus handle, a mouse-down listener that focuses that handle on a bubble-phase press over its hitbox when `!window.default_prevented()`, then prevents the default itself (`elements/div.rs:2025-2037`). It fires for any button. `Window::dispatch_mouse_event` runs bubble listeners in reverse registration order (`window.rs:3703-3712`), so a child's handlers run before its ancestors', and an element's own `on_mouse_down` runs before its own tracked-focus listener. `default_prevented` is reset per event.
- The innermost tracked ancestor of an agent zone's conversation is `ZoneTreeView`'s root (`zone_view.rs:2220`, `.track_focus(&self.focus_handle)`, the tree's own handle, field at `:934`). `ZoneTreeView::focus_handle` (`:1262`) returns the active zone's handle, but that is not the tracked one. `AgentView`'s root (`view.rs:1162-1189`) tracks nothing.
- Controls that take focus go through `brindle_ui::focus_from_pointer` (`ui/src/primitives/focus_visible.rs:72-76`: sets the pointer modality, focuses, prevents the default): a `Button` with a focus handle (`button.rs:192-195`), `TextArea::mouse_down` (`text_area.rs:493`). A `Button` without a handle registers no mouse-down handler at all, so its press bubbles: the message copy button (`view.rs:977-987`), the scroll-to-end button (`message_scroller.rs:395-401`). `Reasoning` (`reasoning.rs:86`), `Tool` (`tool.rs:162`) and the Earlier row's retry (`view.rs:854`) use `on_click` only. Only `CodeBlock` (`code_block.rs:110`), `Diff` (`diff.rs:180`) and the zone header's `inert` wrappers (`zone_view.rs:2128-2137`) prevent the default.
- The inactive-zone capture handler (`zone_view.rs:2061-2065`) runs first and focuses the zone (its prompt); then the bubble phase reaches the tree's listener, which takes focus to the tree's handle.
- So the finding holds as read. It was not observed. With focus on the tree's handle the dump's focus line reads `--- focus other modality=pointer` (`Workspace::describe_focus`, `workspace.rs:3186-3225`: no terminal, no agent stop), where it should read `--- focus agent stop=prompt modality=pointer`. No e2e case can reach it: Linux can't post a press, `--modality pointer` only sets the modality, and the debug pointer options take terminal cells (`debug-pointer-hook` is proposal-only, blocked on gpui). `zone_view.rs`'s tests already build a `ZoneTreeView` with an agent zone in a `VisualTestContext` (the `Tab` fixture, `ZoneStart::Agent`), and gpui's `VisualTestContext::simulate_mouse_down`/`simulate_click` go through the real hit test and dispatch, so the fault is shown there first (task 2.1).

**How the list keeps its place (E-2, E-4, E-8).**
- `MessageScroller` (`message_scroller.rs`) wraps gpui's `ListState` with `ListAlignment::Bottom`. While it follows its end the list keeps no position (`logical_scroll_top` is `None` after a layout) and lays out from the bottom; once away it keeps `ListOffset { item_ix, offset_in_item }`.
- `ListState::splice_focusable` (`list.rs:272-306`) rewrites that position: a top item inside the replaced range becomes `{ old.start, 0 }`; a range ending at or before it shifts `item_ix` by the change in count, offset kept. Replaced items become `Unmeasured`. `layout_items` (`list.rs:604-760`) renders items from the top item down with `item_origin.y -= offset_in_item`; an offset past a shrunk item's height puts the item wholly above the view and shows what follows it shifted up by the excess. A width change marks every item unmeasured and keeps the position (`prepaint`).
- `MessageScroller::splice` (`:237-248`) passes splices through, special-casing only an insert at the owner's start while the top row is the first item in view; `refresh_top_row` (`:215-225`) restores the offset for item 0 only. So a re-splice of the item at the top (an agent message's changed tail, `model.rs:1820-1827`; a thought, one item spliced on every chunk, `:2135-2145`; an open tool row's growing last run, `:1846-1863`) resets its offset (E-2), and `AgentView::set_profile` splices `0..count` (`view.rs:487-498`) on every `ZoneTreeView::restyle` (`zone_view.rs:1467-1476`), which `Workspace::apply_config`, the settings previews and `end_previews` call.
- `top_in_view` is cleared on a prepend only when away with the row first in view (`:239-246`), and otherwise only by a drawn frame (`:360-381`). `AgentView::follow_earlier` (`view.rs:400-414`) asks `fetch_if_due(true, true)` on every `sync` while `top_row_in_view()`; `ChatModel::fetch_due` (`model.rs:1569-1576`) asks whenever `Earlier::Available` and the row is in view. A hidden tab draws no frame, so after a page arrives the stale `true` asks again, and again (E-4, as read).
- A zoom is `Workspace::change_font_size` (`workspace.rs:2908-2911`): it updates `Settings.zoom` and refreshes windows; the view's Settings observer runs `sync`, which splices nothing, so off-screen items keep the old heights (E-8, as read). `Settings::font_size` adds the zoom to a profile's size too (`settings.rs:237-240`).

**Focus that leaves the list (E-3).** `FocusStops::stops` (`view.rs:305-320`) lists the pending permissions' handles, and `FocusScope::step` (`focus_scope.rs`) focuses them with Tab. Blocks are spliced without focus handles (`view.rs:507-513`), so `layout_items`' "render the focused off-screen item" path (`list.rs:741-760`) never sees one, and a focused button out of view isn't in the rendered frame; key dispatch then starts at the window's root. The login row and the queue row are outside the list (`AgentView::render`), so only permission buttons are affected.

**The working row (E-6).** `ChatModel::finish` reports `Absorbed.working` when the row changed; `Agent::apply_absorbed` (`agent/mod.rs:747-759`) emits `AgentUpdate::Splices` then `AgentUpdate::Working`; `AgentView::on_update` (`view.rs:514-522`) reads `model().blocks().len()` when it handles the event. `Agent::drain` applies a whole paced chunk inside one `update` (`mod.rs:668-697`), and gpui delivers emitted events after it, so the length read is the chunk's final one. A chunk of an exit (`ChatModel::ended`, the row goes) then an absorb that adds a block removes `L+1..L+2`, past the end, and the next insert lands before the stale row: one empty item stays. The audit checked this by hand.

**Modifiers (E-7).** `AgentView::render` has `.on_modifiers_changed(cx.listener(|_, _, _, cx| cx.notify()))` (`view.rs:1179`); gpui dispatches the event along the focused path and refreshes nothing itself (`window.rs:3915-3929`), so the notify is the whole cost, a full window render per shift press.

**Notifications (E-9).** `RateLimiter` (`notify/policy.rs:134-178`) keys per-tab buckets by `(tab, is_bell)`; `Source::Agent` shares the programs' `OSC_PER_TAB` bucket, which `agents-profiles` 7.1 pinned (`agent_notices_take_the_program_rate_limit`) when a tab held one kind of zone. `Notifier::send_request` (`notify/mod.rs:413`) drops what the limiter refuses with a debug log.

## Goals / Non-Goals

**Goals:**
- One rule for presses in an agent zone, in the zone's own view, that holds for every control a later change adds.
- One scroll-anchoring contract in `MessageScroller`, stated as rules, each with a unit test, with `brindle-ui` knowing nothing of agents (`ports R2`).
- The working row's place decided where the blocks are decided.

**Non-Goals:**
- Selecting text in the conversation (`agents-chat-selection`).
- Splitting large tables and lists into parts (E-5, `agents-markdown-long-blocks`).
- An in-process pointer hook for e2e cases (`debug-pointer-hook`).
- Anchoring on a line rather than an item: when an item below the top one changes height, the lines below it move, as with browsers' scroll anchoring.
- Any change to the dump, the transcript, a protocol, a config key or a state file.

## Layers

| Part | Layer | Crate |
| --- | --- | --- |
| `top_after_splice`, `Anchor` and `settle_anchor`, `inserts_at_start`, `reveal_plan` | pure functions beside `Follow` and `page_scroll` | `brindle-ui` (`components/message_scroller.rs`) |
| `MessageScroller::{splice, remeasure_all, reveal, on_items_view}` | component entity (shell over gpui's list) | `brindle-ui` |
| `FocusScope::on_step` | primitive | `brindle-ui` (`primitives/focus_scope.rs`) |
| the working row's splice in `finish`; `item_count`; `permission_block` | GPUI-free model | `brindle` (`src/agent/model.rs`) |
| `Measure` and its comparison; `link_gesture_flip` | pure, beside the view | `brindle` (`src/agent/view.rs`) |
| the press rule, the reveal and give-back wiring, the Settings observer's remeasure, the modifier listener | shell (entity) | `brindle` (`src/agent/view.rs`) |
| `LimitClass` and the limiter's key | pure model | `brindle` (`src/notify/policy.rs`) |

No decision moves into a view, and no crate reference changes (`ports R1`, `R3`).

## Decisions

### 1. A press nobody took focuses the prompt, at `AgentView`'s root (`fcis R1`; `ports R2`; `types R12`)

`AgentView::render` adds one bubble-phase `on_any_mouse_down` on its root `div` (the one with the `AgentZone` key context): when `!window.default_prevented()`, it calls `brindle_ui::focus_from_pointer(&prompt_handle, window, cx)`, which focuses the prompt input, sets the pointer modality and prevents the default. Because bubble listeners run child first, this sees every press inside the zone after the controls under it:
- **A control that takes focus** (the text area, a permission's or the login row's button, the queue row's, stop, Continue) has already called `focus_from_pointer` and prevented the default; the rule does nothing. So a press on `Reject` still answers it, with focus on it.
- **Anything else** (text, a thought's trigger, a tool row, the copy and scroll buttons, the Earlier row, an alert, padding) gets the prompt focused, and the zone tree's tracked listener, seeing the default prevented, does nothing. Clicks still fire on release: `on_click` doesn't depend on focus.
- **Any button.** gpui's tracked-focus listener is not button-specific, so a right or middle press in the conversation would also move focus to the tree; the rule takes any button. The remediation plan said "a left press"; this is the reason it covers all three. Nothing in an agent zone binds a right or middle press today.
- **Focus-visible.** The prompt is a text input, which shows its ring whenever focused (`shows_ring`), as after a press into it today; the zone card's ring stays off under the pointer modality.
- **The inactive-zone capture handler** (`zone_view.rs:2061-2065`) stays: it makes the zone the active one (`active`, `last_focused`, `TitleChanged`) before any control takes focus. Its comment changes: the zone's own rule now keeps focus in the zone. Focusing the already-focused prompt again is a no-op (`Window::focus` returns early).
- **Selectable text later** (`agents-chat-selection`, proposal only, question 4). A selectable-text element handles its own press. If it keeps focus in the prompt it prevents the default and focuses nothing, and this rule leaves it alone; if it lets the press through, the prompt is focused, which doesn't clear the prompt's own selection. Either answer to question 4 fits this rule.

The decision is gpui mechanism, not a policy: it chooses between nothing and one fixed action from one flag. No model is added for it (`fcis R10` asked and answered).

*Rejected:*
- `track_focus(&prompt_handle)` on the root: two elements would track one handle, and gpui maps a handle to one dispatch node, so key dispatch could start at the root and lose the text area's `TextArea` context.
- A body handle of the zone's own tracked at the root: focus would land on a handle that is no stop (`FocusScope::step`'s debug assertion) and typing would go nowhere.
- Dropping `ZoneTreeView`'s `track_focus`: the press would bubble to the next tracked ancestor in the workspace; the fault would move, not go.
- A `prevent_default` wrapper per control (the `CodeBlock`/`Diff` way): it is how this was missed, and every later control (`agents-commands`' footer, `agents-plans`' trigger, `agents-prompt-context`'s rows) would need one.
- An opt-in on `FocusScope` ("a press nobody took focuses the first stop"): every overlay is a `FocusScope` (`overlay.rs:325`), and dialogs have their own press behaviour; one user doesn't justify the abstraction (`types R12`).

### 2. The scroller's anchoring contract (`fcis R1`, `R8`; `ports R2`; `types R1`)

`MessageScroller` owns where the view is. Its rules, in list indices (the top row, while shown, is item 0):

1. **Following the end** (`Follow::AtEnd`): the list keeps no position and lays out from the bottom, so any splice, insert or remeasure keeps the end in view. Nothing is restored. (Today's behaviour; `a_growing_last_item_stays_in_view`.)
2. **Away, splice wholly before the top item:** the top item keeps its offset and its index moves by the change in count. (gpui's rule, now computed by the scroller.)
3. **Away, splice wholly after the top item:** the top is unchanged.
4. **Away, the top item inside the replaced range, `count > 0`:** the top keeps its index when that index is still among the new items, else becomes the last new item, and keeps its offset. Agent splices replace from the first changed part to the end of the entry, so an unchanged part keeps its index and a growing one keeps its offset.
5. **Away, the top item inside a removed range (`count == 0`):** the item after the removed run goes to the view's top, offset 0. (gpui's rule.)
6. **An insert at the owner's start** (a splice whose range starts at the owner's first item and adds items) clears `top_in_view`, whatever the follow state; only the next drawn frame can say the row is in view again (E-4). The existing rule stays before rule 2: away with the top row the first item in view, the old first item goes to the view's top.
7. **The top item shrank under its offset:** after the frame that measures it, when its height is at most its offset, the item after it goes to the view's top.
8. **A remeasure of everything** (`remeasure_all`): every item, the top row included, is replaced by itself, so all are measured again; the top keeps its item and its offset scaled by the item's new height over its old (the old from the last drawn frame). The view follows the end as before if it did.

The pure parts are plain values, tested without a window:
- `top_after_splice(top: Top, old: Range<usize>, count: usize) -> Top`, with `Top { item: usize, offset: f32 }`: rules 2–5. The scroller reads `logical_scroll_top` before the splice, calls gpui's splice, then, when away, `scroll_to`s the computed top, so gpui's own rewrite is never relied on.
- `enum Anchor { Offset { item, offset }, Fraction { item, fraction } }` and `settle_anchor(anchor, measured_height) -> Option<Top>`: rules 7 and 8. The scroller keeps a pending `Option<Anchor>` after a splice that touched the top item (`Offset`) or a remeasure (`Fraction`), and its after-draw canvas, which already reads the drawn items' bounds, settles it once the item has been measured. A settle that moves the view notifies, so it takes one more frame.
- `inserts_at_start(old: &Range<usize>, count: usize) -> bool`: rule 6, in owner indices.

`refresh_top_row`'s special case goes: rule 4 covers it (the row, item 0, replaced by itself). `reset` keeps its meaning (every item replaced, scrolled to the end).

*Rejected:* restoring the top only in `AgentView` (the anchoring would be split between the owner and the component again, the gap the review names); measuring the top item synchronously in `splice` with `layout_as_root` to settle rules 7 and 8 in the same frame (the scroller has no window in `splice`, and an item's height depends on the width only the list's prepaint knows).

### 3. Remeasure only when a measure changed (`types R10`; `fcis R1`)

`src/agent/view.rs` gains `Measure { size: Pixels, ui_family: SharedString, code_family: SharedString }` (`PartialEq`), what an item's height depends on besides the width (which the list handles itself) and the text. The view keeps the `Measure` it last measured at; its doc names what refreshes it: `AgentView::remeasure_if_changed`, called from `set_profile` and from the Settings observer (which a zoom, a font preview and a config reload all reach). When the new `Measure` differs, it calls `MessageScroller::remeasure_all` (decision 2, rule 8); otherwise nothing. `set_profile` no longer splices: a theme changes colours only. This changes `agents-profiles` 5.3 ("`set_profile` measures every block again"), which predates the scroller keeping its place. Line height isn't in `Measure` because the zone doesn't read the terminal's line height; a change that makes it do so adds it.

### 4. Reveal a permission on Tab; give focus back when it leaves the view (E-3; `ports R2`; `types R3`)

- **`FocusScope::on_step(f)`** (`brindle-ui`): after `step` focuses a stop, `f(&handle, window, cx)` runs. `AgentView`'s scope passes a closure that, when the handle is a pending permission's, looks up its block (`ChatModel::permission_block(entry) -> Option<usize>`, computed at the call, `types R3`) and calls `MessageScroller::reveal(ix)`. Tab and shift-Tab are the only ways focus reaches a permission button other than a press on it, which is in view.
- **`MessageScroller::reveal(ix)`**: the pure `reveal_plan(drawn: Option<(f32, f32)>, view: (f32, f32)) -> Reveal::{InView, Scroll}` says whether the last frame drew the item wholly in view; if not, the scroller pins the view at what the last frame showed (as `scroll_page` does, since while following the list has no position and gpui's `scroll_to_reveal_item` would read the end as the top), calls `scroll_to_reveal_item`, and sets `Follow::Away`; `Follow::after_draw` returns it to `AtEnd` when the reveal showed the end, as for a page key.
- **Give-back:** `MessageScroller::on_items_view(f)` calls the owner, after a drawn frame whose set of owner items in view changed, with that range. `AgentView` then checks whether focus is on a permission button whose block isn't in the range, and if so focuses the prompt. This extends the zone's existing rule ("a stop the focus is on goes away, focus goes back to the prompt", `sync`) from a stop that left the model to one that left the view. Nothing is answered: a pending request stays pending.

*Rejected:* `splice_focusable` for permission blocks. A permission block is one list item whose stops are its buttons; gpui asks the item's handle `contains_focused`, which needs an element tracking that handle around the buttons. That wrapper would take every press on the row nobody prevented (against decision 1) and be a focus target that is no stop. It would also keep an unseen button live, so enter could answer a request the user can't see. And every splice of a permission block would have to pass its handle again. *Also rejected:* a focus subscription per permission handle (`cx.on_focus`), which needs a window in `refresh` and a subscription list kept in step with the handles.

### 5. The working row's splice comes from the model (E-6; `fcis R1`; `types R1`)

`ChatModel::finish` appends the row's insert or removal to `Absorbed.splices`, after the blocks' splices, at `self.blocks.len()` as it is then: insert `len..len` when the row appears, remove `len..len+1` when it goes, nothing when only its words change (the view redraws it from `working_row()`). The blocks' splices never reach the row's index: an insert at the old end lands before the row. `ChatModel::item_count()` is the blocks plus the row while it shows. `Absorbed.working`, `AgentUpdate::Working`, `view::working_splice` and `AgentView::working` go. The view reads `model().working_row()` in `render_block` (the item after the last block) and in `refresh_motion` (from `sync`). The pure `working_splice` moves into `model.rs`, private, with its unit test.

`Block` gains no variant for the row: `Block.entry` is an `EntryId` and the row has none, so a variant would need a sentinel (`types R7`), and every reader of `blocks()` (`pending_permissions`, `last_code_block`, `message_text`, the dump) would have to skip it. This changes `agents-zone-attention` decision 2, where the view applied the splice "after the model's splices"; the order was right, the length was read too late.

**The dump is unchanged.** `--- working` comes from `working_row()` (`model.rs:3026`), `--- conversation entries=N` counts entries, and the scroller's item count is not printed. `tests/e2e/cases/agent.rs`'s `agent-working` and every other case read the same lines.

### 6. Redraw on the link gesture only (E-7; `fcis R1`)

The view keeps `link_gesture: bool`. The pure `link_gesture_flip(was: bool, modifiers: &Modifiers) -> Option<bool>` returns the new value when `links::wants_links(modifiers)` differs from `was`. The listener stores it and notifies only then. Using `wants_links` (ctrl held, alt not) rather than `control` keeps the redraw tied to what `Response::link_modifiers` shows the pointing hand for.

### 7. Agent notifications get their own per-tab bucket (E-9; `types R1`; `fcis R2`)

`policy.rs` gains `enum LimitClass { Bell, Program, Agent }` with an exhaustive `LimitClass::of(Source)` and `LimitClass::limit(self) -> Limit`: `BELL_PER_TAB`, `OSC_PER_TAB`, and a new `AGENT_PER_TAB` with the programs' numbers (3 at once, one per 2 s). `RateLimiter.tabs` is keyed `(u64, LimitClass)`, replacing the `bool`, and `prune` reads the class's limit. The global bucket is unchanged and charges every class. The limiter stays pure: `allow(tab, source, now)` takes the time. This changes `agents-profiles` 7.1 (agent notices took the programs' bucket), which predates tabs holding terminal and agent zones (`zones-mixed-tabs`).

*Rejected:* exempting `blocked` from the per-tab limit only (an agent that fails and asks in a loop would have no per-tab bound); exempting agents from the global cap (16 pending permissions per session across many zones could flood the desktop).

### 8. Failures, restarts and the wire

Nothing here can fail in an expected way: a permission answered between Tab and the reveal has no block, so `permission_block` is `None` and nothing scrolls; a settle whose item went is dropped. Nothing is server state, so a core restart, a Brindle crash and an upgrade do nothing new: the replay rebuilds the blocks, the scroller starts at its end, and the measure is taken at the first `set_profile`. No wire, transcript, checkpoint, config or state file changes (`process R4`–`R6` not engaged).

### 8a. A fault the first test can't show (`testing R9`)

Each finding's group starts with a test that shows the fault on the code as it is. E-1 was read, not observed; E-3, E-4, E-7, E-8 and E-9 were taken from the report without a second reading. When a first test passes before the fix:
- the test stays, as a pin of the behaviour the spec states;
- the task records why the fault didn't show (the site moved on `main`, or gpui does something the reading missed);
- that finding's fix is not made, unless another finding's fix needs it (decision 2's `top_after_splice` is needed by E-2 whether or not E-8 shows), and the proposal's and code map's lines for it are dropped in the same commit.

E-1 is the one where a test could miss a real fault rather than show its absence: if `VisualTestContext::simulate_mouse_down` turns out not to run the tracked-focus listener (it is meant to, through `Window::dispatch_event`), the press is checked by hand on a build before the fix, through the dump's focus line after a real press (`--- focus other` before, `--- focus agent stop=prompt` after), and the fix goes ahead on that.

### 9. The remediation plan's settled rows

No row of the plan's Decisions (D1) or choices (C1–C5) tables touches this change. Its section 2 is followed, with two refinements recorded above: the press rule covers every button (decision 1), and the working row is the model's last list item rather than a `Block` (decision 5).

### 10. Tests (`testing R1`, `R13`, `R15`, `R16`)

- **Unit, pure:** `top_after_splice` for each of rules 2–5 and for a prepend; `settle_anchor` for growth (nothing), a shrink under the offset, and a fraction at double and half height; `inserts_at_start`; `reveal_plan`; `Measure` equality; `link_gesture_flip`; `LimitClass::of` for every `Source`; the limiter's agent bucket; the model's working splice at `len` after an absorb that also adds blocks, and after `ended`.
- **Component (`VisualTestContext`, `message_scroller.rs`):** a top item that grows while away keeps its lines' bounds; one that shrinks under its offset puts the next item at the top after one frame; a prepend while following with the row in view clears `top_row_in_view` until the next draw; `remeasure_all` at double heights keeps the top item at the same fraction; `reveal` of an item above the view, of one in view, and of the last item.
- **View (`src/agent/view/tests.rs`):** `set_profile` with an unchanged size leaves `scroll_top()` alone; a zoom keeps the top entry at its fraction; a page arriving in a hidden host asks for no other page; Tab to an off-screen permission reveals it and enter answers it; wheel away from a focused permission gives focus to the prompt; one chunk of an exit and an added notice leaves `item_count` equal to the model's; a shift press doesn't notify and a ctrl press does.
- **Zone tree (`src/zone_view.rs`):** presses on message text, a thought's trigger, a tool row's line, a message's copy button and the Earlier row keep the prompt focused; a press on a permission button focuses it and answers; a press into an inactive agent zone focuses its prompt; escape after a press cancels the turn.
- **E2E:** none new: no case can post a press, and the scroll and zoom checks are geometry that unit and component tests pin. Existing cases must pass unchanged (the dump doesn't change).
- **Manual (`CLAUDE.md`):** decision 11.

### 11. Manual checks added to `CLAUDE.md`

What only a real pointer, wheel or zoom shows, on Linux and on Windows:
- In an agent zone, a pointer press on a message's text, a thought's "Thought" label, a tool row's line, a message's copy button (shown on hover), the "Earlier messages" row and the scroll-to-bottom button each leaves the caret in the prompt input: typing goes there, escape stops a running turn, tab reaches the next stop, and shift-pageup scrolls. A right-click on a message does the same. A press on a permission's `Reject` answers it.
- Reading back: wheel up into the middle of a streaming code fence, a streaming thought and a running command's open output; the lines in view stay put. Then reload the config (an edit in the file), preview a theme in the settings dialog, and zoom in and out (ctrl-+, ctrl--); the entry at the top stays at the same place, and nothing jumps when wheeling on.
- Scrolled up when a permission arrives below the view: tab from the prompt scrolls the row into view with `Reject` focused; wheel it out of view and the caret is back in the prompt, the request still pending.

## Risks / Trade-offs

- [Rules 7 and 8 settle one frame late, so a zoom or a shrinking item may show one frame at the old offset] → only a zoom, a font change or an item shrinking under the view's top does it; streaming growth (rule 4) is exact in the same frame. The manual zoom check watches for it; the alternative is in Open Questions.
- [Lines below a changed item in view still move] → the anchor is the top item, by design (Non-Goals); the spec's scenarios are written for the item at the top.
- [The press rule focuses the prompt on a right press too] → nothing in an agent zone binds one; if `agents-chat-selection` adds a context menu, its handler prevents the default first.
- [Giving focus back on scroll surprises a keyboard user who wheels with focus on a button] → keys could do nothing there anyway (the button isn't drawn); the prompt is the zone's focus, and Tab brings the button back into view.
- [`on_items_view` runs after every frame whose drawn range changed] → it runs from the canvas the scroller already has, deferred, and the owner's check is a lookup only while a permission button has focus (`process R14`).

## Migration Plan

None: no state, wire or config changes. A build with this change behaves the same on restore; the next `set_profile` takes the measure.

## Open Questions

1. **If the one-frame settle of rules 7 and 8 shows as a flicker in the manual zoom check:** measure the top item in the scroller's own prepaint before the list lays out (a second `layout_as_root` of that one item at the list's width) and pass the settled offset to the list in the same frame. Recommendation: ship the post-frame settle, and take this only if the manual check shows the flicker.

## As built (2026-11-27)

Built on `fix/agents-client-state` (PR #153) at `dd43e7c`; task 1.1 lists the sites. Numbered flags are decisions taken while building that the design didn't settle.

1. **A respawn's splices (decision 5).** `ChatModel::respawning` used to call `life_event` (whose `finish` now appends the working row's splice) and then append the thoughts' splices, which would have put a block splice after the row's. It now gathers the rows' and thoughts' splices and calls `finish` once, so the row's splice is still the last. A respawn always removes the row (the phase goes to `Connecting`), so the old order was harmless in practice; the change keeps decision 5's rule ("the row's splice ends the absorb's") true for every `Absorbed`.
2. **A pending anchor across splices (decision 2).** Several splices can arrive before the frame that settles an anchor (a paced chunk). The anchor follows its item: a splice before or after the top re-points it to the top's new index, one that removes the top item drops it, and one that replaces the top item again sets a fresh `Offset`, except that a pending `Fraction` is kept (its fraction is of the height last drawn, which a second replacement doesn't change). The design said only "a pending `Option<Anchor>` after a splice that touched the top item".
3. **The settle runs deferred (decision 2).** gpui ignores a notify while it draws (`Invalidator::invalidate_view` marks no window dirty outside `DrawPhase::None`), so the after-draw canvas defers the settle with `cx.defer`; its `scroll_to` and notify then draw the second frame at once. The same holds for the canvas's existing `scroll_to_end`, which only sets state the next frame reads, so it wasn't changed.
