# Design

## Context

- **Themes:** a `Theme` lives in each `Terminal` (`terminal.theme`) and in each `TmuxWindowView`. `Workspace::apply_config` re-resolves them from the `Settings` global on reload. Tab chrome reads the active tab's terminal theme (`Workspace::theme`), so changing the terminal themes also re-themes the chrome.
- **Config:** loaded by serde from `config.toml` (`$BRINDLE_CONFIG` overrides the path). `Theme` is `#[serde(default)]` with `Default = brindle-dark`, so an omitted key takes brindle-dark's value. When the file has any error, `Settings.config` is all defaults (`config_error` is set), so `config.themes` is empty.
- **Reload:** `main.rs` polls the file's mtime about once a second, and `reload_config` rebuilds `Settings` and calls `apply_config` on every window.
- **Overlays:** `Workspace` holds one `Option<OpenPalette>`. `Palette` (`picker.rs`) implements its own single-line text editing in `key_down`, with no IME. Palette navigation uses actions in the `Palette` key context.
- **Debug actions:** `--action` uses `window.dispatch_action`, which dispatches from the focused element, so actions in the dialog's key context reach it once it has focus.
- **Keys:** GPUI names shift+comma `<` (the `Keysym::less` arm of `Keystroke::from_xkb`).

## Goals / Non-Goals

**Goals:**
- An overlay structure that later sections (fonts, cursor, profiles) slot into without reworking the dialog.
- A pure, unit-tested config-editing layer, separate from the UI.
- Preview without touching the `Settings` global, the file, or tmux.

**Non-Goals:**
- Per-profile theme selection, or any setting other than the theme.
- A graphical color picker (HSV or wheel).
- IME in the dialog's text fields. Names and hex values are ASCII; the palette has the same limit.
- Importing theme files from other terminals.

## Decisions

### 1. The dialog is a `Workspace` overlay sharing the palette's slot

Replace `palette: Option<OpenPalette>` with an `overlay: Option<Overlay>` enum (`Palette(OpenPalette)` / `Settings(OpenSettings)`). Exclusivity then follows from the type: opening either one replaces the other, and replacing the settings dialog runs its cancel path. While the dialog is open, `Workspace` drops its own tab, window and font actions. The palette toggles still work, and they replace the dialog.
- *Alternative:* a separate GPUI window. Rejected (as you chose): the preview would need a sample terminal, plus another window's focus and decorations.

### 2. Preview is a per-window override in `Workspace`

`Workspace` gains `theme_preview: Option<Theme>`. One function, `theme_for(tab, config)`, returns the preview when `tab.profile.theme.is_none()` and the profile theme otherwise. Both `apply_config` and a new `set_theme_preview(Option<Theme>)` use it. The latter assigns the theme to each `Terminal` and `TmuxWindowView` and calls `cx.notify()`.
- Cancel sets the preview to `None`, which restores the config themes.
- Apply writes the file, then closes the dialog, which clears the preview, and then the deferred reload applies the new config. The reload runs after the preview is gone, so a stale override can't win.
- `ReloadConfig`'s global listener defers `reload_config`. Global listeners run inside the dispatching window's update, where the reload's per-window update would skip that window; this also fixes ctrl-shift-r for the focused window.
- Tabs opened while the dialog is open can't happen, because the dialog swallows those actions (Decision 1).
- The preview never reaches tmux: no `refresh-client -r` color reports are sent while previewing. The real reload after Apply reports the new colors as it does today.
- *Alternative:* swap the `Settings` global temporarily. Rejected: it would leak into other windows, into new tmux panes, and into a reload triggered mid-preview.

### 3. Config edits go through `toml_edit` in a pure module

`src/config_edit.rs` operates on a `toml_edit::DocumentMut` and has no UI or I/O apart from the `save` helper:
- `set_global_theme(doc, name)`
- `upsert_theme(doc, name, &Theme)`: replaces the table's values and keeps the table's position and the comments before it.
- `rename_theme(doc, old, new)`: renames the table and rewrites top-level `theme` and every `[[profiles]]` `theme` equal to `old`.
- `delete_theme(doc, name)`
- `theme_references(doc, name) -> Vec<Reference>`: what Delete checks.
- `custom_themes(doc) -> Vec<(String, Result<Theme, String>)>`: each `[themes.*]` table deserialized on its own (`toml::from_str` of that table's text), so one bad theme or an error elsewhere doesn't hide the rest.

`save(path, f)` reads and parses the file **at save time**, not when the dialog opens, because the user may have edited it in the `ctrl-,` tab since. It then applies `f`, writes `<path>.tmp` beside the file and renames it over the original. A missing file starts from `DEFAULT_CONFIG`. A parse failure returns the `toml_edit` error and leaves the file untouched.
- `toml_edit = "0.22"` resolves to the 0.22.27 already in `Cargo.lock` (pulled in by `toml` 0.8). Add it without `cargo update`.
- *Alternative:* serialize `Config` with serde. Rejected: it loses the user's comments and the commented default file.
- *Alternative:* a separate themes file. Rejected (as you chose): it adds a second config source.

### 4. Every color is written explicitly, except a cleared selection foreground

`upsert_theme` writes `foreground`, `background`, `cursor`, `cursor_text`, `selection_background`, `accent` and `ansi` (one array) always, and `selection_foreground` only when it's set. An omitted key means brindle-dark's value, not a value derived from this theme. That is correct for `selection_foreground` (brindle-dark's is `None`) but wrong for `cursor_text` and `accent`. So the editor gives those two the copied theme's *effective* values (`cursor_text` falls back to `background`, and `accent()` to `ansi[4]`) and never offers to clear them. This keeps the existing config format: no `"none"` color value is introduced.

### 5. Theme list source

The dialog lists the built-ins (`BUILTIN_THEMES`, in order), then `config_edit::custom_themes` of the file read when the dialog opens, sorted by name. This holds even when `config_error` is set, because the list doesn't come from `Settings.config.themes`. The active theme is `Settings.config.theme`. Custom names that clash with a built-in are already shadowed by `Config::theme`, and they are refused when saving (spec "Theme names").

### 6. Shared text field

The palette's `key_down` editing (append, backspace, paste as a single line) moves into a small `TextField` struct in `picker.rs`. Both the palette and the dialog's name and hex fields use it. Only one field has the keyboard at a time, and tab and shift-tab move it. This adds no cursor movement inside a field, matching the palette today.

### 7. Actions and bindings

- `OpenSettings` ("Open Settings") is bound in `Workspace` to both `ctrl-shift-,` and `ctrl-<`, following the `ctrl-+` / `ctrl-shift-=` precedent.
- The dialog's key context is `SettingsDialog` (not `Settings`, which would collide in name with the global). Its actions:
  - `SettingsUp` and `SettingsDown` (up/down, ctrl-p/ctrl-n);
  - `SettingsConfirm` (enter) and `SettingsCancel` (escape);
  - `SettingsNextField` and `SettingsPrevField` (tab/shift-tab);
  - `SettingsNew` and `SettingsEdit`, unlabelled, so they stay out of the command palette but can be driven by `--action` for e2e.
- The dialog actions go into the `ACTIONS` table **without labels** (`settings_up`, `settings_down`, `settings_confirm`, `settings_cancel`, `settings_new`, `settings_edit`, and so on). That keeps them out of the command palette, while `--action` and user bindings can still reach them. `--action` resolves names only through `ACTIONS`, so this is what lets the e2e cases drive the dialog.

### 8. Saving reloads immediately

After a successful write, the dialog dispatches the existing `ReloadConfig` path (`reload_config(cx)`) instead of waiting for the mtime poll. The poll will then see the same mtime it already reloaded for and might reload once more, which is harmless.

### 9. Dump support

`Workspace::dump_active_screen` adds a `--- settings …` line when the dialog is open, like `Palette::describe`. It includes the section, the selected theme, the editor state, field errors and the last save error. The e2e cases assert on it.

## Risks / Trade-offs

- [Preview of styled tmux panes is approximate. `window-style` indexed colors resolve against the session's config theme (`tmux::resolve_color`), not the preview.] → Accepted; Apply's reload corrects them. Recorded here, not in the spec.
- [`ctrl-shift-,` may be reported differently on non-US layouts.] → Both spellings are bound, and the command palette entry always works. The user can rebind `open_settings`.
- [Rename rewrites profile `theme` values, which touches profile tables the dialog otherwise doesn't own.] → Only values equal to the old name change, and a unit test covers it.
- [The temp-and-rename write replaces a symlinked config with a regular file.] → Resolve the path with `std::fs::canonicalize` first, so the rename goes to the symlink's target.
- [The editor has 23 color rows, more than fit in a small window.] → The editor body scrolls (`overflow_y_scroll` with a `ScrollHandle`), and moving to a field scrolls it into view.
- [Every keystroke re-themes all tabs, and GPUI re-renders the whole window on notify.] → Assigning a theme is a cheap clone and adds no I/O. A preview only reapplies when the parsed color actually changes.

## Migration Plan

None. The config format is unchanged, and existing files keep working. Rollback means removing the dialog. Any themes it wrote are ordinary `[themes.*]` tables.
