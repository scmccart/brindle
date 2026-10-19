# tabs Specification

## Purpose
Organize terminals into windows and tabs. The tab strip doubles as the window title bar and lets users open, close, switch, reorder and identify their sessions.

## Requirements

### Requirement: Tab lifecycle
The user SHALL be able to:
- open a new tab with the default profile (ctrl-shift-t, or the `+` button);
- close the active tab (ctrl-shift-w, its `×` button, or a middle click).

A tab whose program exits SHALL close. A new local tab SHALL open next to the active tab and become active.

#### Scenario: Shell exit closes its tab
- **WHEN** the user runs `exit` in a tab
- **THEN** that tab closes and a neighbouring tab becomes active

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
A tab's title SHALL be:
1. the title the program set (OSC 0/2), when there is one. On Windows the console host starts every program with the program's full path as its title, and cmd builds its title while running a command from that path plus ` - <command>`. A title that is such a path, alone or followed by ` - `, counts as no title;
2. otherwise the name of the terminal's foreground process. Windows has no foreground process group, so there this is the most recently started descendant of the tab's program, or the program itself when it has none, without the `.exe` suffix;
3. otherwise the profile name.

The window title SHALL match the active tab's title.

#### Scenario: Foreground process
- **WHEN** the user starts `htop` in a tab whose program sets no title
- **THEN** within about a second the tab is titled `htop`

#### Scenario: Console's default title on Windows
- **WHEN** a Windows tab runs PowerShell 7, whose title is the console's default `C:\Program Files\PowerShell\7\pwsh.exe`, at its prompt
- **THEN** the tab is titled `pwsh`

#### Scenario: Child process on Windows
- **WHEN** the user runs `python` from PowerShell 7, which sets no title of its own, in a Windows tab
- **THEN** within about a second the tab is titled `python`, and after Python exits it is titled `pwsh` again

#### Scenario: cmd's title on Windows
- **WHEN** the user runs `python` from `cmd.exe`, which sets the title `C:\WINDOWS\system32\cmd.exe - python`, or from a `cmd` started in a PowerShell 7 tab, which sets `C:\Program Files\PowerShell\7\pwsh.exe - python`
- **THEN** the tab is titled `python`, as it would be on Linux, and `cmd` again once Python exits

### Requirement: Working directory inheritance
A new tab whose profile sets no working directory SHALL start in the working directory of the active tab's foreground process. On Windows, where another process's working directory can't be read, it SHALL start in the directory the active tab's program last reported, as defined under "Working directory reports" in `shell-integration`, if that directory exists. Otherwise it SHALL start in the home directory.

#### Scenario: New tab in same directory
- **WHEN** the active tab's shell is in `~/src/brindle` and the user opens a new tab
- **THEN** the new shell starts in `~/src/brindle`

#### Scenario: Reported directory on Windows
- **WHEN** the active tab's PowerShell, with Brindle's integration script loaded, has changed to `C:\src` and the user opens a new tab with a profile that sets no directory
- **THEN** the new shell starts in `C:\src`

#### Scenario: Nothing reported on Windows
- **WHEN** the active tab runs `cmd.exe`, which reports no directory, and the user opens a new tab with a profile that sets no directory
- **THEN** the new shell starts in the user's home directory

### Requirement: Bell indicator
A bell from a tab that is not active, or from any tab while the window is inactive, SHALL mark that tab until the user activates it. This SHALL include bells from any pane of a tmux control-mode tab, including tabs for tmux windows that are not current.

#### Scenario: Background bell
- **WHEN** a background tab rings the bell
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
The user SHALL be able to open additional windows (ctrl-shift-n), each with its own tabs, and quit everything with ctrl-shift-q. A new window SHALL open with one tab using the default profile. When the default profile is a control-mode profile whose session is already shown in any window, ctrl-shift-n SHALL NOT open a window directly; it SHALL ask what to do, as defined by "Choosing when a session is open elsewhere" in `tmux-control-mode`.

#### Scenario: New window
- **WHEN** the user presses ctrl-shift-n
- **THEN** a second window opens with one tab using the default profile

#### Scenario: Default session already open
- **WHEN** the default profile is the control-mode profile for session `main`, a window shows `main`, and the user presses ctrl-shift-n
- **THEN** no window opens yet, the current window offers "Go to main", "New tmux session…" and "New window: <profile>" rows, and `tmux list-clients` still shows one Brindle client for `main`

### Requirement: New tab with the default profile does not move between windows
ctrl-shift-t (or the `+` button) in a tab that is not a control-mode tab SHALL open the default profile. When the default profile is a control-mode profile whose session is shown in the current window, its tab SHALL become active. When that session is shown in another window, Brindle SHALL ask what to do, as defined by "Choosing when a session is open elsewhere" in `tmux-control-mode`, and SHALL NOT switch windows unless the user chooses to.

#### Scenario: Session in another window
- **WHEN** the default profile's session `main` is shown in window A, and the user presses ctrl-shift-t in a local tab of window B
- **THEN** window B asks what to do, offering "New tab: <profile>" rows, and window A is not raised

#### Scenario: Session in this window
- **WHEN** the current window shows session `main` and a local tab is active, and the user presses ctrl-shift-t
- **THEN** a tab of `main` becomes active and no tab is added
