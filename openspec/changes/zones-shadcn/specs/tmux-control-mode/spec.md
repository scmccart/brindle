## ADDED Requirements

### Requirement: A tmux window is one card
A tmux tab SHALL draw its window as one card without a header row: a 1-pixel border and rounded corners, in a border colour derived from the window's default terminal theme, inside a 6-pixel margin. The panes, tmux's dividers and the pane title lines SHALL stay as tmux's layout places them, inside the card. The grid size reported to tmux SHALL be the card's inner area.

#### Scenario: Window card
- **WHEN** a tmux window with two panes is shown
- **THEN** one card surrounds both panes and their divider, and no card is drawn per pane

#### Scenario: Size reported to tmux
- **WHEN** the window area is 1000 by 600 pixels
- **THEN** the columns and rows sent with `refresh-client -C` fit the area less the margin and the border on each side
