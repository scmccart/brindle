# Spec Delta

## MODIFIED Requirements

### Requirement: Behaviour settings
The config SHALL control:
- scrollback length;
- grid padding;
- cursor shape and blinking;
- `copy_on_select`;
- `osc52_paste` (whether programs may read the clipboard);
- `restore_session` (whether a plain launch reopens the last session's windows and tabs, as defined by the `session-restore` capability; default off).

#### Scenario: Blinking disabled
- **WHEN** `cursor.blink = false`
- **THEN** the focused cursor never blinks

#### Scenario: Session restore enabled
- **WHEN** `restore_session = true`
- **THEN** a plain launch reopens the windows and tabs open when Brindle last quit
