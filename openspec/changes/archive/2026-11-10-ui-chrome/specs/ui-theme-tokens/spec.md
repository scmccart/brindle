# Spec Delta

## ADDED Requirements

### Requirement: Window chrome uses the tokens
The tab strip, the zone headers and dividers, the find bar, the window border and the drag preview of a tab SHALL take their colours from the active token set: background, border, foreground, muted foreground, accent and primary. They SHALL NOT follow any tab's profile theme. The one exception SHALL be each tab's face in the strip, active or not, drawn in that tab's terminal colours as the `tabs` capability specifies.

#### Scenario: Profile theme tab active
- **WHEN** the active tab's profile sets `theme = "gruvbox-dark"` and the global theme is `tokyo-night`
- **THEN** the strip's own background, the new-tab and profile buttons, the zone headers and the find bar are drawn with the tokens of `tokyo-night`, and the active tab's face is drawn in `gruvbox-dark`'s background and foreground

#### Scenario: Background tab with its own theme
- **WHEN** a background tab's profile sets `theme = "gruvbox-dark"` and the global theme is `tokyo-night`
- **THEN** that tab's face is drawn on `gruvbox-dark`'s background with its foreground at 60% opacity, and the other tabs' faces are unaffected

#### Scenario: Theme preview
- **WHEN** the settings dialog previews another theme
- **THEN** the strip, headers, dividers and window border redraw in the previewed theme's tokens

### Requirement: Status colours follow the global theme
The program status dots on tabs and zone headers SHALL use the global theme's ANSI colours (yellow for blocked, red for error, green for done) and the muted foreground for working, whatever the active tab's profile theme is.

#### Scenario: Blocked tab with another theme
- **WHEN** a background tab with its own theme shows a blocked program
- **THEN** its dot is the global theme's yellow
