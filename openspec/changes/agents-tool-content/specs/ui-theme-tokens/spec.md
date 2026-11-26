## ADDED Requirements

### Requirement: Diff colours
The component tokens SHALL include a background colour for added lines and one for removed lines, derived from the terminal theme: the card colour mixed with the theme's green, and with its red. The foreground colour SHALL be legible on each at a contrast of at least 4.5:1 for every built-in theme, and each SHALL follow a theme change as the other tokens do. The built-in neutral token sets SHALL carry a green and a red tint of their own that meet the same contrast.

#### Scenario: Every built-in theme
- **WHEN** the tokens are derived from each built-in theme
- **THEN** the foreground on the added colour and on the removed colour each has a contrast of at least 4.5:1

#### Scenario: Distinct from the card
- **WHEN** the tokens are derived from each built-in theme
- **THEN** the added colour, the removed colour and the card colour are three different colours

#### Scenario: Theme change
- **WHEN** the theme changes from a dark to a light built-in
- **THEN** an open diff's added and removed lines take the new theme's colours
