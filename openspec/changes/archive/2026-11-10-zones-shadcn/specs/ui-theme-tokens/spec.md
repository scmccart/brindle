## MODIFIED Requirements

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
