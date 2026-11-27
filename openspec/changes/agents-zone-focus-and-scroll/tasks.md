# Tasks

Each group writes its tests first and watches them fail before the code they check; a first test that passes is handled as design 8a says. Test names are sentences (`testing R15`). Nothing sleeps or asserts on elapsed time (`testing R7`, `R8`). Pure functions come first, then `brindle-ui`, then the app's model and view.

## 1. Re-open the sites on `main`

- [x] 1.1 After `agents-commands` (PR #151) and any sibling remediation change that has merged (`agents-client-state-fixes`, `agents-module-structure`, `agents-absorb-cost`), re-open every site design.md's Context cites on `main`: `AgentView::render`'s root, `set_profile`, `on_update`, `follow_earlier`, `FocusStops::stops`; `ChatModel::finish`, `working_row`, `fetch_due`; `Agent::apply_absorbed` and `drain`; `MessageScroller::splice` and `refresh_top_row`; `FocusScope::step`; `ZoneTreeView::render_leaf` and its root; `RateLimiter`. Note in this task what moved (a module split, a new control in the zone, the `Earlier` shape) and adjust the later tasks' paths before starting them. Verify by listing each site's current file and line here.
  - **Base:** `fix/agents-client-state` at `dd43e7c` (PR #153, `agents-client-state-fixes`, not merged), over `main` at `b6a0316` (with `agents-commands`, #151). `agents-module-structure` and `agents-absorb-cost` haven't landed: `model.rs` is still one file, and absorb still renders per chunk.
  - **Sites:** `src/agent/view.rs`: `AgentView::render`'s root `div` `:1234` (the `AgentZone` key context; `on_modifiers_changed` `:1253`), `set_profile` `:530`, `on_update` `:550`, `follow_earlier` `:443`, `FocusStops::stops` `:346`. `src/agent/model.rs`: `ChatModel::finish` `:2545`, `working_row` `:1749`, `fetch_due` `:1822`, `Absorbed` `:1270`. `src/agent/mod.rs`: `drain` `:712`, `apply_absorbed` `:792`, `AgentUpdate::Working` `:204`. `ui/src/components/message_scroller.rs`: `splice` `:237`, `refresh_top_row` `:215`. `ui/src/primitives/focus_scope.rs`: `FocusScope::step` `:79`. `src/zone_view.rs`: `render_leaf` `:2061` (its capture handler `:2077`), the root's `track_focus` `:2236`, `restyle` `:1483`. `src/notify/policy.rs`: `RateLimiter` `:134`.
  - **What moved:** `agents-commands` added the prompt input's completion list and its footer of option buttons (`PromptInput::set_suggestions`, `set_footer`), which take no focus; their presses reach the root rule as the proposal's Overlap says. `agents-client-state-fixes` made `ChatModel::earlier()` computed (`Paging` plus whether a `Sent::Fetch` is pending) rather than stored; `Earlier`'s four variants and `fetch_due`'s rule are unchanged, so task 6.1's hidden-zone test is written against them as they are. No path in the later tasks changes.

## 2. Agent notifications get their own bucket (E-9, `src/notify/policy.rs`)

- [x] 2.1 Add tests to `policy.rs`'s `mod tests` and verify they fail first (`cargo test -p brindle notify::policy`):
  - `three_program_notifications_leave_an_agents_bucket_alone`: three `Source::Osc` notifications for tab 1, then a `Source::Agent` one for tab 1 at the same instant, is allowed; a fourth `Osc` is not.
  - `an_agent_has_three_at_once_then_one_every_two_seconds`.
  - `every_class_counts_against_the_global_cap`: ten mixed notifications across tabs, then an agent's is refused.
  - `every_source_has_a_limit_class`: `LimitClass::of` for each `Source` (an exhaustive list in the test).
  Replace `agent_notices_take_the_program_rate_limit`, which pins the old behaviour.
  - *As run:* the four tests (with `Source::Osc9` for "`Source::Osc`") failed to build first: `LimitClass` and `AGENT_PER_TAB` didn't exist (12 errors, `cargo test -p brindle --bin brindle notify::policy`). The old pin went.
- [x] 2.2 Add `enum LimitClass { Bell, Program, Agent }` with `of(Source)` (exhaustive, no `_ =>`, `railway R3`) and `limit()`, `AGENT_PER_TAB` with the programs' numbers, and key `RateLimiter.tabs` by `(u64, LimitClass)` (design 7). Verify that 2.1 and the existing limiter tests pass.
  - *As run:* `cargo test -p brindle --bin brindle notify::`: 51 passed.
- [x] 2.3 In `docs/code-map.md`'s `notify/` entry, say that agents have their own per-tab bucket under the global cap, and that this replaces `agents-profiles` 7.1. Verify the entry names `LimitClass`.
  - *As built:* the agent bullet names `AGENT_PER_TAB`, `LimitClass::{Bell, Program, Agent}` and `LimitClass::of`, and says it replaces `agents-profiles` 7.1.

## 3. The working row's splice comes from the model (E-6, `src/agent/model.rs`, `mod.rs`, `view.rs`)

- [x] 3.1 Add tests and verify they fail first:
  - `model.rs`: `the_working_rows_splice_follows_the_blocks_splices`: an absorb that adds a block and starts the row reports the block's splice, then the row's insert at the new block count; `ended` reports the row's removal at the block count; a change of the row's words alone reports no splice. `cargo test -p brindle agent::model::tests::the_working_rows`.
  - `view/tests.rs`: `an_exit_and_a_notice_in_one_chunk_leave_no_empty_item`: with the row showing, one fed chunk ends the session (the row goes) and adds a notice; the scroller's `item_count()` equals `ChatModel::item_count()`, and every item draws a block or the row. `cargo test -p brindle agent::view::tests::an_exit`.
  - *As run:* with `ChatModel::item_count()` added first (blocks plus the row while it shows, no behaviour of its own), both failed on behaviour: the model's absorb reported `[0..0 +1]` with no row insert after it, and the view held `(3, 2)` items, scroller to model: E-6 shown, the stale row left after the notice. The view test checks the counts and that `agent-working` isn't drawn (equal counts leave no item that draws neither).
- [x] 3.2 In `ChatModel::finish`, append the row's splice to `Absorbed.splices` after the blocks' (design 5); move `working_splice` into `model.rs` (private, with its unit test, replacing `the_working_rows_splice_and_the_permission_variants`' assertions on it); add `ChatModel::item_count()`. Remove `Absorbed.working`, `AgentUpdate::Working`, `AgentView::working` and `view::working_splice`; `render_block` and `refresh_motion` read `model().working_row()`. Update `src/agent/tests.rs`' assertion that `Working` is the last update, and `the_working_row_ends_the_conversation_while_the_turn_works` to read the model. Verify that 3.1 and `cargo test -p brindle agent::` pass, and that `--- working` in `ChatModel::dump` is unchanged (`model.rs`'s dump tests).
  - *As built:* `working_splice` and its test (`the_working_rows_splice_inserts_removes_or_leaves_it`) are in `model.rs`; the view test kept its permission variants as `the_permission_variants`. `Before.working` is a `bool` now. `ChatModel::respawning` no longer appends the thoughts' splices after `life_event`'s `finish` (which would have put them after the row's removal): it gathers the rows' and thoughts' splices and calls `finish` itself (flag 1). The view calls `refresh_motion` from `sync`. Tests that pinned `Absorbed.working` read the model instead (`the_working_row_follows_the_turn` checks the row and whether the absorb's last splice is its insert or removal; `a_tool_call_in_progress_above_the_page_shows_working_until_fetched` checks that no fetch's splice reaches past the blocks); `streaming_a_long_message_re_renders_only_what_changed`'s "changes nothing" absorb, which starts the turn, now expects the row's insert alone. `cargo test -p brindle`: 1288 passed; the dump tests unchanged.
- [x] 3.3 In `docs/code-map.md`'s agent view entry ("The working row"), replace "reaches the view as `Absorbed.working` → `AgentUpdate::Working` … the view applies the pure `working_splice`" with the model's splice at the end of `Absorbed.splices`, and note that it replaces `agents-zone-attention` decision 2's view-side splice. Verify the entry names no removed item.
  - *As built:* the entry names `ChatModel::finish`, `item_count` and the private `working_splice`; `grep` finds no `Absorbed.working` or `AgentUpdate::Working` in `docs/`.

## 4. The scroller's anchoring contract (E-2, E-4, E-8, `ui/src/components/message_scroller.rs`)

- [x] 4.1 Add pure tests beside `Follow`'s and verify they fail first (`cargo test -p brindle-ui message_scroller`):
  - `top_after_splice`: `a_splice_before_the_top_shifts_it_and_keeps_its_offset`, `a_splice_after_the_top_leaves_it`, `a_splice_that_replaces_the_top_keeps_its_item_and_offset`, `a_splice_that_shortens_past_the_top_keeps_the_last_new_item`, `a_splice_that_removes_the_top_puts_the_next_item_at_the_top`.
  - `settle_anchor`: `an_item_that_grew_needs_no_settling`, `an_item_shrunk_under_its_offset_gives_the_top_to_the_next_item`, `a_fraction_settles_at_the_new_height`.
  - `inserts_at_start`: `only_an_insert_at_the_owners_first_item_counts`.
  - *As built:* each group is one test whose cases carry the listed names as comments, `the_top_after_a_splice`, `settling_an_anchor` and `an_insert_at_the_start` (a test named after the function would shadow it inside `mod tests`). They failed to build first (the functions didn't exist).
- [x] 4.2 Add component tests in `VisualTestContext` (the existing `host`, `item_bounds`, `redraw`, `scroll_to` helpers) and verify they fail first:
  - `a_top_item_that_grows_while_away_keeps_its_lines`: scrolled 200 px into item 3, item 3 is replaced by a taller one; its top's bounds are the same after the redraw.
  - `a_top_item_that_shrinks_under_the_view_gives_the_top_to_the_next`: 400 px into a 600 px item, replaced by a 100 px one; after the frames settle, item 4's top is at the view's top.
  - `a_prepend_while_following_clears_the_top_row_until_a_frame_shows_it`: with the top row in view and following, a prepend makes `top_row_in_view()` false until the next draw.
  - `remeasuring_every_item_keeps_the_top_item_at_its_fraction`: heights doubled, `remeasure_all`; the top item is at the view's top at the same fraction of its height, and the item above has its new height once scrolled to.
  - `remeasuring_while_following_keeps_the_end`.
  - *As run:* before 4.3, the three that use the existing API failed on behaviour: the grown item's top went from −200 to 0 (E-2 shown: gpui zeroed the offset), the shrunk item's successor was at 100 not 0, and the prepend left `top_row_in_view()` true. The prepend test reads the fact inside the same `update` as the splice, since the test context draws the frame the splice asks for when the update ends; with 4.3's rule 6 narrowed back to the old condition it fails again (checked). The two remeasure tests failed to build (`remeasure_all` didn't exist).
- [x] 4.3 Implement design 2: `Top`, `top_after_splice`, `Anchor`, `settle_anchor`, `inserts_at_start`; `splice` reads the top, splices, and while away `scroll_to`s the computed top, keeping a pending `Anchor::Offset` when the splice touched the top item; rule 6 clears `top_in_view` on every insert at the start; `remeasure_all` with `Anchor::Fraction`; the after-draw canvas settles a pending anchor and notifies when it moved the view; `refresh_top_row`'s special case goes. Verify that 4.1, 4.2 and the existing scroller tests (`a_growing_last_item_stays_in_view`, `an_item_added_while_away_leaves_the_view_alone`, `items_spliced_in_at_the_start_keep_the_view`, `a_page_up_reaches_the_top_row`, `page_keys_leave_and_reach_the_end`) pass, and that `cargo test -p brindle --test architecture` passes (`brindle-ui` names no app type, `ports R2`).
  - *As built:* every list change goes through `splice_list` (list indices), `show_top_row` and `refresh_top_row` included, which reads the top, splices, and while away `scroll_to`s `top_after_splice`'s top; a pending anchor follows its item through later splices, and a `Fraction` survives the item being replaced again before the frame (flag 2). The settle reads the drawn height of the item at the top, checks an `Offset` anchor at the offset the view is at then (a wheel may have moved it within the item), and runs deferred from the after-draw canvas: a notify during a draw doesn't mark the window dirty (gpui's `Invalidator::invalidate_view` ignores it while drawing), so an undeferred settle would wait for some later frame (flag 3). `cargo test -p brindle-ui`: 309 passed; `cargo test -p brindle` (architecture included): 1288 and 51 passed.
- [x] 4.4 Add the gallery's check for the spec's "The item at the top grows" scenario to `--check` if the gallery has a scroller section (`ui/README.md`, "Verify"); otherwise say in this task that the component test covers it. In `ui/README.md`'s `MessageScroller` entry and `docs/code-map.md`'s `brindle-ui` AI-elements entry, replace "items spliced in at the owner's start keep the view, or put the old first item at the top while the row showed" with the eight rules in short and `remeasure_all`. Verify both name `remeasure_all`.
  - *As run:* the gallery has a scroller section: `message-scroller-top-item-that-grows-keeps-the-view` pages up until an item's middle is at the top, replaces that item with a longer message and checks the top is the same item and offset. Under `systemd-run --user --scope scripts/with-xvfb`, it failed on the scroller before 4.3 (`(Away, 13, 42px)` to `(Away, 13, 0px)`) and passes after; the gallery's other scroller checks pass. Both docs name `remeasure_all` and the rules; the module doc lists all eight.

## 5. Reveal and give-back primitives (E-3, `brindle-ui`)

- [x] 5.1 Add tests and verify they fail first:
  - `message_scroller.rs`: pure `reveal_plan` tests (`an_item_drawn_whole_is_in_view`, `an_item_cut_at_either_edge_or_undrawn_scrolls`); component tests `revealing_an_item_above_the_view_puts_its_top_at_the_top_and_shows_the_button`, `revealing_an_item_in_view_moves_nothing`, `revealing_the_last_item_while_away_follows_the_end_again`, `revealing_while_following_reads_the_last_frame_not_the_end` (the gpui `None` case of design 4), and `the_owner_hears_the_items_in_view_after_a_frame_that_changed_them`.
  - `focus_scope.rs`: `tab_tells_the_owner_which_stop_it_focused`, for Tab and shift-Tab, and not for a focus moved by a press.
  `cargo test -p brindle-ui message_scroller focus_scope`.
  - *As run:* all failed to build first (19 errors: no `reveal_plan`, `Reveal`, `reveal`, `on_items_view` or `on_step`). These are new primitives, so no behaviour could fail before them.
- [x] 5.2 Implement `reveal_plan`, `MessageScroller::reveal` and `on_items_view` (design 4), and `FocusScope::on_step`. Verify that 5.1 and the existing `focus_scope` tests pass.
  - *As built:* `Reveal` has four cases, not two (flag 4): `InView`, `ToTop` (cut at the top, or taller than the view), `By(px)` (cut at the bottom: the least distance) and `Undrawn`. Only `Undrawn` below the view goes through gpui's `scroll_to_reveal_item`, after pinning the list at what the last frame showed; an undrawn item above goes to the top. `on_items_view` reports the owner's items with any part in view, an empty `0..0` when none, deferred from the after-draw canvas. `cargo test -p brindle-ui`: 317 passed.
- [x] 5.3 Document `reveal`, `on_items_view` and `on_step` in `ui/README.md` (the scroller's and `FocusScope`'s entries) and the code map's `brindle-ui` entry. Verify each is named once in each.
  - *As run:* `grep -c` finds `reveal(`, `on_items_view` and `on_step` once in `ui/README.md` and once in `docs/code-map.md`.

## 6. Remeasure only when a measure changed, and no fetch from a hidden zone (E-8, E-4, `src/agent/view.rs`)

- [ ] 6.1 Add tests and verify they fail first (`cargo test -p brindle agent::view`):
  - pure: `a_measure_differs_on_size_or_either_family_only`.
  - `a_reload_with_the_same_size_leaves_the_view_where_it_is`: scrolled away with an item's middle at the top, `set_profile` with the same theme family and size and a changed theme; `scroll_top()` is unchanged.
  - `a_zoom_remeasures_and_keeps_the_top_entry`: away, `Settings.zoom` raised and the window refreshed; the top item is the same, at the same fraction, and an item scrolled to from off screen has its new height.
  - `a_streaming_fence_at_the_top_keeps_its_lines`: away with a growing agent part's middle at the top, more lines fed; its top's bounds don't move.
  - `a_page_that_arrives_in_a_hidden_zone_asks_for_no_other`: the Earlier row in view, a fetch asked, the host stops drawing the view (a `SplitHost`-style host that drops it), the page replied; `take_calls` holds no second `Fetch` until the view is drawn again with the row in view. Written against the `Earlier` shape on `main` (task 1.1).
- [ ] 6.2 Add `Measure` and `AgentView::remeasure_if_changed` (design 3), called from `set_profile` and the Settings observer; `set_profile` no longer splices. Verify that 6.1 passes; the hidden-zone test passes through group 4's rule 6 alone; if it doesn't, the view's `follow_earlier` also needs a drawn frame since the last prepend, and the task says why.
- [ ] 6.3 In `docs/code-map.md`'s agent view entry, replace "`set_profile` … splices" wording with the measure rule, noting it replaces `agents-profiles` 5.3. Verify the entry names `Measure`.

## 7. A focused permission button is in view (E-3, `src/agent/view.rs`, `model.rs`)

- [ ] 7.1 Add tests and verify they fail first:
  - `model.rs`: `a_pending_permission_names_its_block`, and `None` once answered.
  - `view/tests.rs`: `tab_to_a_permission_below_the_view_reveals_it_and_enter_answers_it`: away from the end, a permission arrives off screen, tab from the prompt; the block's buttons are drawn, the first is focused, and `--key enter` gives `Call::Answer` with the reject option. `a_permission_in_view_is_not_scrolled_to`. `scrolling_a_focused_permission_out_of_view_gives_focus_to_the_prompt`: the request is still pending (no `Call::Answer`).
- [ ] 7.2 Add `ChatModel::permission_block`, pass `on_step` to the zone's `FocusScope` to reveal a permission's block, and handle `on_items_view` to give focus back (design 4). Verify that 7.1 and `tab_walks_the_prompt_the_row_the_permissions_and_stop` pass.

## 8. One press rule for the zone (E-1, `src/agent/view.rs`, `src/zone_view.rs`)

- [ ] 8.1 Add tests to `zone_view.rs`'s `mod tests`, using the `Tab` fixture with an agent zone in a `VisualTestContext`, and verify they fail first (`cargo test -p brindle zone_view::tests::a_press`); each finds its target through `debug_bounds` and presses with `simulate_mouse_down`/`simulate_click`, as `code_blocks_copy_from_their_button_and_the_palette` does:
  - `a_press_on_a_messages_text_keeps_the_prompt_focused`, then typed text reaches the draft;
  - `a_press_on_a_thoughts_label_opens_it_and_keeps_the_prompt_focused` (`reasoning-trigger`);
  - `a_press_on_a_tool_rows_line_opens_it_and_keeps_the_prompt_focused` (`tool-title`);
  - `a_press_on_a_messages_copy_button_copies_and_keeps_the_prompt_focused`;
  - `a_press_on_the_earlier_row_keeps_the_prompt_focused`;
  - `a_right_press_on_a_message_keeps_the_prompt_focused`;
  - `a_press_from_a_permission_button_to_a_message_focuses_the_prompt`;
  - `escape_after_a_press_cancels_the_turn`;
  - `a_press_on_a_permission_button_answers_it` (passes before and after);
  - `a_press_into_an_inactive_agent_zone_focuses_its_prompt`.
  Record in this task what the dump's focus line reads before the fix (`Workspace::describe_focus` on the same state: `--- focus other modality=pointer` expected) and after (`--- focus agent stop=prompt modality=pointer`). If no test fails, follow design 8a.
- [ ] 8.2 Add the root `on_any_mouse_down` to `AgentView::render` (design 1), with a comment naming the rule and why a control that takes focus is unaffected; update the comment on `render_leaf`'s capture handler. Verify that 8.1 and `cargo test -p brindle agent:: zone_view::` pass.
- [ ] 8.3 In `docs/code-map.md`'s agent view entry, add the press rule in one sentence. Add design 11's three manual checks to `CLAUDE.md`'s "Manual checks" list, after "a pointer press into an inactive agent zone focusing it" (which they extend). Verify both.

## 9. Redraw on the link gesture only (E-7, `src/agent/view.rs`)

- [ ] 9.1 Add tests and verify they fail first: pure `the_link_gesture_flips_on_ctrl_alone`; `view/tests.rs` `a_shift_press_doesnt_redraw_and_ctrl_does`, counting the view's notifies through an `observe` while `simulate_modifiers_change` sends shift, then ctrl, then nothing. `cargo test -p brindle agent::view`.
- [ ] 9.2 Store `link_gesture`, notify only on a flip (design 6). Verify that 9.1 and `markdown`'s link tests in `view/tests.rs` pass.

## 10. Integration

- [ ] 10.1 Run `cargo test --workspace --locked` and `cargo clippy --locked --workspace --all-targets -- -D warnings`; both pass.
- [ ] 10.2 Under `scripts/with-xvfb` (inside `systemd-run --user --scope`), run `BRINDLE_E2E=1 cargo test --test e2e -- agent zones` after `cargo build`; every case passes unchanged (design 5: the dump doesn't change).
- [ ] 10.3 Run the `architecture-review` skill on the branch's diff against `docs/architecture/`; fix what it finds or record why not.
- [ ] 10.4 Hand the user design 11's manual checks with `scripts/run-throwaway.sh` (and `scripts/run-throwaway.ps1` on Windows); record each result here.

## Workflow follow-up

- Commit as `fix(agents): …` with the archive commit (`docs(openspec): archive agents-zone-focus-and-scroll`) on the same PR, syncing the delta specs.
- After `gh pr create`, wait for the checks and report them; the user merges.
