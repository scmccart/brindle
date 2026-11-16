# Design

## Context

- `Settings` (`src/settings.rs`) resolves the terminal font once per config load (`resolve_family`, `Settings::font`, `font_size`) and the chrome font (`resolve_ui_family`, from a built-in list only). `Workspace::render` sets the chrome font with `.font_family(Settings::get(cx).ui_font_family)`; everything in the window inherits it.
- The settings dialog (`src/settings_dialog.rs`) keeps a GPUI-free `Model` with one state per section (`Section::{Profiles, Theme}`; profiles in `settings_dialog/profiles.rs`), and a view that emits `SettingsEvent::Preview`/`ProfilePreview` for the workspace to show. Writes go through `config_edit::save`, which keeps comments.

## Decisions

### 1. `[ui_font]` mirrors `[font]`'s `family`, and nothing else

`UiFontConfig { family: Vec<String> }`, `#[serde(default, deny_unknown_fields)]`, defaulting to an empty list. Resolution is the config's list (case-insensitive, as `resolve_family` matches), then the built-in list that `resolve_ui_family` has today, then `.SystemUIFont`. So "Default" in the dialog means the same for both fonts: the `family` key is absent from the file (for `[font]`, `FontConfig`'s built-in list then applies). A size was left out on purpose: chrome dimensions are shadcn's rem-based ones, and a size would have to scale all of them.

### 2. Previews are an override in `Settings`, not a second config

`Settings` gains `font_preview: Option<FontPreview>`, where `FontPreview` has the terminal family, size and line height and the UI family, each `Option`. `Settings::font()`, `font_size()`, a new `line_height()`, and a new `ui_font_family()` accessor read the preview first, then the resolved config. The terminal `Font` for a previewed family is built the same way as the configured one, with its fallbacks, by one shared function. Every place that reads `config.font.line_height` or `ui_font_family` directly moves to the accessors, so a preview reaches all of them (`cell_metrics`/`fit_grid` included).

The dialog emits `SettingsEvent::FontPreview(Option<FontPreview>)` when its preview changes. The workspace sets or clears the override and re-fits every terminal as a zoom change does (the same path as `increase_font_size`), then notifies. A config reload clears the override. Applying writes the file, clears the override and reloads at once, as Apply does for a theme, so nothing flickers back to the old font.

A profile's own `font_size` still wins over the previewed size, as it wins over `font.size`.

### 3. Names at once, the monospace probe in the background

The names come from `cx.text_system().all_font_names()` (already called on every config load) when the section opens, minus names starting with `.` or `@` (Windows' vertical-writing variants), deduplicated and sorted case-insensitively. A family is monospace when, resolved at 16px, the advances of `i` and `M` are equal. Resolving every family loads it, so only that probe runs on the background executor, the first time the Fonts section opens, with its result kept in a global for the process. Until it arrives the terminal font's list is alphabetical; when it arrives monospace families move to the top under a "Monospace" heading. Choices are kept by family name, never by index, so a reorder never changes what is chosen. A family the probe can't load counts as proportional and stays listed.

### 4. Family pickers are shadcn's Combobox

A family list has hundreds of entries. shadcn's answer for a long, searchable choice is the Combobox: a trigger button showing the choice, opening a `Popover` holding a `Command` (search input plus filtered list). brindle-ui has both, so the change adds `brindle_ui::Combobox` composing them (trigger styled as `Select`'s, the popover at the trigger's width or wider, enter or a click choosing, escape closing back to the trigger), with a gallery entry and `--- menu`-style dump support through the dialog's line. The field rows follow the profile editor's shadcn `Field` layout (`FIELD_HEIGHT`, control centred, errors beneath at natural height).

### 5. A form with Apply, like the theme list

The Fonts section has no list and no editor mode: its fields are always editable. Footer buttons: Cancel, Apply. Its focus stops after the section list are Terminal font, Size, Line height, UI font, then the footer. Enter in a text field applies, as enter saves in an editor; enter on a choice opens its menu. The terminal font and UI font fields are Comboboxes (decision 4). Leaving the section drops unapplied changes (as leaving Theme drops its preview); it doesn't hold the section, because there is no editor to lose.

The model, `FontsState` in `settings_dialog/fonts.rs`, is GPUI-free: it reads the four values from the config file (falling back to the loaded config when the file can't be parsed, read-only then, as profiles do), tracks the fields as text and choice indices, validates (sizes as the profile editor validates its font size), and gives `preview()` and `apply()`.

### 6. Writing family lists

`config_edit` gains `set_font_family(doc, table, Option<&str>)`: `Some(name)` puts `name` first in `[table].family` and keeps the other entries (minus case-insensitive duplicates of `name`); `None` removes `family`, and removes the table when it ends up empty. It creates the table when missing. Numbers go through one `set_font_number(doc, key, value)` for `[font].size` and `line_height`, written only when changed.

### 7. Dump lines

The Fonts section's `--- settings` line is `--- settings section=Fonts mode=form terminal=… size=… line_height=… ui=… invalid=[…] can_apply=… error=…`, with each value `{:?}`-quoted as the other sections' are. A new `--- fonts terminal="…" ui="…"` line, printed in every dump right after the grid line, gives the families in effect (preview included), so e2e cases can check resolution without capturing pixels. The e2e and perf dump parsers (and their `--self-test`s) must accept it first.

### 8. The UI font reaches chrome drawn outside the workspace root

GPUI draws tooltips and drag previews as roots of their own, which don't inherit the `font_family` set on the workspace's root element. brindle-ui can't read `Settings`, so it gains a `UiFont` global beside `UiTheme` (`brindle_ui::UiFont::set(family, cx)`, read with `cx.ui_font()`), which the app sets wherever it sets the tokens. brindle-ui's own roots (tooltips, the dragged tab, popovers and menus if they render as their own roots) apply it, and the workspace root reads it too, so one value drives all chrome.

## Risks

- The monospace probe resolves every installed font. On a machine with many fonts that takes noticeable time, hence the background probe and the cache; the choice works before it finishes.
- It doesn't fix the low text on its own: a Windows machine with only Segoe UI keeps it until its owner installs a font with balanced metrics (Inter, say) and picks it.
- `all_font_names` on Windows can list families GPUI then fails to load; the monospace probe treats a failure as proportional and keeps the family listed.
