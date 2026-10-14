# Spec Delta

## MODIFIED Requirements

### Requirement: Commands listed for the focused tab
The palette SHALL list the commands that apply to the tab that had focus when it opened, each with a readable name. Commands that do not apply to that tab SHALL NOT be listed: for example, tmux commands SHALL appear only for control-mode tabs, except "tmux: New Session", which is listed in every tab while the config has a control-mode profile. Commands that only work inside a palette, and the "none" action, SHALL NOT be listed. The list SHALL be in a stable order.

#### Scenario: Local tab
- **WHEN** a local shell tab is active and the user opens the palette
- **THEN** commands such as "Reload Config" and "Clear Scrollback" are listed, and no tmux command other than "tmux: New Session" is listed

#### Scenario: Control-mode tab
- **WHEN** a control-mode tab is active and the user opens the palette
- **THEN** tmux commands such as "tmux: Split Right" and "tmux: Rename Window" are listed alongside the general ones
