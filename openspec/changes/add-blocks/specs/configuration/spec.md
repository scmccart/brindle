# Spec Delta

## MODIFIED Requirements

### Requirement: Behaviour settings
The config SHALL control:
- scrollback length;
- grid padding;
- cursor shape and blinking;
- `copy_on_select`;
- `osc52_paste` (whether programs may read the clipboard);
- `mark_failed_commands` (whether failed commands get a gutter mark; on by default);
- `blocks` (whether commands are grouped into blocks, as defined by the `blocks` capability; on by default).

Session restore is always on (see `session-restore`). A config that still sets `restore_session` SHALL load, with the key ignored and a warning logged that it no longer has an effect.

Brindle has no tmux support. A config that still sets `tmux_session_end` at the top level SHALL load, with the key ignored and a warning logged that tmux support was removed, whatever its value.

#### Scenario: Blinking disabled
- **WHEN** `cursor.blink = false`
- **THEN** the focused cursor never blinks

#### Scenario: Session restore enabled
- **WHEN** the config contains `restore_session = true`
- **THEN** Brindle loads the config without an error banner, logs that `restore_session` is ignored, and a launch restores as it always does

#### Scenario: Old restore setting
- **WHEN** the config contains `restore_session = false`
- **THEN** Brindle loads the config without an error banner, logs that `restore_session` is ignored, and a launch still restores

#### Scenario: Failed-command marks disabled
- **WHEN** `mark_failed_commands = false` and the config is reloaded
- **THEN** existing failed-command marks disappear from every terminal

#### Scenario: Blocks disabled
- **WHEN** `blocks = false` and the config is reloaded
- **THEN** no terminal shows blocks

#### Scenario: Unknown session-end value
- **WHEN** the config contains `tmux_session_end = "ask"` or `tmux_session_end = "shell"`
- **THEN** Brindle loads the config without an error banner and logs that `tmux_session_end` is ignored because tmux support was removed

#### Scenario: Other unknown keys still fail
- **WHEN** the config contains a top-level key that no version of Brindle defined, such as `tmux_sesion_end`
- **THEN** Brindle starts with defaults and shows a config error banner naming the key
