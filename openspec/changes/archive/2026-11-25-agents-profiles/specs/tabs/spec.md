# Spec Delta

## MODIFIED Requirements

### Requirement: Tab lifecycle
The user SHALL be able to:
- open a new tab with the default profile (ctrl-shift-t, or the `+` button);
- close the active tab (ctrl-shift-w, its `×` button, or a middle click).

A local tab whose last remaining zone's program exits SHALL close. When a local tab has more than one zone and one zone's program exits, only that zone SHALL close, as defined by the `zones` capability. Closing a tab that holds an agent zone whose agent is working or waiting for permission SHALL ask first, as `agent-zones` defines. A new local tab SHALL open next to the active tab and become active.

#### Scenario: Shell exit closes its tab
- **WHEN** the user runs `exit` in a single-zone tab
- **THEN** that tab closes and a neighbouring tab becomes active

#### Scenario: Shell exit in a split tab closes only that zone
- **WHEN** a local tab has two zones and the user runs `exit` in one zone
- **THEN** that zone closes and the tab remains open with the other zone

#### Scenario: New tab position
- **WHEN** the second of three tabs is active and the user opens a new tab
- **THEN** the new tab appears third and is active

#### Scenario: Close a tab with a working agent
- **WHEN** a background tab's agent is running a turn and the user middle-clicks that tab
- **THEN** a dialog asks first, the tab stays until the user confirms, and the active tab doesn't change

#### Scenario: Default profile is an agent
- **WHEN** the default profile is an agent profile and the user presses ctrl-shift-t
- **THEN** a new agent tab of that profile opens

### Requirement: Program status dot
A tab SHALL show a status dot while any of its zones holds a program status record (see the `program-status` capability), or an agent zone status (see `agent-zones`), other than `idle`. The dot's colour SHALL follow the most pressing state across those records and statuses, in the order `blocked` (the theme's yellow, ANSI 3), `error` (red, ANSI 1), `done` (green, ANSI 2), `working` (the muted foreground). It SHALL sit before the title, at the face's left, followed by the bell dot when both apply.

#### Scenario: Blocked in a background tab
- **WHEN** a program in a background tab reports `state=blocked:kind=permission`
- **THEN** that tab shows a yellow dot before its title

#### Scenario: Zones in a tab
- **WHEN** a program in one zone of a tab reports `state=error`, and another zone in the same tab reports `state=working`
- **THEN** that tab shows a red dot

#### Scenario: Agent waiting for permission
- **WHEN** an agent zone in a background tab waits on a permission request, and the tab's other agent zone is running a turn
- **THEN** that tab shows a yellow dot
