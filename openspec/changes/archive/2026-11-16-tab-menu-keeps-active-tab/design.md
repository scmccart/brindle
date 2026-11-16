# Design

## Context

A right-press on a tab face reaches `Workspace` as `TabContextMenu { index, event }`, and `open_tab_menu_at` runs. Today it:
- calls `self.select(ix)`, which makes the tab active and clears its bell;
- focuses the (now active) tab;
- opens the `ContextMenu` with `tab_menu_rows(ix, len)`.

A chosen item reaches `context_menu_selected`, which refocuses the active tab and calls `window.dispatch_action` with the item's action (`RenameTab`, `MoveTabLeft`, `MoveTabRight`, `CloseTab`). Those handlers act on `self.active`. The switch was only there so that this dispatch hit the right tab. The zone header menu works the same way, through `ZoneTreeView` focusing the zone.

Issue #52 was filed as "right-clicking a tab drags it". The reporter has since said the actual problem is the switch. The code agrees on the drag: GPUI 0.2.2 starts a drag only after a left press (`div.rs` arms `pending_mouse_down` only for `MouseButton::Left`), so the trigger's `on_drag` can't start on a right press.

## Goals / Non-Goals

**Goals:**
- A right-click on a background tab opens its menu and changes nothing else: not the active tab, focus or the bell.
- The menu's items act on the right-clicked tab.

**Non-Goals:**
- **Zone header menus.** They still focus their zone. The zone is on screen, focusing it hides nothing, and its items (split, close) act on the focused zone.
- **A visual mark on the menu's tab while the menu is open.** Chrome, Firefox and VS Code show none: the menu opens at the pointer, on that tab.
- **Changes to the tab actions' keybindings or the command palette.** They act on the active tab, as before.

## Decisions

### 1. The menu records its tab by id, not by index

`Workspace` keeps `context_menu_tab: Option<TabId>`. `open_tab_menu_at` sets it, and the zone menu and `close_overlay` clear it. Indices can change while the menu is open: a program can exit and close a tab, or a session can be swept in. The id survives that. A tab that is gone by the time an item is chosen makes the choice do nothing.

### 2. Items run index-taking methods, not dispatched actions

`context_menu_selected` maps a tab menu item to a method on the recorded tab: `rename_tab_at(ix)`, `move_tab_at(ix, -1 | 1)` or `close_tab_at(ix)`. The mapping (`TabMenuItem`) is matched by action type, not label. The existing action handlers become thin wrappers over the same methods with `self.active`, so a key and a menu item share one implementation.

*Rejected:*
- **Activate the tab for the dispatch, then switch back:** this re-renders twice, briefly shows the other tab, and clears its bell.
- **Give the actions a tab parameter** (`MoveTabLeft { tab }`): actions are config names without arguments (`ACTIONS`), and the keymap and palette would need a "current tab" default anyway.

Zone menu items keep dispatching, unchanged.

### 3. Moving keeps the active tab by id

`move_tab_at(ix, delta)` reorders with `reorder` and finds the active tab again by id, as drag-to-reorder (`move_tab_to`) already does. Moving the active tab itself keeps it active, as before. The menu disables the moves at the ends, so the menu never wraps, unlike `move_tab`'s modular step from a key. The key path keeps its wrap.

### 4. Rename prompts for that tab

`rename_tab_at(ix)` builds the prompt from that tab's id and title. The submit closure already finds the tab by id, so it renames the right tab even if tabs move while the prompt is open.

### 5. Focus

While the menu is open, focus stays where `open_context_menu` puts it: on the active tab, which the menu gives back on close. So escape returns the user to exactly where they were.

### 6. Tests

- **Unit:** the item mapping (`TabMenuItem::of`), and `move_tab_at`'s effect on order and the active id, as pure helpers over a vector of ids.
- **Windows e2e** (`windows-tab-menu-background`): two tabs with the second active. The case posts `WM_RBUTTONDOWN` and `WM_RBUTTONUP` to Brindle's own window at the first tab's face, as the caption probes post their messages (`cases/win.rs`). The dump must then show the first tab's menu in its `--- menu` line, with the second tab still active. The first tab is the one clicked because its left edge is fixed, while the second's place depends on the first's title: on CI's elevated runner the console's title makes the first tab wider.
- **Linux:** pointer presses can't be posted, so the right-click stays a manual check (CLAUDE.md's list, updated).

## Risks / Trade-offs

- [The user may not notice which tab a menu belongs to] → It opens at the pointer on that tab, as in browsers. Accepted.
- [The Windows e2e case is advisory on CI] → It is backed by the unit tests and the manual check.
