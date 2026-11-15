## MODIFIED Requirements

### Requirement: Launch reattaches detached terminals
On every restoring launch, Brindle SHALL attach every detached local terminal server session it doesn't restore into a zone, each as a new tab, in the order the sessions were started. Each tab SHALL use the profile its session was started with, or the default profile when that profile is gone or isn't local. The same SHALL happen for each remote endpoint the launch connects to: the endpoints of restored remote zones and of profiles that connect at launch. When such an endpoint first connects during the launch, its detached sessions SHALL open as new tabs at the end of the strip. Each SHALL use the first of these profiles that exists: the profile it was started with, when that profile targets the endpoint; the first profile that targets the endpoint; the profile of the first tab on the endpoint; the default profile. It SHALL run on that endpoint, whatever its profile targets. Brindle SHALL stop waiting for an endpoint's first connection once the window has no tab on it, unless a profile connects to it at launch; that endpoint's sessions SHALL then not be attached this way. Local or remote, a session whose owner left without detaching it less than 5 minutes earlier SHALL NOT be attached this way, since its owner may be reconnecting; it stays in the sessions palette. The active tab SHALL stay the one the launch would otherwise make active. Replayed output SHALL raise no bells or notifications.

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
