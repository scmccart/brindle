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
The `[notifications]` table SHALL enable or disable each source with `bell`, `osc9`, `osc777`, `osc99` and `osc7501` (all `true` by default) and set `when` to `"unfocused"` or `"always"`. Changes SHALL apply on config reload.

#### Scenario: Bells off
- **WHEN** the config sets `notifications.bell = false` and a background tab rings the bell
- **THEN** no notification appears, and the tab still shows its bell dot

#### Scenario: Program status off
- **WHEN** the config sets `notifications.osc7501 = false` and a background tab reports `state=blocked`
- **THEN** no notification appears, and the tab still shows its status dot

#### Scenario: OSC 99 off
- **WHEN** the config sets `notifications.osc99 = false` and a program prints `ESC ] 99 ; i=q:p=? ; ESC \`
- **THEN** no notification appears for any OSC 99 sequence, and the query gets no reply

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

### Requirement: Program status notifications
A program status report (OSC 7501, see the `program-status` capability) that leaves a record `blocked`, `done` or `error` with a different state, kind, title or message than the record had before SHALL raise a desktop notification, subject to the focus rule and the program (OSC) rate limits. `idle` and `working` reports, repeated identical reports, and clears SHALL NOT notify.

#### Scenario: Blocked on permission
- **WHEN** a background tab titled `bash` prints `ESC ] 7501 ; state=blocked:kind=permission:app=claude-code:msg=UnVuIGNhcmdvIHRlc3Q/ BEL`
- **THEN** a notification appears with the title `claude-code — bash` and the body `Run cargo test?`

#### Scenario: Working is quiet
- **WHEN** a background tab prints `ESC ] 7501 ; state=working:progress=40 BEL`
- **THEN** no notification appears

#### Scenario: Same report twice
- **WHEN** a background tab prints the same `state=blocked` report twice
- **THEN** one notification appears

### Requirement: Program status notification text
The notification's title SHALL be the record's `title`, else its (inherited) `app`, laid out as for OSC 777 (`<title> — <tab title>`, or the tab title alone when there is neither). Its body SHALL be the record's `msg`, or when there is none a phrase for the state: `Waiting for permission`, `Waiting for an answer`, `Waiting for a login` (by `kind`), `Waiting for you` (`blocked` without a kind), `Done`, or `Failed`.

#### Scenario: Done without a message
- **WHEN** a background tab prints `ESC ] 7501 ; state=done:app=brew BEL`
- **THEN** a notification appears with the title `brew — <tab title>` and the body `Done`

#### Scenario: Neither title nor app
- **WHEN** a background tab titled `make` prints `ESC ] 7501 ; state=error BEL`
- **THEN** a notification appears with the title `make` and the body `Failed`

### Requirement: OSC 99 recognition
Brindle SHALL recognise kitty's `ESC ] 99 ; <metadata> ; <payload>`, ended by BEL or ST, however the output is split into reads. Metadata is `:`-separated `key=value` pairs with one-letter keys; unknown keys, and values a known key doesn't take, SHALL be ignored. A sequence without its second `;`, with a metadata part that isn't `key=value`, with a payload over 4096 bytes, or with a payload outside the base64 alphabet under `e=1`, SHALL be ignored, as SHALL one that is cancelled. A base64 payload MAY be split into chunks before or after encoding, padded or not.

#### Scenario: Simplest form
- **WHEN** a background tab prints `ESC ] 99 ; ; Hello world ESC \`
- **THEN** a notification appears whose title contains `Hello world`

#### Scenario: Base64 payload
- **WHEN** a background tab prints `ESC ] 99 ; e=1 ; SGk= ESC \`
- **THEN** a notification appears whose title contains `Hi`

#### Scenario: Base64 split after encoding
- **WHEN** a background tab prints `ESC ] 99 ; i=s:d=0:e=1 ; SGV ESC \` then `ESC ] 99 ; i=s:e=1 ; sbG8= ESC \`
- **THEN** a notification appears whose title contains `Hello`

#### Scenario: Base64 split before encoding
- **WHEN** a background tab prints `ESC ] 99 ; i=u:d=0:e=1 ; SGk= ESC \` then `ESC ] 99 ; i=u:e=1 ; IQ== ESC \`
- **THEN** a notification appears whose title contains `Hi!`

### Requirement: OSC 99 assembly
`p=title` (the default) and `p=body` SHALL add their payload to the notification named by `i` (or the unnamed one). Chunks with `d=0` SHALL be held, and a chunk with `d` absent or anything but `0` SHALL complete and show it. Metadata given on any chunk of a notification SHALL apply to it, a later chunk's value replacing an earlier one's. A notification with no title SHALL use its body as the title; one with neither SHALL be ignored. A terminal SHALL hold at most 8 incomplete notifications, dropping the oldest, and at most 2048 bytes each of title and body.

#### Scenario: Title and body in chunks
- **WHEN** a background tab prints `ESC ] 99 ; i=b:d=0 ; Build ESC \`, then `ESC ] 99 ; i=b:d=0:p=body ; 3 failed ESC \`, then `ESC ] 99 ; i=b:p=body ; , 9 passed ESC \`
- **THEN** one notification appears, titled `Build — <tab title>` with the body `3 failed, 9 passed`

#### Scenario: Body only
- **WHEN** a background tab prints `ESC ] 99 ; p=body ; just a body ESC \`
- **THEN** a notification appears titled `just a body — <tab title>`, with an empty body

### Requirement: OSC 99 identifiers
An `i` value SHALL be 1 to 64 characters of `[A-Za-z0-9_+.-]`; a sequence with any other `i` SHALL be ignored. A completed notification whose id names an open notification from the same terminal SHALL replace it instead of adding another. A notification without an `i` SHALL never replace another, and its reports SHALL use `i=0`.

#### Scenario: Update in place
- **WHEN** a background tab shows a notification with `i=job` titled `Running`, then completes another with `i=job` titled `Failed`
- **THEN** one notification is open, titled `Failed — <tab title>`

#### Scenario: Bad id
- **WHEN** a program prints `ESC ] 99 ; i=a b ; x ESC \`
- **THEN** no notification appears

### Requirement: OSC 99 replies
Every reply SHALL be written byte for byte as given here, with no spaces: `ESC ] 99;` then the metadata, `;`, the payload and `ESC \`. Replies SHALL go only to the terminal whose program raised or asked, and only while that terminal is open.

#### Scenario: Reply bytes
- **WHEN** a program raised `ESC ] 99 ; i=r:a=report ; x ESC \` and the user clicks it
- **THEN** the program reads exactly the bytes `ESC ] 9 9 ; i = r ; ESC \` (`\e]99;i=r;\e\\`)

### Requirement: OSC 99 closing
`p=close` with an `i` SHALL close that terminal's open notification with that id; an unknown id, or no `i`, SHALL be ignored. Closing a tab SHALL NOT close its notifications.

#### Scenario: Program closes its notice
- **WHEN** a program raised a notification with `i=n1` and then prints `ESC ] 99 ; i=n1:p=close ; ESC \`
- **THEN** the notification disappears

### Requirement: OSC 99 queries
`p=?` SHALL write `ESC ] 99;i=<id>:p=?;<capabilities> ESC \` (`i=0` when the query has no `i`) to that terminal, listing only what Brindle supports there, in kitty's order: `a=focus,report`, `o=always,unfocused,invisible`, on Linux `u=0,1,2`, `p=title,body,?,close,alive` plus `,buttons` where buttons can be shown, `c=1`, on Linux `w=1`, and `s=system,silent`, joined by `:`. `p=alive` with an `i` SHALL write `ESC ] 99;i=<id>:p=alive;<ids> ESC \`, the comma-separated ids of that terminal's open notifications, oldest first; without an `i` it SHALL be ignored.

#### Scenario: Query on Linux
- **WHEN** a program on Linux prints `ESC ] 99 ; i=q:p=? ; ESC \`
- **THEN** it reads `ESC ] 99;i=q:p=?;a=focus,report:o=always,unfocused,invisible:u=0,1,2:p=title,body,?,close,alive,buttons:c=1:w=1:s=system,silent ESC \`, without `,buttons` when the notification service shows no actions

#### Scenario: Query on Windows
- **WHEN** a program on Windows prints `ESC ] 99 ; i=q:p=? ; ESC \`
- **THEN** it reads `ESC ] 99;i=q:p=?;a=focus,report:o=always,unfocused,invisible:p=title,body,?,close,alive,buttons:c=1:s=system,silent ESC \`

#### Scenario: Alive
- **WHEN** a terminal has open notifications `a` and `b`, and prints `ESC ] 99 ; i=x:p=alive ; ESC \`
- **THEN** it reads `ESC ] 99;i=x:p=alive;a,b ESC \`

### Requirement: OSC 99 click actions
`a` SHALL list actions, a leading `-` removing one: `focus` (on unless removed) raises the window and activates the tab as for other notifications; `report` writes `ESC ] 99;i=<id>; ESC \` to the notification's terminal on a click. A notification without `focus` SHALL do nothing else on a click.

#### Scenario: Click report
- **WHEN** a program raises a notification with `i=r:a=report`, and the user clicks it
- **THEN** the window comes forward with that tab active, and the program reads `ESC ] 99;i=r; ESC \`

#### Scenario: No focus
- **WHEN** a notification raised with `a=-focus,report` is clicked
- **THEN** the window is not raised, and the program reads the click report

### Requirement: OSC 99 close reports
A notification raised with `c=1` SHALL, when it closes for any reason (dismissed, expired, clicked away, or the program's own `p=close`), write `ESC ] 99;i=<id>:p=close; ESC \` to its terminal. A notification replaced by an update SHALL NOT count as closed, and only the latest update's `c` SHALL count.

#### Scenario: Dismissed
- **WHEN** a notification raised with `i=d:c=1` is dismissed by the user
- **THEN** the program reads `ESC ] 99;i=d:p=close; ESC \`

#### Scenario: Closed by the program
- **WHEN** a notification raised with `i=d:c=1` is closed with `ESC ] 99 ; i=d:p=close ; ESC \`
- **THEN** the program reads `ESC ] 99;i=d:p=close; ESC \`

### Requirement: OSC 99 occasion and hints
`o=always` (the default), `o=unfocused` (only when the focus rule's tab-not-visible test holds) and `o=invisible` (only when the tab is not its window's active tab) SHALL apply on top of the `when` setting, never overriding it. On Linux, `u` (0, 1, 2) SHALL set the urgency hint and `w` (milliseconds, `-1` for the server's default, `0` for never) the expiry. Elsewhere they SHALL be ignored.

#### Scenario: Program asks for always, user says unfocused
- **WHEN** `when` is `"unfocused"` and the active tab of the focused window prints `ESC ] 99 ; o=always ; hi ESC \`
- **THEN** no notification appears

#### Scenario: Critical on Linux
- **WHEN** a background tab on Linux prints `ESC ] 99 ; u=2 ; Disk full ESC \`
- **THEN** the notification is sent with urgency critical

### Requirement: OSC 99 buttons
`p=buttons` SHALL set the notification's buttons: labels separated by U+2028, at most 4 (later ones dropped), each at most 64 characters, empty labels dropped. Where the notification service shows actions (on Windows, always), the buttons SHALL be shown, and pressing button N (from 1) of a notification raised with `a=report` SHALL write `ESC ] 99;i=<id>;<N> ESC \` to its terminal, without raising the window whatever `a` says of `focus`. Without `report`, a press SHALL write nothing. Where actions aren't shown, the notification SHALL be shown without them.

#### Scenario: Permission prompt
- **WHEN** a background tab sends `ESC ] 99 ; i=p:a=report:d=0:p=buttons ; Allow<U+2028>Deny ESC \` then `ESC ] 99 ; i=p ; Run cargo test? ESC \`, and the user presses Deny
- **THEN** the program reads `ESC ] 99;i=p;2 ESC \`, and the window is not raised

#### Scenario: No actions on the server
- **WHEN** the Linux notification service doesn't advertise actions and a program sends a notification with buttons
- **THEN** the notification appears without buttons

#### Scenario: Update replaces buttons
- **WHEN** a notification with `i=p` has buttons, and the program completes an update with `i=p` and no `p=buttons` chunk
- **THEN** the updated notification has no buttons

### Requirement: OSC 99 sounds
`s=silent` (base64 of `silent`, as every `s` value is) SHALL show the notification without a sound, on Linux with the `suppress-sound` hint and on Windows as a silent toast. Any other `s` value, including `system` and one that isn't base64, SHALL play the platform's default sound.

#### Scenario: Silent
- **WHEN** a background tab prints `ESC ] 99 ; s=c2lsZW50 ; Synced ESC \`
- **THEN** the notification appears without a sound

#### Scenario: Named sound
- **WHEN** a background tab prints a notification whose `s` names a sound such as `warning`
- **THEN** it appears with the default sound

### Requirement: Unsupported OSC 99 parts
`p=icon` and any other payload type not named above SHALL be ignored whole. The keys `f`, `g`, `n` and `t` SHALL be ignored, and the notification shown without them.

#### Scenario: Icon chunk
- **WHEN** a program sends `ESC ] 99 ; i=x:d=0:p=icon ; <data> ESC \` and then `ESC ] 99 ; i=x ; Done ESC \`
- **THEN** one notification titled `Done — <tab title>` appears, with Brindle's own icon
