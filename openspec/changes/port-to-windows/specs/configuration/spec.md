# Spec Delta

## MODIFIED Requirements

### Requirement: Config location and first run
The config SHALL be read from `$BRINDLE_CONFIG` when set. Otherwise it SHALL be read from `brindle/config.toml` in the platform's user config directory:
- `$XDG_CONFIG_HOME` on Linux;
- `%APPDATA%` on Windows.

If the file does not exist, a commented default config SHALL be written there on first launch.

#### Scenario: First launch
- **WHEN** Brindle starts and no config file exists
- **THEN** a commented default config is written and Brindle starts with those settings

#### Scenario: Config on Windows
- **WHEN** Brindle starts on Windows with `BRINDLE_CONFIG` unset
- **THEN** it reads, or on first launch writes, `%APPDATA%\brindle\config.toml`
