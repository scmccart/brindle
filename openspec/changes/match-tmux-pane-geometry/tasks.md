# Tasks

## 1. Subscription plumbing

- [x] 1.1 Parse `%subscription-changed <name> $<s> @<w> <idx> <%p|-> ... : <value>` in `src/tmux/protocol.rs` into `Notification::SubscriptionChanged { name, window, pane, value }`. The value is everything after the first ` : `, and the pane is `None` for `-`. Unit-test it with the lines captured from tmux 3.6 in proposal.md, a window-level line, and a value containing ` : `. Verify with `cargo test tmux::protocol`.
- [x] 1.2 After the first successful window list, send the geometry subscription once (design.md decision 4) as `Pending::Ignore`. Route `SubscriptionChanged` by name in `on_notification`, ignoring unknown names. Verify with `cargo test` and a clean `cargo build`.

## 2. Pane rectangles

- [x] 2.1 Add `Inset`, `Rect::inset` and `Rect::inset_of` to `src/tmux/layout.rs`, and per-pane insets with `TmuxSession::pane_rect` (design.md decisions 1 and 2). Unit-test both helpers: top and bottom insets, a report equal to the cell, stale reports with a different x, width or height, and an inset that doesn't fit the new cell. Verify with `cargo test tmux::layout`.
- [x] 2.2 Use `pane_rect` in `sync_panes` for terminal sizes, and expose it so `TmuxWindowView` positions and sizes pane views from it. Dividers and the active highlight keep using cells. Verify with a clean `cargo build`, and check that `--dump-screen-after` on a throwaway server without border status still shows panes at their cell sizes.
- [x] 2.3 Append the geometry fields to `PANE_STATE_FORMAT`. When the state reply arrives, record the geometry and inset and resize the terminal before the captures are replayed (design.md decision 3). Update the `restore_bytes` field-index test if it covers field counts. Verify with `cargo test tmux`.

## 3. Correcting geometry changes

- [x] 3.1 On a geometry subscription value, update the inset. If the size differs from the terminal's size, resize it and start a re-snapshot (design.md decisions 4 and 5): reset `restoring` and re-send the three restore commands. A pane already restoring is marked to be snapshotted again when its restore finishes. Verify with a clean `cargo build`.
- [x] 3.2 Make `finish_restore` prefix the replay with `ESC c` when it is a re-snapshot, not a first restore. Unit-test the bytes, either through `restore_bytes` with a flag or a wrapper. Verify with `cargo test tmux`.
- [x] 3.3 Update `CLAUDE.md`'s tmux notes: pane rectangles come from tmux's pane geometry (state query plus subscription), not the layout cells; cells drive dividers; a size change outside a layout change re-snapshots the pane. Verify the note matches the code.
- [x] 3.4 When a reported geometry doesn't fit the pane's cell, store it, mark the pane and fetch the window list again. In `sync_panes`, derive insets from stored reports that fit, and re-snapshot marked panes whose size changed (design.md decision 4). Added during implementation after finding that `rotate-window` sends no `%layout-change`. Verify with a clean `cargo build` and 4.6.

## 4. End-to-end verification

Use a throwaway server (`-L brindle-e2e -f /dev/null`) and kill it afterwards.

- [x] 4.1 Without border status: split a window, attach with `--dump-screen-after 4`, and confirm the dumped pane size equals the layout cell, as today.
- [x] 4.2 With `pane-border-status top` set before attaching: run `seq 1 60` in the pane, attach with `--dump-screen-after 4`, and confirm the dumped grid has tmux's `pane_height` rows and the same lines as `capture-pane -p`.
- [x] 4.3 Border status set while attached: attach with `--send 'clear; seq 1 60\r'` and `--dump-screen-after 6`, have a second command run `set -w pane-border-status top` about 2 s in, and confirm the dump matches `capture-pane -p` afterwards.
- [x] 4.4 Full-screen program: run `less` or `vi` on a long file in the pane, toggle border status while attached as in 4.3, and confirm the dump matches `capture-pane -p` row for row.
- [x] 4.5 Run `cargo test`, `cargo build --release` and `openspec validate match-tmux-pane-geometry --strict`.
- [x] 4.6 Rotate: with `pane-border-status top` and a vertical split, run `rotate-window` while attached, and confirm both panes match `capture-pane` (one dump per pane, selecting it first). Repeat with border status off and unequal widths (`split-window -h -l 30`).
