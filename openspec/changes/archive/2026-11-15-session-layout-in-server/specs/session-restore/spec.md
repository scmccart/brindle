# Spec Delta

## ADDED Requirements

### Requirement: Brindle records each tab with its sessions
Brindle SHALL keep each tab's layout with its sessions, local or remote: the split tree with its sessions as leaves, the active zone, the manual title and the tab's place among the tabs. While it runs, a change SHALL reach a server that speaks protocol 3 within about 2 seconds. When it quits and leaves a tab's sessions detached, it SHALL send the layout with each. Zones without a running session SHALL be left out, their sibling taking their place.

#### Scenario: Split tab detached with its layout
- **WHEN** a tab split into two zones runs two shells and the user quits Brindle
- **THEN** `brindle-remote list` shows both sessions detached, each with a layout naming both sessions

#### Scenario: Split recorded while running
- **WHEN** the user splits a tab into two zones and Brindle is killed with SIGKILL 3 seconds later
- **THEN** `brindle-remote list` shows both sessions with a layout naming both sessions

#### Scenario: Layout follows a change
- **WHEN** the user closes one zone of a two-zone tab while Brindle runs
- **THEN** within about 2 seconds the other zone's session is listed with a layout of that zone alone

#### Scenario: Zone showing only a message
- **WHEN** a two-zone tab's one zone shows only a failed attach's message and the user quits Brindle
- **THEN** the other zone's session is detached with a layout of that zone alone

## MODIFIED Requirements

### Requirement: Launch reattaches detached terminals
On every restoring launch, Brindle SHALL attach every detached local terminal server session it doesn't restore into a zone, each as a new tab, in the order the sessions were started. Each tab SHALL use the profile its session was started with, or the default profile when that profile is gone or isn't local. The same SHALL happen for each remote endpoint the launch connects to: the endpoints of restored remote zones and of profiles that connect at launch. When such an endpoint first connects during the launch, its detached sessions SHALL open as new tabs at the end of the strip. Each SHALL use the first of these profiles that exists: the profile it was started with, when that profile targets the endpoint; the first profile that targets the endpoint; the profile of the first tab on the endpoint; the default profile. It SHALL run on that endpoint, whatever its profile targets. Brindle SHALL stop waiting for an endpoint's first connection once the window has no tab on it, unless a profile connects to it at launch; that endpoint's sessions SHALL then not be attached this way. Local or remote, a session whose owner left without detaching it less than 5 minutes earlier SHALL NOT be attached this way, since its owner may be reconnecting; it stays in the sessions palette. The active tab SHALL stay the one the launch would otherwise make active. Replayed output SHALL raise no bells or notifications.

Sessions attached this way whose layouts name one tab (see "Brindle records each tab with its sessions") SHALL open together as that tab: its split tree without the zones whose sessions aren't attached this way (a split's sibling takes a missing zone's place), its recorded active zone focused, or its first zone when that one is missing, and its manual title. The tab SHALL use the profile its active zone's session gives it by the rules above. Tabs recorded by the same Brindle SHALL keep their recorded order, together at the place of the oldest session among them; a session without a usable layout SHALL open as a tab of its own.

#### Scenario: Session left by a skipped launch
- **WHEN** a session is detached because the last launch was `--no-restore`, and the user quits and launches `brindle`
- **THEN** the window shows the recorded tabs, the recorded active tab still active, followed by a tab showing that session

#### Scenario: No bell from the replay
- **WHEN** a detached session's output contained a bell before the launch
- **THEN** reattaching it marks no bell on its tab and sends no notification

#### Scenario: Session started on the host itself
- **WHEN** profile `A8` connects at launch, and a Brindle running on `A8` left a session detached there
- **THEN** that session opens as a tab with profile `A8`, after the restored tabs, and the active tab doesn't change

#### Scenario: Session started from another machine
- **WHEN** Brindle on Windows quit, leaving a session detached on `A8`, and Brindle is then launched on `A8` itself
- **THEN** the `A8` launch attaches that session as a tab with its default profile

#### Scenario: Owner reconnecting
- **WHEN** Brindle on Windows has a tab on `A8`, its connection dropped a minute ago, and Brindle is launched on `A8` itself
- **THEN** the `A8` launch doesn't attach that session, and Brindle on Windows gets it back when it reconnects

#### Scenario: Endpoint no profile targets
- **WHEN** profile `Remote` now targets `new-host`, a restored zone of `Remote` attaches its recorded session on `old-host`, no profile targets `old-host`, and `old-host` has another detached session with no owner
- **THEN** that session opens as a tab with profile `Remote`, and its zone and its splits run on `old-host`

#### Scenario: Restored tab closed before its host answers
- **WHEN** a launch restored a tab on `A8`, `A8` can't be reached, no profile connects to `A8` at launch, and the user closes that tab
- **THEN** Brindle stops connecting to `A8`, and `A8`'s detached sessions don't open as tabs when `A8` can be reached later in that run

#### Scenario: Splits come back without the state file
- **WHEN** Brindle quit with a tab split into two zones, and Brindle is launched with another state directory against the same terminal server
- **THEN** the launch opens one tab with both sessions in the recorded split, the recorded active zone focused

#### Scenario: Splits of a skipped launch's tabs
- **WHEN** Brindle quit with a split tab titled `logs`, then a `--no-restore` launch quit with one other tab, which replaced the state file's record
- **THEN** the next launch restores that other tab, followed by one tab titled `logs` with both zones in their split

#### Scenario: Splits from another machine
- **WHEN** Brindle on Windows quit with a split tab on `A8`, and Brindle on `A8` launches with profile `A8` connecting at launch
- **THEN** the `A8` launch opens one tab with both sessions in their split

#### Scenario: One session of a split ended
- **WHEN** a split tab's sessions were detached with its layout, and one of them was ended with `brindle-remote kill` before the next launch
- **THEN** the other session opens as a single-zone tab

#### Scenario: Splits after a crash
- **WHEN** Brindle was killed with SIGKILL with a split tab, and more than 5 minutes later Brindle is launched with another state directory against the same terminal server
- **THEN** the launch opens one tab with both sessions in their split

#### Scenario: One session of a split attached elsewhere
- **WHEN** one session of a recorded split was attached through the sessions palette by another Brindle that still shows it
- **THEN** the launch opens the other session as a single-zone tab
