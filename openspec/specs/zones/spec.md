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
Closing a zone (with `Close Zone`, ctrl-shift-x, its header `×` button, or when its terminal's program exits) SHALL shut down that zone's terminal and replace its parent split with the sibling subtree. Shutting down a local Linux terminal SHALL end its program (see "Closing ends the program"). Focus SHALL move to the most recently focused zone in that sibling subtree. Closing or exiting the only zone in a tab SHALL close the tab. `Close Tab` (ctrl-shift-w) SHALL close the entire tab and all of its zones.

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

### Requirement: Remote zone split
Splitting a zone whose profile has a remote target SHALL create the new PTY on the same SSH host or container and SHALL inherit the active remote working directory. A local directory SHALL NOT be passed to a remote profile, and a directory from one remote target SHALL NOT be passed to another target.

#### Scenario: Split on the same SSH host
- **WHEN** an SSH-backed zone on host `A8` is in `/srv/app` and the user selects Split Zone Right
- **THEN** the new zone connects to `A8` and starts in `/srv/app`

#### Scenario: Open remote profile from a local tab
- **WHEN** the active local tab is in `C:\src` and the user opens an SSH profile
- **THEN** Brindle does not send `C:\src` as the remote working directory

### Requirement: Closing ends the program
When the user closes a local Linux terminal (its zone or its tab), Brindle SHALL end its program in the terminal server, without asking, including a terminal attached to a session that an earlier Brindle left. Quitting Brindle SHALL NOT end it (see "Quitting keeps programs running" in `tabs`).

#### Scenario: Close a zone
- **WHEN** a zone runs `sleep 1000` and the user closes it
- **THEN** the zone closes, the `sleep` process is gone within a few seconds, and no session is listed for it

#### Scenario: Close a reattached tab
- **WHEN** a tab reattached at launch runs `sleep 1000` and the user closes it with ctrl-shift-w
- **THEN** the `sleep` process is gone within a few seconds

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
