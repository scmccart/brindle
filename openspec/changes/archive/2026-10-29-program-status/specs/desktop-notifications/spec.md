# Spec Delta

## ADDED Requirements

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

## MODIFIED Requirements

### Requirement: Notification settings
The `[notifications]` table SHALL enable or disable each source with `bell`, `osc9`, `osc777` and `osc7501` (all `true` by default) and set `when` to `"unfocused"` or `"always"`. Changes SHALL apply on config reload.

#### Scenario: Bells off
- **WHEN** the config sets `notifications.bell = false` and a background tab rings the bell
- **THEN** no notification appears, and the tab still shows its bell dot

#### Scenario: Program status off
- **WHEN** the config sets `notifications.osc7501 = false` and a background tab reports `state=blocked`
- **THEN** no notification appears, and the tab still shows its status dot
