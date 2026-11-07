## ADDED Requirements

### Requirement: Close behaviour
A profile SHALL be able to set `on_close` to `detach` (the default) or `ask`: whether closing or quitting asks before leaving its local Linux terminals running (see `tabs` and `zones`). No profile setting SHALL end a program on close. The value when a terminal starts SHALL apply for its whole life. Windows, remote and tmux profiles SHALL ignore it. An unknown value SHALL be a config error naming the key and the accepted values.

#### Scenario: Default
- **WHEN** a profile doesn't mention `on_close`
- **THEN** its local tabs behave as `on_close = "detach"`

#### Scenario: Changed while a tab runs
- **WHEN** a tab was started with the default `on_close`, the user changes the profile to `ask` and then closes the tab
- **THEN** the tab closes without asking and its program keeps running detached

#### Scenario: Kill is not a value
- **WHEN** the config sets `on_close = "kill"`
- **THEN** the config error names `on_close` and lists `detach` and `ask`
