# Spec Delta

## MODIFIED Requirements

### Requirement: Attach to a session
Opening a profile with `tmux = "control"` SHALL attach to the profile's session through `tmux [tmux_args] -C new-session -A -s <session>`, creating the session if needed. Each of the session's windows SHALL appear as a tab, in tmux's window order, with the tmux window name as the tab title. A session SHALL be shown by at most one Brindle window. Opening a control profile whose session is already attached in the current window SHALL make one of its tabs active. When it is attached in another window, Brindle SHALL NOT attach it again and SHALL ask what to do, as defined by "Choosing when a session is open elsewhere". Two profiles that run the same tmux program with the same `tmux_args` and session name SHALL count as the same session.

#### Scenario: Existing session with two windows
- **WHEN** the user opens a control profile whose session has windows 0 and 1
- **THEN** two tabs appear, named after the two windows, and tmux's current window is active

#### Scenario: Session open in this window
- **WHEN** the window shows a local tab (active) and the tabs of session `main`, and the user picks the profile for `main` in the new-tab palette
- **THEN** a tab of `main` becomes active, nothing is asked, and no second tmux client attaches

#### Scenario: Session open in another window
- **WHEN** window A shows session `main`, and the user picks the profile for `main` from the new-tab palette in window B
- **THEN** window B asks what to do, window A is not raised, and `tmux list-clients` shows one Brindle client for `main`

### Requirement: Asking when a session leaves
With `tmux_session_end = "ask"`, the session's tabs SHALL stay in the window, with their panes ended, while a palette offers "Close window" first and selected, followed by "New tab: <profile>" for every profile in config order. A control-mode profile whose session is open in another window SHALL be listed as "Go to <session>" instead; choosing it SHALL make that session's tab active in its window, ask the desktop to raise that window, and close this one. Choosing any other profile SHALL open a tab with it and close the session's tabs. Choosing "Close window", or pressing escape, SHALL close the window; a click outside the palette SHALL NOT dismiss it. Opening this palette SHALL close any other open overlay.

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

#### Scenario: Profile whose session is open elsewhere
- **WHEN** the palette is asking after session `work` ended, and another window shows session `main` of profile `tmux`
- **THEN** the palette lists "Go to main" in place of "New tab: tmux"

## ADDED Requirements

### Requirement: Choosing when a session is open elsewhere
When an action would attach a control-mode session that another window shows, Brindle SHALL show a palette in the current window instead, with the placeholder `tmux session "<session>" is open in another window`. The actions are: picking the profile in the new-tab palette, ctrl-alt-N, and opening the default profile with ctrl-shift-t, the `+` button or ctrl-shift-n (see `tabs`). The rows SHALL be, in order, with the first selected:
- "Go to <session>": make that session's tab active in its window and ask the desktop to raise that window;
- "New tmux session…": ask for a name, as "Create a new session" does, with the same profile;
- "New tab: <profile>" for each other profile in config order, or "New window: <profile>" when the action was ctrl-shift-n. The profile that was picked, and control-mode profiles whose session is open in any window, SHALL NOT be listed.

What a row opens SHALL go where the action would have opened the profile: a tab in the current window, or a new window for ctrl-shift-n. Escape or a click outside SHALL dismiss the palette without opening anything or raising any window.

#### Scenario: Go to the session
- **WHEN** window A shows session `main`, the user picks `tmux` in window B's new-tab palette, and presses enter in the palette that appears
- **THEN** window A's tab for tmux's current window of `main` becomes active, Brindle asks the desktop to raise window A, and window B has no new tab

#### Scenario: Another profile instead
- **WHEN** the palette is asking about `main` after ctrl-shift-t, and the user chooses "New tab: Shell"
- **THEN** a `Shell` tab opens in the current window

#### Scenario: New window with another profile
- **WHEN** the palette is asking about `main` after ctrl-shift-n, and the user chooses "New window: Shell"
- **THEN** a new window opens with one `Shell` tab

#### Scenario: New session from the palette
- **WHEN** the palette is asking about `main` after ctrl-shift-n, the user chooses "New tmux session…" and enters `scratch`
- **THEN** a new window opens showing session `scratch`, created on the same server as `main`

#### Scenario: Dismissed
- **WHEN** the palette is asking about `main` and the user presses escape
- **THEN** the palette closes, no tab or window opens, and no other window is raised

### Requirement: Create a new session
A "tmux: New Session" command SHALL prompt for a session name, then create that session and attach it in the current window, beside the window's other tabs. An empty name SHALL let tmux choose the name. The new session SHALL start in the active tab's directory. It SHALL use, in this order, the active tab's control-mode profile, the default profile if it is a control-mode profile, or the first control-mode profile. The command SHALL be available from every tab when the config has at least one control-mode profile, and SHALL NOT be listed otherwise. It has no default keybinding; its action name is `tmux_new_session`.

#### Scenario: Named session
- **WHEN** the user runs "tmux: New Session" from a local tab and enters `work`
- **THEN** `tmux ls` lists a session named `work`, its window appears as a tab in the current window, and the prompt closes

#### Scenario: Name left empty
- **WHEN** the user runs "tmux: New Session" and submits an empty name
- **THEN** a session with a name tmux chose is created and shown, and its tabs are recorded for session restore under that name

#### Scenario: From a tmux tab
- **WHEN** the active tab belongs to a session of profile `remote`, whose `tmux_args` select another server, and the user creates session `logs`
- **THEN** `logs` is created on the server `remote` uses

#### Scenario: Name already taken
- **WHEN** the user enters the name of a session that already exists
- **THEN** the prompt stays open showing tmux's error (`duplicate session: <name>`), no tab is added, and no attach-failure tab opens

#### Scenario: No control-mode profile
- **WHEN** no profile in the config uses `tmux = "control"`
- **THEN** "tmux: New Session" is not listed in the command palette

### Requirement: A new session's name is tracked
The name Brindle records for a control-mode session, for restore and for detecting a session that is already open, SHALL be the name tmux reports for the attached session. A session whose name is not yet known SHALL NOT be treated as matching any other session.

#### Scenario: tmux-chosen name
- **WHEN** a session created with an empty name was named `3` by tmux, and the user later picks a profile whose session is `3` on the same server
- **THEN** the existing tabs for `3` are focused instead of a second attach
