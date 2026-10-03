# Tasks

## 1. Window list carries the active pane

- [ ] 1.1 Add `#{pane_id}` to the `list_windows` format (design.md §1). Move line parsing into a pure `parse_window_line`, with the pane id optional, and use it in `on_window_list`. Unit-test it in `src/tmux/mod.rs`: a full line, a name containing tabs, and a line whose pane field is empty. Verify with `cargo test tmux`.
- [ ] 1.2 In `on_window_list`, set `active_pane` from the parsed line only when it is `None` (design.md §2). Verify with `cargo test` and a clean `cargo build`.

## 2. No selections written back on attach

- [ ] 2.1 Change `TmuxSession::select_pane` as in design.md §3: no-op when the pane is already active, adopt locally without sending when the active pane is unknown, and send as today otherwise. If the decision is factored into a pure helper, unit-test all three cases. Verify with `cargo test tmux`.
- [ ] 2.2 In `on_window_list`, record the reported current window before emitting `WindowClosed` and `WindowAdded`, and emit `WindowActivated` afterwards only if it changed (design.md §4). Verify with `cargo test` and a clean `cargo build`.
- [ ] 2.3 Add `Workspace::insert_tab_inactive`, which shifts `self.active` when the insertion is at or before it, and use it for `TmuxEvent::WindowAdded` (design.md §5). Verify with a clean `cargo build`. Behaviour is checked in group 3.
- [ ] 2.4 Update `CLAUDE.md`'s tmux notes: the window list carries each window's active pane, and tabs follow tmux's current window and never select it on attach. Verify the note matches the code.

## 3. End-to-end verification

Warn the user and wait for their go-ahead before launching Brindle windows. Use a throwaway server (`-L brindle-e2e -f /dev/null`) and kill it afterwards.

- [ ] 3.1 Prepare a session from the tmux CLI:
  - three windows, with window 2 current and window 0 as the last window;
  - window 2 split into three panes, with the second pane selected.

  Record `list-windows -F '#{window_index} #{window_active} #{window_last_flag}'` and `list-panes -a -F '#{pane_id} #{pane_active}'`. Attach with `--dump-screen-after 4`, then confirm that both listings are unchanged and that the dump shows window 2's tab as active.
- [ ] 3.2 On the same session, attach with `--action tmux_split_right --dump-screen-after 5`, and confirm with `list-panes` that the pane split was the one tmux had active.
- [ ] 3.3 With Brindle attached (`--dump-screen-after 6`), have a second command run `tmux -L brindle-e2e new-window -d` after about 2 s. Confirm that the dump lists the new tab but not as active, and that tmux's current window is unchanged. Then confirm `--action tmux_new_window --dump-screen-after 4` shows the new tab active.
- [ ] 3.4 Run `cargo test`, `cargo build --release` and `openspec validate restore-active-pane-on-attach --strict`.
