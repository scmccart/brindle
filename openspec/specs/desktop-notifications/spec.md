# desktop-notifications Specification

## Purpose
Let programs and bells in Brindle's tabs reach the user through desktop notifications when the tab isn't in front of them, and take the user back to that tab when they click one.

## Requirements

### Requirement: OSC 9 notifications
`OSC 9 ; <message>`, terminated by BEL or ST, SHALL raise a desktop notification whose body is the message. ConEmu sub-commands SHALL NOT notify: when the first `;`-separated field of the payload is a whole number from 1 to 12, the sequence is a ConEmu command (progress, working directory and so on) and is ignored. A sequence with an empty message SHALL NOT notify.

#### Scenario: Message
- **WHEN** a program prints `ESC ] 9 ; Build finished BEL` in a background tab
- **THEN** a desktop notification with the body `Build finished` appears

#### Scenario: ConEmu progress is not a message
- **WHEN** a program prints `ESC ] 9 ; 4 ; 1 ; 50 BEL` or `ESC ] 9 ; 9 ; /home/me BEL`
- **THEN** no notification appears

#### Scenario: Number that is not a sub-command
- **WHEN** a program prints `ESC ] 9 ; 42 BEL`
- **THEN** a notification with the body `42` appears

### Requirement: OSC 777 notifications
`OSC 777 ; notify ; <title> ; <body>`, terminated by BEL or ST, SHALL raise a desktop notification with that title and body. Any `;` after the title belongs to the body. Other OSC 777 sub-commands SHALL be ignored. The title MAY be empty; the notification then carries only the tab's title.

#### Scenario: Title and body
- **WHEN** a program prints `ESC ] 777 ; notify ; Tests ; 12 passed; 0 failed BEL` in a background tab
- **THEN** a notification appears whose title contains `Tests` and whose body is `12 passed; 0 failed`

#### Scenario: Other sub-command
- **WHEN** a program prints `ESC ] 777 ; precmd BEL`
- **THEN** no notification appears

### Requirement: Sequences are recognised across reads
OSC 9 and OSC 777 SHALL be recognised however the program's output is split into reads, and SHALL NOT disturb how the rest of the output is displayed. A sequence that is cancelled (CAN, SUB, or ESC not followed by `\`) SHALL NOT notify. Payloads longer than 4096 bytes SHALL be ignored.

#### Scenario: Split sequence
- **WHEN** a program writes `ESC ] 777 ; noti` and, in a later write, `fy ; T ; B BEL`
- **THEN** one notification with title `T` and body `B` appears

### Requirement: Bell notifications
A bell (BEL) SHALL raise a desktop notification, subject to the focus rule, in addition to the tab's bell indicator. A BEL that terminates an OSC sequence is not a bell.

#### Scenario: Background bell
- **WHEN** a background tab rings the bell
- **THEN** a notification appears saying the bell rang in that tab, and the tab shows its bell dot

### Requirement: Notifications identify their tab
A notification's title SHALL include the title of the tab it came from. For OSC 777 with a non-empty title, the title SHALL read `<title> — <tab title>`; otherwise it SHALL be the tab title. Control characters SHALL be removed from titles and bodies, and long titles and bodies SHALL be shortened.

#### Scenario: OSC 9 in a tab titled `make`
- **WHEN** a tab titled `make` prints `ESC ] 9 ; done BEL`
- **THEN** the notification's title is `make` and its body is `done`

### Requirement: Clicking a notification shows its tab
When the notification service supports actions, clicking a notification (its default action) SHALL raise the window it came from and activate the tab it came from. Windows toasts always support this. If that tab has closed, the window SHALL still be raised. If the window has closed, nothing SHALL happen.

#### Scenario: Click
- **WHEN** a notification from the third tab of a background window is clicked
- **THEN** that window comes to the front with the third tab active

#### Scenario: Click a toast on Windows
- **WHEN** on Windows, a toast from the second tab of a minimized Brindle window is clicked
- **THEN** that window is restored and comes to the front with the second tab active

### Requirement: Focus rule
With `when = "unfocused"` (the default) a source SHALL notify only when its tab is not visible to the user: the tab is not the window's active tab, or the window is not focused. With `when = "always"` it SHALL notify regardless.

#### Scenario: Visible tab stays quiet
- **WHEN** `when` is `"unfocused"` and the active tab of the focused window prints `ESC ] 9 ; hi BEL`
- **THEN** no notification appears

#### Scenario: Always
- **WHEN** `when` is `"always"` and the active tab of the focused window prints `ESC ] 9 ; hi BEL`
- **THEN** a notification appears

### Requirement: Notification settings
The `[notifications]` table SHALL enable or disable each source with `bell`, `osc9` and `osc777` (all `true` by default) and set `when` to `"unfocused"` or `"always"`. Changes SHALL apply on config reload.

#### Scenario: Bells off
- **WHEN** the config sets `notifications.bell = false` and a background tab rings the bell
- **THEN** no notification appears, and the tab still shows its bell dot

### Requirement: Rate limits
Notifications SHALL be rate limited: at most one bell notification per tab every 5 seconds; per tab, at most 3 program (OSC) notifications at once, then one every 2 seconds; across all tabs, at most 10 at once, then one every 2 seconds. Notifications over a limit SHALL be dropped. A tab's bell notification SHALL replace its previous one instead of adding another.

#### Scenario: Bell storm
- **WHEN** a background tab rings the bell 100 times within a second
- **THEN** one notification appears

#### Scenario: Notification loop
- **WHEN** a background tab prints 50 OSC 9 messages within a second
- **THEN** three notifications appear

### Requirement: Delivery never blocks the window
Notifications SHALL be sent without blocking the window: on Linux to the desktop's freedesktop notification service, and on Windows as toast notifications shown under Brindle's name and icon. If notifications can't be delivered, because there is no notification service or because the system or the user has turned Brindle's notifications off, they SHALL be dropped with a logged warning and the terminal SHALL keep working.

#### Scenario: No notification service
- **WHEN** Brindle runs in a session with no notification service and a background tab rings the bell
- **THEN** the tab shows its bell dot, a warning is logged, and Brindle stays responsive

#### Scenario: Toast on Windows
- **WHEN** on Windows a background tab titled `make` prints `ESC ] 9 ; done BEL`
- **THEN** a toast titled `make` with the body `done` appears, attributed to Brindle with Brindle's icon

#### Scenario: Toasts turned off on Windows
- **WHEN** the user has turned off notifications for Brindle in Windows Settings and a background tab rings the bell
- **THEN** no toast appears, the tab shows its bell dot, and Brindle stays responsive

#### Scenario: Toasts without a Start Menu shortcut
- **WHEN** Brindle was installed with WinGet or `cargo install`, with no Start Menu shortcut, and a background tab sends an OSC 9 notification
- **THEN** a toast attributed to Brindle appears
