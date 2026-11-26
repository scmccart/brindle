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

#### Scenario: A conversation longer than a page
- **WHEN** a client attaches a session whose conversation is longer than the page an attach sends, and whose agent reported a plan
- **THEN** the replay holds the newest page of entries, and the plan whole

### Requirement: Plans belong to the conversation
A session's plans, and what was dismissed, SHALL stay when its agent stops and when the same conversation is continued by a new agent. When the session starts a new conversation with its agent, the server SHALL remove them and tell attached clients. Plans an agent reports while replaying a conversation it was asked to load SHALL be kept as at any other time, whether or not the replay is shown, and SHALL NOT be counted among the updates a hidden replay left out.

#### Scenario: The agent exits
- **WHEN** a session has a plan and its agent exits
- **THEN** the session still holds the plan, and a client can dismiss it

#### Scenario: Continued
- **WHEN** that conversation is continued by a new agent that takes up the same ACP session
- **THEN** the session holds the plan as before, until the agent next reports it

#### Scenario: A new conversation
- **WHEN** a session with a plan can't continue its conversation and starts a new one
- **THEN** attached clients are told the plan is gone

#### Scenario: A dismissed plan replayed
- **WHEN** a client dismissed a plan, the agent exits, and the continued agent replays the conversation with that plan unchanged
- **THEN** the session still holds no plan

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
A plan SHALL hold at most 256 items, and a Markdown plan at most 64 KiB of text, with texts cleaned and bounded as tool call titles are. The number of plans SHALL NOT be limited, but a plan that would bring the session's plans over 4 MiB together SHALL NOT be kept. An item, or a plan, the server can't read SHALL be left out without losing the others. What is left out SHALL be said in one notice.

#### Scenario: Too many items
- **WHEN** the agent reports a plan of 300 items
- **THEN** the first 256 are kept, and a notice says 300 were sent and 256 are kept

#### Scenario: One unreadable item
- **WHEN** the agent reports a plan of three items, the second with no text
- **THEN** the first and third are kept, and a notice says one item couldn't be read

#### Scenario: Many plans
- **WHEN** the agent sends 40 small plans with different ids
- **THEN** the session holds all 40

#### Scenario: Too much plan text together
- **WHEN** the session's plans hold 4 MiB together and the agent sends another
- **THEN** it isn't kept, and a notice says the session's plans are at their limit

### Requirement: The server never clears a plan on its own
A plan SHALL stay as the agent last reported it until the agent replaces or removes it, or a client dismisses it. A turn's end, every item being completed, and a new prompt SHALL NOT remove a plan.

#### Scenario: Finished and kept
- **WHEN** the agent's plan has every item completed, the turn ends, and the user sends another prompt
- **THEN** the session still holds the plan

#### Scenario: Stale and kept
- **WHEN** the agent's plan has an item in progress, the agent ends the turn without updating it, and a day passes
- **THEN** the session still holds the plan, with that item in progress

### Requirement: A client dismisses a plan
A client attached to an agent session SHALL be able to dismiss one plan, by its id, or every plan the session holds. The server SHALL remove it from the session and tell every attached client, and SHALL NOT tell the agent. Dismissing a plan the session doesn't hold SHALL be refused with an error.

#### Scenario: Dismissed
- **WHEN** a client dismisses the session's plan
- **THEN** every attached client is told the plan is gone, a later replay holds no plan, and the agent receives nothing

#### Scenario: Dismiss all
- **WHEN** the session holds two plans and a client dismisses every plan
- **THEN** both are removed

#### Scenario: Nothing to dismiss
- **WHEN** a client dismisses a plan with an id the session doesn't hold
- **THEN** it gets an error saying there is no such plan

### Requirement: A dismissed plan stays away until the agent changes it
After a plan is dismissed, the same plan sent again by the agent, unchanged, SHALL NOT bring it back. A plan the agent sends under that id that differs from the dismissed one SHALL be kept and shown again. This SHALL hold across detaches, core restarts and upgrades.

#### Scenario: The agent repeats itself
- **WHEN** a client dismisses a plan, and the agent then sends that plan again with the same items and statuses
- **THEN** the session still holds no plan

#### Scenario: The agent makes progress
- **WHEN** a client dismisses a plan, and the agent then sends it with another item completed
- **THEN** the session holds the plan again, as the agent now reports it

#### Scenario: Across a restart
- **WHEN** a client dismisses a plan, the core restarts, and the agent sends the same plan again
- **THEN** the session still holds no plan
