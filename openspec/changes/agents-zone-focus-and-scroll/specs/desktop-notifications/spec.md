# Spec Delta

## MODIFIED Requirements

### Requirement: Rate limits
Notifications SHALL be rate limited: at most one bell notification per tab every 5 seconds; per tab, at most 3 program (OSC) notifications at once, then one every 2 seconds; per tab, apart from those, at most 3 agent notifications at once, then one every 2 seconds; across all tabs, at most 10 at once, then one every 2 seconds. Notifications over a limit SHALL be dropped. A tab's bell notification SHALL replace its previous one instead of adding another.

#### Scenario: Bell storm
- **WHEN** a background tab rings the bell 100 times within a second
- **THEN** one notification appears

#### Scenario: Notification loop
- **WHEN** a background tab prints 50 OSC 9 messages within a second
- **THEN** three notifications appear

#### Scenario: An agent beside a noisy terminal
- **WHEN** a background tab holds a terminal zone that prints three OSC 9 messages within a second, and an agent zone whose agent then asks permission
- **THEN** four notifications appear, the last titled for the agent's permission

### Requirement: Agent notifications
An agent zone whose status becomes `blocked`, `done` or `error` from another status SHALL raise a desktop notification, subject to the focus rule and the agent rate limits. Its title SHALL be the zone's profile name, followed by ` needs permission` for a request, then ` — <zone title>` when the zone's title differs from the profile's name, then ` — <tab title>` when the tab has a manual title that is neither. Its body SHALL be the request's text, `Done`, or the failure's message. Replayed conversations SHALL NOT notify.

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
