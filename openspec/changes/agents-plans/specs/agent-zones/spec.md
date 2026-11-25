## ADDED Requirements

### Requirement: The plan above the prompt input
While its session has a plan, an agent zone SHALL show it between the conversation and the prompt input, where it stays in view as the conversation scrolls. Closed, a checklist plan SHALL be one line: how many of its items are completed, how many there are, and the text of the item in progress, else of the first pending one. A Markdown plan's line SHALL be its first heading, else its first line. It SHALL follow the plan as the agent changes it, and SHALL go when the session has no plan.

#### Scenario: Shown while working
- **WHEN** the agent reports a plan of seven items with three completed and `Run the tests` in progress
- **THEN** a line above the prompt input reads 3 of 7 and `Run the tests`

#### Scenario: Stays in view
- **WHEN** the agent then streams a reply longer than the zone
- **THEN** the plan's line is still in view above the prompt input

#### Scenario: Follows the agent
- **WHEN** the agent reports `Run the tests` completed and `Fix the lint` in progress
- **THEN** the line reads 4 of 7 and `Fix the lint`

#### Scenario: No plan
- **WHEN** the session has no plan
- **THEN** nothing shows between the conversation and the prompt input for one

### Requirement: The open plan
An open plan SHALL list its items in the agent's order, each with a mark for its status and its text rendered as agent messages are, completed and cancelled items muted and high-priority items marked. A Markdown plan SHALL show its text rendered, and its file's path when it has one. The list SHALL be no taller than a part of the zone and SHALL scroll within itself. Status SHALL never be shown by colour alone.

#### Scenario: A checklist
- **WHEN** an open plan has a completed, an in-progress and a pending item
- **THEN** the three show in order, each with its own status mark, the completed one muted

#### Scenario: A long plan
- **WHEN** an open plan has 60 items
- **THEN** the list scrolls within itself, and the conversation and the prompt input keep their places

#### Scenario: Several plans
- **WHEN** the session has two plans
- **THEN** both show, in the order the agent first sent them, each with its own line

### Requirement: Opening and closing the plan
A plan SHALL start closed, unless `[agent] plan_details` says otherwise. A pointer press on its line, and the `toggle_agent_plan` action, SHALL open or close it for the zone, and it SHALL stay as the user left it while the agent changes the plan. The command palette SHALL list Toggle Agent Plan only while the active agent zone's session has a plan. The plan SHALL add no keyboard stop.

#### Scenario: Opened from the palette
- **WHEN** a closed plan shows and the user runs Toggle Agent Plan
- **THEN** its items show, and running it again leaves the one line

#### Scenario: Stays open
- **WHEN** the plan is open and the agent reports an item completed
- **THEN** the plan is still open, with that item marked completed

#### Scenario: Not listed without a plan
- **WHEN** the session has no plan and the user opens the command palette
- **THEN** Toggle Agent Plan is not listed

#### Scenario: Tab passes over it
- **WHEN** a plan shows, the prompt input has focus and the user presses tab
- **THEN** focus goes to the zone's next stop, never into the plan
