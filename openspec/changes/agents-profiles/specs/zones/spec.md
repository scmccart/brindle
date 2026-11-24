# Spec Delta

## MODIFIED Requirements

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
