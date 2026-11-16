# Tasks

## 1. Config and resolution

- [ ] 1.1 Add `UiFontConfig` and `Config::ui_font` (design decision 1), with the commented default config showing a `[ui_font]` example. Verify: `config.rs` tests that `[ui_font] family = [...]` parses, `ui_font.size` is an unknown-key error, and a config without it gets an empty list (the built-in list then applies through resolution).
- [ ] 1.2 Resolve the UI family from the config (config list, built-in list, `.SystemUIFont`), as a pure function over the installed names. Verify: unit tests for an installed first choice, a missing one falling back, and an empty list.
- [ ] 1.3 Add the preview override and accessors to `Settings` (design decision 2), and move every direct read of `config.font.line_height` and `ui_font_family` to them. Verify: `grep -rn "font.line_height\|ui_font_family" src` finds only `settings.rs` and the config.

## 2. Writing fonts

- [ ] 2.1 `config_edit::set_font_family` and `set_font_number` (design decision 6). Verify: unit tests for adding `[ui_font]` to a commented config, moving a fallback to the front, removing `family` with "Default" (and an emptied table), and comments kept.

## 3. Settings dialog

- [ ] 3.1 The installed-family list (design decision 3): a pure sort/filter/group function, unit-tested, plus the background monospace probe and its process global.
- [ ] 3.1a `brindle_ui::Combobox` (design decision 4): Popover + Command, trigger styled as Select's, unit tests for its pure parts, a gallery entry and README listing in `ui/README.md`.
- [ ] 3.1b `brindle_ui::UiFont` (design decision 8), set with the tokens, applied by brindle-ui's own roots and the workspace root.
- [ ] 3.2 `FontsState` (design decision 5): reading, validation, preview, apply. Verify: unit tests for each settings-dialog scenario in the change's spec that the model decides (current values shown, size out of range, leave unapplied, choose a UI font, choose a listed fallback).
- [ ] 3.3 The section in the view: `Section::Fonts` after Theme, its four fields, focus stops, footer, the `FontPreview` event, and the `--- settings` line (design decision 7). Update the existing section tests (`SECTIONS` labels, section cycling).
- [ ] 3.4 The workspace side: set or clear the override on `FontPreview`, re-fit terminals, clear on reload and apply; print the `--- fonts` line.

## 4. Checks and docs

- [ ] 4.0 The e2e and perf dump parsers accept the `--- fonts` line, and their `--self-test`s cover it.
- [ ] 4.1 e2e cases in `tests/e2e/`: `ui-font-config` (a config naming an installed UI family and a missing one; `--- fonts` shows the expected families) and `settings-fonts` (open the dialog, go to Fonts, change Size, see the grid's cell change in the dump before applying, apply, and find the size in the config file). Both are registered for Linux and in `tests/e2e/cases/win.rs`, with the UI family picked per platform from fonts the CI runners have. They run in CI's `e2e` and `windows-e2e` jobs.
- [ ] 4.2 README.md's configuration section (`[ui_font]`, the Fonts section) and CLAUDE.md (`settings.rs` bullet, the dump lines).
- [ ] 4.3 `cargo test --workspace --locked` and `cargo build --release --locked` pass on Windows; the CI jobs pass on the PR.
- [ ] 4.4 Archive the change in the PR (`openspec archive add-font-settings`).
