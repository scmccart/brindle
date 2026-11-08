## MODIFIED Requirements

### Requirement: Attach to a session
Opening a profile with `tmux = "control"` SHALL attach to the profile's session through `tmux -u [tmux_args] -C new-session -A -s <session>`, creating the session if needed. For a WSL profile on Windows, the command line is the one defined by "Control mode through WSL on Windows". Each of the session's windows SHALL appear as a tab, in tmux's window order, with the tmux window name as the tab title. A session SHALL be shown at most once, also when several Brindle processes run (see "Session shown by another Brindle"). Opening a control profile whose session the window already shows SHALL make one of its tabs active instead of attaching it again. Two profiles that run the same tmux program with the same `tmux_args` and session name SHALL count as the same session.

#### Scenario: Existing session with two windows
- **WHEN** the user opens a control profile whose session has windows 0 and 1
- **THEN** two tabs appear, named after the two windows, and tmux's current window is active

#### Scenario: Session open in this window
- **WHEN** the window shows a local tab (active) and the tabs of session `main`, and the user picks the profile for `main` in the new-tab palette
- **THEN** a tab of `main` becomes active, nothing is asked, and no second tmux client attaches

#### Scenario: Session open in another window
- **WHEN** another Brindle (with its own state directory) shows session `main`, and the user picks the profile for `main` from the new-tab palette
- **THEN** the window asks what to open instead, as "Asking about a session another Brindle shows" says, and `tmux list-clients` shows one Brindle client for `main`

#### Scenario: Session open in another window, opened by shortcut
- **WHEN** the window shows session `main` in a background tab, and the user presses the ctrl-alt-N shortcut of the profile for `main`
- **THEN** a tab of `main` becomes active, nothing is asked, and no second tmux client attaches

### Requirement: A session leaving the window's only tabs
When a control-mode session detaches or ends and its tabs were all of the window's tabs, Brindle SHALL act on `tmux_session_end`:
- `close_window` (the default) SHALL close the window, as closing its last tab does, which quits Brindle;
- `new_tab` SHALL open a tab with the default profile, or with the first profile that does not use tmux when the default is a control-mode profile, and then close the session's tabs; with no such profile, the window SHALL close;
- `ask` SHALL ask the user, as defined by "Asking when a session leaves".

#### Scenario: Last shell exits
- **WHEN** `tmux_session_end` is unset, the window shows only a tmux session's single tab, and the user types `exit` in its last pane
- **THEN** the window closes and Brindle exits

#### Scenario: Last window closed
- **WHEN** `tmux_session_end` is `close_window`, the window's only tab is a session's only tmux window, and the user closes it with ctrl-shift-w
- **THEN** the tmux window is killed, the window closes and Brindle exits

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
With `tmux_session_end = "ask"`, the session's tabs SHALL stay in the window, with their panes ended, while a palette offers "Close window" first and selected, followed by "New tab: <profile>" for every profile in config order. Choosing a profile SHALL open a tab with it and close the session's tabs. Choosing "Close window", or pressing escape, SHALL close the window, which quits Brindle; a click outside the palette SHALL NOT dismiss it. Opening this palette SHALL close any other open overlay.

#### Scenario: Choose a profile
- **WHEN** `tmux_session_end` is `ask`, the window shows only a tmux session's tabs, the session ends, and the user selects "New tab: Shell" and presses enter
- **THEN** a `Shell` tab opens and the tmux tabs close

#### Scenario: Escape closes the window
- **WHEN** the palette is asking after a session ended and the user presses escape
- **THEN** the window closes and Brindle exits

#### Scenario: Stray click
- **WHEN** the palette is asking after a session ended and the user clicks on the ended tab behind it
- **THEN** the palette stays open and the window stays open

#### Scenario: Enter closes the window
- **WHEN** the palette is asking after a session ended and the user presses enter without moving the selection
- **THEN** the window closes

#### Scenario: Profile whose session is open elsewhere
- **WHEN** the palette is asking after session `work` ended, and another Brindle shows session `main` of profile `tmux`
- **THEN** the palette lists "New tab: tmux" as for any profile, and choosing it asks as "Asking about a session another Brindle shows" says

### Requirement: Quitting leaves windows as they were
When Brindle quits while attached to a control-mode session, the session leaving SHALL NOT close the window, open a tab, or ask, so that session restore records the window with its tmux tabs.

#### Scenario: Quit with only tmux tabs
- **WHEN** `tmux_session_end` is `close_window`, the window shows only tmux tabs, and the user quits Brindle
- **THEN** the next launch reattaches the session in the window, as it was

### Requirement: Session shown by another Brindle
Before attaching a control-mode profile's session by name, Brindle SHALL ask tmux for the session's clients. When one of them is a control client of another running Brindle process (one with its own state directory, or a debug run), Brindle SHALL NOT attach the session, and the window SHALL ask what to open instead. Clients of other programs, and control clients started from a shell, SHALL NOT count. When tmux can't list the session's clients, Brindle SHALL attach it as before. On Windows the check SHALL NOT be made, and Brindle SHALL attach the session as before.

#### Scenario: Second launch
- **WHEN** a Brindle with its own `BRINDLE_STATE_DIR` shows session `main`, and the user opens `main`'s profile
- **THEN** the window asks, and `tmux list-clients` shows one Brindle client for `main`

#### Scenario: Actions in the second Brindle
- **WHEN** another Brindle shows session `main`, this Brindle asked about it, and the user opens a new tmux window in another session here
- **THEN** the other Brindle's tabs stay as they were and `main` keeps its windows

#### Scenario: Shortcut in the second Brindle
- **WHEN** another Brindle shows session `main`, and the user presses the ctrl-alt-N shortcut of `main`'s profile in a local tab
- **THEN** Brindle asks in the window, its local tab stays, and `tmux list-clients` shows one Brindle client for `main`

#### Scenario: Restored in the second Brindle
- **WHEN** another Brindle shows session `main`, and a launch restores tabs that include `main`
- **THEN** the window asks about `main`, and the other restored tabs open as recorded

#### Scenario: Another program's control client
- **WHEN** a `tmux -C attach -t main` started from a shell is attached, and the user opens `main`'s profile in Brindle
- **THEN** Brindle attaches `main` and shows its windows as tabs

#### Scenario: No server yet
- **WHEN** the profile's tmux server isn't running, and the user opens the profile
- **THEN** Brindle starts the session and attaches it

### Requirement: Asking about a session another Brindle shows
When another Brindle shows the session, Brindle SHALL show a palette with the placeholder `tmux session "<session>" is open in another Brindle`. Its rows SHALL be "New tmux session…", which asks for a name as "Create a new session" does and creates it with the same profile, then "New tab: <profile>" for each other profile in config order, leaving out control-mode profiles whose session the window shows. The first row SHALL be selected. Both SHALL open in the window. There SHALL be no "Go to" row. Escape or a click outside SHALL dismiss the palette without opening anything.

#### Scenario: Rows
- **WHEN** Brindle asks about `main`, and its profiles are `tmux` (session `main`) and `Shell`
- **THEN** the palette reads `tmux session "main" is open in another Brindle` and lists "New tmux session…" (selected), then "New tab: Shell"

#### Scenario: Another profile instead
- **WHEN** Brindle asks about `main` with no tabs open, and the user chooses "New tab: Shell"
- **THEN** a `Shell` tab opens and no tmux client attaches `main`

#### Scenario: New session instead
- **WHEN** Brindle asks about `main`, and the user chooses "New tmux session…" and enters `scratch`
- **THEN** session `scratch` is created on the same server and shown in the window

## REMOVED Requirements

### Requirement: Choosing when a session is open elsewhere
**Reason**: Brindle has one window, so a session this Brindle shows is always in the window; opening its profile activates its tab. A session another Brindle shows is covered by "Asking about a session another Brindle shows".
**Migration**: None.
