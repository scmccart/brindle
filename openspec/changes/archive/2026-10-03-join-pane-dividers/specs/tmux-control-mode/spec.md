# Spec Delta

## MODIFIED Requirements

### Requirement: Panes drawn natively
Each tab SHALL lay out its window's panes exactly on tmux's cell grid, with divider lines between panes. The part of a divider next to the active pane SHALL be highlighted. A zoomed window SHALL show only the zoomed pane, with a "zoomed" indicator.

#### Scenario: Split layout
- **WHEN** a window has one pane on the left and two stacked on the right
- **THEN** the tab shows the same three-pane arrangement with dividers between them

#### Scenario: Dividers meet cleanly
- **WHEN** dividers meet at a T-junction or cross, for example in a 2x2 tiled window
- **THEN** the lines join with no gap, and the active pane's highlight turns its corner on the lines it meets
