## MODIFIED Requirements

### Requirement: Tab faces
The strip's tab faces SHALL each be drawn on its terminal's background in its terminal's foreground, with its icons and close button in that foreground, with 4 pixels between neighbouring faces and no background behind them. The strip SHALL have no background of its own beyond the window's, no border under it, and a 6-pixel margin above and at the left of the faces, matching the margin round the zone cards below. The strip's buttons SHALL use the global tokens.

#### Scenario: Mixed themes
- **WHEN** one tab's profile sets a theme other than the global one
- **THEN** that tab's face is drawn on its own theme's background and foreground, whether active or not, and the other faces are drawn in the global theme's

#### Scenario: Faces are apart
- **WHEN** a window has two tabs
- **THEN** 4 pixels of the window background show between their faces, and none shows a shared pill behind them

#### Scenario: Strip margins
- **WHEN** a window is drawn
- **THEN** the first face starts 6 pixels from the window's left edge, and 6 pixels of window background are above the faces and between them and the zone cards

#### Scenario: Active tab marker
- **WHEN** a tab is active
- **THEN** its face is raised with a shadow and its text is at full opacity, and no line runs along its top edge

#### Scenario: Inactive tab
- **WHEN** a tab is inactive and not hovered
- **THEN** its text is its foreground at 60% opacity, and at full opacity while hovered
