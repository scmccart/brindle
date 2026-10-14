# Tasks

Cases open Brindle windows. Write the cases, check them without a display (`cargo build --tests`, `--list`, `--self-test`), then ask the user before one batched GUI run with the new prefixes as filters (`reattach- selection- palette- commands- dividers- reload-`). A ported check that fails because of Brindle is reported, not fixed here.

## 1. Pane restore (`reattach-*`, design.md §3)

- [x] 1.1 Add `cases/reattach.rs` with `reattach-alt-screen` and `reattach-mouse-modes`. Each runs Brindle twice against a pane whose modes a `printf … ; cat` set. In each run it compares the dumped `TermMode` tokens with tmux's `display-message` flags and with the expected flags, and checks the screen with `matches_tmux`. Verify that `--list` shows both and that they pass in the GUI run.

## 2. tmux's selection (`selection-*`, design.md §4)

- [x] 2.1 Add `cases/selection.rs` with the three-window, three-pane setup and a snapshot of window and pane selection. Add `selection-current-window`, `selection-active-pane`, `selection-split-active-pane` and `selection-background-window`. Verify that `--list` shows them and that they pass in the GUI run.

## 3. Command palette (`palette-*`, design.md §5)

- [x] 3.1 Add `cases/palette.rs` with `palette-tmux-action` (Split Right through the palette: two panes, the new one active, and the dump matching it) and `palette-lists-for-tab` (tmux-only rows and the detach shortcut in a tmux tab, none of them in a local tab, and `tmux: New Session` in both). Verify that `--list` shows them and that they pass in the GUI run.

## 4. Arrangement commands (`commands-*`, design.md §6)

- [x] 4.1 Add `cases/commands.rs` with `commands-break-pane` and `commands-layouts`. The layouts are checked by the `list-panes` geometry after each action, one Brindle run per action. Verify that `--list` shows them and that they pass in the GUI run.
- [x] 4.2 Add `commands-rotate-swap`: the pane order after rotate, and the active pane moving with each swap. Verify that it passes in the GUI run.
- [x] 4.3 Add `commands-detach-beside-local`. `tmux_detach` in the local tab changes nothing. In a tmux tab it leaves only the live shell tab, and the session survives. Verify that it passes in the GUI run.

## 5. Divider junctions (`dividers-*`, design.md §7)

- [x] 5.1 Add a `junction` helper to `capture.rs`. It returns the four arms' colors (or none, or a gap) of a junction cell, and gets a `--self-test` entry built on a synthetic image (a cross, a T, and a cross with a one-pixel gap). Verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`.
- [x] 5.2 Add `cases/dividers.rs` with `dividers-tiled` and `dividers-t-junction`, using magenta and yellow border styles and the arms listed in design.md §7. Verify that `--list` shows them and that they pass in the GUI run.

## 6. Live reload (`reload-*`, design.md §8)

- [x] 6.1 Add `cases/reload.rs` with `reload-theme-attached`: one run checks the tmux panes' background and text colors after the switch, and one checks a local tab with a session open beside it. Verify that `--list` shows it and that it passes in the GUI run.

## 7. Documentation

- [x] 7.1 Add the "Manual checks" list from design.md §9 to `CLAUDE.md` under "Verifying GUI behaviour", next to the end-to-end suite notes. Verify that each item names a behaviour no case covers.

## 8. Integration

- [x] 8.1 Run `cargo test`, `BRINDLE_E2E=1 cargo test --test e2e -- --self-test` and `openspec validate port-earlier-e2e-checks --strict`. Then, with the user's go-ahead, run the batched GUI filters, and afterwards the full suite to confirm that no existing case broke. Report any failure with its artifacts directory.
