# Tasks

## 1. Search logic

- [x] 1.1 Add `src/terminal/search.rs` with `SearchOptions`, `escape`, `pattern(query, options) -> Option<Result<RegexSearch, String>>` (design §4), `find_from` (incremental and next/previous, §5), `visible_matches` (§7), `count` (§10, capped at 1000, numbered from the bottom) and `centered_offset` (§6). Unit-test against a real `Term` fed through `Processor`: literal escaping (`a.c`), case on/off in both directions (`Foo`/`foo`), regex on, invalid regex, stepping up and down with wrap-around, the count and index, a match across a wrapped line, visible matches limited to the viewport, and centering clamped at both ends. Verify with `cargo test terminal::search`.

## 2. Theme colors

- [x] 2.1 Add the four `search_*` fields to `Theme` with derived accessors, and fill them in `with_effective_colors` (design §9). Unit-test the defaults for brindle-dark and a partial custom theme, and that explicit colors win. Verify with `cargo test theme`.
- [x] 2.2 Write the four colors in `config_edit::upsert_theme`, and add four non-optional slots to the settings editor after Accent. Update any tests that count slots or compare written theme text, and add one that a saved copy of a built-in theme carries the derived search colors. Verify with `cargo test config_edit settings_dialog`.
- [x] 2.3 Document the keys in `assets/default-config.toml`'s custom theme example, and verify that the default config still parses (`cargo test config`).

## 3. Actions and keys

- [x] 3.1 Add `Find`, `FindNext`, `FindPrevious`, `FindClose`, `FindToggleCase` and `FindToggleRegex` to `actions!` and `ACTIONS` (only `Find` labelled, as "Find"). Add the default bindings of design §2, including `Paste` in `FindBar`. Extend the actions tests: the new names resolve, and ctrl-shift-f, enter and escape parse. Verify with `cargo test actions` and `cargo run -- --list-actions | grep find`.

## 4. Find bar and highlights

- [x] 4.1 In `src/terminal_view.rs`, add `FindBar` state and the handlers: open/focus, edit keys, paste, toggles, next/previous, close with offset restore, the remembered query with the select-on-restore flag, and the debounced count (§§1, 5, 10–12). Guard `key_down` so keys only reach the program when the terminal itself is focused. Verify that `cargo build` passes with no warnings.
- [x] 4.2 Render the bar (query with caret or selected state, count or "No results"/"Invalid regex", Aa and .* toggles, ↑ ↓ × buttons) at the top right, occluding the mouse, with theme colors. Verify that `cargo build` passes.
- [x] 4.3 In `src/terminal_element.rs`, compute the visible matches in prepaint and apply match and focused-match colors with the precedence of design §8. Verify that `cargo test` passes, with a unit test of the precedence helper.

## 5. Debug hook, dump and docs

- [x] 5.1 Add the `--find <TEXT>` step to `main.rs` (help text included), `Workspace::find_in_active`, and the `--- find` dump line (design §13). Verify with a locked run: `flock /tmp/brindle-gui.lock ./target/debug/brindle -p Shell --send 'echo needle\r' --find needle --dump-screen-after 4`, using a temporary `BRINDLE_CONFIG`. The dump should show `count=1` and a focused position.
- [x] 5.2 Update the README: ctrl-shift-f in the keybinding table, a short find bar section (enter/shift-enter, alt-c/alt-r, escape), and the theme keys. Verify that the README matches `default_bindings()`.

## 6. End-to-end cases

- [x] 6.1 In `tests/e2e`, add a profile setter to `Run` and `--- find` parsing to `Dump`, plus a self-test line. Give the `e2e` theme explicit search colors. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`.
- [x] 6.2 Add the case `search-local`: a local shell prints numbered lines with a token near the top, `--find` the token, then dump and capture. Check that the screen scrolled to the token, the dump count and focused position, that the focused cells are drawn in the focused match background, and that another visible match is drawn in the match background. Add `search-step`: `find_next` then `find_previous` past the newest match, checking the index, the current line and the wrap. Add `search-close-restores`: after `find_close` the screen is back at the bottom and has no highlight.
- [x] 6.3 Add the case `search-tmux-pane`: two panes print the same token, find it in the active pane, and check that the active pane's token is highlighted and the other pane's is not.
- [x] 6.4 Run the new cases and a regression sample (`settings-`, `colors-attach`, `dump-grid-geometry`) under `flock /tmp/brindle-gui.lock`, and verify that they pass.

## 7. Integration

- [x] 7.1 Run `cargo build`, `cargo test` and `cargo build --release`, and verify that all pass with no new warnings.
