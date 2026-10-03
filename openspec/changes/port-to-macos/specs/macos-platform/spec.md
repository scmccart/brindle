# Spec Delta

## Purpose

Make Brindle a native-feeling macOS terminal: the same tabs, profiles and tmux support as on Linux, following Mac conventions for the title bar, shortcuts, menu bar, Option key, shell startup and fonts.

## ADDED Requirements

### Requirement: Runs on macOS
Brindle SHALL build and run on macOS on both Apple Silicon and Intel Macs, drawing all terminal content and window chrome with GPUI's Metal renderer. Behaviour specified for other capabilities SHALL hold on macOS unless a requirement in this capability says otherwise.

#### Scenario: Window opens on a Mac
- **WHEN** the user starts Brindle on a Mac
- **THEN** a window opens with one tab running the default profile, and the terminal grid is drawn

### Requirement: Foreground process info on macOS
On macOS, tab titles SHALL fall back to the foreground process name, and new tabs SHALL inherit the foreground process's working directory, as specified for tabs on Linux.

#### Scenario: Foreground process title
- **WHEN** the user runs `htop` in a tab whose program sets no title
- **THEN** within about a second the tab is titled `htop`

#### Scenario: New tab in the same directory
- **WHEN** the active tab's shell is in `~/src/brindle` and the user presses cmd-t
- **THEN** the new tab's shell starts in `~/src/brindle`

### Requirement: Native title bar on macOS
On macOS the window SHALL use the system's window buttons (close, minimize, zoom), shown inside the tab strip at its left end, with the tabs starting to their right. Dragging empty space in the strip SHALL move the window. Double-clicking empty space SHALL do what the system setting for double-clicking a title bar says. Brindle SHALL NOT draw its own window buttons on macOS.

#### Scenario: Traffic lights do not cover tabs
- **WHEN** a window has several tabs
- **THEN** the close, minimize and zoom buttons appear in the tab strip and no tab is drawn under them

#### Scenario: Double-click follows the system setting
- **WHEN** the system setting is "Minimize" and the user double-clicks empty space in the tab strip
- **THEN** the window minimizes to the Dock

### Requirement: Command-key shortcuts on macOS
On macOS, each default shortcut that uses ctrl-shift-KEY SHALL use cmd-KEY instead. Shortcuts that use ctrl with a punctuation key, `0` or Insert (ctrl-, ctrl-= ctrl-+ ctrl-- ctrl-0 ctrl-insert) SHALL use cmd in place of ctrl. Exceptions: the new-tab palette SHALL be cmd-shift-space, because cmd-space opens Spotlight, and ctrl-shift-tab SHALL stay, because cmd-tab switches apps. Other default shortcuts SHALL be unchanged. Plain ctrl-letter SHALL still reach the program.

#### Scenario: Mac copy and paste
- **WHEN** the user selects text and presses cmd-c, then presses cmd-v
- **THEN** the selection is copied and pasted, and nothing is sent to the program for the keys themselves

#### Scenario: Open a tab
- **WHEN** the user presses cmd-t
- **THEN** a new tab opens with the default profile

#### Scenario: Control keys still reach the shell
- **WHEN** the user presses ctrl-c
- **THEN** the program receives byte 0x03

### Requirement: Tab shortcuts on macOS
On macOS, cmd-N SHALL jump to tab N (cmd-9 is the last tab), cmd-alt-N SHALL open a tab with the Nth profile, cmd-shift-[ and cmd-shift-] SHALL switch to the previous and next tab, and in tmux control-mode tabs cmd-alt-arrow SHALL move focus between panes.

#### Scenario: Jump to a tab
- **WHEN** there are five tabs and the user presses cmd-2
- **THEN** the second tab becomes active

#### Scenario: Next tab
- **WHEN** the first of three tabs is active and the user presses cmd-shift-]
- **THEN** the second tab becomes active

### Requirement: Menu bar on macOS
On macOS, Brindle SHALL provide a menu bar with Brindle (About, Settings…, Quit), Shell (New Tab, New Window, Close Tab, the new-tab palette), Edit (Copy, Paste, Select All, Clear Scrollback), View (font size, Command Palette) and Window menus. Each item SHALL run the same action as its shortcut and show that shortcut.

#### Scenario: Quit from the menu
- **WHEN** the user chooses Brindle → Quit Brindle
- **THEN** every window closes and the application exits, as with cmd-q

#### Scenario: Settings opens the config
- **WHEN** the user chooses Brindle → Settings…
- **THEN** the config file opens in a new tab, as with cmd-,

### Requirement: Option key on macOS
On macOS, the Option key SHALL type the character the keyboard layout gives it unless the config sets `macos_option_as_meta = true`, in which case Option SHALL act as Meta by prefixing ESC, as Alt does on Linux. Option combined with special keys (arrows, Home/End, function keys) SHALL always be encoded as Alt-modified keys. The setting SHALL apply on config reload.

#### Scenario: Option composes characters by default
- **WHEN** `macos_option_as_meta` is not set, the layout is German, and the user presses option-l
- **THEN** the program receives `@`

#### Scenario: Option as Meta
- **WHEN** `macos_option_as_meta = true` and the user presses option-f
- **THEN** the program receives `ESC f`

#### Scenario: Option-arrow
- **WHEN** the user presses option-left
- **THEN** the program receives `ESC [ 1 ; 3 D`, whatever `macos_option_as_meta` is set to

### Requirement: Config location on macOS
On macOS, when `$BRINDLE_CONFIG` is not set, the config SHALL be read from `$XDG_CONFIG_HOME/brindle/config.toml`, or from `~/.config/brindle/config.toml` when `$XDG_CONFIG_HOME` is not set, and the default config SHALL be written there on first launch. `~/Library/Application Support` SHALL NOT be used.

#### Scenario: First launch on a Mac
- **WHEN** Brindle starts on a Mac with no config and no `XDG_CONFIG_HOME`
- **THEN** a commented default config is written to `~/.config/brindle/config.toml`

### Requirement: Login shell on macOS
On macOS, a profile that sets no command SHALL start the user's login shell from the user database as a login shell, the way Terminal.app does. `$SHELL` SHALL NOT override it.

#### Scenario: Login shell reads the profile
- **WHEN** the user's shell is zsh and `~/.zprofile` sets a variable, and the user opens a tab with a profile that sets no command
- **THEN** the variable is set in that tab's shell

### Requirement: PATH from the login shell on macOS
When Brindle starts on macOS outside a terminal (from Finder, the Dock, Spotlight or `open`), it SHALL take `PATH` from the user's login shell before starting any program, so profile commands and tmux resolve as they do in a terminal. If that fails or takes more than two seconds, Brindle SHALL keep its own `PATH` with `/opt/homebrew/bin` and `/usr/local/bin` appended, and log a warning.

#### Scenario: Homebrew tmux from the Dock
- **WHEN** tmux is installed with Homebrew, Brindle is opened from the Dock, and the user opens a `tmux = "control"` profile
- **THEN** Brindle attaches to the session instead of reporting that tmux could not be started

#### Scenario: Started from a terminal
- **WHEN** Brindle is started from a shell in another terminal
- **THEN** it keeps that shell's `PATH` unchanged

### Requirement: Font defaults on macOS
On macOS, when no family in `font.family` is installed, Brindle SHALL use Menlo. Glyphs missing from the terminal font SHALL fall back to the system's symbol, emoji (Apple Color Emoji) and CJK fonts.

#### Scenario: Default font on a fresh Mac
- **WHEN** none of the configured families is installed
- **THEN** text is drawn in Menlo

#### Scenario: Emoji fallback
- **WHEN** a program prints 🐕
- **THEN** the emoji is drawn in color from Apple Color Emoji
