# Proposal

## Why

Brindle keeps 10,000 lines of scrollback per terminal, and in control mode each tmux pane has its own, but the only way to find something in it is to scroll and read. Every mainstream terminal has a find bar. Control-mode panes need one most, because tmux's copy-mode search isn't reachable there: the prefix doesn't work.

## What Changes

- Add a **find bar** for the focused terminal, opened with ctrl-shift-f or "Find" in the command palette. It works the same in local tabs and in control-mode panes, and searches only that terminal's own scrollback and screen.
- Search runs as you type. Every match on screen is highlighted, and the current match is highlighted differently and scrolled into view.
- Enter moves to the next match up (older output) and shift-enter to the next match down, wrapping at either end. The bar shows the current match's number and the match count.
- Toggles for **match case** (alt-c) and **regular expression** (alt-r), off by default. An invalid regular expression is reported in the bar.
- Escape closes the bar, removes the highlights, and puts the view back where it was when the bar opened. Clicking the terminal keeps the bar open.
- New bindable actions: `find`, `find_next`, `find_previous`, `find_close`, `find_toggle_case`, `find_toggle_regex`.
- New theme colors for match highlights: `search_match_background`, `search_match_foreground`, `search_focused_match_background`, `search_focused_match_foreground`. A theme that leaves them out gets colors derived from its own palette. The settings dialog's theme editor shows them, like the other colors.
- A `--find <TEXT>` debug step and a find-bar line in `--dump-screen-after` output, so the end-to-end suite can check search without synthetic input.

Out of scope: searching every pane or tab at once, a search history, whole-word matching, and text-input method (IME) composition in the find bar.

## Capabilities

### New Capabilities
- `scrollback-search`: the find bar: opening and closing it, incremental search, match highlighting, navigation, case and regex options, match counts, and how it applies to local terminals and control-mode panes.

### Modified Capabilities
- `terminal-rendering`: search match highlights are drawn, and how they combine with selection.
- `configuration`: themes gain four search highlight colors, with defaults derived from the theme.
- `settings-dialog`: the theme editor shows and saves the search highlight colors.
- `command-line`: the debug hooks gain a `--find` step, and the screen dump reports the find bar's state.

## Impact

- **Code:**
  - new `src/terminal/search.rs`: pattern building, finding, visible matches, counting and scrolling.
  - `src/terminal_view.rs`: find bar state, rendering and actions.
  - `src/terminal_element.rs`: highlights.
  - `src/actions.rs`: new actions and default bindings.
  - `src/theme.rs`, `src/config_edit.rs`, `src/settings_dialog.rs`: theme colors.
  - `src/workspace.rs`, `src/main.rs`: debug hook and dump.
- **Config:** new optional theme keys and action names. Existing configs keep working unchanged.
- **Tests:** unit tests for search, and e2e cases for a local tab and a control-mode pane.
- **Docs:** README keybinding tables, the commented default config.
- **Dependencies:** none. alacritty_terminal's regex search is already available.
