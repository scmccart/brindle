# Proposal

## Why

Brindle sizes and positions each tmux pane from the window's layout string. When `pane-border-status` is `top` or `bottom`, tmux takes a row from some panes for a title line, and neither the layout string nor any `%layout-change` reflects it. Brindle's grid then ends up one row taller than the real pane, and sits one row off. Programs scroll at a different row than Brindle does, so full-screen programs drift and garble. Claude Code's tmux teammate mode turns `pane-border-status top` on when it spawns teammates, so this breaks the main use of agent teams in a Brindle tmux tab.

Tested on tmux 3.6 with a throwaway server:
- With `top`, panes touching the window's top edge get `pane_top` 1 and lose one row. Other panes keep their size, and their title sits in the divider row above them. `bottom` does the same at the bottom edge. Zoomed and single-pane windows lose the row too.
- After `seq 1 60` in a pane, Brindle showed lines 30-60 while tmux had 31-60.
- Setting the option emits no `%layout-change`. A `refresh-client -B` format subscription on `#{pane_top} #{pane_height} ...` for all panes (`%*`) reports the change within about 1 s.

## What Changes

- Brindle takes each pane's rectangle from tmux's own pane geometry (`pane_left`, `pane_top`, `pane_width`, `pane_height`) instead of assuming it equals the pane's layout cell. The layout string still drives dividers.
- New panes get their exact rectangle from the state query Brindle already sends for every pane, so they are right from the first frame, including when attaching to a window that already has border status lines.
- A format subscription covering all panes keeps the rectangles current when they change without a layout change. One example is `pane-border-status` being set or unset.
- When a pane's rectangle changes without a layout change, Brindle resizes the pane's terminal and restores its contents again, the same way it does on attach. A program that redrew for the new size before Brindle heard about the change is then shown correctly.
- Rows of a cell outside its pane, such as a title line, are drawn as divider space. `show-tmux-pane-titles` fills them with titles later.
- Adds the control-mode plumbing for format subscriptions (`refresh-client -B` and `%subscription-changed`). `show-tmux-pane-titles` and `apply-tmux-pane-styles` reuse it.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: a new requirement that each pane's grid matches tmux's pane geometry, including rows tmux reserves for border status lines, and is corrected when it changes without a layout change. This sits next to "Panes drawn natively" and "Existing pane contents are restored".

## Impact

- **Code:**
  - `src/tmux/protocol.rs`: parse `%subscription-changed`.
  - `src/tmux/mod.rs`:
    - extend `PANE_STATE_FORMAT` with pane geometry;
    - register the subscription after attach;
    - store each pane's rectangle;
    - re-run the pane restore when the rectangle changes outside a layout change.
  - `src/tmux_view.rs`: position and size pane views from pane rectangles, and leave the rest of each cell to the divider painting.
- **Behaviour without the option:** pane rectangles equal their cells, so nothing changes visibly.
- **Timing:** tmux checks subscriptions about once a second. For up to about 1 s after the option changes, a pane can show at the wrong size before the restore corrects it.
- **Tests:**
  - Unit tests for parsing `%subscription-changed` and the extended pane state.
  - An end-to-end check on a throwaway server: with `pane-border-status top` set before and after attaching, the dumped screen matches `capture-pane`.
- **Dependencies:** none. `show-tmux-pane-titles` and `apply-tmux-pane-styles` build on this change.
