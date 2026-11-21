# configuration Specification

## Purpose
Give users one TOML file controlling fonts, themes, profiles, cursor, clipboard behaviour and keybindings. The file is safe to edit while Brindle is running, and mistakes never prevent startup.

## Requirements

### Requirement: Config location and first run
The config SHALL be read from `$BRINDLE_CONFIG` when set. Otherwise it SHALL be read from `brindle/config.toml` in the platform's user config directory:
- `$XDG_CONFIG_HOME` on Linux;
- `%APPDATA%` on Windows.

If the file does not exist, a commented default config SHALL be written there on first launch. If that write fails, Brindle SHALL still start with the defaults, and the window's config line SHALL name the path and the cause.

#### Scenario: First launch
- **WHEN** Brindle starts and no config file exists
- **THEN** a commented default config is written and Brindle starts with those settings

#### Scenario: Config on Windows
- **WHEN** Brindle starts on Windows with `BRINDLE_CONFIG` unset
- **THEN** it reads, or on first launch writes, `%APPDATA%\brindle\config.toml`

#### Scenario: Default config can't be written
- **WHEN** `BRINDLE_CONFIG` names a file in a directory Brindle can't create, and Brindle starts
- **THEN** the window opens with the defaults, and its config line says the default config could not be written to that path, and why

### Requirement: Invalid config falls back to defaults
Unknown keys, wrong types and malformed TOML SHALL be reported. Brindle SHALL still start, using the defaults, and show a banner (the window's config line) describing the error until the config is fixed. Out-of-range font sizes and line heights SHALL be clamped. Where Brindle uses something other than what a valid file says (a clamped value, an unknown theme, a rejected keybinding, a removed key it ignores), the window's config line SHALL name each one and what was used instead, until the file is fixed.

#### Scenario: Typo in a key
- **WHEN** the config contains `fontsize = 3`
- **THEN** Brindle starts with defaults and shows a config error banner naming the problem

#### Scenario: Font size out of range
- **WHEN** the config sets `font.size = 200` and is saved while Brindle runs
- **THEN** the terminals use size 96, and the config line says `font.size` 200 is out of range and 96 is used

#### Scenario: Fixed
- **WHEN** the config line names a clamped `font.size`, and the user sets `font.size = 14` and saves
- **THEN** the config line goes away

### Requirement: Live reload
Saving the config file SHALL reload it automatically, within about a second, and apply the result to open windows:
- theme changes;
- profile theme and font-size changes;
- keybindings;
- fonts, the terminal font (`[font]`) and the UI font (`[ui_font]`).

ctrl-shift-r SHALL force a reload, and ctrl-, SHALL open the config in `$VISUAL` or `$EDITOR` in a new tab. That tab SHALL run on this machine, where the config file is, whatever endpoint the default profile targets.

#### Scenario: Theme change while attached
- **WHEN** the user changes `theme` and saves while tabs are open
- **THEN** all of them redraw with the new theme without being restarted

#### Scenario: UI font change
- **WHEN** the user changes `ui_font.family` and saves
- **THEN** the chrome redraws in the new family without a restart, and the terminals are unchanged

#### Scenario: Default profile on a remote host
- **WHEN** the default profile targets SSH host `A8`, another profile is local, and the user presses ctrl-,
- **THEN** a new tab runs the editor on this machine with this machine's config file, and nothing is started on `A8`

### Requirement: Themes
Brindle SHALL ship the themes `brindle-dark`, `brindle-light`, `tokyo-night`, `gruvbox-dark` and `solarized-dark`. Users SHALL be able to define themes under `[themes.<name>]`; any color a custom theme leaves out SHALL be inherited from `brindle-dark`. Colors SHALL accept `#rrggbb` and `#rgb`. An unknown theme name SHALL fall back to `brindle-dark`, and the window's config line SHALL name the unknown theme and the profile that names it, if any. Window chrome colors SHALL be derived from the active theme.

#### Scenario: Partial custom theme
- **WHEN** a custom theme sets only `background = "#000000"`
- **THEN** the background is black and every other color matches `brindle-dark`

#### Scenario: Misspelled theme
- **WHEN** the config sets `theme = "tokyonight"`
- **THEN** the window uses `brindle-dark`, and its config line says theme `tokyonight` is unknown and `brindle-dark` is used

#### Scenario: Misspelled profile theme
- **WHEN** profile `Work` sets `theme = "gruvbox"`
- **THEN** `Work`'s tabs use `brindle-dark`, and the config line names theme `gruvbox` and profile `Work`

### Requirement: Behaviour settings
The config SHALL control:
- scrollback length;
- grid padding;
- cursor shape and blinking;
- `copy_on_select`;
- `right_click`: `"none"` (the default) or `"paste"` (a right-click pastes the clipboard when no text is selected, and does nothing while text is selected, as defined by `terminal-emulation`);
- `osc52_paste` (whether programs may read the clipboard);
- `mark_failed_commands` (whether failed commands get a gutter mark; on by default).

A reload SHALL apply `copy_on_select` and `right_click` to open tabs as well as new ones.

Session restore is always on (see `session-restore`). A config that still sets `restore_session` SHALL load, with the key ignored and a warning logged that it no longer has an effect.

Brindle has no tmux support. A config that still sets `tmux_session_end` at the top level SHALL load, with the key ignored and a warning logged that tmux support was removed, whatever its value.

#### Scenario: Blinking disabled
- **WHEN** `cursor.blink = false`
- **THEN** the focused cursor never blinks

#### Scenario: Right-click paste enabled
- **WHEN** the config contains `right_click = "paste"`
- **THEN** a right-click in a terminal that isn't reporting the mouse and has no text selected pastes the clipboard, and one while text is selected does nothing

#### Scenario: Unknown right-click value
- **WHEN** the config contains `right_click = "menu"`
- **THEN** Brindle starts with defaults and shows a config error banner saying `menu` isn't one of `none` or `paste`

#### Scenario: Session restore enabled
- **WHEN** the config contains `restore_session = true`
- **THEN** Brindle loads the config without an error banner, logs that `restore_session` is ignored, and a launch restores as it always does

#### Scenario: Old restore setting
- **WHEN** the config contains `restore_session = false`
- **THEN** Brindle loads the config without an error banner, logs that `restore_session` is ignored, and a launch still restores

#### Scenario: Failed-command marks disabled
- **WHEN** `mark_failed_commands = false` and the config is reloaded
- **THEN** existing failed-command marks disappear from every terminal

#### Scenario: Unknown session-end value
- **WHEN** the config contains `tmux_session_end = "ask"` or `tmux_session_end = "shell"`
- **THEN** Brindle loads the config without an error banner and logs that `tmux_session_end` is ignored because tmux support was removed

#### Scenario: Other unknown keys still fail
- **WHEN** the config contains a top-level key that no version of Brindle defined, such as `tmux_sesion_end`
- **THEN** Brindle starts with defaults and shows a config error banner naming the key

### Requirement: Keybinding overrides
`[keybindings]` SHALL map key combinations to named actions. User bindings SHALL take precedence over the defaults, and binding a key to `"none"` SHALL disable the default binding so the key reaches the program. A binding to an unknown action name, or with keys that can't be parsed, SHALL be ignored, and the window's config line SHALL name it and why. The set of bindable actions SHALL be listable.

#### Scenario: Free a key for the program
- **WHEN** the user binds `"ctrl-shift-t" = "none"`
- **THEN** pressing ctrl-shift-t no longer opens a tab and the key is sent to the program

#### Scenario: Unknown action
- **WHEN** the config binds `"ctrl-shift-y" = "split_zone_rihgt"`
- **THEN** every other binding works, and the config line says `ctrl-shift-y` is bound to an unknown action `split_zone_rihgt`

### Requirement: Writes from Brindle preserve the file
When Brindle writes the config file itself, for example from the settings dialog, it SHALL write to the same path it reads (`$BRINDLE_CONFIG` when set). It SHALL change only the keys and tables the write concerns, keeping comments, ordering, formatting and every other key as they were. It SHALL NOT write a file that is not valid TOML. A write SHALL replace the file in one step, so a failed or interrupted write never leaves a partial file.

#### Scenario: Comments survive
- **WHEN** the config has comments and a `[keybindings]` table, and the user applies `tokyo-night` from the settings dialog
- **THEN** only the `theme` value changes; the comments and `[keybindings]` are unchanged

#### Scenario: Malformed file left alone
- **WHEN** the config file has a TOML syntax error and Brindle is asked to write it
- **THEN** the write is refused with the parse error and the file is unchanged

#### Scenario: First write with no file
- **WHEN** the config file does not exist and the user applies a theme
- **THEN** the file is created with the default config contents and the new `theme` value

### Requirement: Search highlight colors
Themes SHALL accept `search_match_background`, `search_match_foreground`, `search_focused_match_background` and `search_focused_match_foreground`. When a theme leaves one out, it SHALL be derived from that theme's own colors: matches on a background halfway between its yellow and its background, in its foreground; the focused match in its background color on its yellow.

#### Scenario: Theme sets the colors
- **WHEN** a custom theme sets `search_focused_match_background = "#ff00ff"`
- **THEN** the current search match is drawn on magenta

#### Scenario: Derived colors
- **WHEN** a custom theme sets `ansi` with yellow `#c4a000` and no search colors
- **THEN** the current search match is drawn on `#c4a000`, in the theme's background color

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

### Requirement: Transient window notices
Besides config errors and findings, the window's config line SHALL show transient notices, such as a link Brindle didn't open. Each SHALL go away 10 seconds after it was shown, or when the user dismisses it with the config line's dismiss button or the `dismiss_notices` action, whichever comes first. Dismissing SHALL leave config errors and findings shown.

#### Scenario: Notice expires
- **WHEN** a link refusal is shown on the config line and the user does nothing
- **THEN** 10 seconds later the config line no longer shows it

#### Scenario: Dismiss keeps config errors
- **WHEN** the config names an unknown theme and a link refusal is shown, and the user runs `dismiss_notices`
- **THEN** the config line still names the unknown theme, and no longer shows the refusal

#### Scenario: Repeated and many notices
- **WHEN** the user ctrl-clicks the same refused link twice, then three other refused links
- **THEN** the first link's refusal shows once, and after the fourth different refusal only the three latest show
