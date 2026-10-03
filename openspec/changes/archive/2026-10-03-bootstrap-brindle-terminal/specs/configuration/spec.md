# Spec Delta

## Purpose

Give users one TOML file controlling fonts, themes, profiles, cursor, clipboard behaviour and keybindings. The file is safe to edit while Brindle is running, and mistakes never prevent startup.

## ADDED Requirements

### Requirement: Config location and first run
The config SHALL be read from `$BRINDLE_CONFIG` when set, otherwise from `$XDG_CONFIG_HOME/brindle/config.toml`. If the file does not exist, a commented default config SHALL be written there on first launch.

#### Scenario: First launch
- **WHEN** Brindle starts and no config file exists
- **THEN** a commented default config is written and Brindle starts with those settings

### Requirement: Invalid config falls back to defaults
Unknown keys, wrong types and malformed TOML SHALL be reported. Brindle SHALL still start, using the defaults, and show a banner describing the error until the config is fixed. Out-of-range font sizes and line heights SHALL be clamped.

#### Scenario: Typo in a key
- **WHEN** the config contains `fontsize = 3`
- **THEN** Brindle starts with defaults and shows a config error banner naming the problem

### Requirement: Live reload
Saving the config file SHALL reload it automatically, within about a second, and apply the result to open windows:
- theme changes;
- profile theme and font-size changes, including in tmux panes;
- keybindings;
- fonts.

ctrl-shift-r SHALL force a reload, and ctrl-, SHALL open the config in `$VISUAL` or `$EDITOR` in a new tab.

#### Scenario: Theme change while attached
- **WHEN** the user changes `theme` and saves while tabs and tmux panes are open
- **THEN** all of them redraw with the new theme without being restarted

### Requirement: Themes
Brindle SHALL ship the themes `brindle-dark`, `brindle-light`, `tokyo-night`, `gruvbox-dark` and `solarized-dark`. Users SHALL be able to define themes under `[themes.<name>]`; any color a custom theme leaves out SHALL be inherited from `brindle-dark`. Colors SHALL accept `#rrggbb` and `#rgb`. An unknown theme name SHALL fall back to `brindle-dark` with a warning. Window chrome colors SHALL be derived from the active theme.

#### Scenario: Partial custom theme
- **WHEN** a custom theme sets only `background = "#000000"`
- **THEN** the background is black and every other color matches `brindle-dark`

### Requirement: Behaviour settings
The config SHALL control:
- scrollback length;
- grid padding;
- cursor shape and blinking;
- `copy_on_select`;
- `osc52_paste` (whether programs may read the clipboard).

#### Scenario: Blinking disabled
- **WHEN** `cursor.blink = false`
- **THEN** the focused cursor never blinks

### Requirement: Keybinding overrides
`[keybindings]` SHALL map key combinations to named actions. User bindings SHALL take precedence over the defaults, and binding a key to `"none"` SHALL disable the default binding so the key reaches the program. Unknown action names and invalid keys SHALL be logged and ignored. The set of bindable actions SHALL be listable.

#### Scenario: Free a key for the program
- **WHEN** the user binds `"ctrl-shift-t" = "none"`
- **THEN** pressing ctrl-shift-t no longer opens a tab and the key is sent to the program
