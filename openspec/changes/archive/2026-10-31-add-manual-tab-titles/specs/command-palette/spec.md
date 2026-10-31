# Spec Delta

## MODIFIED Requirements

### Requirement: Commands listed for the focused tab
The palette SHALL list the commands that apply to the tab that had focus when it opened, each with a readable name. Commands that do not apply to that tab SHALL NOT be listed: for example, tmux commands SHALL appear only for control-mode tabs, except "tmux: New Session", which is listed in every tab while the config has a control-mode profile. Commands that only work inside a palette, and the "none" action, SHALL NOT be listed. The list SHALL be in a stable order. The general `Rename Tab` command SHALL be listed for both local and control-mode tabs.

#### Scenario: Local tab
- **WHEN** a local shell tab is active and the user opens the palette
- **THEN** commands such as "Reload Config", "Clear Scrollback" and "Rename Tab" are listed, and no tmux command other than "tmux: New Session" is listed

#### Scenario: Control-mode tab
- **WHEN** a control-mode tab is active and the user opens the palette
- **THEN** tmux commands such as "tmux: Split Right" and "tmux: Rename Window" are listed alongside general commands such as "Rename Tab"

### Requirement: Text prompt step
A command that needs text SHALL turn the palette into a prompt showing what is asked and, when the command provides one, a pre-filled value. The prompt SHALL be a text input, as defined by the `text-input` capability, with the pre-filled value selected so typing replaces it. Enter SHALL submit the text and escape SHALL cancel without running the command. A command started from its keybinding SHALL open the same prompt directly.

#### Scenario: Prompt from the list
- **WHEN** the user picks "tmux: Rename Window" in the palette
- **THEN** the palette asks for the new name, pre-filled with the current one

#### Scenario: Rename tab prompt
- **WHEN** the user picks "Rename Tab" in the palette while the active tab is titled `bash`
- **THEN** the palette asks for the new tab title, pre-filled with `bash`

#### Scenario: Prompt cancelled
- **WHEN** the user presses escape in a prompt
- **THEN** the palette closes and the command does not run

#### Scenario: Edit the pre-filled value
- **WHEN** the rename prompt is pre-filled with `editor` and the user presses end and types `-2`
- **THEN** enter renames the window to `editor-2`
