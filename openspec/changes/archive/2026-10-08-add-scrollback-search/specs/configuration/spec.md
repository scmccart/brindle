# Spec Delta

## ADDED Requirements

### Requirement: Search highlight colors
Themes SHALL accept `search_match_background`, `search_match_foreground`, `search_focused_match_background` and `search_focused_match_foreground`. When a theme leaves one out, it SHALL be derived from that theme's own colors: matches on a background halfway between its yellow and its background, in its foreground; the focused match in its background color on its yellow.

#### Scenario: Theme sets the colors
- **WHEN** a custom theme sets `search_focused_match_background = "#ff00ff"`
- **THEN** the current search match is drawn on magenta

#### Scenario: Derived colors
- **WHEN** a custom theme sets `ansi` with yellow `#c4a000` and no search colors
- **THEN** the current search match is drawn on `#c4a000`, in the theme's background color
