# zones Specification

## Purpose
Split local tabs into a binary tree of zones with native controls around each zone, so multiple terminals (and future zone types) can share a tab side by side without tmux.

## Requirements

### Requirement: Splitting the active zone
The user SHALL be able to split the active zone of a local tab in half horizontally (`Split Zone Right`, ctrl-shift-e, or the zone header's split-right button) or vertically (`Split Zone Down`, ctrl-shift-o, or the zone header's split-down button). The existing zone SHALL keep the left or top half, and a new terminal zone SHALL occupy the right or bottom half and receive keyboard focus.

#### Scenario: Split right with the shortcut
- **WHEN** a local tab has one zone and the user presses ctrl-shift-e
- **THEN** the tab has two side-by-side zones of equal width, and the right zone has keyboard focus

#### Scenario: Split down with the shortcut
- **WHEN** a local tab has one zone and the user presses ctrl-shift-o
- **THEN** the tab has two stacked zones of equal height, and the bottom zone has keyboard focus

#### Scenario: Nested binary splits
- **WHEN** a local tab is split right and the user presses ctrl-shift-o in the right zone
- **THEN** the left zone occupies the left half of the tab and the right half contains two vertically stacked zones, with the bottom-right zone focused

### Requirement: New zone profile and working directory
A zone created by splitting SHALL run the tab's profile and start in the working directory of the zone that was split, following the same working-directory rules as a new tab in `tabs`.

#### Scenario: Split inherits working directory
- **WHEN** the active zone's shell is in `~/src/brindle` and the user splits the zone right
- **THEN** the new zone's shell starts in `~/src/brindle` using the tab's profile

#### Scenario: Split in a profile with a configured directory
- **WHEN** a tab's profile sets `cwd = "/var/log"`, the active zone has changed to `/tmp`, and the user splits the zone down
- **THEN** the new zone starts in `/tmp`

### Requirement: Minimum size for splitting
Splitting the active zone SHALL do nothing when the active zone's current bounds cannot fit two minimum 2-column by 2-row terminal grids together with the split divider and the zone header bars. Shrinking the window or restoring a saved session SHALL NOT remove existing zones when zones become smaller than this threshold.

#### Scenario: Split rejected when zone is too small
- **WHEN** a zone is too narrow to fit two minimum terminal grids side by side and the user runs `Split Zone Right`
- **THEN** no new zone is created and the existing zones and focus remain unchanged

#### Scenario: Window shrink preserves zones
- **WHEN** a tab has four split zones and the user resizes the window to its minimum size
- **THEN** all four zones remain in the tab

### Requirement: Single-zone tabs hide zone chrome
While a local tab contains only one zone, it SHALL NOT show a zone header bar or split dividers, and the terminal SHALL fill the tab content area as an unsplit tab.

#### Scenario: Unsplit tab has no header bar
- **WHEN** a new local tab opens with one zone
- **THEN** no zone header bar is shown above the terminal

#### Scenario: Closing down to one zone hides chrome
- **WHEN** a local tab has two zones and one zone closes
- **THEN** the remaining zone fills the tab content area and its zone header bar is no longer shown

### Requirement: Multi-zone chrome and header controls
While a local tab contains two or more zones, Brindle SHALL draw a 1-pixel divider between each split pair and a native header bar above each zone. Each zone header bar SHALL show:
- that zone's program status dot, when its terminal holds a non-`idle` status record;
- that zone's automatic title;
- distinct styling when that zone is the active zone in the tab;
- buttons to split that zone right, split that zone down, and close that zone (`×`).

#### Scenario: Header bars appear on split
- **WHEN** a single-zone local tab is split right
- **THEN** both zones show a header bar with their title and split-right, split-down and close buttons

#### Scenario: Clicking a zone header button
- **WHEN** a local tab has two zones and the user clicks the close button on the unfocused zone's header bar
- **THEN** that unfocused zone closes and the remaining zone fills the tab

#### Scenario: Clicking an unfocused zone focuses it
- **WHEN** a local tab has two zones and the user clicks inside the unfocused zone
- **THEN** keyboard focus moves to that zone and its header bar shows the active styling

### Requirement: Closing a zone
Closing a zone (with `Close Zone`, ctrl-shift-x, its header `×` button, or when its terminal's program exits) SHALL shut down that zone's terminal and replace its parent split with the sibling subtree. Focus SHALL move to the most recently focused zone in that sibling subtree. Closing or exiting the only zone in a tab SHALL close the tab. `Close Tab` (ctrl-shift-w) SHALL close the entire tab and all of its zones.

#### Scenario: Shell exit in one of two zones
- **WHEN** a local tab has two zones and the user runs `exit` in the focused right zone
- **THEN** the right zone closes, the tab stays open with the left zone filling it, and the left zone has focus

#### Scenario: Close Zone shortcut
- **WHEN** a local tab has three zones and the user presses ctrl-shift-x
- **THEN** only the active zone closes and the remaining two zones stay open

#### Scenario: Close Tab closes all zones
- **WHEN** a local tab has three zones and the user presses ctrl-shift-w
- **THEN** the entire tab and all three terminals close

### Requirement: Directional focus between zones
The user SHALL be able to move focus between zones with `Focus Zone Left` (alt-left), `Focus Zone Right` (alt-right), `Focus Zone Up` (alt-up) and `Focus Zone Down` (alt-down). Using normalized split-tree rectangles, Brindle SHALL focus the zone in that direction whose perpendicular span overlaps the active zone's span, preferring the nearest edge, then the closest perpendicular center, then tree order. When no zone lies in that direction, focus SHALL NOT move.

#### Scenario: Move between side-by-side zones
- **WHEN** a local tab is split right with the right zone focused and the user presses alt-left
- **THEN** focus moves to the left zone

#### Scenario: Move into a further-split half
- **WHEN** the left half is one tall zone and the right half is split into top-right and bottom-right zones, the left zone is focused, and the user presses alt-right
- **THEN** focus moves to the top-right zone

#### Scenario: Move at an outer edge
- **WHEN** the left-most zone has focus and the user presses alt-left
- **THEN** focus stays on that zone
