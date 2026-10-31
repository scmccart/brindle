# Spec Delta

## MODIFIED Requirements

### Requirement: Commands listed for the focused tab
The palette SHALL list the commands that apply to the tab that had focus when it opened, each with a readable name. Commands that do not apply to that tab SHALL NOT be listed: for example, zone commands ("Split Zone Right", "Split Zone Down", "Close Zone", "Focus Zone Left", "Focus Zone Right", "Focus Zone Up", "Focus Zone Down") SHALL appear only for local tabs, and tmux commands SHALL appear only for control-mode tabs, except "tmux: New Session", which is listed in every tab while the config has a control-mode profile. Commands that only work inside a palette, and the "none" action, SHALL NOT be listed. The list SHALL be in a stable order. The general `Rename Tab` command SHALL be listed for both local and control-mode tabs.

#### Scenario: Local tab
- **WHEN** a local shell tab is active and the user opens the palette
- **THEN** commands such as "Reload Config", "Clear Scrollback", "Rename Tab" and "Split Zone Right" are listed, and no tmux command other than "tmux: New Session" is listed

#### Scenario: Control-mode tab
- **WHEN** a control-mode tab is active and the user opens the palette
- **THEN** tmux commands such as "tmux: Split Right" and "tmux: Rename Window" are listed alongside general commands such as "Rename Tab", and no zone commands are listed
