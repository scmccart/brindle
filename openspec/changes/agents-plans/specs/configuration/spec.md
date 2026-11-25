## ADDED Requirements

### Requirement: Agent plan setting
The config SHALL control, with `plan_details` in the `[agent]` table, whether an agent zone's plan starts as one line (`"closed"`, the default) or with its items showing (`"open"`). Another value SHALL be reported as an invalid config is. A reload SHALL apply it to open agent zones as well as new ones, leaving a zone whose plan the user opened or closed by hand as it is.

#### Scenario: Default
- **WHEN** the config sets no `plan_details` and an agent reports a plan
- **THEN** the plan shows as one line

#### Scenario: Open by default
- **WHEN** the config sets `plan_details = "open"` under `[agent]` and an agent reports a plan
- **THEN** the plan's items show

#### Scenario: Reload
- **WHEN** an agent zone shows a closed plan the user hasn't touched, and the user sets `plan_details = "open"` and saves
- **THEN** the plan opens without a restart

#### Scenario: A value that isn't one of the two
- **WHEN** the config sets `plan_details = "auto"` under `[agent]`
- **THEN** Brindle starts with the defaults and its config line names `agent.plan_details` and the values it takes
