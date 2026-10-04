# Proposal

## Why

The end-to-end suite (`tests/e2e/`) covers the tmux work from `match-tmux-pane-geometry` onwards. Changes archived before it were verified with one-off Brindle runs described only in their `tasks.md`, so none of those checks can be repeated:
- `bootstrap-brindle-terminal`: live theme reload while attached, reattaching to a running editor with its mouse modes, the native tmux bindings, and detaching from the only tabs;
- `add-command-palette`: running actions through the palette, `tmux_break_pane`, the layout commands, and the detach key routing;
- `restore-active-pane-on-attach`: attaching keeps tmux's current window and active pane, and background windows stay in the background;
- `join-pane-dividers`: divider junctions in tiled layouts.

## What Changes

- Port each of those checks to a named case, grouped by area:
  - `restore-*`: reattach to `vi` with modes 1002/1006, and the alternate screen;
  - `selection-*`: the current window, the active pane, `new-window -d` staying in the background, and a split acting on tmux's active pane;
  - `palette-*`: running an action through the palette, and the palette's dump description;
  - `commands-*`: break pane, the layouts, rotate and swap, and the detach key in a local tab;
  - `dividers-*`: pixel checks at T-junctions and crosses in a 2x2 tiled window;
  - `reload-*`: a theme change while attached.
- Where a check needs Brindle to open a local shell tab, use the `Shell` profile the test config already has.
- Leave out checks that need real keyboard input, such as typing into IME pre-edit. The suite never sends synthetic input, so these stay manual and are listed in `CLAUDE.md`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. This change adds tests for behaviour that is already specified (`skip_specs: true`).

## Impact

- `tests/e2e/cases/`: new modules. Helpers may grow, for example a palette-dump parser and a divider-junction pixel helper.
- `CLAUDE.md`: list the checks that remain manual.
- No changes to Brindle itself are expected. A check that fails when ported is a bug, and gets fixed in its own change.
