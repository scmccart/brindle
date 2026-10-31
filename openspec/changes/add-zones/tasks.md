# Tasks

## 1. Zone tree model and geometry

- [ ] 1.1 Create `src/zone_view.rs` (and declare it in `src/main.rs`) with `ZoneId`, `SplitAxis`, `ZoneTree`, and `UnitRect`, implementing `split`, `remove` (returning promoted sibling leaves), and `rects`; verify with `cargo test zone_view` unit tests for nested splits, sibling promotion, and normalized rect computation.
- [ ] 1.2 Implement `ZoneTree::focus_target` over normalized `[0, 1] x [0, 1]` rectangles (`Left`, `Right`, `Up`, `Down`) with overlap filtering, primary-axis distance, perpendicular-center distance, and tree-order tie-breaking; verify with unit tests covering side-by-side moves, moving into a further-split half, and no-op moves at outer edges.
- [ ] 1.3 Implement the minimum split size check from zone pixel bounds, `CellMetrics`, `padding`, and `ZONE_HEADER_HEIGHT`, and the `describe_zones()` formatter (`--- zones count=N active=ID` and `--- zone id=ID rect=X,Y,W,H[ active]`); verify with unit tests for minimum size rejection and dump line formatting.

## 2. Actions and keybindings

- [ ] 2.1 Add `SplitZoneRight`, `SplitZoneDown`, `CloseZone`, `FocusZoneLeft`, `FocusZoneRight`, `FocusZoneUp`, and `FocusZoneDown` to `src/actions.rs` with config names (`split_zone_right`, `split_zone_down`, `close_zone`, `focus_zone_left`, `focus_zone_right`, `focus_zone_up`, `focus_zone_down`), command palette labels, and default bindings (`ctrl-shift-e`, `ctrl-shift-o`, `ctrl-shift-x`, `alt-left/right/up/down`) in the `ZoneTree` key context; verify with `cargo test actions`.

## 3. ZoneTreeView entity and chrome

- [ ] 3.1 Implement `ZoneContent::Terminal(Entity<TerminalView>)`, `Zone`, and `ZoneTreeView` in `src/zone_view.rs`, handling terminal spawning with the tab's profile and active zone's working directory, focus tracking (`on_focus_in` and `last_focused` serials), `on_modifiers_changed` forwarding, and `TerminalEvent` aggregation (`Exited`, `Bell`, `Notify`, `Status`, `TitleChanged`).
- [ ] 3.2 Implement `Render` for `ZoneTreeView`: render a single leaf directly with no header bar or divider, and render multi-zone trees with 1px dividers and native GPUI zone header bars (status dot, automatic title, active styling, and Split Right, Split Down, and Close Zone buttons).
- [ ] 3.3 Wire `SplitZoneRight`, `SplitZoneDown`, `CloseZone`, and `FocusZoneLeft/Right/Up/Down` action listeners on `ZoneTreeView` so splitting focuses the new zone and closing a zone focuses the most recently focused leaf in the promoted sibling subtree (or emits exit when the last zone closes).

## 4. Workspace and debug hook integration

- [ ] 4.1 Replace `TabContent::Terminal(Entity<TerminalView>)` with `TabContent::Zones(Entity<ZoneTreeView>)` in `src/workspace.rs`, routing `Tab::focus_handle`, `active_terminal`, `active_view`, `automatic_title`, `status_summary`, `set_tab_theme`, `launch`, `open_message_tab`, `remove_tab`, and `1s` title refreshes through `ZoneTreeView` (and updating the tab title immediately when the active zone changes).
- [ ] 4.2 Update `Workspace::dump_active_screen` in `src/workspace.rs` to include `ZoneTreeView::describe_zones()` when the active local tab has two or more zones, keeping single-zone dumps unchanged; update `tests/e2e/` dump parsing if needed and verify `cargo test --test e2e -- --self-test`.

## 5. Session restore

- [ ] 5.1 Extend `TabState::Local`, `SnapshotTab::Local`, and `RestoredTab::Local` in `src/session.rs` with optional zone tree state (`zones`), keeping top-level `cwd` as the active zone's working directory and omitting `zones` for single-zone tabs; verify with `cargo test session` unit tests for 1-zone, 2-zone, and nested 3-zone round-tripping, missing directory fallback per zone, and backward compatibility with existing `VERSION = 1` files.
- [ ] 5.2 Update `Workspace::snapshot` and `Workspace::restore` in `src/workspace.rs` to record and reconstruct a tab's `ZoneTreeView` split tree, per-zone working directories, and active zone focus.

## 6. End-to-end verification and documentation

- [ ] 6.1 Add e2e cases in `tests/e2e/` covering splitting right and down, working directory inheritance, directional focus (`focus_zone_*`), closing a zone via `close_zone` and `exit` vs closing the tab, command palette filtering in local vs tmux tabs, and session restore of a split tab; verify `cargo test`.
- [ ] 6.2 Update `README.md` and `CLAUDE.md` with the zone actions, shortcuts, architecture notes, and `--dump-screen-after` zone lines.
