## MODIFIED Requirements

### Requirement: Detach keeps the session
Detaching, or tmux exiting after the session had attached, SHALL close that session's tabs and leave the tmux session running. If those were the window's only tabs, what happens next SHALL follow the `tmux_session_end` setting, as defined by "A session leaving the window's only tabs".

#### Scenario: Detach beside other tabs
- **WHEN** the window shows a local shell tab and two tmux tabs, and the user presses ctrl-shift-d in a tmux tab
- **THEN** the two tmux tabs close, the shell tab stays, and `tmux ls` still lists the session

#### Scenario: Detach from the only tabs
- **WHEN** `tmux_session_end` is `new_tab`, the window shows only tmux tabs, and the user presses ctrl-shift-d
- **THEN** the tmux tabs close, a shell tab opens in their place, and `tmux ls` still lists the session

## ADDED Requirements

### Requirement: A session leaving the window's only tabs
When a control-mode session detaches or ends and its tabs were all of the window's tabs, Brindle SHALL act on `tmux_session_end`:
- `close_window` (the default) SHALL close the window, as closing its last tab does;
- `new_tab` SHALL open a tab with the default profile, or with the first profile that does not use tmux when the default is a control-mode profile, and then close the session's tabs; with no such profile, the window SHALL close;
- `ask` SHALL ask the user, as defined by "Asking when a session leaves".

#### Scenario: Last shell exits
- **WHEN** `tmux_session_end` is unset, the window shows only a tmux session's single tab, and the user types `exit` in its last pane
- **THEN** the window closes

#### Scenario: Last window closed
- **WHEN** `tmux_session_end` is `close_window`, another Brindle window is open, and the user closes the only tab of a session with ctrl-shift-w
- **THEN** the tmux window is killed and the window that showed it closes, while the other Brindle window stays open

#### Scenario: Detach closes the window
- **WHEN** `tmux_session_end` is `close_window`, the window shows only tmux tabs, and the user presses ctrl-shift-d
- **THEN** the window closes, and `tmux ls` still lists the session

#### Scenario: Session killed elsewhere, new tab
- **WHEN** `tmux_session_end` is `new_tab`, the default profile is a control-mode profile, the window shows only that session's tabs, and the session is killed with `tmux kill-session` from another terminal
- **THEN** a tab with the first non-tmux profile replaces the tmux tabs

#### Scenario: Window keeps other tabs
- **WHEN** `tmux_session_end` is `ask`, the window shows a local shell tab and a tmux tab, and the session ends
- **THEN** the tmux tab closes, the shell tab stays, and nothing is asked

### Requirement: Asking when a session leaves
With `tmux_session_end = "ask"`, the session's tabs SHALL stay in the window, with their panes ended, while a palette offers "Close window" first and selected, followed by "New tab: <profile>" for every profile in config order. Choosing a profile SHALL open a tab with it and close the session's tabs. Choosing "Close window", or pressing escape, SHALL close the window; a click outside the palette SHALL NOT dismiss it. Opening this palette SHALL close any other open overlay.

#### Scenario: Choose a profile
- **WHEN** `tmux_session_end` is `ask`, the window shows only a tmux session's tabs, the session ends, and the user selects "New tab: Shell" and presses enter
- **THEN** a `Shell` tab opens and the tmux tabs close

#### Scenario: Escape closes the window
- **WHEN** the palette is asking after a session ended and the user presses escape
- **THEN** the window closes

#### Scenario: Stray click
- **WHEN** the palette is asking after a session ended and the user clicks on the ended tab behind it
- **THEN** the palette stays open and the window stays open

#### Scenario: Enter closes the window
- **WHEN** the palette is asking after a session ended and the user presses enter without moving the selection
- **THEN** the window closes

### Requirement: Quitting leaves windows as they were
When Brindle quits while attached to a control-mode session, the session leaving SHALL NOT close a window, open a tab, or ask, so that session restore records each window with its tmux tabs.

#### Scenario: Quit with only tmux tabs
- **WHEN** `restore_session = true`, `tmux_session_end` is `close_window`, the only window shows only tmux tabs, and the user quits Brindle
- **THEN** the next plain launch reattaches the session in a window, as it was
