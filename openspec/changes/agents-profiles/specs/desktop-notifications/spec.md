# Spec Delta

## MODIFIED Requirements

### Requirement: Notification settings
The `[notifications]` table SHALL enable or disable each source with `bell`, `osc9`, `osc777`, `osc99`, `osc7501` and `agent` (all `true` by default) and set `when` to `"unfocused"` or `"always"`. Changes SHALL apply on config reload.

#### Scenario: Bells off
- **WHEN** the config sets `notifications.bell = false` and a background tab rings the bell
- **THEN** no notification appears, and the tab still shows its bell dot

#### Scenario: Program status off
- **WHEN** the config sets `notifications.osc7501 = false` and a background tab reports `state=blocked`
- **THEN** no notification appears, and the tab still shows its status dot

#### Scenario: OSC 99 off
- **WHEN** the config sets `notifications.osc99 = false` and a program prints `ESC ] 99 ; i=q:p=? ; ESC \`
- **THEN** no notification appears for any OSC 99 sequence, and the query gets no reply

#### Scenario: Agents off
- **WHEN** the config sets `notifications.agent = false` and an agent in a background tab asks permission
- **THEN** no notification appears, and the tab still shows its status dot

## ADDED Requirements

### Requirement: Agent notifications
An agent zone whose status becomes `blocked`, `done` or `error` from another status SHALL raise a desktop notification, subject to the focus rule and the program rate limits. Its title SHALL be `<agent name> — <tab title>`, and its body the permission request's title, `Done`, or the failure's message. Replayed conversations SHALL NOT notify.

#### Scenario: Permission in a background tab
- **WHEN** the agent `Fake Agent` in a background tab titled `Fake Agent` asks `Run cargo test?`
- **THEN** a notification appears titled `Fake Agent — Fake Agent` with the body `Run cargo test?`

#### Scenario: Turn done
- **WHEN** an agent's turn ends normally in a background tab
- **THEN** a notification appears with the body `Done`

#### Scenario: Visible tab stays quiet
- **WHEN** `when` is `"unfocused"` and the agent in the active tab of the focused window ends its turn
- **THEN** no notification appears
