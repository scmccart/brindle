# Spec Delta

## MODIFIED Requirements

### Requirement: Attach to a session
Opening a profile with `tmux = "control"` SHALL attach to the profile's session through `tmux -u [tmux_args] -C new-session -A -s <session>`, creating the session if needed. Each of the session's windows SHALL appear as a tab, in tmux's window order, with the tmux window name as the tab title. A session SHALL be shown by at most one Brindle window, also when several Brindle processes run (see "Session shown by another Brindle"). Opening a control profile whose session is already attached in the current window SHALL make one of its tabs active. When it is attached in another window, Brindle SHALL NOT attach it again. Picking the profile in the new-tab palette, which says beforehand that the session is open in another window, SHALL go to that session as "Go to <session>" does, without asking. The other ways of opening it SHALL ask what to do, as defined by "Choosing when a session is open elsewhere". Two profiles that run the same tmux program with the same `tmux_args` and session name SHALL count as the same session.

#### Scenario: Existing session with two windows
- **WHEN** the user opens a control profile whose session has windows 0 and 1
- **THEN** two tabs appear, named after the two windows, and tmux's current window is active

#### Scenario: Session open in this window
- **WHEN** the window shows a local tab (active) and the tabs of session `main`, and the user picks the profile for `main` in the new-tab palette
- **THEN** a tab of `main` becomes active, nothing is asked, and no second tmux client attaches

#### Scenario: Session open in another window
- **WHEN** window A shows session `main`, and the user picks the profile for `main` from the new-tab palette in window B
- **THEN** nothing is asked, window A's tab for tmux's current window of `main` becomes active, Brindle asks the desktop to raise window A, and `tmux list-clients` shows one Brindle client for `main`

#### Scenario: Session open in another window, opened by shortcut
- **WHEN** window A shows session `main`, and the user presses the ctrl-alt-N shortcut of the profile for `main` in window B
- **THEN** window B asks what to do, window A is not raised, and `tmux list-clients` shows one Brindle client for `main`


## ADDED Requirements

### Requirement: Session shown by another Brindle
Before attaching a control-mode profile's session by name, Brindle SHALL ask tmux for the session's clients. When one of them is a control client of another running Brindle, Brindle SHALL NOT attach the session, and the window that would have shown it SHALL ask what to open instead. Clients of other programs, and control clients started from a shell, SHALL NOT count. When tmux can't list the session's clients, Brindle SHALL attach it as before.

#### Scenario: Second launch
- **WHEN** one Brindle shows session `main`, and a second Brindle is launched whose default profile attaches `main`
- **THEN** the second Brindle's window has no tabs and asks, and `tmux list-clients` shows one client for `main`

#### Scenario: Actions in the second Brindle
- **WHEN** one Brindle shows session `main`, a second Brindle was launched with `main`'s profile, and the user opens a new tmux window there
- **THEN** the first Brindle's tabs stay as they were and `main` keeps its windows

#### Scenario: Shortcut in the second Brindle
- **WHEN** one Brindle shows session `main`, and the user presses the ctrl-alt-N shortcut of `main`'s profile in a local tab of a second Brindle
- **THEN** the second Brindle asks in that window, its local tab stays, and `tmux list-clients` shows one client for `main`

#### Scenario: Restored in the second Brindle
- **WHEN** one Brindle shows session `main`, and a second Brindle restores a window whose tabs include `main`
- **THEN** that window asks about `main` and the window's other tabs open as recorded

#### Scenario: Another program's control client
- **WHEN** a `tmux -C attach -t main` started from a shell is attached, and the user opens `main`'s profile in Brindle
- **THEN** Brindle attaches `main` and shows its windows as tabs

#### Scenario: No server yet
- **WHEN** the profile's tmux server isn't running, and the user opens the profile
- **THEN** Brindle starts the session and attaches it

### Requirement: Asking about a session another Brindle shows
When another Brindle shows the session, Brindle SHALL show a palette with the placeholder `tmux session "<session>" is open in another Brindle`. Its rows SHALL be "New tmux session…", as in "Choosing when a session is open elsewhere", then "New tab: <profile>" for each other profile in config order, leaving out control-mode profiles whose session this Brindle shows. The first row SHALL be selected. Both SHALL open in the window that asked. There SHALL be no "Go to" row.

#### Scenario: Rows
- **WHEN** a second Brindle asks about `main`, and its profiles are `tmux` (session `main`) and `Shell`
- **THEN** the palette reads `tmux session "main" is open in another Brindle` and lists "New tmux session…" (selected), then "New tab: Shell"

#### Scenario: Another profile instead
- **WHEN** a second Brindle's window with no tabs asks about `main`, and the user chooses "New tab: Shell"
- **THEN** a `Shell` tab opens in that window and no tmux client attaches `main`

#### Scenario: New session instead
- **WHEN** a second Brindle asks about `main`, and the user chooses "New tmux session…" and enters `scratch`
- **THEN** session `scratch` is created on the same server and shown in that window

### Requirement: Dismissing the question about another Brindle's session
Escape or a click outside SHALL dismiss the palette without opening anything. When the window has no tabs, dismissing the palette, or the prompt its "New tmux session…" row opened, SHALL close the window. When the palette replaced a session that ended, the ended session's tabs SHALL close once what the user chose shows up, or when the palette or its prompt is dismissed.

#### Scenario: Dismissed at launch
- **WHEN** a second Brindle's only window has no tabs and asks about `main`, and the user presses escape
- **THEN** the window closes and the second Brindle quits

#### Scenario: Dismissed beside a local tab
- **WHEN** a second Brindle asks about `main` after ctrl-alt-N in a local tab, and the user presses escape
- **THEN** the palette closes and the local tab stays

#### Scenario: Asked after a session ended
- **WHEN** `tmux_session_end` is `ask`, session `work` ended, the user chooses "New tab: tmux" for session `main`, which another Brindle shows, and then chooses "New tab: Shell"
- **THEN** a `Shell` tab opens and `work`'s ended tabs close
