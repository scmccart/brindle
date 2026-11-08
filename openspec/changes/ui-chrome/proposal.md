# Proposal

> **Skeleton.** This holds the context from the exploration that produced `add-brindle-ui`. Flesh it out (specs, design, tasks) before implementing. **Depends on:** `add-brindle-ui`, `ui-tokens-from-theme`. It is the last change in the migration sequence.

## Why

After the settings dialog and the palettes move, the remaining hand-styled chrome is the window's own:
- the find bar (`find_bar.rs`);
- the tab strip, which is the client-side-decorated title bar (`workspace.rs`);
- zone headers with their split and close buttons (`zone_view.rs`);
- tmux pane title lines (`tmux_view.rs`).

Moving them to `brindle-ui` finishes the migration and lets `theme.rs`'s chrome derivations go.

## What Changes

- Add the components these surfaces need, which `add-brindle-ui` deliberately left out:
  - **Tabs**, for the tab strip's look and keyboard model;
  - **Tooltip**, building on GPUI's `tooltip`/`hoverable_tooltip`, plus delay groups if wanted;
  - **DropdownMenu**, for the `⌄` new-tab button, and possibly tab and zone context menus;
  - **Toggle**, for the find bar's option toggles;
  - **Kbd**, for shortcuts in menus and tooltips;
  - **Separator**.
- Possibly a Presence primitive for exit animations, if menus want them.
- Move the find bar to `Input` + `Toggle` + `Button`(ghost, icon), and zone headers and the tab strip to `Button`(ghost, icon), Tabs and Tooltip.
- Remove `theme.rs`'s chrome derivations once nothing uses them.

## Capabilities

### New Capabilities
- None expected. The new components extend `ui-components` (MODIFIED/ADDED requirements there).

### Modified Capabilities
- `ui-components`: adds Tabs, Tooltip, DropdownMenu, Toggle, Kbd and Separator behaviour.
- `tabs`, `zones`, `scrollback-search` (find bar): only where visible behaviour changes, for example new tooltips or menus. Confirm when fleshing out.

## Impact

- `src/find_bar.rs`, `src/workspace.rs` (the tab strip), `src/zone_view.rs`, `src/tmux_view.rs`, `src/theme.rs`, `ui/src/components/*`.

## Notes carried from exploration

- **Windows caption hit-testing.** The tab strip is the CSD title bar. On Windows a caption press works only when GPUI leaves it unhandled, and GPUI's focus transfer counts as handling it (CLAUDE.md, Windows pitfalls). Tabs and buttons in the strip must not take focus on press where that would break dragging, or the caption buttons. `Button` without a `.focus(&handle)` already doesn't move focus.
- Performance: GPUI 0.2.2 re-renders the whole window on any `notify`. Chrome components must stay allocation-light in `render`, as CLAUDE.md asks.
- Manual checks that stay manual: dragging tabs to reorder, and the palette `⌄` toggle.
