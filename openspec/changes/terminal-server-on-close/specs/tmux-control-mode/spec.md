## MODIFIED Requirements

### Requirement: Quitting leaves windows as they were
When Brindle quits while attached to a control-mode session, the session leaving SHALL NOT close a window, open a tab, or ask, so that session restore records each window with its tmux tabs.

#### Scenario: Quit with only tmux tabs
- **WHEN** `tmux_session_end` is `close_window`, the only window shows only tmux tabs, and the user quits Brindle
- **THEN** the next plain launch reattaches the session in a window, as it was
