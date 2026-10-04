# Tasks

## 1. Dump grid geometry

- [ ] 1.1 Print `--- grid origin=X,Y cell=WxH scale=S` after the screen header in `--dump-screen-after`, from the active terminal view's `GridLayout` and the window's scale factor (design.md decision 5). Put the formatting in a pure function, and unit-test it with fractional cell widths and scale 1 and 2. Verify with `cargo test` and by running `--dump-screen-after 2` once, with the user's go-ahead, to see the line.

## 2. Harness skeleton

- [ ] 2.1 Add the `[[test]]` target (`tests/e2e/main.rs`, `harness = false`) and the `x11rb = "=0.13.2"` dev-dependency. Confirm the `Cargo.lock` diff only adds the dev-dependency edge, with no version changes. The runner skips unless `BRINDLE_E2E=1` is set, and supports `--list`, substring filters, per-case `ok`/`FAILED`/`skipped`, a non-zero exit on failure, and `BRINDLE_E2E_SLOW`. Verify that `cargo test` prints the skip and passes, and that `BRINDLE_E2E=1 cargo test --test e2e -- --list` lists the cases.
- [ ] 2.2 `tests/e2e/tmux.rs`: `TmuxServer` (throwaway socket, `-f /dev/null`, killed on `Drop`) and `ControlClient` (raw `tmux -C` with a timestamped transcript). Verify with a placeholder case that starts and kills a server, and check with `tmux -L` afterwards that no server is left.
- [ ] 2.3 `tests/e2e/brindle.rs`: the test config with the `e2e` theme, the `Run` builder with steps and a tmux timeline, and the `Dump` parser, including the grid line. Unit-test the parser on a saved dump with plain functions run by the harness's own `--self-test` flag. With `harness = false` there is no libtest, and Cargo doesn't set `cfg(test)`. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`.
- [ ] 2.4 `tests/e2e/capture.rs` and `tests/e2e/query.rs`: window capture by PID, cell-region color helpers, PPM artifacts on failure, and the OSC 10/11 query script and parser. Verify with the `--self-test` parser tests and with the grid-geometry case in 3.1.

## 3. Cases

Each case below is one named function. Run each new group with the user's go-ahead, since they open windows.

- [ ] 3.1 `dump-grid-geometry`: the spec scenario. A red cell at a known row and column is found at the dumped origin plus the cell offsets, times the scale.
- [ ] 3.2 Geometry: `geometry-plain`, `geometry-top-before-attach`, `geometry-top-while-attached`, `geometry-fullscreen-less`, `geometry-bottom`, `geometry-zoom`, `geometry-rotate` (both panes) and `geometry-rotate-unequal`. Each compares text and size with tmux.
- [ ] 3.3 Color queries: `colors-attach`, `colors-new-pane`, `colors-reload` (rewrites the test config's theme mid-run) and `colors-styled-pane`.
- [ ] 3.4 Titles: `titles-default-format`, `titles-claude-code-format`, `titles-rename`, `titles-bottom-and-off` and `titles-stacked`. These are pixel cases on the title rows and their colors.
- [ ] 3.5 Styles: `styles-tint` (SGR red kept), `styles-active-pane` and `styles-borders` (set, then unset back to the theme with no green).
- [ ] 3.6 tmux behaviour, over a raw control client and with no display: `tmux-subscription-format`, `tmux-rotate-no-layout-change`, `tmux-border-status-no-layout-change` and `tmux-color-report-precedence`.

## 4. Documentation and full run

- [ ] 4.1 Update `CLAUDE.md`'s "Verifying GUI behaviour": the suite's command, filters, `BRINDLE_E2E_BIN`, where artifacts go, the rule that new GUI checks become cases, and that running it opens windows, so ask first. Update the `cargo test` line in "Build and test" to mention the opt-in suite. Verify the documented commands run as written.
- [ ] 4.2 Run the whole suite with the user's go-ahead, and confirm every case passes and no `brindle-e2e-*` tmux servers are left. Then run `cargo test`, `cargo build --release` and `openspec validate add-e2e-test-harness --strict`.
