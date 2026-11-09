# Proposal

**Depends on:** `add-brindle-ui`, `ui-tokens-from-theme`, `ui-settings-dialog`, `ui-command-palette` (all archived). This is the last change in the migration sequence.

## Why

After the settings dialog and the palettes moved, the window's own chrome is the last hand-styled UI:
- the find bar (`find_bar.rs`);
- the tab strip, which is the client-side-decorated title bar (`workspace.rs`), and the drag preview of a tab;
- zone headers with their split and close buttons (`zone_view.rs`), and the zone dividers;
- the window border and the tmux "zoomed" badge.

Each draws its colours from `theme.rs`'s chrome derivations (`chrome_background`, `chrome_border`, `muted_foreground`, `hover_background`) and its own hover and active styling. Moving them onto `brindle-ui` finishes the migration, gives them one look with the rest of the interface, and lets those derivations become private inputs to `ui_tokens()`. Two small additions come with it: tooltips that name each icon-only control with its shortcut, and right-click menus on tabs and zone headers.

## What Changes

- Add components to `brindle-ui`, modelled on shadcn/ui and Radix:
  - **Tooltip**, building on GPUI's `tooltip`, with an optional `Kbd` for the shortcut;
  - **Toggle**, for the find bar's option buttons (pressed/unpressed, as shadcn's Toggle);
  - **Kbd**, a keyboard shortcut drawn as keycaps (tooltips and menu items);
  - **Separator**, a horizontal or vertical rule;
  - **Tabs**, shadcn's tab list and triggers (Radix keys: roving focus, arrows, home/end, automatic activation), with the additions the strip needs (closable and reorderable triggers, middle-click close, status adornments), each listed as a difference;
  - **DropdownMenu** (opened from a trigger) and **ContextMenu** (right-click, placed at the pointer), built on shared menu pieces: items with an optional shortcut, disabled items, separators, and Radix's keys.
- Add two `ButtonSize`s, shadcn's `icon-sm` and `icon-xs`, because the strip and the headers are shorter than a 36px icon button.
- Add icons the surfaces need (plus, minus, arrows, split columns and rows) to `Icon`.
- Move the find bar to `Input` + `Toggle` + ghost icon `Button` + `Separator`, in a popover-styled panel.
- Move the tab strip to `Tabs` (an `attached` variant of our own, so the active tab still joins the terminal in the tab's colours), with ghost icon `Button`s, `Tooltip` and token colours, and the zone headers to ghost icon `Button`, `Tooltip` and token colours. The rest of the strip follows the global theme tokens.
- Menu items may carry a description line, so the profile menu keeps what the palette shows.
- The strip's `⌄` button opens a **profile dropdown menu** (one item per profile, with its description and ctrl-alt-N shortcut) instead of the new-tab palette. ctrl-shift-space and `open_profile_picker` still open the palette.
- Make the strip a keyboard-reachable tab list: a hidden `focus_tab_strip` action puts focus on the active tab, where arrows, home and end move and activate tabs; a pointer press still activates a tab and leaves focus in its terminal.
- Give tabs and zone headers a right-click menu built on `ContextMenu`, whose items are existing actions.
- Colours of the strip, headers, dividers, window border and badge come from the `UiTheme` tokens. The tmux pane title lines keep their canvas painting (they are terminal rows drawn from tmux's formats) and change only where their fallback colours come from.
- Make `theme.rs`'s chrome helpers private to `ui_tokens()` and delete what has no user left.

Out of scope, with reasons:
- Replacing the new-tab palette itself: it stays on ctrl-shift-space, `open_profile_picker` and the "session shown elsewhere" flow.
- Tab panels (`TabsContent`): the tab's content is the workspace below the strip, owned by the host.
- "Close other tabs" and any menu item that has no existing action.
- Exit animations, UI zoom (the chrome stays fixed-size; terminal zoom does not touch `rem_size`, as decided in `ui-tokens-from-theme`), per-tab chrome themes.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `ui-components`: adds Tabs, DropdownMenu, ContextMenu, Tooltip, Toggle, Kbd and Separator behaviour, and the two icon sizes.
- `ui-theme-tokens`: the window chrome (strip, headers, dividers, border, find bar) takes its colours from the tokens, with no per-tab exception.
- `tabs`: the strip is a tab list with keyboard navigation; "Title bar behaviour" gains right-click on a tab; a tab menu; tooltips on the strip's buttons.
- `profiles`: the strip's `⌄` opens a profile dropdown menu; the palette stays on its shortcut.
- `zones`: a zone header menu; tooltips on the header buttons.
- `scrollback-search`: the find bar's controls show tooltips with their shortcuts, and its option toggles show their state.

## Impact

- `src/find_bar.rs`, `src/workspace.rs` (strip, drag preview, window border, menu slot), `src/zone_view.rs`, `src/tmux_view.rs` (fallback colours, zoomed badge), `src/theme.rs`, `src/actions.rs` (four hidden actions, see design), `src/picker.rs` (profile rows shared with the menu).
- `ui/src/components/*`, `ui/src/style.rs`, `ui/src/icon.rs`, `ui/examples/gallery.rs`, `ui/README.md`.
- `CLAUDE.md` (architecture and manual checks), `README.md` (the tab menu, the profile menu, `focus_tab_strip`), the e2e suite (`tests/e2e/cases`).

## Notes carried from exploration

- **Windows caption hit-testing.** On Windows a caption press works only when GPUI leaves it unhandled, and GPUI's focus transfer counts as handling it (CLAUDE.md, Windows pitfalls). Strip controls must not take focus on press where that would break dragging or the caption buttons. `Button` without `.focus(&handle)` already doesn't move focus. Radix focuses a tab trigger on press; Brindle's strip does not (a press activates the tab and focus goes to its terminal), listed as a difference. The native caption buttons (minimize, maximize, close) keep their hand-built element with no handlers on Windows; they take the ghost button's colours from `button_style`.
- **Performance.** GPUI 0.2.2 re-renders the whole window on any `notify`. Chrome components stay allocation-light in `render`, as CLAUDE.md asks: tooltips and menus build their content on demand, not per frame.
- **Manual checks that stay manual:** dragging tabs to reorder, the `⌄` profile menu, ctrl-shift-space still toggling the palette, right-click opening a menu and a hover showing a tooltip (a pointer can't be posted on Linux).
