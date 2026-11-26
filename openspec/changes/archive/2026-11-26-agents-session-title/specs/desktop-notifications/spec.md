## MODIFIED Requirements

### Requirement: Agent notifications
An agent zone whose status becomes `blocked`, `done` or `error` from another status SHALL raise a desktop notification, subject to the focus rule and the program rate limits. Its title SHALL be the zone's title, followed by ` — <tab title>` only when the tab has a manual title that differs from it, and its body the permission request's title, `Done`, or the failure's message. Replayed conversations SHALL NOT notify.

#### Scenario: Permission in a background tab
- **WHEN** the agent of profile `Fake`, which names itself `Fake Agent`, asks `Run cargo test?` in a background tab with no manual title
- **THEN** a notification appears titled `Fake` with the body `Run cargo test?`

#### Scenario: A named conversation
- **WHEN** the agent has titled its conversation `Hello.txt file creation` and its turn ends normally in a background tab
- **THEN** a notification appears titled `Hello.txt file creation` with the body `Done`

#### Scenario: A renamed tab
- **WHEN** the user renamed the agent's tab `Build`, and the agent of profile `Fake` ends its turn normally in the background
- **THEN** a notification appears titled `Fake — Build` with the body `Done`

#### Scenario: Turn done
- **WHEN** an agent's turn ends normally in a background tab
- **THEN** a notification appears with the body `Done`

#### Scenario: Visible tab stays quiet
- **WHEN** `when` is `"unfocused"` and the agent in the active tab of the focused window ends its turn
- **THEN** no notification appears
