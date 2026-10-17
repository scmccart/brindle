# Delta for settings-dialog

## MODIFIED Requirements

### Requirement: General section
The General section SHALL show the `tmux_session_end` setting as a choice field: "Close window", "Open a new tab" or "Ask". It SHALL be focused when the section is shown, and labelled with what it governs. Changing the choice SHALL write the key to the config at once, keeping the rest of the file, and SHALL apply it without waiting for the file watcher. Cancel SHALL NOT revert a change already written. A failed write SHALL show the reason and keep the previous value. The section SHALL have a Close button that closes the dialog, as escape does there.

#### Scenario: Current value shown
- **WHEN** the config has `tmux_session_end = "ask"` and the user opens the dialog
- **THEN** the General section shows "Ask"

#### Scenario: Change is saved at once
- **WHEN** the config has comments and no `tmux_session_end`, and the user changes the choice to "Ask" and closes the dialog
- **THEN** the config has `tmux_session_end = "ask"` and its comments, and a session ending in this window then asks

#### Scenario: Malformed config file
- **WHEN** the config file has a TOML syntax error and the user changes the choice
- **THEN** the section shows that the config file must be fixed first, the choice stays as it was, and the file is unchanged

#### Scenario: Closing with the mouse
- **WHEN** the General section is showing and the user clicks Close
- **THEN** the dialog closes, and a choice already changed stays saved
