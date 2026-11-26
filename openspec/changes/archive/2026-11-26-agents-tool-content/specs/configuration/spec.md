## ADDED Requirements

### Requirement: Agent tool detail settings
The config SHALL control, in an `[agent]` table, how tool call rows in agent zones open on their own: `tool_details` (`"open"`, the default, or `"closed"`) for a tool call that is running, and `tool_details_when_done` (`"closed"`, the default, or `"open"`) for one that has finished. Another value SHALL be reported as an invalid config is. A reload SHALL apply both to open agent zones as well as new ones, leaving rows the user opened or closed by hand as they are.

#### Scenario: Defaults
- **WHEN** the config has no `[agent]` table
- **THEN** a running tool call's row is open, and it closes when the tool call finishes

#### Scenario: Everything closed
- **WHEN** the config sets `tool_details = "closed"` under `[agent]`
- **THEN** a running tool call's row shows its line alone

#### Scenario: Kept open
- **WHEN** the config sets `tool_details_when_done = "open"` under `[agent]`
- **THEN** a tool call's row is still open after the tool call finishes

#### Scenario: Reload
- **WHEN** an agent zone shows three finished tool calls, closed, and the user sets `tool_details_when_done = "open"` and saves
- **THEN** the three rows open without a restart

#### Scenario: A value that isn't one of the two
- **WHEN** the config sets `tool_details = "maybe"` under `[agent]`
- **THEN** Brindle starts with the defaults and its config line names `agent.tool_details` and the values it takes
