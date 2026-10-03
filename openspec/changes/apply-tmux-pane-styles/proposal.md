# Proposal

## Why

tmux lets programs and users style panes. `window-style` and `window-active-style` set a pane's default foreground and background, and `pane-border-style` and `pane-active-border-style` color the dividers. In control mode tmux sends pane output as raw bytes and applies none of these, so Brindle shows every pane in plain theme colors with theme-colored dividers. Claude Code's tmux teammate mode uses them to color-code agents: it sets each teammate pane's `window-style` to `fg=<agent color>` and sets the border styles to that color. A user's own tmux configuration may set them too.

Tested on tmux 3.6:
- `window-style` and `window-active-style` can be set per pane.
- `pane-border-style` and `pane-active-border-style` are window options. `set -p` on them changes the whole window, so tmux itself shows one border color per window, the last one set.
- Format subscriptions report all four values (`#{window-style}` and similar).

## What Changes

- A pane's default foreground and background follow `window-style`, or `window-active-style` while it is the active pane, when those set a color. With `default`, the theme's colors are used as today. Explicit colors from the program are unaffected, matching tmux.
- Dividers use the window's `pane-border-style` color, and the active pane's highlight uses `pane-active-border-style`, when those set a foreground color. Otherwise the theme's divider and accent colors are used as today.
- Programs that query the default colors (OSC 10/11) get the pane's effective colors, so a styled pane answers with its style.
- tmux's built-in border style values (such as the default green active border) count as unset, so Brindle keeps its theme colors unless a style is actually configured.
- Styles update when tmux reports a change, through the subscription plumbing from `match-tmux-pane-geometry`.
- Colors can be tmux color names, `colourN` or `#rrggbb`, sharing the parser from `show-tmux-pane-titles`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: a new requirement that tmux pane styles (`window-style`, `window-active-style`, `pane-border-style` and `pane-active-border-style`) are honored for pane default colors and divider colors.

## Impact

- **Code:**
  - `src/tmux/mod.rs`: subscribe to the four style options.
  - `src/terminal/mod.rs` or `src/terminal_element.rs`: a per-terminal override of the default foreground and background, used by `default_color` for remote terminals. The OSC 10/11 reports from `report-pane-colors-to-tmux` carry each pane's effective colors, because tmux checks a control client's report before `window-style` (design.md decision 4).
  - `src/tmux_view.rs`: divider colors from the window's styles.
- **Depends on:** `match-tmux-pane-geometry` for subscriptions, and `show-tmux-pane-titles` for the color parser. If this change lands first, it brings the parser itself.
- **Tests:**
  - Unit tests for resolving a style string to optional foreground and background colors.
  - An end-to-end check: set `window-style fg=blue` on one pane of a throwaway server, and confirm that pane's default text renders blue while the other pane's doesn't. Use a window capture, because the dump is text only.
- **Out of scope:** style attributes other than colors, such as bold and dim set through `window-style`, and `pane-border-lines` styles. Dividers stay Brindle's single-pixel lines.
