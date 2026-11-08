## MODIFIED Requirements

### Requirement: Behaviour settings
The config SHALL control:
- scrollback length;
- grid padding;
- cursor shape and blinking;
- `copy_on_select`;
- `osc52_paste` (whether programs may read the clipboard);
- `mark_failed_commands` (whether failed commands get a gutter mark; on by default);
- `tmux_session_end` (what happens when a control-mode session leaves a window that showed only its tabs: `close_window`, `new_tab` or `ask`, as defined by the `tmux-control-mode` capability; default `close_window`).

Session restore is always on (see `session-restore`). A config that still sets `restore_session` SHALL load, with the key ignored and a warning logged that it no longer has an effect.

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

#### Scenario: Unknown session-end value
- **WHEN** the config contains `tmux_session_end = "shell"`
- **THEN** Brindle starts with defaults and shows a config error banner naming the problem
