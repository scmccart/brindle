## MODIFIED Requirements

### Requirement: Closing a zone
Closing a zone (with `Close Zone`, ctrl-shift-x, its header `×` button, or when its terminal's program exits) SHALL shut down that zone's terminal and replace its parent split with the sibling subtree. Shutting down a local Linux terminal SHALL follow its close behaviour (see "Closing keeps or ends the program"); a program that exited has nothing left to keep. Focus SHALL move to the most recently focused zone in that sibling subtree. Closing or exiting the only zone in a tab SHALL close the tab. `Close Tab` (ctrl-shift-w) SHALL close the entire tab and all of its zones.

#### Scenario: Shell exit in one of two zones
- **WHEN** a local tab has two zones and the user runs `exit` in the focused right zone
- **THEN** the right zone closes, the tab stays open with the left zone filling it, and the left zone has focus

#### Scenario: Close Zone shortcut
- **WHEN** a local tab has three zones and the user presses ctrl-shift-x
- **THEN** only the active zone closes and the remaining two zones stay open

#### Scenario: Close Tab closes all zones
- **WHEN** a local tab has three zones and the user presses ctrl-shift-w
- **THEN** the entire tab and all three terminals close

## ADDED Requirements

### Requirement: Closing keeps or ends the program
When the user closes a local Linux terminal (a zone, its tab or its window), its program SHALL keep running in the terminal server, detached, so it can be attached again. With `on_close = "ask"`, Brindle SHALL ask first, as defined by "Asking before closing", and end the program only when the user picks "end". A terminal running a command given with `-e` SHALL end its program instead, since it is never reattached.

#### Scenario: Detach keeps the program
- **WHEN** a zone with the default `on_close` runs `sleep 1000` and the user closes it
- **THEN** the zone closes, and `brindle-remote list` shows its session as detached with the same pid

#### Scenario: Ask and end
- **WHEN** a zone with `on_close = "ask"` runs `sleep 1000`, the user closes it and picks "end"
- **THEN** the zone closes and the `sleep` process is gone within a few seconds

#### Scenario: Command window
- **WHEN** the user runs `brindle -e sleep 1000` and closes the window
- **THEN** the `sleep` process is gone within a few seconds and no session is listed

### Requirement: Asking before closing
When a close action would close at least one terminal whose close behaviour is `ask`, Brindle SHALL show one prompt for that action before closing anything, offering to keep the asking programs running, to end them, or to cancel. Cancelling SHALL close nothing. The choice SHALL apply to every asking terminal the action closes; other terminals in the same action SHALL follow their own behaviour.

#### Scenario: Keep running
- **WHEN** the user closes a zone with `on_close = "ask"` and picks "keep running"
- **THEN** the zone closes and its session is listed as detached

#### Scenario: Cancel
- **WHEN** the user closes a tab with two asking zones and dismisses the prompt
- **THEN** the tab and both zones stay open

#### Scenario: One prompt for a split tab
- **WHEN** the user closes a tab with three asking zones
- **THEN** one prompt is shown, and its answer applies to all three
