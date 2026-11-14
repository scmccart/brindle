## MODIFIED Requirements

### Requirement: Remote endpoint identity
A recorded local tab backed by a remote profile SHALL store the SSH or container target identity used when its CWD and zone directories were observed. Restore SHALL reuse those directories only when the current profile resolves to the same endpoint. State written before endpoint identity was recorded, a missing/renamed profile, or a profile whose endpoint changed SHALL discard the recorded directories and use the current profile's remote directory default. A recorded zone with a terminal server session SHALL instead be attached on the recorded endpoint, whatever the profile now targets, as defined by "Restored zones get their sessions back".

#### Scenario: SSH profile changes host
- **WHEN** a tab recorded `/srv/old` for profile `Remote` on SSH host `old-host` without a session record, and the profile now points to `new-host` with `cwd = "/home/new"`
- **THEN** restore starts on `new-host` in `/home/new` and never sends `/srv/old`

#### Scenario: Existing local state
- **WHEN** an older state file has no endpoint identity and resolves to a local profile
- **THEN** its local directory is restored under the existing local-directory fallback rules

#### Scenario: Recorded session on the old host
- **WHEN** a zone of profile `Remote` recorded a session on `old-host`, the profile now points to `new-host`, and the session is still detached on `old-host`
- **THEN** after a launch the zone shows that session from `old-host`

### Requirement: Restored zones get their sessions back
Each recorded zone, local on Linux and Windows or remote, SHALL record which terminal server session it showed. On a restoring launch, a local zone whose recorded session is still running and detached SHALL be attached to it, showing its screen and scrollback, without bells or notifications from the replayed output. A local zone whose recorded session is gone, attached elsewhere, or not the recorded one (the server was restarted in between and reused its id) SHALL NOT be restored: its split's sibling takes its place, a tab with no zone left SHALL NOT be restored, and a launch left with nothing to restore SHALL open one default-profile tab. A recorded zone without a session record SHALL restore as "Local tabs restore their profile, zones and directory" says.

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
On every restoring launch, Brindle SHALL attach every detached local terminal server session it doesn't restore into a zone, each as a new tab, in the order the sessions were started. Each tab SHALL use the profile its session was started with, or the default profile when that profile is gone or isn't local. The same SHALL happen for each remote endpoint the launch connects to: the endpoints of restored remote zones and of profiles that connect at launch. When such an endpoint first connects during the launch, its detached sessions SHALL open as new tabs at the end of the strip. Each SHALL use the profile it was started with when that profile targets the endpoint, else the first profile that does. Local or remote, a session whose owner left without detaching it less than 5 minutes earlier SHALL NOT be attached this way, since its owner may be reconnecting; it stays in the sessions palette. The active tab SHALL stay the one the launch would otherwise make active. Replayed output SHALL raise no bells or notifications.

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
