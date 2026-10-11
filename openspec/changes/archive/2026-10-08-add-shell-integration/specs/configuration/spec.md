# Spec Delta

## MODIFIED Requirements

### Requirement: Behaviour settings
The config SHALL control:
- scrollback length;
- grid padding;
- cursor shape and blinking;
- `copy_on_select`;
- `osc52_paste` (whether programs may read the clipboard);
- `mark_failed_commands` (whether failed commands get a gutter mark; on by default).

#### Scenario: Blinking disabled
- **WHEN** `cursor.blink = false`
- **THEN** the focused cursor never blinks

#### Scenario: Failed-command marks disabled
- **WHEN** `mark_failed_commands = false` and the config is reloaded
- **THEN** existing failed-command marks disappear from every terminal
