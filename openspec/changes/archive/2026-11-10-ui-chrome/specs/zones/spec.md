# Spec Delta

## ADDED Requirements

### Requirement: Zone header menu
Right-clicking a zone's header bar SHALL focus that zone and open a menu beside the pointer listing Split Zone Right, Split Zone Down and Close Zone, each running the action of the same name on that zone and showing its shortcut when it has one. Split items SHALL be disabled where the zone is too small to split.

#### Scenario: Split from the menu
- **WHEN** the user right-clicks the unfocused zone's header and selects Split Zone Down
- **THEN** that zone gains focus and is split with a new zone below it

#### Scenario: Too small to split
- **WHEN** a zone is at the minimum size for splitting and the user opens its header menu
- **THEN** the split items are disabled and Close Zone is enabled

#### Scenario: Keyboard path
- **WHEN** the config binds a key to `open_zone_menu`
- **THEN** the key opens the active zone's header menu beside the header

### Requirement: Zone header button tooltips
The split-right, split-down and close buttons on a zone header SHALL show a tooltip with their name and shortcut, when they have one: "Split right", "Split down" and "Close zone".

#### Scenario: Close tooltip
- **WHEN** the pointer rests on a zone header's close button
- **THEN** a tooltip reads "Close zone" followed by its shortcut
