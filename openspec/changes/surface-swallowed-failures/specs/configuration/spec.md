# Spec Delta

## MODIFIED Requirements

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

### Requirement: Keybinding overrides
`[keybindings]` SHALL map key combinations to named actions. User bindings SHALL take precedence over the defaults, and binding a key to `"none"` SHALL disable the default binding so the key reaches the program. A binding to an unknown action name, or with keys that can't be parsed, SHALL be ignored, and the window's config line SHALL name it and why. The set of bindable actions SHALL be listable.

#### Scenario: Free a key for the program
- **WHEN** the user binds `"ctrl-shift-t" = "none"`
- **THEN** pressing ctrl-shift-t no longer opens a tab and the key is sent to the program

#### Scenario: Unknown action
- **WHEN** the config binds `"ctrl-shift-y" = "split_zone_rihgt"`
- **THEN** every other binding works, and the config line says `ctrl-shift-y` is bound to an unknown action `split_zone_rihgt`
