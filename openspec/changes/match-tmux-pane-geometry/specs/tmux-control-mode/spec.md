# Spec Delta

## ADDED Requirements

### Requirement: Panes match tmux's pane geometry
Each pane SHALL be drawn at the position and size tmux gives the pane itself, not the layout cell around it, so that rows tmux reserves for border status lines (`pane-border-status top` or `bottom`) are not part of the pane's grid. When a pane's geometry changes without a layout change, Brindle SHALL resize the pane and show tmux's current contents for it within about a second.

#### Scenario: Attach to a window with border status lines
- **WHEN** a two-pane window has `pane-border-status top`, and the user attaches
- **THEN** each pane's grid has the height tmux reports for it, one row less than its layout cell, and its text lines up with `capture-pane`

#### Scenario: Border status turned on while attached
- **WHEN** the user runs `seq 1 60` in a pane, and `set -w pane-border-status top` is then run in the attached window
- **THEN** within about a second the pane shows the same lines as `capture-pane` for that pane, with a blank row above it

#### Scenario: Full-screen program after the change
- **WHEN** a full-screen program is running in a pane and border status lines are turned on
- **THEN** after the program redraws, Brindle's pane matches tmux's screen row for row, with no stale or doubled lines

#### Scenario: No border status lines
- **WHEN** `pane-border-status` is `off`
- **THEN** panes fill their layout cells exactly as before
