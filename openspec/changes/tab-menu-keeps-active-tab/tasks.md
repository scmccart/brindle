## 1. Workspace

- [ ] 1.1 In `src/workspace.rs`, add `context_menu_tab: Option<TabId>`. `open_tab_menu_at` sets it and no longer calls `select`. The zone menu's opener and `close_overlay` clear it. Update the doc comments on `open_tab_menu_at` and `open_context_menu`.
- [ ] 1.2 Add `rename_tab_at(ix)`, `move_tab_at(ix, delta)` (no wrap; the active tab found again by id) and use the existing `close_tab_at(ix)`. Make the `RenameTab`, `MoveTabLeft` and `MoveTabRight` handlers call these with `self.active`, keeping the keys' wrap for moves.
- [ ] 1.3 Add `TabMenuItem::of(&dyn Action)`. When `context_menu_tab` is set, `context_menu_selected` resolves the tab by id, runs the matching method and returns. A tab that is gone does nothing. Zone items still dispatch.
- [ ] 1.4 Unit tests: `TabMenuItem::of` for each item (and `None` for a zone action), and `move_tab_at`'s order and active tab for moving a background tab left, moving the active tab right, and the edges. Verify with `cargo test -p brindle workspace`.

## 2. Windows e2e

- [ ] 2.1 Add `windows-tab-menu-background` to `tests/e2e/cases/win.rs`. Open two tabs with the first active, then post `WM_MOUSEMOVE`, `WM_RBUTTONDOWN` and `WM_RBUTTONUP` at the second tab's face, using the caption probe's DPI and coordinate handling. The dump must show the tab menu's `--- menu` line, the first tab active and `--- focus` on the terminal. Verify that it builds on Linux (`cargo build --tests`); it runs on CI's `windows-e2e`.

## 3. Docs and checks

- [ ] 3.1 Update CLAUDE.md:
  - the manual check line: right-click opens a background tab's menu without activating it;
  - the `picker.rs` paragraph: a right press on a tab face opens its menu without activating it, and items act on that tab.
- [ ] 3.2 Run `cargo test --workspace --locked` and `cargo build --release --locked`, and check that both pass.
