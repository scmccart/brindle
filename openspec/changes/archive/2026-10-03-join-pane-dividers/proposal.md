# Proposal

## Why

Pane dividers stopped half a cell short where they met another divider, which left visible gaps at T-junctions and crosses. The active pane's highlight also changed colour half a cell away from the corner it outlines.

## What Changes

- Divider lines that run into another divider reach that divider's line, so they meet without gaps.
- The highlight next to the active pane starts and stops on the line of a crossing divider, so it outlines the pane's corner cleanly.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: "Panes drawn natively" gains a scenario saying dividers meet cleanly.

## Impact

- **Code:** `src/tmux_view.rs` (`paint_dividers`, and a new pure `divider_edge` with unit tests).
- **Dependencies:** none.
