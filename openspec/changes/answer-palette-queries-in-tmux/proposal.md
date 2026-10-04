# Proposal

## Why

Programs in control-mode tmux panes get no reply when they query a palette color with OSC 4 (`ESC ] 4 ; <n> ; ? BEL`). This was seen while testing `report-pane-colors-to-tmux`. tmux answers OSC 4 from the pane's palette and the outer terminal's colors, and a control client has neither. Programs that adapt their colors to the terminal's palette time out waiting and fall back to guesses. In a local Brindle tab, they get the theme's colors.

## What Changes

To be decided in design. The options found so far:
- **Report the palette to tmux.** `refresh-client -r` is documented for OSC 10-style reports. Check whether tmux 3.6, or newer, accepts OSC 4 reports from a control client.
- **Set the pane palette.** tmux's `pane-colours` option holds a palette that OSC 4 queries are answered from. Setting it per pane to the theme's palette would answer queries. But it is visible state on the user's server, so other terminal clients would render with Brindle's palette, and it would have to be removed on detach.
- **Ask upstream.** Propose that tmux forward unanswered queries to control clients, or accept OSC 4 reports.

If none is acceptable, record the limitation in the `tmux-control-mode` spec and in `CLAUDE.md` instead.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: depending on the outcome, either a requirement that programs in panes get the theme's palette colors from OSC 4 queries, or a documented limitation.

## Impact

- **Code:** `src/tmux/mod.rs`, next to the color reports, if Brindle can supply the palette.
- **Tests:** an end-to-end color-query case for OSC 4, like the OSC 10/11 ones.
- **Risk:** the `pane-colours` option changes state other clients see; design must weigh that against the benefit.
