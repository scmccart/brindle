# Spec Delta

## ADDED Requirements

### Requirement: Window chrome uses the tokens
The tab strip, the zone headers and dividers, the find bar, the window border and the drag preview of a tab SHALL take their colours from the active token set: background, border, foreground, muted foreground, accent and primary. They SHALL NOT follow the active tab's profile theme. The exception SHALL be the active tab itself, which SHALL be drawn in the active tab's terminal background and foreground, so that it joins the terminal beneath it.

#### Scenario: Profile theme tab active
- **WHEN** the active tab's profile sets `theme = "gruvbox-dark"` and the global theme is `tokyo-night`
- **THEN** the strip, the inactive tabs, the zone headers and the find bar are drawn with the tokens of `tokyo-night`, and the active tab is drawn in `gruvbox-dark`'s background and foreground

#### Scenario: Theme preview
- **WHEN** the settings dialog previews another theme
- **THEN** the strip, headers, dividers and window border redraw in the previewed theme's tokens

### Requirement: Status colours follow the global theme
The program status dots on tabs and zone headers SHALL use the global theme's ANSI colours (yellow for blocked, red for error, green for done) and the muted foreground for working, whatever the active tab's profile theme is.

#### Scenario: Blocked tab with another theme
- **WHEN** a background tab with its own theme shows a blocked program
- **THEN** its dot is the global theme's yellow
