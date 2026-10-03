# Tasks

## 1. Action labels and new actions

- [x] 1.1 Add `label: Option<&str>` to `ACTIONS` in `src/actions.rs` and label every user-facing action. tmux labels start with `tmux: `; `none` gets no label. Extend the `actions` unit tests to assert that labels are unique and every labelled name resolves, and verify with `cargo test actions`.
- [x] 1.2 Add the new unit actions to `ACTIONS` with labels and no default keys:
  - `OpenCommandPalette` (`open_command_palette`, the only one of these with no label);
  - `TmuxNewWindow`;
  - the four `TmuxLayout*` actions;
  - `TmuxNextLayout`, `TmuxRotatePanes`, `TmuxSwapPanePrev` and `TmuxSwapPaneNext`;
  - `TmuxBreakPane`, `TmuxRenameWindow` and `TmuxCommand`.

  Also add the hidden `Swallow` action, which stays out of `ACTIONS`. Verify that `brindle --list-actions` prints the new names and not `swallow`.
- [x] 1.3 Change the default bindings:
  - ctrl-shift-p → `OpenCommandPalette` (Workspace);
  - ctrl-shift-space → `OpenProfilePicker` (Workspace);
  - ctrl-shift-d → `TmuxDetach` in `TmuxWindow`, plus ctrl-shift-d → `Swallow` in Workspace;
  - the `ProfilePicker` context renamed to `Palette`, with ctrl-shift-v → paste bound there.

  Verify that `cargo test` passes.

## 2. Shared palette widget

- [x] 2.1 Generalize `src/picker.rs` into `Palette`. List mode takes rows (label, detail, shortcut) and emits `Confirmed(row)` / `Dismissed`, using the existing `fuzzy_match`. Keep the profile-row rendering (name, description, ctrl-alt-N hint) by building the rows in the caller. Factor the filtering into a pure function with unit tests: order preserved, empty query matches everything, and no-match state. Verify with `cargo test picker`.
- [x] 2.2 Add prompt mode to `Palette`:
  - label, pre-filled value and single-line editing;
  - paste with line breaks replaced by spaces;
  - enter calls the submit closure and shows a busy state until its future resolves;
  - `Ok` closes the palette, `Err(msg)` shows the message and keeps the text, a cancelled future closes it, and escape closes it at any time.

  Unit-test the line-break sanitising, and verify with `cargo test picker`.
- [x] 2.3 In `src/workspace.rs`, replace `picker` with a single `palette: Option<(Entity<Palette>, PaletteKind, Subscription)>`. Opening a kind toggles it if it is already open, and replaces the other kind otherwise. Focus returns to the active tab on dismiss. The new-tab palette (the `⌄` button and `OpenProfilePicker`) builds profile rows and launches on confirm, as before. Verify with a Brindle run using `--action open_profile_picker --dump-screen-after 3` (no crash), then ask the user to check the toggle and the switch between the two palettes by hand.

## 3. Command palette

- [x] 3.1 Implement `Workspace::open_command_palette`. Before moving focus, it:
  1. builds the labelled `ACTIONS` entries;
  2. keeps those that are available, using both `window.is_action_available` and the `TypeId`s from `window.available_actions(cx)`, as in design.md §3;
  3. resolves each entry's shortcut with `highest_precedence_binding_for_action_in` against the tab's focus handle;
  4. stores the rows together with the tab's `FocusHandle`.

  Factor the filtering step (available `TypeId`s plus the table, giving rows) into a pure function with a unit test: tmux actions are dropped when unavailable, and unlabelled actions never appear, a global-only action is kept, and `last_tab` (`no_json`) is kept when its type is available.
- [x] 3.2 On `Confirmed`, drop the palette, focus the stored tab handle, and `window.dispatch_action(action.boxed_clone(), cx)`. Verify with a Brindle run (`--action open_command_palette --dump-screen-after 3`, no crash), then ask the user to check by hand that "Clear Scrollback" in a local tab and "tmux: Split Right" in a control-mode tab both behave like their shortcuts.
- [x] 3.3 Update the README keybinding tables: ctrl-shift-p is the command palette, ctrl-shift-space the new-tab palette. Mark the ctrl-shift-p change as breaking, and mention that the `⌄` button still opens profiles. Verify that the README matches `default_bindings()`.

## 4. Detach scoped to control-mode tabs

- [x] 4.1 Move the `TmuxDetach` handler from `Workspace` to `TmuxWindowView`, and add a no-op `Swallow` handler on `Workspace`. Verify on a throwaway tmux server (`-L brindle-e2e`) that `--action tmux_detach` still detaches and `tmux -L brindle-e2e ls` still lists the session. Kill the server afterwards. The ctrl-shift-d keystroke guard on local tabs can't be exercised without synthetic input, so the hand check in 7.1 covers it.
- [x] 4.2 Update `CLAUDE.md`'s architecture notes: the `Palette` key context replaces `ProfilePicker`, detach lives in the TmuxWindow context, and the palettes are described.

## 5. tmux arrangement commands

- [x] 5.1 Add the `TmuxSession` methods `select_layout(window, name)`, `next_layout(window)`, `rotate(window)`, `swap_pane(window, up)`, `break_pane(window)` and `rename_window(window, name)`, using the mapping in design.md §7. `rename_window` ignores empty names and quotes with `protocol::quote`. Add unit tests for any pure command-string builders, and verify with `cargo test tmux`.
- [x] 5.2 Wire the new actions in `TmuxWindowView::render` through `with_session`. `TmuxNewWindow` reuses `new_window()`. Verify against tmux 3.6 on a throwaway server: split twice, then run `--action tmux_layout_tiled`, `tmux_rotate_panes` and `tmux_swap_pane_next`, and confirm the layout with `tmux -L brindle-e2e list-panes -F '#{pane_id} #{pane_active} #{pane_left},#{pane_top}'`. In particular, confirm that after a swap the active pane is still the one that moved. Kill the server afterwards.
- [x] 5.3 Verify `--action tmux_break_pane` on a two-pane window: `--dump-screen-after` shows a new active tab, and the original window has one pane left in `list-panes -a`. Kill the server afterwards.

## 6. Prompted tmux commands

- [x] 6.1 Add a `PromptRequested { label, initial, submit }` event on `TmuxWindowView`. Subscribe to it in `Workspace` wherever tmux views are created, and open the palette in prompt mode. Implement `TmuxRenameWindow`, pre-filled from `session.window(w).name`, submitting through `rename_window` and resolving to `Ok`. Ask the user to check by hand that the palette path and a keybinding path (a temporary user binding) both open the same pre-filled prompt, and that the tab title updates.
- [x] 6.2 Implement `TmuxSession::run_user_command` with `Pending::UserCommand { token, first_error, reply }` and the sentinel `display-message -p brindle-sync-<n>`, both written under one `io` borrow, following design.md §6. Put the per-block reply handling in a pure helper and unit-test it in `src/tmux/mod.rs` tests:
  - a single `%end` followed by the sentinel gives `Ok`;
  - `%error` followed by the sentinel gives `Err` with the message;
  - two `%end`s followed by the sentinel gives `Ok`;
  - the next queued `Pending` is still matched correctly afterwards.

  Verify with `cargo test tmux`.
- [x] 6.3 Implement the pure `with_start_dir(line)` rewrite from design.md §6a in `src/tmux/protocol.rs`, and call it from `run_user_command` before framing. Unit-test that it:
  - inserts after `new-window`, `neww`, `split-window` and `splitw`;
  - adjusts every top-level command in `neww ; splitw -v` and in `neww; splitw`;
  - leaves commands in `{ … }` blocks and inside quotes untouched;
  - passes through `rename-window 'a;b'` unchanged;
  - leaves lines with no matching command byte-identical.

  Verify with `cargo test tmux::protocol`.
- [x] 6.4 Implement `TmuxCommand`: an empty prompt, submitting through `run_user_command`, and closing the palette on `Ok`, on cancellation, or when the session ends. Verify against a throwaway tmux 3.6 server, using a temporary debug path or unit harness that calls `run_user_command` directly, that:
  - `rename-window a ; split-window` renames and splits, and a following `--action tmux_split_right` still renders correctly in `--dump-screen-after`;
  - `bogus-command` returns tmux's error text;
  - an `if-shell true 'split-window'` line resolves;
  - from a pane `cd`'d into a temporary directory, `split-window -h` starts its pane there;
  - `new-window -c /tmp` starts in `/tmp`, which confirms that the last `-c` wins. If it doesn't, switch to the skip-if-present fallback in design.md §6a and update its unit tests.

  Check the directories with `tmux -L brindle-e2e list-panes -a -F '#{pane_id} #{pane_current_path}'`.

  Record the observed block counts in design.md if they differ from the assumption. Kill the server afterwards.

## 7. Integration check

- [x] 7.1 Run `cargo test`, `cargo build --release` and `openspec validate add-command-palette --strict`, and ask the user for one interactive pass:
  - ctrl-shift-p on a local tab and on a control-mode tab, comparing the lists, the shortcuts shown and a user override;
  - the run-command error-and-retry flow;
  - ctrl-shift-space and `⌄`;
  - ctrl-shift-d in a local tab doing nothing.
