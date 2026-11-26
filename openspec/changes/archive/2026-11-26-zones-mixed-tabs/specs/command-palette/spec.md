## MODIFIED Requirements

### Requirement: Running a command
Running a command SHALL close the palette, return keyboard focus to the tab, and act exactly as the command's keybinding would in that tab.

#### Scenario: Same as the shortcut
- **WHEN** the user runs "Split Terminal Right" from the palette in a tab
- **THEN** the active zone is split, as with its shortcut, and focus moves to the new zone

### Requirement: Commands listed for the active tab
The palette SHALL list the commands that apply to the zone that had focus when it opened, and to its tab, each with a readable name. Commands that do not apply there SHALL NOT be listed. Commands that only work inside a palette, and the "none" action, SHALL NOT be listed. The list SHALL be in a stable order. The `Rename Tab` command SHALL be listed for every tab. The split commands for a kind SHALL be listed only while the config has a profile of that kind.

#### Scenario: Shell tab
- **WHEN** a shell tab is active and the user opens the palette
- **THEN** commands such as "Reload Config", "Clear Scrollback", "Rename Tab" and "Split Terminal Right" are listed, and no command whose name starts with `tmux:` is listed

#### Scenario: Both kinds of split
- **WHEN** the config has a terminal profile and an agent profile, and the user opens the palette in a shell
- **THEN** it lists "Split Terminal Right", "Split Terminal Down", "Split Agent Right", "Split Agent Down", "Split Right With Profile…" and "Split Down With Profile…"

#### Scenario: No agent profile
- **WHEN** the config has no agent profile and the user opens the palette
- **THEN** it lists neither "Split Agent Right" nor "Split Agent Down"

#### Scenario: A mixed tab
- **WHEN** a tab holds a shell and an agent zone, and the palette is opened from the agent zone
- **THEN** it lists the agent zone's commands and not "Clear Scrollback"
