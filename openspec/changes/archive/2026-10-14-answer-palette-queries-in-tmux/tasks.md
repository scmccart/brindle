# Tasks

## 1. Answering in panes

- [x] 1.1 Give `Backend::Remote` an optional palette handler (`Terminal::remote` takes it; `Terminal::message` passes `None`), and in `handle_event` route `ColorRequest` for `index < 256` with no program-set color to it, leaving every other reply to `reply()`. Verify with `cargo build` and `cargo test`
- [x] 1.2 In `src/tmux/mod.rs`, add the pure `palette_clients_allow(lines)` and `Pending::PaletteClients`/`Pending::PaletteOption`, and pass each pane a handler that asks `list-clients`, then `#{pane-colours[I]}`, then sends the reply with `send-keys -H`. Unit tests cover only control clients, a terminal client, no lines, and the `#{pane-colours[I]}` command. Verify with `cargo test tmux::`
- [x] 1.3 Check by hand against a throwaway 3.6 server: a `--dump-screen-after` run that runs an OSC 4 query script shows one reply with the theme's color for indexes 4 and 196. Verify by reading the dump
- [x] 1.4 Add a "Palette queries" note to CLAUDE.md's `tmux/` section (Brindle answers OSC 4 itself, and when it stays silent), and update `reply()`'s doc comment. Verify by reading both back

## 2. End-to-end cases

- [x] 2.1 Extend `tests/e2e/query.rs` with a palette query script that, per index, prints the first reply and any second reply that arrives within half a second, plus a parser with a self-test. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`
- [x] 2.2 Add the headless `tmux-osc4-unanswered` case: over a raw `tmux -C` client, an OSC 4 query appears in `%output` and gets no reply in the pane. Verify it passes
- [x] 2.3 Add display cases `colors-palette` (indexes 4 and 196 after attach, and index 1 in a new split), `colors-palette-reload` (index 4 after a theme reload), `colors-palette-tmux-answers` (a program-set index and a `pane-colours` index, each answered once with tmux's color) and `colors-palette-tty-client` (with a terminal client attached through `script`, no reply). Register them in `cases/mod.rs`, and verify they pass (ask the user before opening windows)

## 3. Integration

- [x] 3.1 Run `cargo test` and the whole e2e suite. Verify everything passes, and report any failures with their artifacts directory (ask the user before opening windows)
