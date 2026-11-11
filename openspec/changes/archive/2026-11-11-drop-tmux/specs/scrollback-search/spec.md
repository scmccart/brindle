## ADDED Requirements

### Requirement: Search covers one terminal
The find bar SHALL search one terminal: its scrollback and its screen. In a tab with several zones, the bar SHALL belong to the focused zone. Other zones and tabs SHALL NOT be searched or highlighted.

#### Scenario: Two zones
- **WHEN** two zones of a tab both show `needle`, and the user searches for `needle` in the left zone
- **THEN** only the left zone's `needle` is highlighted

## REMOVED Requirements

### Requirement: Search scope
**Reason**: Rewritten without the tmux control-mode pane wording and scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Search covers one terminal".
