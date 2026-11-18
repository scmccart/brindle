# Spec Delta

## MODIFIED Requirements

### Requirement: Theme list
The Theme section SHALL list every built-in theme followed by the user's custom themes, in a stable order. The active global theme SHALL be marked, and selected whenever the Theme section is shown, and custom themes SHALL be marked as custom. The custom themes SHALL be read from the config file even when another part of the file has an error. A custom theme that cannot be read SHALL be listed with its error and SHALL NOT be selectable. When the file can't be read, isn't valid TOML, or has a `themes` key that isn't a table, the section SHALL say so, naming the file and the cause, and list the custom themes of the config in effect.

#### Scenario: Custom theme listed
- **WHEN** the config defines `[themes.mine]` and `theme = "mine"`, and the user opens the dialog and selects "Theme"
- **THEN** "mine" is listed after the built-ins, marked custom and active, and is selected

#### Scenario: Error elsewhere in the config
- **WHEN** the config has an unknown key `fontsize = 3` and defines `[themes.mine]`
- **THEN** "mine" is still listed in the dialog

#### Scenario: Themes of the wrong shape
- **WHEN** the config sets `themes = 3`, and the user opens the dialog and selects "Theme"
- **THEN** the section says `themes` in the config file is not a table, and lists the built-in themes

### Requirement: Profile list
The Profiles section SHALL list every profile in config order, each with its ctrl-alt-N number (for the first nine), a short description of what it runs, and a marker on the default profile. When the config defines no profiles, the implicit "Shell" profile SHALL be listed. The profiles SHALL be read from the config file even when another part of the file has an error. A profile that cannot be read SHALL be listed with its error and SHALL NOT be editable. When the file can't be read, isn't valid TOML, or has a `profiles` key that isn't an array, the section SHALL be read-only and say so, naming what was being done, the file and the cause. A config file that doesn't exist SHALL read as an empty one.

#### Scenario: Profiles in order
- **WHEN** the config defines `Shell`, `tmux` and `Work`, with `default_profile = "work"`
- **THEN** the list shows `1 Shell`, `2 tmux` and `3 Work`, and `Work` is marked default

#### Scenario: Error elsewhere in the config
- **WHEN** the config has an unknown top-level key and defines a profile `Work`
- **THEN** `Work` is still listed and editable

#### Scenario: Unreadable config file
- **WHEN** the config file exists but can't be read, and the user selects "Profiles"
- **THEN** the section is read-only and says reading that file failed, with the path and the system's error

#### Scenario: No config file
- **WHEN** no config file exists and the user selects "Profiles"
- **THEN** the implicit "Shell" profile is listed and editable

### Requirement: Profile field validation
The editor SHALL mark invalid fields and SHALL block saving while any field is invalid:
- a name that is empty after trimming, or that matches another profile's name ignoring case;
- arguments or environment that are not valid shell words, such as an unclosed quote;
- arguments or environment from the file that can't be written as shell words, such as a word with a NUL character;
- an environment word without `=`, or whose key is empty or not a valid variable name;
- a font size that is not a number from 4 to 96.

#### Scenario: Duplicate name
- **WHEN** profiles `Shell` and `Work` exist and the user renames `Work` to `shell`
- **THEN** the name field is marked as already used, and Save is unavailable

#### Scenario: Unclosed quote
- **WHEN** the user types `-c "echo hi` as arguments
- **THEN** the arguments field is marked invalid and Save is unavailable

#### Scenario: Bad environment word
- **WHEN** the user types `EDITOR=nvim verbose` as the environment
- **THEN** the environment field is marked invalid because `verbose` has no `=`

#### Scenario: Argument that can't be quoted
- **WHEN** a profile's `args` in the file holds a word with a NUL character, and the user edits that profile
- **THEN** the arguments field is marked invalid, saying the word can't be written as a shell word, and Save is unavailable
