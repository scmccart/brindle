# tabs Specification

## Purpose
Organize terminals into windows and tabs. The tab strip doubles as the window title bar and lets users open, close, switch, reorder and identify their sessions.

## Requirements

### Requirement: Tab lifecycle
The user SHALL be able to:
- open a new tab with the default profile (ctrl-shift-t, or the `+` button);
- close the active tab (ctrl-shift-w, its `×` button, or a middle click).

A local tab whose last remaining zone's program exits SHALL close. When a local tab has more than one zone and one zone's program exits, only that zone SHALL close, as defined by the `zones` capability. A new local tab SHALL open next to the active tab and become active.

#### Scenario: Shell exit closes its tab
- **WHEN** the user runs `exit` in a single-zone tab
- **THEN** that tab closes and a neighbouring tab becomes active

#### Scenario: Shell exit in a split tab closes only that zone
- **WHEN** a local tab has two zones and the user runs `exit` in one zone
- **THEN** that zone closes and the tab remains open with the other zone

#### Scenario: New tab position
- **WHEN** the second of three tabs is active and the user opens a new tab
- **THEN** the new tab appears third and is active

### Requirement: Window closes with its last tab
Closing the last tab of a window SHALL close the window. Closing the last window SHALL quit the application.

#### Scenario: Last tab closed
- **WHEN** a window's only tab is closed
- **THEN** the window closes

### Requirement: Switching and reordering tabs
The user SHALL be able to:
- switch tabs with ctrl-tab / ctrl-shift-tab and ctrl-pagedown / ctrl-pageup;
- jump to tab N with alt-N (alt-9 is the last tab);
- move the active tab with ctrl-shift-pageup / ctrl-shift-pagedown;
- reorder tabs by dragging them.

#### Scenario: Jump to last tab
- **WHEN** there are five tabs and the user presses alt-9
- **THEN** the fifth tab becomes active

#### Scenario: Drag to reorder
- **WHEN** the user drags the first tab onto the third
- **THEN** the tab order changes and the same tab stays active

### Requirement: Tab titles
A tab's title SHALL be the manual title the user set for that tab, when one is set. Otherwise a local tab's title SHALL follow its active zone's terminal:
1. the title the program set (OSC 0/2), when there is one. On Windows the console host starts every program with the program's full path as its title, and cmd builds its title while running a command from that path plus ` - <command>`. A title that is such a path, alone or followed by ` - `, counts as no title;
2. otherwise the name of the terminal's foreground process. Windows has no foreground process group, so there this is the most recently started descendant of the terminal's program, or the program itself when it has none, without the `.exe` suffix;
3. otherwise the profile name.

Changing the focused zone in a local tab SHALL update the tab's automatic title immediately. The window title SHALL match the active tab's title.

A manual title SHALL stay in effect until the user changes or clears it. Program title changes, foreground process changes, zone focus changes, profile name changes, and tmux window name changes SHALL NOT replace a manual title.

#### Scenario: Foreground process
- **WHEN** the user starts `htop` in a tab whose program sets no title
- **THEN** within about a second the tab is titled `htop`

#### Scenario: Switching zones updates the tab title
- **WHEN** a local tab has no manual title, one zone's program has set the title `server` and the other zone's program has set the title `logs`, and the user moves focus from `server` to `logs`
- **THEN** the tab and window title change to `logs`

#### Scenario: Console's default title on Windows
- **WHEN** a Windows tab runs PowerShell 7, whose title is the console's default `C:\Program Files\PowerShell\7\pwsh.exe`, at its prompt
- **THEN** the tab is titled `pwsh`

#### Scenario: Child process on Windows
- **WHEN** the user runs `python` from PowerShell 7, which sets no title of its own, in a Windows tab
- **THEN** within about a second the tab is titled `python`, and after Python exits it is titled `pwsh` again

#### Scenario: cmd's title on Windows
- **WHEN** the user runs `python` from `cmd.exe`, which sets the title `C:\WINDOWS\system32\cmd.exe - python`, or from a `cmd` started in a PowerShell 7 tab, which sets `C:\Program Files\PowerShell\7\pwsh.exe - python`
- **THEN** the tab is titled `python`, as it would be on Linux, and `cmd` again once Python exits

#### Scenario: Manual title overrides automatic changes
- **WHEN** the user renames a tab to `prod logs` and that tab's program later changes its OSC title or foreground process, or the user focuses another zone in that tab
- **THEN** the tab remains titled `prod logs`

#### Scenario: Clearing a manual title restores automatic titles
- **WHEN** the user clears a manual title from a tab whose active zone's foreground process is `bash`
- **THEN** the tab is titled `bash` by the automatic title rules

### Requirement: Manual tab title command
The user SHALL be able to set a manual title for the active tab with a `Rename Tab` command. The command SHALL prompt for one line of text, pre-filled with the tab's current displayed title and selected for replacement.

Brindle SHALL trim the submitted text before applying it. A non-empty trimmed value SHALL become the tab's manual title. An empty trimmed value SHALL clear the tab's manual title and restore automatic title updates.

The command SHALL be Brindle-local for tmux control-mode tabs and SHALL NOT rename the tmux window. Users who want a tmux-owned durable window name SHALL use `tmux: Rename Window`.

#### Scenario: Rename active tab
- **WHEN** the active tab is titled `bash`, the user runs `Rename Tab`, enters `  server  `, and submits it
- **THEN** the active tab is titled `server`

#### Scenario: Whitespace clears manual title
- **WHEN** a tab has the manual title `server`, the user runs `Rename Tab`, enters only whitespace, and submits it
- **THEN** the manual title is cleared and the tab follows the automatic title rules

#### Scenario: Rename tmux tab locally
- **WHEN** the active tab shows tmux window `editor`, the user runs `Rename Tab`, enters `review`, and submits it
- **THEN** Brindle shows that tab as `review` and the tmux window remains named `editor`

### Requirement: Working directory inheritance
A new tab whose profile sets no working directory SHALL start in the working directory of the active tab's active terminal's foreground process. On Windows, where another process's working directory can't be read, it SHALL start in the directory the active tab's active terminal last reported, as defined under "Working directory reports" in `shell-integration`, if that directory exists. Otherwise it SHALL start in the home directory.

#### Scenario: New tab in same directory
- **WHEN** the active tab's active zone is in `~/src/brindle` and the user opens a new tab
- **THEN** the new shell starts in `~/src/brindle`

#### Scenario: New tab follows the focused zone
- **WHEN** a local tab has two zones in `/tmp/a` and `/tmp/b`, the second zone is focused, and the user opens a new tab with a profile that sets no directory
- **THEN** the new tab's shell starts in `/tmp/b`

#### Scenario: Reported directory on Windows
- **WHEN** the active tab's PowerShell, with Brindle's integration script loaded, has changed to `C:\src` and the user opens a new tab with a profile that sets no directory
- **THEN** the new shell starts in `C:\src`

#### Scenario: Nothing reported on Windows
- **WHEN** the active tab runs `cmd.exe`, which reports no directory, and the user opens a new tab with a profile that sets no directory
- **THEN** the new shell starts in the user's home directory

### Requirement: Bell indicator
A bell from a tab that is not active, or from any tab while the window is inactive, SHALL mark that tab until the user activates it. This SHALL include bells from any zone of a local tab and from any pane of a tmux control-mode tab, including tabs for tmux windows that are not current.

#### Scenario: Background bell
- **WHEN** a background tab rings the bell
- **THEN** a dot appears on that tab and disappears when the tab is activated

#### Scenario: Bell in a background local zone
- **WHEN** an unfocused zone in a background local tab rings the bell
- **THEN** a dot appears on that tab and disappears when the tab is activated

#### Scenario: Bell in a background tmux window
- **WHEN** a pane in a tmux window whose tab is not active rings the bell
- **THEN** a dot appears on that window's tab and disappears when the tab is activated

### Requirement: Title bar behaviour
When the window draws its own decorations, as it always does on Windows:
- empty space in the tab strip SHALL move the window when dragged;
- double-clicking empty space SHALL maximize or restore the window;
- right-clicking SHALL open the window menu;
- minimize, maximize and close buttons SHALL be shown;
- the window edges SHALL resize the window.

On Windows these areas SHALL behave like a native caption. This includes Snap when the window is dragged to a screen edge, and the Snap Layouts flyout when the pointer hovers the maximize button.

#### Scenario: Drag to move
- **WHEN** the user drags empty space in the tab strip
- **THEN** the window moves

#### Scenario: Snap Layouts on Windows 11
- **WHEN** the user hovers the maximize button on Windows 11
- **THEN** the Snap Layouts flyout appears

#### Scenario: Close button on Windows
- **WHEN** the user clicks the close button of the only window
- **THEN** the window closes and Brindle exits

#### Scenario: Tabs stay clickable on Windows
- **WHEN** the user clicks a tab, the new-tab button or the profile chevron in the strip on Windows
- **THEN** that control responds as it does on Linux, and the window does not start moving

### Requirement: Multiple windows
The user SHALL be able to open additional windows (ctrl-shift-n), each with its own tabs, and quit everything with ctrl-shift-q. A new window SHALL open with one tab using the default profile. When the default profile is a control-mode profile whose session is already shown in any window of this Brindle, ctrl-shift-n SHALL NOT open a window directly; it SHALL ask what to do, as defined by "Choosing when a session is open elsewhere" in `tmux-control-mode`. When another Brindle shows it instead, the new window SHALL open and ask there, as defined by "Session shown by another Brindle".

#### Scenario: New window
- **WHEN** the user presses ctrl-shift-n
- **THEN** a second window opens with one tab using the default profile

#### Scenario: Default session already open
- **WHEN** the default profile is the control-mode profile for session `main`, a window shows `main`, and the user presses ctrl-shift-n
- **THEN** no window opens yet, the current window offers "Go to main", "New tmux session…" and "New window: <profile>" rows, and `tmux list-clients` still shows one Brindle client for `main`

#### Scenario: Default session shown by another Brindle
- **WHEN** the default profile is the control-mode profile for session `main`, another Brindle shows `main`, and the user presses ctrl-shift-n
- **THEN** a new window opens with no tabs and asks about `main`, and the current window stays as it was

### Requirement: New tab with the default profile does not move between windows
ctrl-shift-t (or the `+` button) in a tab that is not a control-mode tab SHALL open the default profile. When the default profile is a control-mode profile whose session is shown in the current window, its tab SHALL become active. When that session is shown in another window, Brindle SHALL ask what to do, as defined by "Choosing when a session is open elsewhere" in `tmux-control-mode`, and SHALL NOT switch windows unless the user chooses to.

#### Scenario: Session in another window
- **WHEN** the default profile's session `main` is shown in window A, and the user presses ctrl-shift-t in a local tab of window B
- **THEN** window B asks what to do, offering "New tab: <profile>" rows, and window A is not raised

#### Scenario: Session in this window
- **WHEN** the current window shows session `main` and a local tab is active, and the user presses ctrl-shift-t
- **THEN** a tab of `main` becomes active and no tab is added

### Requirement: Program status indicator
A tab SHALL show a status dot while any zone of its local tab, or any pane of its tmux control-mode window, holds a program status record (see the `program-status` capability) other than `idle`. The dot's colour SHALL follow the most pressing state across those records, in the order `blocked` (the theme's yellow, ANSI 3), `error` (red, ANSI 1), `done` (green, ANSI 2), `working` (the muted foreground). It SHALL sit beside the bell dot when both apply.

#### Scenario: Blocked in a background tab
- **WHEN** a program in a background tab reports `state=blocked:kind=permission`
- **THEN** that tab shows a yellow dot

#### Scenario: Zones in a local tab
- **WHEN** a program in one zone of a local tab reports `state=error`, and another zone in the same tab reports `state=working`
- **THEN** that tab shows a red dot

#### Scenario: Pane in a tmux window
- **WHEN** a program in one pane of a tmux window reports `state=error`, and another pane reports `state=working`
- **THEN** that window's tab shows a red dot

### Requirement: Status indicator lifetime
The status dot SHALL stay until the terminal's records change (a report, a clear, a new prompt, the process exiting, or the user typing into the terminal), whether or not the tab is active. Activating the tab SHALL NOT clear it.

#### Scenario: Done survives the prompt
- **WHEN** a program in the active tab reports `state=done`, exits, and the shell prints its prompt
- **THEN** the tab shows a green dot until the user types a key into that tab

#### Scenario: Activated while blocked
- **WHEN** a background tab shows the yellow dot and the user activates it
- **THEN** the dot remains until the program reports again
