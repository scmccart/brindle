## MODIFIED Requirements

### Requirement: Agent notifications
An agent zone whose status becomes `blocked`, `done` or `error` from another status SHALL raise a desktop notification, subject to the focus rule and the program rate limits. Its title SHALL be the zone's profile name, followed by ` needs permission` for a request, then ` — <zone title>` when the zone's title differs from the profile's name, then ` — <tab title>` when the tab has a manual title that is neither. Its body SHALL be the request's text, `Done`, or the failure's message. Replayed conversations SHALL NOT notify.

#### Scenario: Permission in a background tab
- **WHEN** the agent of profile `Fake`, in a background tab with no manual title and a zone the agent hasn't named, asks `Run cargo test?`
- **THEN** a notification appears titled `Fake needs permission`

#### Scenario: A named conversation
- **WHEN** the agent of profile `Fake` has named its conversation `Hello.txt file creation` and its turn ends normally in a background tab
- **THEN** a notification appears titled `Fake — Hello.txt file creation` with the body `Done`

#### Scenario: A renamed tab
- **WHEN** the user renamed the agent's tab `Build`, and the agent of profile `Fake`, whose zone the agent hasn't named, fails its turn in the background
- **THEN** a notification appears titled `Fake — Build` with the failure's message as its body

#### Scenario: Turn done
- **WHEN** an agent's turn ends normally in a background tab
- **THEN** a notification appears with the body `Done`

#### Scenario: Visible tab stays quiet
- **WHEN** `when` is `"unfocused"` and the agent in the active tab of the focused window ends its turn
- **THEN** no notification appears

## ADDED Requirements

### Requirement: Agent permission notification text
A permission notification's body SHALL be two lines. The first SHALL be the request's title, after its tool kind's word and a colon when the request names a tool call whose kind is known and isn't `other` (`Command: cargo test`). The second SHALL be the request's options in its row's order, joined by ` · `, ending ` · +N more` when N more of the zone's requests wait. Every platform's notifier SHALL show both lines.

#### Scenario: A command
- **WHEN** the agent asks permission titled `cargo test` for an `execute` tool call, with options `Allow once` and `Reject`
- **THEN** the notification's body is `Command: cargo test` then `Reject · Allow once`

#### Scenario: No tool call
- **WHEN** the agent asks permission titled `Use the network?` naming no tool call, with options `Allow` and `Deny`
- **THEN** the notification's body is `Use the network?` then `Deny · Allow`

#### Scenario: More requests waiting
- **WHEN** the agent of a working zone asks two permissions at once, the later with options `Allow once` and `Reject`
- **THEN** one notification is raised, its first line naming the later request and its second line `Reject · Allow once · +1 more`

#### Scenario: A Windows toast
- **WHEN** a permission notification is shown on Windows
- **THEN** the toast shows the title, and the body's two lines as its two text lines

#### Scenario: The test log
- **WHEN** `BRINDLE_NOTIFY_LOG` is set and a permission notification is sent
- **THEN** its log line holds the title, and the body with its line break written as a space
