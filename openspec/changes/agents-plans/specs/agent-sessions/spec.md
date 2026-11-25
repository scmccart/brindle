## ADDED Requirements

### Requirement: The agent's plans are kept
The server SHALL keep the plans the agent reports: each a checklist of items, every item with its text, its status (pending, in progress, completed or cancelled) and its priority, or else a Markdown text with an optional file path. A plan the agent sends again SHALL replace the plan it had with that id, whole. Plans SHALL be sent to attached clients when they change, SHALL be in every replay, and SHALL survive detaches, core restarts and upgrades.

#### Scenario: A plan arrives
- **WHEN** the agent reports a plan of three items, the first in progress and the others pending
- **THEN** an attached client receives the plan with those three items and their statuses

#### Scenario: The plan moves on
- **WHEN** the agent then reports the same plan with the first item completed and the second in progress
- **THEN** the session holds one plan, with those statuses

#### Scenario: Replayed
- **WHEN** a session whose agent reported a plan is detached and a client attaches it
- **THEN** the replay holds the plan as the agent last reported it

#### Scenario: The same plan again
- **WHEN** the agent reports the plan it already reported, unchanged
- **THEN** attached clients receive nothing for it

#### Scenario: Kept across a restart
- **WHEN** a session has a plan, the core restarts, and a client attaches the session
- **THEN** the replay holds the same plan

### Requirement: Both forms of a plan are read
The server SHALL read a plan the agent sends without an id as the session's one unnamed plan, and SHALL read plans the agent sends with ids, as checklists or as Markdown, each as a plan of its own, telling the agent at the start of the session that it accepts them. A plan sent again with its id as the other type SHALL replace the first. The agent removing a plan by its id, or sending the unnamed plan with no items, SHALL remove it; removing a plan the session doesn't hold SHALL change nothing.

#### Scenario: The unnamed plan
- **WHEN** the agent sends a `plan` update with two entries
- **THEN** the session holds one plan with those two items

#### Scenario: A named Markdown plan
- **WHEN** the agent sends a plan with the id `design` as Markdown text and the path `/p/PLAN.md`
- **THEN** the session holds a Markdown plan `design` with that text and path

#### Scenario: Two plans
- **WHEN** the agent sends plans with the ids `a` and `b`
- **THEN** the session holds both, in the order they arrived

#### Scenario: Removed
- **WHEN** the agent removes the plan `a`
- **THEN** the session holds `b` alone, and attached clients are told `a` is gone

#### Scenario: An empty unnamed plan
- **WHEN** the agent sends a `plan` update with no entries
- **THEN** the session's unnamed plan is removed

#### Scenario: Removing a plan that isn't there
- **WHEN** the agent removes a plan with an id the session doesn't hold
- **THEN** nothing changes, and the session carries on

### Requirement: Plans are bounded
At most 8 plans SHALL be kept for a session, each with at most 256 items, and at most 64 KiB of a Markdown plan's text, with texts cleaned and bounded as tool call titles are. An item, or a plan, the server can't read SHALL be left out without losing the others. What is left out SHALL be said in one notice.

#### Scenario: Too many items
- **WHEN** the agent reports a plan of 300 items
- **THEN** the first 256 are kept, and a notice says 300 were sent and 256 are kept

#### Scenario: One unreadable item
- **WHEN** the agent reports a plan of three items, the second with no text
- **THEN** the first and third are kept, and a notice says one item couldn't be read

#### Scenario: A ninth plan
- **WHEN** the session holds 8 plans and the agent sends one with a new id
- **THEN** it isn't kept, and a notice says a session keeps at most 8 plans

### Requirement: A finished plan is cleared when the next turn starts
When a prompt is sent to the agent, the server SHALL first remove every checklist plan whose items are all completed or cancelled, and tell attached clients. A plan with an item still pending or in progress, and a Markdown plan, SHALL stay. A turn's end SHALL NOT remove a plan.

#### Scenario: Finished, then a new prompt
- **WHEN** the agent's plan has every item completed, the turn ends, and the user sends another prompt
- **THEN** the plan is removed before the agent receives the prompt

#### Scenario: Still on view between turns
- **WHEN** the agent's plan has every item completed and the turn ends
- **THEN** the session still holds the plan

#### Scenario: Unfinished
- **WHEN** the agent's plan has one item pending, the turn ends, and the user sends another prompt
- **THEN** the plan stays

#### Scenario: A queued prompt
- **WHEN** a prompt waits in the queue, the agent's plan becomes all completed, and the turn ends with `end_turn`
- **THEN** the plan is removed when the queued prompt is sent
