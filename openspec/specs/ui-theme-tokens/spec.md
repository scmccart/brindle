# ui-theme-tokens Specification

## Purpose
Makes Brindle's interface components take their colours from the active terminal theme, so dialogs, palettes and chrome match the terminal and follow every theme change the terminal does.

## Requirements

### Requirement: Component tokens derive from the terminal theme
The interface component token set SHALL be derived from the window's default terminal theme: its background and foreground become the token background and foreground, its accent becomes primary and ring, and its red becomes destructive. Borders, muted and accent surfaces SHALL be mixes of the theme's background and foreground. A dark or light token set SHALL be chosen from the theme's background.

#### Scenario: Accent follows the theme
- **WHEN** the window's default theme has accent `#7aa2f7`
- **THEN** the primary and ring token colours are `#7aa2f7`

#### Scenario: Dark hint
- **WHEN** the default theme's background is dark
- **THEN** the token set is marked dark

### Requirement: Tokens follow theme changes
The active token set SHALL be rebuilt at startup, when the config is reloaded, and while the settings dialog previews a theme, so open components redraw in the new theme without a restart. Cancelling a preview SHALL restore the tokens of the theme from before it.

#### Scenario: Live preview re-themes dialogs
- **WHEN** the user moves the theme selection from `brindle-dark` to a light theme in the settings dialog
- **THEN** the dialog's own controls and every other component redraw with tokens from that theme at once

#### Scenario: Config reload
- **WHEN** the config file's `theme` changes while Brindle runs
- **THEN** component colours change to the new theme without other action

### Requirement: Chrome ignores per-tab themes
The token set SHALL come from the global (window default) theme only. A profile that sets its own theme SHALL change its terminal, not the component tokens.

#### Scenario: Profile theme tab active
- **WHEN** the active tab's profile sets `theme = "gruvbox-dark"` and the global theme is `tokyo-night`
- **THEN** component tokens stay derived from `tokyo-night`

### Requirement: Legible token pairs
For every built-in theme, the foreground on background, muted foreground on background, primary foreground on primary, and destructive foreground on destructive pairs SHALL each reach a contrast ratio of at least 4.5:1 (3:1 for muted foreground on background).

#### Scenario: Every built-in theme
- **WHEN** tokens are derived from each built-in theme
- **THEN** every listed pair meets its minimum contrast
