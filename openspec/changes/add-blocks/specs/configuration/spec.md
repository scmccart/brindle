# Spec Delta

## MODIFIED Requirements

### Requirement: Behaviour settings
The config SHALL control:
- scrollback length;
- grid padding;
- cursor shape and blinking;
- `copy_on_select`;
- `osc52_paste` (whether programs may read the clipboard);
- `restore_session` (whether a plain launch reopens the last session's windows and tabs, as defined by the `session-restore` capability; default off);
- `mark_failed_commands` (whether failed commands get a gutter mark; on by default);
- `blocks` (whether commands are grouped into blocks, as defined by the `blocks` capability; on by default);
- `tmux_session_end` (what happens when a control-mode session leaves a window that showed only its tabs: `close_window`, `new_tab` or `ask`, as defined by the `tmux-control-mode` capability; default `close_window`).

#### Scenario: Blinking disabled
- **WHEN** `cursor.blink = false`
- **THEN** the focused cursor never blinks

#### Scenario: Session restore enabled
- **WHEN** `restore_session = true`
- **THEN** a plain launch reopens the windows and tabs open when Brindle last quit

#### Scenario: Failed-command marks disabled
- **WHEN** `mark_failed_commands = false` and the config is reloaded
- **THEN** existing failed-command marks disappear from every terminal

#### Scenario: Blocks disabled
- **WHEN** `blocks = false` and the config is reloaded
- **THEN** no terminal shows blocks

#### Scenario: Unknown session-end value
- **WHEN** the config contains `tmux_session_end = "shell"`
- **THEN** Brindle starts with defaults and shows a config error banner naming the problem
