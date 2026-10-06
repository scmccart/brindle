# Spec Delta

## ADDED Requirements

### Requirement: Writes from Brindle preserve the file
When Brindle writes the config file itself, for example from the settings dialog, it SHALL write to the same path it reads (`$BRINDLE_CONFIG` when set). It SHALL change only the keys and tables the write concerns, keeping comments, ordering, formatting and every other key as they were. It SHALL NOT write a file that is not valid TOML. A write SHALL replace the file in one step, so a failed or interrupted write never leaves a partial file.

#### Scenario: Comments survive
- **WHEN** the config has comments and a `[keybindings]` table, and the user applies `tokyo-night` from the settings dialog
- **THEN** only the `theme` value changes; the comments and `[keybindings]` are unchanged

#### Scenario: Malformed file left alone
- **WHEN** the config file has a TOML syntax error and Brindle is asked to write it
- **THEN** the write is refused with the parse error and the file is unchanged

#### Scenario: First write with no file
- **WHEN** the config file does not exist and the user applies a theme
- **THEN** the file is created with the default config contents and the new `theme` value
