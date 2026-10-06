# Proposal

## Why

At the moment the only way to change Brindle's look is to edit `config.toml`. Users have to know the built-in theme names (or run `--list-themes`), and the only way to see a theme is to save the file and wait for live reload. Writing a custom theme means typing 20-odd hex colors blind. A settings dialog makes this discoverable and visual. The theme is the first setting it covers, and other settings can be added to it later.

## What Changes

- Add an **Open Settings** action, listed in the command palette and bound by default to `ctrl-shift-,`. GPUI reports that key as `ctrl-<` on US layouts, so both spellings are bound, as with `ctrl-+`/`ctrl-shift-=`. It opens a settings dialog as a modal overlay over the current window, as the palettes do.
- The dialog has a list of sections. This change adds one section, **Theme**, built so later sections (fonts, cursor, profiles, and so on) can sit beside it.
- **Theme section:**
  - It lists the built-in themes and the user's `[themes.*]` themes, marking the active one and which themes are custom.
  - Moving the selection **previews** that theme at once on every tab in the window that follows the global theme. The real terminal content behind the dialog shows the result.
  - **Apply** writes `theme = "<name>"` to the config. **Cancel** or escape restores the previous theme and changes nothing on disk.
- **Custom themes in the dialog:**
  - **New** copies the selected theme (built-in or custom) under a new name.
  - **Edit** opens a custom theme's colors: foreground, background, cursor, cursor text, selection background and foreground, accent, and the 16 ANSI colors.
  - Each color has a swatch and an editable `#rrggbb` field, and valid edits preview immediately. Selection foreground can be cleared, which means "keep each cell's own color". Cursor text and accent always hold an explicit color, pre-filled with the copied theme's effective value. An omitted key in `[themes.*]` falls back to `brindle-dark`'s value rather than a derived one, so the dialog writes them out.
  - **Rename** and **Delete** apply to custom themes. A custom theme can't take a built-in theme's name, because it would shadow the built-in. A rename updates references to the theme in the file. Delete refuses while the theme is in use.
  - Built-in themes are read-only, so editing one means copying it first.
- **Saving edits `config.toml` in place.** The user's comments, ordering and formatting are kept, and only the keys the dialog owns change: top-level `theme` and the `[themes.<name>]` tables (plus `profiles[].theme` on rename). The write goes to `$BRINDLE_CONFIG` when that is set, and the config is reloaded right away instead of waiting for the file watcher.
- If the config file is not valid TOML, the dialog still previews but refuses to save, and says why.
- The scope is the global theme only. Per-profile theme selection is left for a later change.

## Capabilities

### New Capabilities

- `settings-dialog`: the in-window settings overlay. Covers how it opens and closes, its sections, live theme preview and revert, the custom-theme editor (create, edit, rename, delete), and how it saves to the config.

### Modified Capabilities

- `configuration`: adds a requirement that, when Brindle writes the config file itself, it keeps everything it doesn't own (comments, formatting, other keys and tables) and refuses to write a file that isn't valid TOML. The existing requirements are unchanged.

## Impact

- **Code:**
  - a new `src/settings_dialog.rs` (view, editor state, preview);
  - a new `src/config_edit.rs` (pure `toml_edit` document edits, unit-tested);
  - `workspace.rs`: overlay slot, preview override applied to tabs that follow the global theme, and exclusivity with the palettes;
  - `actions.rs`: `OpenSettings` plus dialog navigation actions in a `SettingsDialog` key context;
  - `main.rs`: an immediate reload after a save;
  - the single-line text input is shared with `picker.rs`.
- **Dependencies:** `toml_edit = "0.22"`. That version is already in `Cargo.lock` through `toml` 0.8, so no new crate is fetched.
- **Specs:** a new `settings-dialog` spec and a delta to `configuration`.
- **Tests:**
  - unit tests for the config edits (comment preservation, upsert, rename, delete, malformed input);
  - an e2e case that opens the dialog with `--action`, moves the selection, and samples the window's background color to check the preview and the revert.
