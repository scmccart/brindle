## MODIFIED Requirements

### Requirement: Chrome ignores per-tab themes
The window's token set SHALL come from the global (window default) theme only. A profile that sets its own theme SHALL change its terminal, and its agent zones' token set, not the window's token set.

#### Scenario: Profile theme tab active
- **WHEN** the active tab's profile sets `theme = "gruvbox-dark"` and the global theme is `tokyo-night`
- **THEN** component tokens stay derived from `tokyo-night`

#### Scenario: Agent profile theme
- **WHEN** the active tab is an agent tab whose profile sets `theme = "gruvbox-dark"` and the global theme is `tokyo-night`
- **THEN** the tab strip, the find bar and every overlay outside the agent zone are drawn with `tokyo-night`'s tokens, and the agent zone with the zone token set of `gruvbox-dark`

### Requirement: Legible token pairs
For every built-in theme, in both its window token set and its zone token set, the foreground on background, muted foreground on background, primary foreground on primary, destructive foreground on destructive, foreground on muted, link on card, link on background and muted foreground on card pairs SHALL each reach a contrast ratio of at least 4.5:1 (3:1 for muted foreground on background and on card).

#### Scenario: Every built-in theme
- **WHEN** tokens are derived from each built-in theme
- **THEN** every listed pair meets its minimum contrast

#### Scenario: Every built-in theme's zone set
- **WHEN** zone tokens are derived from each built-in theme
- **THEN** every listed pair meets its minimum contrast

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

## ADDED Requirements

### Requirement: Zone token sets
A theme SHALL also give a zone token set, derived as its window token set is but over the theme's own background: its background token SHALL be the theme's background, and its card, muted, accent and border colours mixes of that background and the theme's foreground. Its primary, ring and link SHALL come from the theme's accent and its destructive from the theme's red, as in the window set.

#### Scenario: Background is the theme's
- **WHEN** zone tokens are derived from a theme whose background is `#282020`
- **THEN** the zone set's background token is `#282020`, while the window set's is that background mixed towards black

#### Scenario: Accent carries over
- **WHEN** zone tokens are derived from a theme whose accent is `#7aa2f7`
- **THEN** the zone set's primary and ring are `#7aa2f7`
