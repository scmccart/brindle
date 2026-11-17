# Spec Delta

## MODIFIED Requirements

### Requirement: Direct profile shortcuts
ctrl-alt-N SHALL open a tab with the Nth profile, for N from 1 to 9. When the config has fewer than N profiles, ctrl-alt-N, and a profile chosen from a menu or picker that the config no longer has, SHALL open a message tab saying there is no such profile and how many the config has.

#### Scenario: Second profile
- **WHEN** the user presses ctrl-alt-2
- **THEN** a tab opens using the second profile

#### Scenario: No fifth profile
- **WHEN** the config has three profiles and the user presses ctrl-alt-5
- **THEN** a message tab opens saying there is no profile 5 and that the config has three
