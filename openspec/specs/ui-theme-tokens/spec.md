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

### Requirement: Window chrome uses the tokens
The tab strip, the find bar, the window border and the drag preview of a tab SHALL take their colours from the active token set: background, border, foreground, muted foreground, accent and primary. They SHALL NOT follow any tab's profile theme. The exceptions SHALL be each tab's face in the strip, active or not, drawn in that tab's terminal colours as the `tabs` capability specifies, and each zone's card and header bar, drawn in that zone's terminal colours as the `zones` capability specifies, with the card's border derived from them. The ring colour and the program status colours SHALL stay those of the tokens and the global theme.

#### Scenario: Profile theme tab active
- **WHEN** the active tab's profile sets `theme = "gruvbox-dark"` and the global theme is `tokyo-night`
- **THEN** the strip's own background, the new-tab and profile buttons and the find bar are drawn with the tokens of `tokyo-night`, and the active tab's face and its zones' headers are drawn in `gruvbox-dark`'s background and foreground

#### Scenario: Background tab with its own theme
- **WHEN** a background tab's profile sets `theme = "gruvbox-dark"` and the global theme is `tokyo-night`
- **THEN** that tab's face is drawn on `gruvbox-dark`'s background with its foreground at 60% opacity, and the other tabs' faces are unaffected

#### Scenario: Zone ring stays on the tokens
- **WHEN** a zone with its own theme shows the active ring
- **THEN** the ring is drawn in the ring token of the global theme

#### Scenario: Theme preview
- **WHEN** the settings dialog previews another theme
- **THEN** the strip and window border redraw in the previewed theme's tokens

### Requirement: Status colours follow the global theme
The program status dots on tabs and zone headers SHALL use the global theme's ANSI colours (yellow for blocked, red for error, green for done) and the muted foreground for working, whatever the active tab's profile theme is.

#### Scenario: Blocked tab with another theme
- **WHEN** a background tab with its own theme shows a blocked program
- **THEN** its dot is the global theme's yellow
