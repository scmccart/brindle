## MODIFIED Requirements

### Requirement: Legible token pairs
For every built-in theme, the foreground on background, muted foreground on background, primary foreground on primary, destructive foreground on destructive, foreground on muted, link on card, link on background and muted foreground on card pairs SHALL each reach a contrast ratio of at least 4.5:1 (3:1 for muted foreground on background and on card).

#### Scenario: Every built-in theme
- **WHEN** tokens are derived from each built-in theme
- **THEN** every listed pair meets its minimum contrast

## ADDED Requirements

### Requirement: A link colour
The token set SHALL have a link colour: the theme's accent, pushed towards black or white only as far as it takes to reach 4.5:1 on the card colour.

#### Scenario: Accent already legible
- **WHEN** the window's default theme is `brindle-dark`, whose accent `#e6a756` reads at more than 4.5:1 on its card
- **THEN** the link colour is `#e6a756`

#### Scenario: Accent too light
- **WHEN** the window's default theme is `brindle-light`, whose accent `#b56a12` reads at under 4.5:1 on its card
- **THEN** the link colour is a darker mix of `#b56a12` that reads at 4.5:1 or more on the card
