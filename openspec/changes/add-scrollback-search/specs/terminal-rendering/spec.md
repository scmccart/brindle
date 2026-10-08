# Spec Delta

## ADDED Requirements

### Requirement: Search match highlight
While a terminal's find bar is open, cells inside a search match SHALL be drawn with the theme's search match background and foreground, and cells inside the current match with the theme's focused match colors. A selected cell SHALL keep the selection colors even inside a match. Highlights SHALL follow the text when the view scrolls.

#### Scenario: Matches highlighted
- **WHEN** the user searches for `needle` and two lines on screen contain it
- **THEN** both `needle`s are drawn in the search colors, the current one in the focused match colors

#### Scenario: Selection wins
- **WHEN** the user selects text that contains a match
- **THEN** the selected cells are drawn in the selection colors
