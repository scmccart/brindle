## MODIFIED Requirements

### Requirement: Closing a zone
Closing a zone (with `Close Zone`, ctrl-shift-x, its header `×` button, or when its terminal's program exits) SHALL shut down that zone's terminal and replace its parent split with the sibling subtree. Shutting down a local terminal SHALL end its program (see "Closing ends the program"). Focus SHALL move to the most recently focused zone in that sibling subtree. Closing or exiting the only zone in a tab SHALL close the tab. `Close Tab` (ctrl-shift-w) SHALL close the entire tab and all of its zones.

#### Scenario: Shell exit in one of two zones
- **WHEN** a local tab has two zones and the user runs `exit` in the focused right zone
- **THEN** the right zone closes, the tab stays open with the left zone filling it, and the left zone has focus

#### Scenario: Close Zone shortcut
- **WHEN** a local tab has three zones and the user presses ctrl-shift-x
- **THEN** only the active zone closes and the remaining two zones stay open

#### Scenario: Close Tab closes all zones
- **WHEN** a local tab has three zones and the user presses ctrl-shift-w
- **THEN** the entire tab and all three terminals close

### Requirement: Closing ends the program
When the user closes a local terminal (its zone or its tab), on Linux or on Windows, Brindle SHALL end its program in the terminal server, without asking, including a terminal attached to a session that an earlier Brindle left. Quitting Brindle SHALL NOT end it (see "Quitting keeps programs running" in `tabs`).

#### Scenario: Close a zone
- **WHEN** a zone runs `sleep 1000` and the user closes it
- **THEN** the zone closes, the `sleep` process is gone within a few seconds, and no session is listed for it

#### Scenario: Close a reattached tab
- **WHEN** a tab reattached at launch runs `sleep 1000` and the user closes it with ctrl-shift-w
- **THEN** the `sleep` process is gone within a few seconds

#### Scenario: Close a Windows zone
- **WHEN** a Windows zone runs `ping -t localhost` and the user closes it
- **THEN** the zone closes, the `ping` process is gone within a few seconds, and no session is listed for it
