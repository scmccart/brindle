# zones Specification

## Purpose
Split local tabs into a binary tree of zones with native controls around each zone, so multiple terminals (and future zone types) can share a tab side by side.

## Requirements

### Requirement: Splitting the active zone
The user SHALL be able to split the active zone of a local tab in half horizontally (`Split Zone Right`, ctrl-shift-e, or the zone header's split-right button) or vertically (`Split Zone Down`, ctrl-shift-o, or the zone header's split-down button). The existing zone SHALL keep the left or top half, and a new zone of the tab's kind (a terminal zone in a terminal tab, an agent zone in an agent tab) SHALL occupy the right or bottom half and receive keyboard focus.

#### Scenario: Split right with the shortcut
- **WHEN** a local tab has one zone and the user presses ctrl-shift-e
- **THEN** the tab has two side-by-side zones of equal width, and the right zone has keyboard focus

#### Scenario: Split down with the shortcut
- **WHEN** a local tab has one zone and the user presses ctrl-shift-o
- **THEN** the tab has two stacked zones of equal height, and the bottom zone has keyboard focus

#### Scenario: Nested binary splits
- **WHEN** a local tab is split right and the user presses ctrl-shift-o in the right zone
- **THEN** the left zone occupies the left half of the tab and the right half contains two vertically stacked zones, with the bottom-right zone focused

#### Scenario: Split an agent tab
- **WHEN** an agent tab has one zone and the user presses ctrl-shift-e
- **THEN** the right zone is a new agent zone, with its own agent session, and its prompt input has keyboard focus

### Requirement: New zone profile and working directory
A zone created by splitting SHALL run the tab's profile and start in the working directory of the zone that was split, following the same working-directory rules as a new tab in `tabs`. An agent zone's working directory SHALL be the one its agent session was started in. A tab SHALL start a zone only of its own kind: when the tab's profile, as the config has it now, is gone or of the other kind, splitting SHALL do nothing.

#### Scenario: Split inherits working directory
- **WHEN** the active zone's shell is in `~/src/brindle` and the user splits the zone right
- **THEN** the new zone's shell starts in `~/src/brindle` using the tab's profile

#### Scenario: Split in a profile with a configured directory
- **WHEN** a tab's profile sets `cwd = "/var/log"`, the active zone has changed to `/tmp`, and the user splits the zone down
- **THEN** the new zone starts in `/tmp`

#### Scenario: Agent split in the same directory
- **WHEN** an agent zone's session was started in `~/src/brindle` and the user splits it down
- **THEN** the new agent zone's session starts in `~/src/brindle`

#### Scenario: Profile changed kind
- **WHEN** a terminal tab's profile is changed to `type = "agent"` and saved, and the user splits the tab's zone
- **THEN** no zone is created

### Requirement: Minimum size for splitting
Splitting the active zone SHALL do nothing when the active zone's current bounds cannot fit two minimum 10-column by 2-row terminal grids together with the gap between the new cards, each card's border and each card's header bar. No terminal grid Brindle creates or resizes SHALL be narrower than 10 columns, so a pane narrower than that is laid out as 10 columns wide. Shrinking the window or restoring a saved session SHALL NOT remove existing zones when zones become smaller than this threshold.

#### Scenario: Split rejected when zone is too small
- **WHEN** a zone is too narrow to fit two minimum terminal grids side by side and the user runs `Split Zone Right`
- **THEN** no new zone is created and the existing zones and focus remain unchanged

#### Scenario: Window shrink preserves zones
- **WHEN** a tab has four split zones and the user resizes the window to its minimum size
- **THEN** all four zones remain in the tab

#### Scenario: Card chrome counts
- **WHEN** a zone is exactly as wide as two minimum grids plus their padding, but not the gap and the borders
- **THEN** `Split Zone Right` does nothing

#### Scenario: A pane narrower than 10 columns is laid out as 10 columns wide
- **WHEN** a terminal's pane is only 4 columns wide
- **THEN** its grid, and the PTY size it reports, are 10 columns wide

### Requirement: Single-zone tabs hide zone chrome
While a local tab contains only one zone, it SHALL NOT show a zone header bar, and the terminal SHALL fill its card, which fills the tab content area inside the card margin.

#### Scenario: Unsplit tab has no header bar
- **WHEN** a new local tab opens with one zone
- **THEN** no zone header bar is shown above the terminal

#### Scenario: Closing down to one zone hides chrome
- **WHEN** a local tab has two zones and one zone closes
- **THEN** the remaining zone's card fills the tab content area inside the margin and its zone header bar is no longer shown

### Requirement: Multi-zone chrome and header controls
While a local tab contains two or more zones, Brindle SHALL draw a native header bar at the top of each zone's card, in the zone terminal's background and foreground colours. Each zone header bar SHALL show:
- that zone's program status dot, when its terminal holds a non-`idle` status record;
- that zone's automatic title, in the terminal's foreground colour when that zone is the active zone in the tab and in its muted foreground otherwise;
- buttons to split that zone right, split that zone down, and close that zone (`×`).

#### Scenario: Header bars appear on split
- **WHEN** a single-zone local tab is split right
- **THEN** both zones show a header bar with their title and split-right, split-down and close buttons

#### Scenario: Clicking a zone header button
- **WHEN** a local tab has two zones and the user clicks the close button on the unfocused zone's header bar
- **THEN** that unfocused zone closes and the remaining zone fills the tab

#### Scenario: Clicking an unfocused zone focuses it
- **WHEN** a local tab has two zones and the user clicks inside the unfocused zone
- **THEN** keyboard focus moves to that zone and its title is drawn in the foreground colour

#### Scenario: Header uses the terminal's theme
- **WHEN** a zone's terminal theme is `gruvbox-dark` and the global theme is `tokyo-night`
- **THEN** the zone's header bar is drawn on `gruvbox-dark`'s background with its foreground and muted foreground for the title and buttons

### Requirement: Closing a zone
Closing a zone (with `Close Zone`, ctrl-shift-x, its header `×` button, or when its terminal's program exits) SHALL shut down that zone's terminal or agent session and replace its parent split with the sibling subtree. Shutting down a local terminal SHALL end its program (see "Closing ends the program"). Closing an agent zone whose agent is working or waiting for permission SHALL ask first, and an agent zone whose agent exits SHALL stay open until the user closes it (see `agent-zones`). Focus SHALL move to the most recently focused zone in that sibling subtree. Closing or exiting the only zone in a tab SHALL close the tab. `Close Tab` (ctrl-shift-w) SHALL close the entire tab and all of its zones.

#### Scenario: Shell exit in one of two zones
- **WHEN** a local tab has two zones and the user runs `exit` in the focused right zone
- **THEN** the right zone closes, the tab stays open with the left zone filling it, and the left zone has focus

#### Scenario: Close Zone shortcut
- **WHEN** a local tab has three zones and the user presses ctrl-shift-x
- **THEN** only the active zone closes and the remaining two zones stay open

#### Scenario: Close Tab closes all zones
- **WHEN** a local tab has three zones and the user presses ctrl-shift-w
- **THEN** the entire tab and all three terminals close

#### Scenario: Close a working agent zone
- **WHEN** an agent tab has two zones, the active one's turn is running, and the user presses ctrl-shift-x
- **THEN** a dialog asks first, and the zone closes only once the user confirms

#### Scenario: Agent exit keeps its zone
- **WHEN** an agent tab has two zones and one zone's agent exits
- **THEN** both zones stay, and the exited one says its agent exited

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
Splitting a zone whose profile has a remote target SHALL create the new session on the same SSH host or container the zone runs on, through that endpoint's existing connection, and SHALL inherit the active remote working directory. A local directory SHALL NOT be passed to a remote profile, and a directory from one remote target SHALL NOT be passed to another target. The split SHALL stay on the zone's endpoint even when the profile's `remote` target has changed since the tab opened.

#### Scenario: Split on the same SSH host
- **WHEN** an SSH-backed zone on host `A8` is in `/srv/app` and the user selects Split Zone Right
- **THEN** the new zone connects to `A8` and starts in `/srv/app`

#### Scenario: Open remote profile from a local tab
- **WHEN** the active local tab is in `C:\src` and the user opens an SSH profile
- **THEN** Brindle does not send `C:\src` as the remote working directory

#### Scenario: Split after the profile moved
- **WHEN** a zone of profile `Remote` runs on `old-host`, the profile's host is changed to `new-host` and saved, and the user splits the zone
- **THEN** the new zone runs on `old-host`

### Requirement: Closing ends the program
When the user closes a local or remote terminal (its zone or its tab), on Linux or on Windows, Brindle SHALL end its program in the terminal server, without asking, including a terminal attached to a session that an earlier Brindle left. Quitting Brindle SHALL NOT end it (see "Quitting keeps programs running" in `tabs`). A remote terminal closed while its endpoint is disconnected SHALL have its program ended once Brindle reconnects, if Brindle is still running then.

#### Scenario: Close a zone
- **WHEN** a zone runs `sleep 1000` and the user closes it
- **THEN** the zone closes, the `sleep` process is gone within a few seconds, and no session is listed for it

#### Scenario: Close a reattached tab
- **WHEN** a tab reattached at launch runs `sleep 1000` and the user closes it with ctrl-shift-w
- **THEN** the `sleep` process is gone within a few seconds

#### Scenario: Close a Windows zone
- **WHEN** a Windows zone runs `ping -t localhost` and the user closes it
- **THEN** the zone closes, the `ping` process is gone within a few seconds, and no session is listed for it

#### Scenario: Close a remote tab
- **WHEN** an SSH tab on `A8` runs `sleep 1000` and the user closes it
- **THEN** the tab closes, and on `A8` the `sleep` process is gone within a few seconds and no session is listed for it

### Requirement: Zone header menu
Right-clicking a zone's header bar SHALL focus that zone and open a menu beside the pointer listing Split Zone Right, Split Zone Down and Close Zone, each running the action of the same name on that zone and showing its shortcut when it has one. Split items SHALL be disabled where the zone is too small to split, and where the tab can't start a zone of its kind (see "New zone profile and working directory").

#### Scenario: Split from the menu
- **WHEN** the user right-clicks the unfocused zone's header and selects Split Zone Down
- **THEN** that zone gains focus and is split with a new zone below it

#### Scenario: Too small to split
- **WHEN** a zone is at the minimum size for splitting and the user opens its header menu
- **THEN** the split items are disabled and Close Zone is enabled

#### Scenario: Keyboard path
- **WHEN** the config binds a key to `open_zone_menu`
- **THEN** the key opens the active zone's header menu beside the header

#### Scenario: Agent tab without its agent profile
- **WHEN** an agent tab's profile has been deleted from the config and the user opens a zone header's menu
- **THEN** the split items are disabled and Close Zone is enabled

### Requirement: Zone header button tooltips
The split-right, split-down and close buttons on a zone header SHALL show a tooltip with their name and shortcut, when they have one: "Split right", "Split down" and "Close zone".

#### Scenario: Close tooltip
- **WHEN** the pointer rests on a zone header's close button
- **THEN** a tooltip reads "Close zone" followed by its shortcut

### Requirement: Zones are cards
Every zone of a local tab SHALL be drawn as a card: a 1-pixel border and rounded corners round the zone, in a border colour derived from the zone terminal's theme (a mix of its background and foreground). A 6-pixel gap SHALL separate adjacent cards and surround the cards of a tab on every side, in the window chrome's background colour (the same as behind the tab strip). The gaps replace the dividers between zones. Dividers SHALL NOT be draggable.

#### Scenario: Unsplit tab is a card
- **WHEN** a local tab has one zone
- **THEN** the terminal is drawn inside a bordered card with rounded corners and a 6-pixel margin on every side, and the card has no header row

#### Scenario: Split tab shows a gap, not a divider
- **WHEN** a local tab is split right
- **THEN** two cards are drawn side by side with a 6-pixel gap between them and no 1-pixel divider

#### Scenario: Card border follows the zone's theme
- **WHEN** a tab's profile sets `theme = "gruvbox-dark"` and the global theme is `tokyo-night`
- **THEN** the border of that tab's cards is derived from `gruvbox-dark`'s background and foreground

#### Scenario: Corners are rounded over the terminal
- **WHEN** a zone's terminal paints its background to the corners of the card
- **THEN** the card's corners are still drawn rounded, with the chrome background colour outside the rounded edge

### Requirement: Active zone ring
While a local tab has two or more zones, the active zone's card SHALL show the focus ring in the ring colour when keyboard focus is in it and focus got there from the keyboard. A zone focused with the pointer SHALL NOT show the ring. A tab with one zone SHALL NOT show the ring.

#### Scenario: Keyboard focus moves between zones
- **WHEN** a tab has two zones and the user presses alt-right to focus the right zone
- **THEN** the right zone's card shows the ring and the left zone's does not

#### Scenario: Pointer focus
- **WHEN** the user clicks the unfocused zone
- **THEN** that zone has focus and its card shows no ring, and its title is drawn in the foreground colour

#### Scenario: One zone
- **WHEN** a tab has one zone and the user focuses it with the keyboard
- **THEN** its card shows no ring

### Requirement: Zone header progress bar
While a local tab has two or more zones, each zone whose terminal has progress SHALL show it as a 2-pixel bar along the bottom edge of its header bar, drawn as the tab's bar is for that one terminal (length by percent, colour by state, indeterminate across the whole width without animation).

#### Scenario: Two builds side by side
- **WHEN** a tab has two zones, one reporting normal at 30% and the other paused at 70%
- **THEN** the first zone's header shows a muted bar 30% of its width long, the second a yellow bar 70% long, and the tab shows the yellow one

### Requirement: Zone header context badge
While a local tab has two or more zones, each zone header SHALL show its zone's notable-context badge after the title, drawn as on the tab face (a yellow warning icon with the target user in its tooltip for `elevate`, `chpriv` and a root `session`, the muted label otherwise).

#### Scenario: Two zones, one elevated
- **WHEN** a tab has two zones and one of them runs `run0`
- **THEN** that zone's header shows the yellow warning icon, and the other zone's header shows none
