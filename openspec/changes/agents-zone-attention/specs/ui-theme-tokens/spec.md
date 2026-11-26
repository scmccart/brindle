## MODIFIED Requirements

### Requirement: Legible token pairs
For every built-in theme, the foreground on background, muted foreground on background, primary foreground on primary, destructive foreground on destructive, foreground on muted, link on card, link on background and muted foreground on card pairs SHALL each reach a contrast ratio of at least 4.5:1 (3:1 for muted foreground on background and on card). The card foreground on the destructive tint (the card with the destructive colour laid over it at the tint's strength) SHALL reach 4.5:1, and the destructive colour on that tint 3:1.

#### Scenario: Every built-in theme
- **WHEN** tokens are derived from each built-in theme
- **THEN** every listed pair meets its minimum contrast

#### Scenario: A pending permission's text
- **WHEN** a pending permission row is drawn in any built-in theme
- **THEN** its title on the tinted background reaches 4.5:1, and its warning icon and border 3:1
