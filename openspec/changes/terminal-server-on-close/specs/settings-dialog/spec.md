## MODIFIED Requirements

### Requirement: Profile editor fields
The profile editor SHALL edit:
- name;
- command (blank means the user's shell);
- arguments, as one line of shell words;
- working directory (blank means the default);
- environment, as one line of `KEY=value` shell words;
- theme: "follow global", or any built-in or custom theme;
- font size (blank means the global size);
- on close: keep running (`detach`) or ask, shown only when the profile has no tmux mode and no remote target;
- tmux mode: none, plain or control;
- tmux session (blank means `main`) and tmux arguments, as shell words, shown only when the tmux mode isn't none;
- whether the profile is the default.

The editor SHALL say that command, arguments, directory, environment, on close and tmux settings apply to new tabs only.

#### Scenario: Shell words
- **WHEN** the user enters `-L work -f "~/my tmux.conf"` as tmux arguments and saves
- **THEN** the saved `tmux_args` is `["-L", "work", "-f", "~/my tmux.conf"]`

#### Scenario: Existing values shown as shell words
- **WHEN** a profile has `env = { PAGER = "less -R" }` and the user edits it
- **THEN** the environment field reads `PAGER='less -R'`

#### Scenario: tmux fields hidden
- **WHEN** the tmux mode is none
- **THEN** the session and tmux arguments fields are not shown

#### Scenario: On close saved
- **WHEN** the user picks "ask" for on close and saves
- **THEN** the saved profile has `on_close = "ask"`

#### Scenario: On close left at the default
- **WHEN** a profile without `on_close` is saved with on close still "keep running"
- **THEN** the saved profile doesn't gain an `on_close` key
