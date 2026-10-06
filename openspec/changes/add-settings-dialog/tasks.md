# Tasks

## 1. Config editing layer

- [x] 1.1 Add `toml_edit = "0.22"` to `Cargo.toml` without running `cargo update`. Verify that `git diff Cargo.lock` shows only the new dependency edge on the existing 0.22.27 and that `cargo build` succeeds.
- [x] 1.2 Create `src/config_edit.rs` with `set_global_theme`, `upsert_theme` (writes every color explicitly and omits only an unset `selection_foreground`), `rename_theme` (rewrites top-level and `[[profiles]]` references), `delete_theme`, `theme_references` and `custom_themes` (each table deserialized on its own). Verify with unit tests run by `cargo test config_edit`:
  - comments and `[keybindings]` survive every edit;
  - an upsert keeps the table's position;
  - a rename updates the global and profile references;
  - references are found;
  - one malformed theme table doesn't hide the others.
- [x] 1.3 Add `config_edit::save(path, edit)`:
  - reads and parses the file at call time, starting from `DEFAULT_CONFIG` when the file is missing;
  - refuses invalid TOML and returns the parse error;
  - resolves symlinks, writes `<path>.tmp` and renames it over the file.

  Verify with unit tests in a temp dir: a malformed file is left byte-for-byte unchanged, a missing file gets the default contents plus the edit, and writing through a symlink updates the target.
- [x] 1.4 Add theme-name validation (non-empty after trimming, not a built-in, not another custom theme's name) and a helper giving a theme's effective `cursor_text` and `accent`. Verify with unit tests, including that copying `tokyo-night` yields its blue accent and not brindle-dark's orange.

## 2. Preview plumbing in `Workspace`

- [x] 2.1 Add `theme_preview: Option<Theme>` and a single `theme_for(tab, config)` that returns the preview only for tabs whose profile sets no theme. Route `apply_config` through it, and add `set_theme_preview` to update terminals and `TmuxWindowView`s without sending tmux color reports. Verify with `cargo test`, then by checking that a reload with no preview set behaves as before (the e2e `reload`/theme cases still pass in task 5.2).
- [x] 2.2 Replace `palette: Option<OpenPalette>` with an `overlay` enum holding either a palette or the settings dialog. Opening one replaces the other, and replacing the dialog runs its cancel path. Verify that the existing palette unit tests and `cargo build` pass and that the palette behaves as before in the e2e palette cases.

## 3. Settings dialog: theme list

- [x] 3.1 Move the palette's single-line editing (append, backspace, single-line paste) into a reusable `TextField` in `picker.rs` and use it from `Palette`. Verify that the existing `picker` unit tests pass, and add a `TextField` test for paste with line breaks.
- [x] 3.2 Add actions: `OpenSettings` ("Open Settings", `open_settings`) bound to `ctrl-shift-,` and `ctrl-<` in `Workspace`, plus the `SettingsDialog` context actions from design Decision 7. Verify that the `actions.rs` tests for unique labels pass and that `brindle --list-actions` shows `open_settings`.
- [x] 3.3 Create `src/settings_dialog.rs` with the section list ("Theme") and the theme list (built-ins, then sorted custom themes from `custom_themes`, active and custom markers, unreadable themes shown with their error and not selectable). Selecting a theme emits a preview event that `Workspace` applies through `set_theme_preview`. Verify with unit tests of the list model: ordering, initial selection on the active theme, skipping unreadable entries when moving.
- [x] 3.4 Implement Apply (clear the preview, `save` with `set_global_theme`, `reload_config`, close) and Cancel (escape, Cancel button, click outside: clear the preview and close). Save errors stay in the dialog. While the dialog is open, `Workspace` actions other than the palette toggles are swallowed. Verify with a unit test that Apply on a malformed file keeps the dialog state and reports the error.
- [x] 3.5 Add a `--- settings …` line to `dump_active_screen` (section, selected theme, editor state, field errors, save error). Verify by running `brindle --action open_settings --dump-screen-after 3` against a throwaway config and checking the line. This opens a window, so ask the user first.

## 4. Settings dialog: theme editor

- [x] 4.1 Add the editor view, opened by New (a copy of the selected theme under an unused `<name>-custom`) and by Edit (a custom theme; on a built-in, Edit acts as New). It has a name field and 23 color rows (swatch plus `TextField`), ANSI colors labelled by name, a clear control only on selection foreground, a scrolling body, and tab/shift-tab between fields. Verify with unit tests of the editor model: a copy keeps the effective cursor text and accent, and a fresh name doesn't collide.
- [x] 4.2 Live-preview each valid color edit. An invalid field is marked, the last valid color stays in the preview, and Save is disabled while any field or the name is invalid. Verify with unit tests: `#12345` is rejected, `#abc` expands, and an invalid field keeps the previous preview theme.
- [x] 4.3 Implement Save (`upsert_theme`, or `rename_theme` then `upsert_theme` when a saved theme's name changed; return to the list with the theme selected and previewed) and Discard (no write; restore the pre-edit selection and preview). Verify with a unit test of the dialog's save path against a temp config: rename the active theme and check the file contents.
- [x] 4.4 Implement Delete for custom themes, with a confirm step, refused with the reason when `theme_references` is non-empty. Verify with a unit test: an unused theme is removed from the file and the list, and a theme in use is refused and left alone.

## 5. Docs and end-to-end checks

- [x] 5.1 Document the dialog in README.md (keybinding table and theme section) and point to it from the themes comment in `assets/default-config.toml`. Verify that the `default_config_file_parses` test still passes and the README keybinding table lists `ctrl-shift-,`.
- [x] 5.2 Add e2e cases in `tests/e2e/cases/` using the `e2e`/`e2e-alt` themes from the test config. Each one checks colors by sampling the window's background and checks state through the `--- settings` dump line:
  - `settings-preview`: `--action open_settings`, then `settings_down` to reach `e2e-alt`. The background shows `e2e-alt`. Then `settings_cancel` brings back `e2e`, and the config file is unchanged.
  - `settings-apply`: the same, then `settings_confirm`. The config has `theme = "e2e-alt"` and its comments are intact.

  Ask the user before running, since these open windows. Then verify that `BRINDLE_E2E=1 cargo test --test e2e -- settings` passes and the full suite still passes.
- [x] 5.3 Integration check: run `cargo test` and `cargo build --release`. Offer to reinstall with `scripts/install.sh`, and give the user a short manual checklist for the interactive parts (typing hex values, mouse clicks, delete confirm), which the e2e harness can't drive without synthetic input.
