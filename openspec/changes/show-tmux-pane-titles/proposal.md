# Proposal

## Why

With `pane-border-status` on, tmux shows a title line for each pane, built from the pane's `pane-border-format`. Its default is the pane index and `#{pane_title}`. Claude Code's tmux teammate mode relies on these lines to tell agents apart: it names each teammate's pane with `select-pane -T @<agent>` and gives it a colored format, `#[fg=<color>,bold] #{pane_title} #[default]`. tmux draws title lines only for terminal clients, so a control client gets nothing. Once `match-tmux-pane-geometry` reserves the rows, Brindle shows them blank.

Tested on tmux 3.6: a subscription on `#{T:pane-border-format}` for all panes (`%*`) delivers each pane's expanded title with its style directives, for example `#[fg=blue,bold] @researcher #[default]`. It updates within about 1 s when the title or format changes. `pane-border-format` can be set per pane.

## What Changes

- When a window has `pane-border-status` set to `top` or `bottom`, Brindle draws each pane's title line in the row tmux reserves for it, on the side tmux puts it.
- The text comes from tmux expanding the pane's `pane-border-format`, so custom formats work as well as the default.
- Brindle renders the style directives commonly used in border formats: `#[fg=...]`, `#[bg=...]`, `bold`, `default` and `reverse`. Colors can be tmux color names, `colourN` or `#rrggbb`. Other directives are dropped rather than shown as text.
- Titles update when tmux reports a change.
- With `pane-border-status off`, which is the default, nothing changes.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: a new requirement that pane title lines tmux would show (`pane-border-status` with `pane-border-format`) are drawn in Brindle, with their colors and bold.

## Impact

- **Code:**
  - `src/tmux/mod.rs`: subscribe to `#{T:pane-border-format}` and `#{pane-border-status}`, and store per-pane title runs.
  - A small parser for tmux `#[...]` style runs and color names, with unit tests.
  - `src/tmux_view.rs`: paint title text in the reserved rows, clipped to the pane width, using the terminal font and cell grid.
- **Depends on:** `match-tmux-pane-geometry`, which supplies the reserved rows and the subscription plumbing.
- **Tests:**
  - Unit tests for the style parser: Claude Code's format, the default format, unknown directives, and the three color forms.
  - An end-to-end check: set titles and formats on a throwaway server, then confirm the dump or a window capture shows them.
- **Out of scope:** making title lines clickable or draggable, and range directives (`#[range=...]`). Brindle drops these.
