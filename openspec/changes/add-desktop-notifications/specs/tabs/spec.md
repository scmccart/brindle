# Spec Delta

## MODIFIED Requirements

### Requirement: Bell indicator
A bell from a tab that is not active, or from any tab while the window is inactive, SHALL mark that tab until the user activates it. This SHALL include bells from any pane of a tmux control-mode tab, including tabs for tmux windows that are not current.

#### Scenario: Background bell
- **WHEN** a background tab rings the bell
- **THEN** a dot appears on that tab and disappears when the tab is activated

#### Scenario: Bell in a background tmux window
- **WHEN** a pane in a tmux window whose tab is not active rings the bell
- **THEN** a dot appears on that window's tab and disappears when the tab is activated
