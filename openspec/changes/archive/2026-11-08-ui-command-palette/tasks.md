# Tasks

## 1. brindle-ui

- [x] 1.1 Add `Overlay::set_ignore_outside_press(bool)` (settable before `open`, same effect as the constructor flag) with a unit test of the pure decision. Verify: `cargo test -p brindle-ui`.

## 2. Palette host on Command

- [x] 2.1 Rebuild `Palette`'s list mode on `Command` (rows to `CommandItem` with label, detail and shortcut; placeholder; empty message; `Confirmed` to `PaletteEvent::Confirmed`, `Cancelled` to `Dismissed`), keeping `Palette::list`, `input`, `describe` and `PaletteEvent`. Delete the hand-rolled list, highlight, scroll and styling. Verify: `--- palette` dump text unchanged for the existing `palette-*` and `input-palette*` e2e cases.
- [x] 2.2 Rebuild prompt mode on `Input` in the same view (label as the dialog title, error line, running state, selected pre-filled value, submit/cancel as today). Verify: `palette-tmux-action` and the prompt-failure e2e cases pass.
- [x] 2.3 Chrome through `Dialog`, and `FocusStops` with the active mode's input as the one stop; `picker_kill` handled in the host on `Command::highlighted()`; `picker_*` actions mapped to the shared list and `Dismiss` actions with config and `--action` names kept (keymap unit test pins each alias). Verify: `cargo test --workspace`, and the sessions palette kill case passes.

## 3. Workspace

- [x] 3.1 Replace the `Overlay` enum with `palette: UiOverlay<Palette>`: open, close, toggle, refresh (sessions) and the one-overlay rule with settings go through the slot; the outside-press flag is set per `PaletteKind` before opening; the actions snapshot is taken before it opens. Verify: `cargo test --workspace` and the full palette, session-end and settings e2e cases under `scripts/with-xvfb`.
- [x] 3.2 `--- focus` reports `palette` when focus is inside the palette. Verify: e2e self-test passes.

## 4. e2e and docs

- [x] 4.1 Add `--key`-driven cases: escape returns focus to the tab (`--- focus terminal`); tab stays in the palette; up wraps from the first row to the last and down from the last to the first; home and end edit the filter; for a question palette (session ended) a click-outside flag test is a unit test since pointer presses can't be posted on Linux. Verify: `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- palette`.
- [x] 4.2 Update CLAUDE.md (`picker.rs`, the overlay notes) and README keybinding notes if they describe the palette. Verify: `grep -n -i "picker\|palette" CLAUDE.md README.md` reviewed.
- [x] 4.3 Run the full gate. Verify: `cargo test --workspace --locked`, `cargo build --release --locked`, the full e2e suite under `scripts/with-xvfb` and the gallery check are green.

## Manual checks (hand to the user)

- Pointer: click outside a plain palette closes it and focus is on the tab; click outside the session-ended question leaves it open.
- Real keyboard: ctrl-shift-p, type, up/down/ctrl-p/ctrl-n, enter, escape; ctrl-shift-delete in the sessions palette; IME in the filter and the prompt (IBus ctrl-shift-u `e9` space gives `é`).
