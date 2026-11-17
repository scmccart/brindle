# Spec Delta

## MODIFIED Requirements

### Requirement: Restored zones get their sessions back
Each recorded zone, local on Linux and Windows or remote, SHALL record which terminal server session it showed. On a restoring launch whose listing of this machine's terminal server answered (see "Launch reattaches detached terminals"), a local zone whose recorded session is still running and detached SHALL be attached to it, showing its screen and scrollback, without bells or notifications from the replayed output. A local zone whose recorded session is gone, attached elsewhere, or not the recorded one (the server was restarted in between and reused its id) SHALL NOT be restored: its split's sibling takes its place, a tab with no zone left SHALL NOT be restored, and a launch left with nothing to restore SHALL open one default-profile tab. When that listing failed, nothing tells a lost session from a live one, so no local zone SHALL be dropped: each local zone with a session record SHALL start its profile's program in its recorded directory, and its recorded session, if it still runs, SHALL stay detached. A recorded zone without a session record SHALL restore as "Local tabs restore their profile, zones and directory" says.

A remote zone with a session record SHALL be restored without waiting for its endpoint. It SHALL show that it is connecting, and attach once its endpoint answers. If the session is gone, attached elsewhere or not the recorded one, the zone SHALL say so and stay open until the user closes it. A zone that only shows such a message SHALL NOT be recorded.

#### Scenario: Programs survive a quit
- **WHEN** a tab runs `htop`, and the user quits and relaunches
- **THEN** the restored tab shows the same `htop` process, with the same pid

#### Scenario: Programs survive a quit on Windows
- **WHEN** a Windows tab runs `ping -t localhost`, and the user quits and relaunches
- **THEN** the restored tab shows the same `ping` process, with the same pid, and its earlier output in the scrollback

#### Scenario: Programs survive a crash
- **WHEN** Brindle is killed with SIGKILL while a split tab's zones run `vim` and a shell with scrollback
- **THEN** the next launch restores the split with `vim` running and the shell's scrollback intact

#### Scenario: Session gone from a split
- **WHEN** one zone of a two-zone tab had its program ended with `brindle-remote kill` before the relaunch
- **THEN** the tab is restored with only the other zone, reattached

#### Scenario: Every session gone
- **WHEN** Brindle quit with two local tabs and `brindle-remote stop` ended their sessions before the relaunch
- **THEN** the next launch opens one default-profile tab, and neither recorded tab comes back

#### Scenario: Reused id
- **WHEN** the server stopped after Brindle quit, and a new server gave the recorded id to a different session
- **THEN** the zone is not restored, and that other session opens as its own tab

#### Scenario: Server not listed at launch
- **WHEN** Brindle quit with a local tab in `/tmp` whose session stayed detached, and the next launch can't list this machine's terminal server
- **THEN** the tab is restored with a new shell in `/tmp`, and the recorded session is still running and detached

#### Scenario: Remote programs survive a quit
- **WHEN** a split SSH tab on `A8` runs `htop` and a shell, and the user quits and relaunches
- **THEN** the restored tab shows the same `htop` and shell processes on `A8`, with their pids and scrollback

#### Scenario: Remote host unreachable at launch
- **WHEN** Brindle quit with a tab on `A8`, and `A8` can't be reached at the next launch
- **THEN** the window opens at once with that tab saying it is connecting, and the tab shows its session once `A8` can be reached

#### Scenario: Remote session gone
- **WHEN** a recorded zone's session on `A8` was ended before the relaunch
- **THEN** the zone says its session on `A8` has ended, and the next saved state doesn't record it

### Requirement: Launch reattaches detached terminals
On every restoring launch, Brindle SHALL attach every detached local terminal server session it doesn't restore into a zone, each as a new tab, in the order the sessions were started. Each tab SHALL use the profile its session was started with, or the default profile when that profile is gone or isn't local. The same SHALL happen for each remote endpoint the launch connects to: the endpoints of restored remote zones and of profiles that connect at launch. When such an endpoint first connects during the launch, its detached sessions SHALL open as new tabs at the end of the strip. Each SHALL use the first of these profiles that exists: the profile it was started with, when that profile targets the endpoint; the first profile that targets the endpoint; the profile of the first tab on the endpoint; the default profile. It SHALL run on that endpoint, whatever its profile targets. Brindle SHALL stop waiting for an endpoint's first connection once the window has no tab on it, unless a profile connects to it at launch; that endpoint's sessions SHALL then not be attached this way. Local or remote, a session whose owner left without detaching it less than 5 minutes earlier SHALL NOT be attached this way, since its owner may be reconnecting; it stays in the sessions palette. The active tab SHALL stay the one the launch would otherwise make active. Replayed output SHALL raise no bells or notifications.

Sessions attached this way whose layouts name one tab (see "Brindle records each tab with its sessions") SHALL open together as that tab: its split tree without the zones whose sessions aren't attached this way (a split's sibling takes a missing zone's place), its recorded active zone focused, or its first zone when that one is missing, and its manual title. The tab SHALL use the profile its active zone's session gives it by the rules above. Tabs recorded by the same Brindle SHALL keep their recorded order, together at the place of the oldest session among them; a session without a usable layout SHALL open as a tab of its own.

A restoring launch SHALL list this machine's terminal server's sessions once before its window opens, without starting a server. The window SHALL open once that listing has answered or failed, and SHALL show the tabs and active tab these rules give for that answer, whether it came at once or late. When the listing fails, no detached session of this machine SHALL open as a tab this way, a log line SHALL name the failure, and those sessions SHALL stay detached; remote endpoints SHALL be swept as above.

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

#### Scenario: Slow listing
- **WHEN** Brindle quit with a tab whose session stayed detached, a `--no-restore` launch then left another session detached, and at the next launch this machine's terminal server takes 3 seconds to answer the listing
- **THEN** the window opens once the server has answered, showing the recorded tab, still active, followed by a tab showing the other session, as when the server answers at once

#### Scenario: Listing fails
- **WHEN** a session is detached because the last launch was `--no-restore`, and the next launch can't list this machine's terminal server
- **THEN** the window shows the recorded tabs and no tab for that session, the log names the failure, and a later launch whose listing answers opens that session as a tab
