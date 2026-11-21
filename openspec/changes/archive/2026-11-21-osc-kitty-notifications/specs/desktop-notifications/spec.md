# Spec Delta

## ADDED Requirements

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

## MODIFIED Requirements

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
