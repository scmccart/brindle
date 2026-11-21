# Spec Delta

## ADDED Requirements

### Requirement: Tab progress bar
A tab SHALL show a 2-pixel bar along the bottom edge of its face while any of its terminals has progress (see `program-progress`), showing the most pressing one: error, then paused, then normal (the lowest percent among them), then indeterminate.

#### Scenario: Background download
- **WHEN** a program in a background tab reports `ESC ] 9 ; 4 ; 1 ; 25 BEL`
- **THEN** that tab's face shows a muted bar along its bottom edge a quarter of its width long

#### Scenario: Error wins across zones
- **WHEN** one zone of a tab reports normal at 80% and another reports `ESC ] 9 ; 4 ; 2 ; 10 BEL`
- **THEN** the tab shows a red bar 10% of its width long

#### Scenario: Gone after the prompt
- **WHEN** a tab shows a progress bar and its shell prints a new prompt
- **THEN** the bar is gone

### Requirement: Progress bar appearance
A normal, error or paused bar SHALL fill its percent of the width from the left, in the muted foreground, the theme's red (ANSI 1) or its yellow (ANSI 3) respectively. An indeterminate bar SHALL span the whole width in the muted foreground at 40% opacity, without animation. The bar SHALL NOT change the face's size.

#### Scenario: Paused
- **WHEN** a program reports `ESC ] 9 ; 4 ; 4 ; 50 BEL`
- **THEN** its tab shows a yellow bar half its face's width long

#### Scenario: Indeterminate
- **WHEN** a program reports `ESC ] 9 ; 4 ; 3 BEL`
- **THEN** its tab shows a faint bar across its whole width, and the face is the same height as without one
