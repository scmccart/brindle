# Tasks

## 1. Subscription plumbing

- [ ] 1.1 Parse `%subscription-changed <name> $<s> @<w> <idx> <%p|-> ... : <value>` in `src/tmux/protocol.rs` into `Notification::SubscriptionChanged { name, window, pane, value }`. The value is everything after the first ` : `, and the pane is `None` for `-`. Unit-test it with the lines captured from tmux 3.6 in proposal.md, a window-level line, and a value containing ` : `. Verify with `cargo test tmux::protocol`.
- [ ] 1.2 After the first successful window list, send the geometry subscription once (design.md decision 4) as `Pending::Ignore`. Route `SubscriptionChanged` by name in `on_notification`, ignoring unknown names. Verify with `cargo test` and a clean `cargo build`.

## 2. Pane rectangles

- [ ] 2.1 Add `geometry` and per-pane insets to `TmuxSession`, plus a pure `pane_rect(cell, reported, inset) -> Rect` helper (design.md decisions 1 and 2). Unit-test it: no report gives the cell; a report inside the cell is used as is; a report outside the new cell falls back to the cell with the stored inset applied; a top and a bottom inset. Verify with `cargo test tmux`.
- [ ] 2.2 Use `pane_rect` in `sync_panes` for terminal sizes, and expose it so `TmuxWindowView` positions and sizes pane views from it. Dividers and the active highlight keep using cells. Verify with a clean `cargo build`, and check that `--dump-screen-after` on a throwaway server without border status still shows panes at their cell sizes.
- [ ] 2.3 Append the geometry fields to `PANE_STATE_FORMAT`. When the state reply arrives, record the geometry and inset and resize the terminal before the captures are replayed (design.md decision 3). Update the `restore_bytes` field-index test if it covers field counts. Verify with `cargo test tmux`.

## 3. Correcting geometry changes

- [ ] 3.1 On a geometry subscription value, update `geometry` and the inset. If the size differs from the terminal's size, resize it and start a re-snapshot (design.md decisions 4 and 5): reset `restoring`, re-send the three restore commands, and skip panes already restoring. Verify with a clean `cargo build`.
- [ ] 3.2 Make `finish_restore` prefix the replay with `ESC c` when it is a re-snapshot, not a first restore. Unit-test the bytes, either through `restore_bytes` with a flag or a wrapper. Verify with `cargo test tmux`.
- [ ] 3.3 Update `CLAUDE.md`'s tmux notes: pane rectangles come from tmux's pane geometry (state query plus subscription), not the layout cells; cells drive dividers; a size change outside a layout change re-snapshots the pane. Verify the note matches the code.

## 4. End-to-end verification

Use a throwaway server (`-L brindle-e2e -f /dev/null`) and kill it afterwards.

- [ ] 4.1 Without border status: split a window, attach with `--dump-screen-after 4`, and confirm the dumped pane size equals the layout cell, as today.
- [ ] 4.2 With `pane-border-status top` set before attaching: run `seq 1 60` in the pane, attach with `--dump-screen-after 4`, and confirm the dumped grid has tmux's `pane_height` rows and the same lines as `capture-pane -p`.
- [ ] 4.3 Border status set while attached: attach with `--send 'clear; seq 1 60\r'` and `--dump-screen-after 6`, have a second command run `set -w pane-border-status top` about 2 s in, and confirm the dump matches `capture-pane -p` afterwards.
- [ ] 4.4 Full-screen program: run `less` or `vi` on a long file in the pane, toggle border status while attached as in 4.3, and confirm the dump matches `capture-pane -p` row for row.
- [ ] 4.5 Run `cargo test`, `cargo build --release` and `openspec validate match-tmux-pane-geometry --strict`.
