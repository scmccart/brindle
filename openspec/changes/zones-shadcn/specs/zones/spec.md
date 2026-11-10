## ADDED Requirements

### Requirement: Zones are cards
Every zone of a local tab SHALL be drawn as a card: a 1-pixel border and rounded corners round the zone, in a border colour derived from the zone terminal's theme (a mix of its background and foreground). A 6-pixel gap SHALL separate adjacent cards and surround the cards of a tab on every side, in the tab theme's background. The gaps replace the dividers between zones. Dividers SHALL NOT be draggable.

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
- **THEN** the card's corners are still drawn rounded, with the gap colour outside the rounded edge

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

## MODIFIED Requirements

### Requirement: Minimum size for splitting
Splitting the active zone SHALL do nothing when the active zone's current bounds cannot fit two minimum 2-column by 2-row terminal grids together with the gap between the new cards, each card's border and each card's header bar. Shrinking the window or restoring a saved session SHALL NOT remove existing zones when zones become smaller than this threshold.

#### Scenario: Split rejected when zone is too small
- **WHEN** a zone is too narrow to fit two minimum terminal grids side by side and the user runs `Split Zone Right`
- **THEN** no new zone is created and the existing zones and focus remain unchanged

#### Scenario: Window shrink preserves zones
- **WHEN** a tab has four split zones and the user resizes the window to its minimum size
- **THEN** all four zones remain in the tab

#### Scenario: Card chrome counts
- **WHEN** a zone is exactly as wide as two minimum grids plus their padding, but not the gap and the borders
- **THEN** `Split Zone Right` does nothing

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
