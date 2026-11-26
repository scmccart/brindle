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
The window's token set SHALL come from the global (window default) theme only. A profile that sets its own theme SHALL change its terminal, and its agent zones' token set, not the window's token set.

#### Scenario: Profile theme tab active
- **WHEN** the active tab's profile sets `theme = "gruvbox-dark"` and the global theme is `tokyo-night`
- **THEN** component tokens stay derived from `tokyo-night`

#### Scenario: Agent profile theme
- **WHEN** the active tab is an agent tab whose profile sets `theme = "gruvbox-dark"` and the global theme is `tokyo-night`
- **THEN** the tab strip, the find bar and every overlay outside the agent zone are drawn with `tokyo-night`'s tokens, and the agent zone with the zone token set of `gruvbox-dark`

### Requirement: Legible token pairs
For every built-in theme, in both its window token set and its zone token set, the foreground on background, muted foreground on background, primary foreground on primary, destructive foreground on destructive, foreground on muted, link on card, link on background and muted foreground on card pairs SHALL each reach a contrast ratio of at least 4.5:1 (3:1 for muted foreground on background and on card). The card foreground on the destructive tint (the card with the destructive colour laid over it at the tint's strength) SHALL reach 4.5:1, and the destructive colour on that tint 3:1.

#### Scenario: Every built-in theme
- **WHEN** tokens are derived from each built-in theme
- **THEN** every listed pair meets its minimum contrast

#### Scenario: Every built-in theme's zone set
- **WHEN** zone tokens are derived from each built-in theme
- **THEN** every listed pair meets its minimum contrast

#### Scenario: A pending permission's text
- **WHEN** a pending permission row is drawn in any built-in theme
- **THEN** its title on the tinted background reaches 4.5:1, and its warning icon and border 3:1

### Requirement: Window chrome uses the tokens
The tab strip, the find bar, the window border and the drag preview of a tab SHALL take their colours from the active token set: background, border, foreground, muted foreground, accent and primary. They SHALL NOT follow any tab's profile theme. The exceptions SHALL be each tab's face in the strip, active or not, drawn in that tab's terminal colours as the `tabs` capability specifies, each zone's card and header bar, drawn in that zone's terminal colours as the `zones` capability specifies, with the card's border derived from them, and each agent zone's contents and header buttons, drawn in that zone's token set as the `agent-zones` capability specifies. The ring colour and the program status colours SHALL stay those of the tokens and the global theme.

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

### Requirement: A link colour
The token set SHALL have a link colour: the theme's accent, pushed towards black or white only as far as it takes to reach 4.5:1 on the card colour.

#### Scenario: Accent already legible
- **WHEN** the window's default theme is `brindle-dark`, whose accent `#e6a756` reads at more than 4.5:1 on its card
- **THEN** the link colour is `#e6a756`

#### Scenario: Accent too light
- **WHEN** the window's default theme is `brindle-light`, whose accent `#b56a12` reads at under 4.5:1 on its card
- **THEN** the link colour is a darker mix of `#b56a12` that reads at 4.5:1 or more on the card

### Requirement: Zone token sets
A theme SHALL also give a zone token set, derived as its window token set is but over the theme's own background: its background token SHALL be the theme's background, and its card, muted, accent and border colours mixes of that background and the theme's foreground. Its primary, ring and link SHALL come from the theme's accent and its destructive from the theme's red, as in the window set.

#### Scenario: Background is the theme's
- **WHEN** zone tokens are derived from a theme whose background is `#282020`
- **THEN** the zone set's background token is `#282020`, while the window set's is that background mixed towards black

#### Scenario: Accent carries over
- **WHEN** zone tokens are derived from a theme whose accent is `#7aa2f7`
- **THEN** the zone set's primary and ring are `#7aa2f7`

### Requirement: Diff colours
A theme's window and zone token sets SHALL each include a background colour for added lines and one for removed lines: that set's muted colour mixed towards the theme's green, and towards its red, only as far as keeps the set's foreground at a contrast of at least 4.5:1 on it. Both SHALL follow a theme change, and an agent zone SHALL draw its diffs in its zone set's. The built-in neutral token sets SHALL carry a green and a red tint that meet the same contrast.

#### Scenario: Every built-in theme
- **WHEN** the window and zone tokens are derived from each built-in theme
- **THEN** the foreground on the added colour and on the removed colour each has a contrast of at least 4.5:1, in both sets

#### Scenario: Distinct from the muted colour
- **WHEN** the tokens are derived from each built-in theme
- **THEN** the added colour, the removed colour and the muted colour are three different colours

#### Scenario: Theme change
- **WHEN** the theme changes from a dark to a light built-in
- **THEN** an open diff's added and removed lines take the new theme's colours

#### Scenario: A profile theme unlike the window's
- **WHEN** an agent profile sets a dark theme while the global theme is light, and its zone shows a diff
- **THEN** the diff's added and removed lines are drawn in the dark theme's zone token set's colours
