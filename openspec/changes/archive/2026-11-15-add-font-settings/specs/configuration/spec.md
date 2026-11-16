## ADDED Requirements

### Requirement: UI font
The config SHALL accept a `[ui_font]` table whose `family` is a list of font families in order of preference. The window chrome (the tab strip, zone headers, find bar, overlays and dialogs) SHALL use the first installed family from `ui_font.family`. `ui_font` SHALL have no size. The terminal font SHALL remain `[font]`, unaffected by `[ui_font]`.

#### Scenario: Chosen UI font installed
- **WHEN** the config has `ui_font.family = ["Inter", "Segoe UI"]` and Inter is installed
- **THEN** the tab strip, menus and dialogs are drawn in Inter, and the terminals keep the `[font]` family

### Requirement: UI font fallback
When `ui_font.family` is empty or none of its families is installed, the chrome SHALL use the first installed family from the built-in list (Inter, Cantarell, Ubuntu Sans, Ubuntu, Noto Sans, DejaVu Sans, Liberation Sans, Segoe UI Variable Text, Segoe UI), and when none of those is installed either, the platform's UI font.

#### Scenario: Chosen UI font missing
- **WHEN** the config has `ui_font.family = ["Nonexistent"]`
- **THEN** the chrome uses the first installed family from the built-in list, and no error is shown

#### Scenario: Unknown key in ui_font
- **WHEN** the config has `ui_font.size = 12`
- **THEN** Brindle starts with defaults and shows a config error banner naming the unknown key

## MODIFIED Requirements

### Requirement: Live reload
Saving the config file SHALL reload it automatically, within about a second, and apply the result to open windows:
- theme changes;
- profile theme and font-size changes;
- keybindings;
- fonts, the terminal font (`[font]`) and the UI font (`[ui_font]`).

ctrl-shift-r SHALL force a reload, and ctrl-, SHALL open the config in `$VISUAL` or `$EDITOR` in a new tab.

#### Scenario: Theme change while attached
- **WHEN** the user changes `theme` and saves while tabs are open
- **THEN** all of them redraw with the new theme without being restarted

#### Scenario: UI font change
- **WHEN** the user changes `ui_font.family` and saves
- **THEN** the chrome redraws in the new family without a restart, and the terminals are unchanged
