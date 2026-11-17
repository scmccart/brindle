# Spec Delta

## MODIFIED Requirements

### Requirement: Minimum size for splitting
Splitting the active zone SHALL do nothing when the active zone's current bounds cannot fit two minimum 10-column by 2-row terminal grids together with the gap between the new cards, each card's border and each card's header bar. No terminal grid Brindle creates or resizes SHALL be narrower than 10 columns, so a pane narrower than that is laid out as 10 columns wide. Shrinking the window or restoring a saved session SHALL NOT remove existing zones when zones become smaller than this threshold.

#### Scenario: Split rejected when zone is too small
- **WHEN** a zone is too narrow to fit two minimum terminal grids side by side and the user runs `Split Zone Right`
- **THEN** no new zone is created and the existing zones and focus remain unchanged

#### Scenario: Window shrink preserves zones
- **WHEN** a tab has four split zones and the user resizes the window to its minimum size
- **THEN** all four zones remain in the tab

#### Scenario: Card chrome counts
- **WHEN** a zone is exactly as wide as two minimum grids plus their padding, but not the gap and the borders
- **THEN** `Split Zone Right` does nothing

#### Scenario: A pane narrower than 10 columns is laid out as 10 columns wide
- **WHEN** a terminal's pane is only 4 columns wide
- **THEN** its grid, and the PTY size it reports, are 10 columns wide
