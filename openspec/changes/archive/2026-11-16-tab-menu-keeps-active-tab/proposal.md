# Proposal

## Why

Right-clicking a background tab switches to it before opening its menu (GitHub issue #52). That loses the user's place: they only wanted the tab's menu, and dismissing the menu leaves them on another tab. Browsers and editors open a background tab's menu without switching (Chrome, Firefox, VS Code), and the tab menu was specified to activate the tab only so that its items, which run the tab actions, would act on it.

## What Changes

- A right-click on a tab opens that tab's menu without activating it. The active tab, its focus and its bell state stay as they were.
- The menu's items act on the right-clicked tab, not the active one:
  - Rename Tab renames it;
  - Move Tab Left and Move Tab Right move it, and the active tab stays active wherever it ends up;
  - Close Tab closes it.
- Choosing an item on a background tab doesn't activate it either. Rename's prompt names that tab's title.
- `open_tab_menu` (keyboard) still opens the active tab's menu, and the keybindings for the tab actions still act on the active tab.
- Zone header menus are unchanged: right-clicking a zone's header still focuses that zone. Zones are in the visible tab, so focusing one doesn't hide anything.
- Issue #52 was filed as "right-clicking a tab drags it". The reporter has since said the problem is the switch, and this change fixes that.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tabs`: "Tab context menu" no longer activates the right-clicked tab, and its items act on that tab.

## Impact

- `src/workspace.rs`:
  - `open_tab_menu_at` no longer calls `select`.
  - The context menu remembers the tab it was opened for, by tab id.
  - `context_menu_selected` runs the tab items on that tab, through index-taking methods (`rename_tab_at`, `move_tab_at`, `close_tab_at`), instead of dispatching the action to the active tab.
- CLAUDE.md's manual-check line for the tab menu.
- No protocol, config or keymap change.
