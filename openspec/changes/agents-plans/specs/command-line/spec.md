## ADDED Requirements

### Requirement: Plans in the dump
In an agent zone's dump, after the `--- agent` line and the lines for the session's commands and options, the dump SHALL print a `--- plan` line per plan, with its id, its kind, whether the zone shows plans open, and its progress and current step or its path and text, followed for a checklist by one `--- plan-item` line per item, whether open or closed. Texts SHALL be escaped onto one line, and `--wait-for` SHALL match these lines.

#### Scenario: A checklist in the dump
- **WHEN** the fake ACP agent reports an unnamed plan of two items, `Read the test` completed and `Fix it` in progress, and the dump is taken
- **THEN** the dump has `--- plan id="" kind=items done=1 total=2 open=no current="Fix it"`, then `--- plan-item plan="" index=0 status=completed priority=medium "Read the test"` and `--- plan-item plan="" index=1 status=in-progress priority=medium "Fix it"`

#### Scenario: Waiting for the plan to finish
- **WHEN** Brindle runs with `--wait-for 'done=2 total=2'` against a fake ACP agent that completes both items after a delay
- **THEN** the later steps run once both are completed

#### Scenario: Opened by an action
- **WHEN** Brindle runs `--action toggle_agent_plan --dump-screen-after 2` while a plan shows
- **THEN** the plan's line has `open=yes`
