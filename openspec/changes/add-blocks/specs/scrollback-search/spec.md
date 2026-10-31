# Spec Delta

## MODIFIED Requirements

### Requirement: Current match is scrolled into view
When the current match changes and is not fully visible, the view SHALL scroll so that the match is near the middle of the terminal, as far as the scrollback allows. A match inside a collapsed block SHALL expand that block. A match outside a block's body window SHALL scroll that body to show it, and only then SHALL the view scroll if the block is not on screen.

#### Scenario: Match in history
- **WHEN** the only match is 500 lines up in the scrollback
- **THEN** the view scrolls up and the match is shown near the middle of the terminal

#### Scenario: Match in a collapsed block
- **WHEN** the current match is in the hidden output of a collapsed block
- **THEN** the block expands and the match is visible

#### Scenario: Match in a scrolled body
- **WHEN** the current match is 2000 lines above the window of a tall block that is on screen
- **THEN** that block's body scrolls to show the match and the view does not move
