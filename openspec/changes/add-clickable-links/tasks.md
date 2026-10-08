# Tasks

## 1. Detection

- [ ] 1.1 Add `src/terminal/links.rs` with `find_url(text, at)` (scheme list, run boundaries, trailing trimming, bracket balancing) and `valid_scheme(uri)`; verify with unit tests for sentence punctuation, parentheses, markdown links, balanced Wikipedia URLs, quotes, `mailto:`, unknown schemes, schemes inside words, and `at` outside the URL (`cargo test terminal::links`)
- [ ] 1.2 Add `Link` and `Terminal::link_at(point)` in `src/terminal/mod.rs`: OSC 8 first (all visible cells with an equal hyperlink), else detection over the logical line joined by `WRAPLINE` (bounded to 32 rows each side, skipping wide-char spacers); verify with unit tests on a `Term` fed wrapped URLs, hard line breaks, wide characters and OSC 8 text (`cargo test terminal::tests`)

## 2. Opening

- [ ] 2.1 Add `links::open(uri)`: `BRINDLE_LINK_OPENER` or `xdg-open`, spawned and reaped on a background thread with null stdio and its own process group, failures logged; verify with a unit test that runs a recording script through the override and finds the URI it wrote

## 3. Hover, click and copy in the view

- [ ] 3.1 Track pointer and hovered link in `TerminalView` (`update_hover`), recomputed on mouse move (cell or modifier change), modifiers-changed, terminal output while hovered and wheel scroll, and cleared on leave and window deactivation; verify `cargo build` and the hover e2e case's `--- link` line
- [ ] 3.2 Underline hovered-link cells in `prepaint` and use the pointing-hand cursor in `paint`, registering the modifiers-changed and widened mouse-move listeners; verify with the e2e capture that the underline is drawn under the link cells
- [ ] 3.3 Implement the click rule in `mouse_down`/`mouse_up` (ctrl without alt on a link: consume, open on release over the same link; `is_dragging` covers the pending link); verify with the e2e cases for a local shell and a mouse-reporting program
- [ ] 3.4 Add the `copy_link` action ("Copy Link Under Mouse", `Terminal` context, no default key) to `ACTIONS` and handle it in `TerminalView`; verify `brindle --list-actions` lists it and the actions unit tests pass

## 4. Debug hooks

- [ ] 4.1 Add `--hover`/`--click` steps with `[mods-]COL,ROW` parsing in `src/main.rs`, plus `Workspace::debug_mouse` that drives the active `TerminalView` handlers, and a `--- link <uri>` dump line; document them in `--help`; verify with parser unit tests and `brindle --help`
- [ ] 4.2 Extend the e2e harness: `Run::profile`, `Run::hover`, `Run::click`, `Dump::link` parsing (with `--self-test` coverage); verify `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`

## 5. End-to-end cases

- [ ] 5.1 Add `links-local-click` (Shell profile: ctrl-click a URL and an OSC 8 link, recorded by the opener override), `links-mouse-mode` (ctrl-click opens with mouse reporting on, and nothing is reported to the program), `links-hover-underline` (capture shows the underline) and `links-tmux` (OSC 8 in a control-mode pane, including content present before attach); verify each passes with `BRINDLE_E2E=1 cargo test --test e2e -- links-`, run under `flock /tmp/brindle-gui.lock`

## 6. Docs and integration

- [ ] 6.1 Document links in README (usage table: ctrl-click opens, ctrl-hover, `copy_link`, `BRINDLE_LINK_OPENER`) and add the links module to CLAUDE.md's architecture notes; verify by reading the rendered text
- [ ] 6.2 Run `cargo build`, `cargo test`, `cargo build --release`, and confirm `git diff Cargo.lock` is empty
