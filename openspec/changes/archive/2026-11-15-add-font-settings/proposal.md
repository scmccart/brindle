# Proposal

## Why

The window chrome's font is not configurable. Brindle picks the first installed family from a built-in list (Inter, Cantarell, Ubuntu Sans, …, Segoe UI), so on Windows, where none of the Linux families are installed, the chrome is always Segoe UI. GPUI centres a line on the font's ascent plus descent. Segoe UI's ascent leaves room for accents above the capitals, so text in the tab strip, buttons, menus and palette rows sits low: about 1px for capitals and 2.5px for lowercase at 14px, measured on a tab face. A font with balanced metrics, such as Inter (shadcn's own pattern is a web font with balanced metrics), centres properly, but users can't choose one.

The terminal font is configurable, but only by hand-editing `[font]` in the config file. The settings dialog has Profiles and Theme sections and nothing for fonts.

Bundling a font was considered and rejected: the user prefers a setting, so that each machine uses fonts its owner installed.

## What Changes

- A new `[ui_font]` config table with `family`, a preference list like `[font] family`. The first installed family is the chrome's font. It defaults to today's built-in list, and when none of its families are installed the built-in list is tried, then the platform's UI font. It has no size: chrome sizes stay as designed.
- `[font]` stays the terminal font, unchanged (`family`, `size`, `line_height`).
- Live reload applies a `[ui_font]` change to the chrome at once, as it does for `[font]`.
- A **Fonts** section in the settings dialog, after Theme:
  - a **Terminal font** select listing installed families, monospace families first, plus "Default" (the built-in list);
  - **Size** and **Line height** fields for the terminal font, validated to the config's ranges (4–96 and 0.8–3.0);
  - a **UI font** select listing installed families, plus "Default".
  - Changes preview at once: the terminal font in every tab of the window, the UI font on the chrome. Apply writes them to the config with `config_edit`, keeping comments, and closes the dialog. Cancel restores what was shown.
  - Choosing a family moves it to the front of that table's `family` list, so the rest of the list stays as fallbacks. "Default" removes `family` from the table.
- `--dump-screen-after` prints the Fonts section's state on its `--- settings` line, and the active UI font's family on a new `--- fonts` line.

## Capabilities

### Modified Capabilities

- `configuration`: the `[ui_font]` table, its resolution and its live reload.
- `settings-dialog`: the Fonts section, its preview, apply and cancel.

## Impact

- `src/config.rs`: `UiFontConfig` (`family: Vec<String>`), `Config::ui_font`; the commented default config gains a `[ui_font]` example.
- `src/settings.rs`: `resolve_ui_family` reads the config's list, then the built-in list, then `.SystemUIFont`. A preview override for the terminal font and the UI font, read where the resolved ones are read today.
- `src/config_edit.rs`: writing `[font]` and `[ui_font]` keys with comments kept.
- `src/settings_dialog.rs` (and a new `settings_dialog/fonts.rs`): the section, its model (GPUI-free, unit-tested) and view.
- `ui/`: `Combobox` (shadcn's Popover + Command pattern, for the family pickers) and a `UiFont` global that brindle-ui's own roots (tooltips, drag previews) apply.
- `src/workspace.rs`: the preview and apply events, re-fitting every terminal's grid when the terminal font changes.
- `tests/e2e/`: a `settings-fonts` case.
- README.md (configuration) and CLAUDE.md (`settings.rs` bullet, the `--- settings` and `--- fonts` dump lines).
