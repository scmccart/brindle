# Spec Delta

## ADDED Requirements

### Requirement: Agent profiles in the profile list
The Profiles section SHALL list agent profiles in config order with the others, each row showing its agent description and marked as an agent. Edit and Duplicate SHALL be unavailable for an agent profile, with a note that agent profiles are edited in the config file. Delete and reordering SHALL work on it as on any profile, and it SHALL show the default marker when `default_profile` names it.

#### Scenario: Listed and marked
- **WHEN** the config defines `Shell` and an agent profile `Agent` running `agent-acp`
- **THEN** the list shows `2 Agent` with `agent · agent-acp`, marked as an agent, and Edit is unavailable while it is selected

#### Scenario: Reordered intact
- **WHEN** the user selects `Agent` and presses alt-up
- **THEN** `Agent` is first in the config, and its table still has `type = "agent"`, its command, arguments and environment, and its comments

### Requirement: Agent profiles survive the dialog's writes
No write the settings dialog makes (saving another profile, renaming one, changing the default, reordering, deleting another profile, applying a theme, fonts or clipboard settings) SHALL change an agent profile's table, other than moving it whole.

#### Scenario: Saving another profile
- **WHEN** the config has an agent profile `Agent` and the user edits and saves `Shell`
- **THEN** the `Agent` table is unchanged, byte for byte
