## ADDED Requirements

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
