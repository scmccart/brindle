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
Closing the last tab SHALL close the window, which quits Brindle. Closing the window, by its close button, the window manager or `Close Window`, SHALL quit Brindle, as defined by "Quitting keeps programs running".

#### Scenario: Last tab closed
- **WHEN** the window's only tab is closed
- **THEN** the window closes and Brindle exits

#### Scenario: Window closed with tabs
- **WHEN** the window has two local tabs running `sleep 1000` and the user clicks its close button
- **THEN** Brindle exits without asking, and both sessions are listed as detached

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
A tab's title SHALL be the manual title the user set for that tab, when one is set. Otherwise a tab's title SHALL follow its active zone's terminal:
1. the title the program set (OSC 0/2), when there is one. On Windows the console host starts every program with the program's full path as its title, and cmd builds its title while running a command from that path plus ` - <command>`. A title that is such a path, alone or followed by ` - `, counts as no title;
2. otherwise the name of the terminal's foreground process. Windows has no foreground process group, so there this is the most recently started descendant of the terminal's program, or the program itself when it has none, without the `.exe` suffix;
3. otherwise the profile name.

Changing the focused zone in a tab SHALL update the tab's automatic title immediately. The window title SHALL match the active tab's title.

A manual title SHALL stay in effect until the user changes or clears it. Program title changes, foreground process changes, zone focus changes and profile name changes SHALL NOT replace a manual title.

#### Scenario: Foreground process
- **WHEN** the user starts `htop` in a tab whose program sets no title
- **THEN** within about a second the tab is titled `htop`

#### Scenario: Switching zones updates the tab title
- **WHEN** a tab has no manual title, one zone's program has set the title `server` and the other zone's program has set the title `logs`, and the user moves focus from `server` to `logs`
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

### Requirement: Title bar behaviour
When the window draws its own decorations, as it always does on Windows:
- empty space in the tab strip SHALL move the window when dragged;
- double-clicking empty space SHALL maximize or restore the window;
- right-clicking empty space SHALL open the window menu, and right-clicking a tab SHALL open the tab menu instead;
- minimize, maximize and close buttons SHALL be shown;
- the window edges SHALL resize the window.

On Windows these areas SHALL behave like a native caption. This includes Snap when the window is dragged to a screen edge, and the Snap Layouts flyout when the pointer hovers the maximize button. Right-clicking a tab there SHALL open the tab menu and not the system menu.

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

#### Scenario: Right-click a tab on Windows
- **WHEN** the user right-clicks a tab on Windows
- **THEN** the tab menu opens and the system menu does not

#### Scenario: Right-click empty space
- **WHEN** the window draws its own decorations and the user right-clicks empty space in the strip
- **THEN** the window menu opens

### Requirement: Status indicator lifetime
The status dot SHALL stay until the terminal's records change (a report, a clear, a new prompt, the process exiting, or the user typing into the terminal), whether or not the tab is active. Activating the tab SHALL NOT clear it.

#### Scenario: Done survives the prompt
- **WHEN** a program in the active tab reports `state=done`, exits, and the shell prints its prompt
- **THEN** the tab shows a green dot until the user types a key into that tab

#### Scenario: Activated while blocked
- **WHEN** a background tab shows the yellow dot and the user activates it
- **THEN** the dot remains until the program reports again

### Requirement: One window
Brindle SHALL show a single window. No command SHALL open another window; opening a profile or an attached session SHALL add a tab to that window.

#### Scenario: No new window
- **WHEN** the user presses ctrl-shift-n
- **THEN** no window opens

#### Scenario: Not in the command palette
- **WHEN** the user opens the command palette and types `window`
- **THEN** "Close Window" is listed and "New Window" is not

### Requirement: Quitting keeps programs running
Quitting Brindle (`Quit`, ctrl-shift-q, closing the window) SHALL leave every open local Linux terminal's program running detached, so the next launch brings it back (see `session-restore`), except `-e` command terminals, which end. Brindle SHALL NOT ask. When Brindle ends without quitting (a crash, a signal, the desktop stopping it), the same terminals SHALL keep running.

#### Scenario: Quit
- **WHEN** the window holds three local tabs and the user presses ctrl-shift-q
- **THEN** Brindle exits without asking, and the three sessions are listed as detached with their pids unchanged

#### Scenario: Crash
- **WHEN** Brindle is killed with SIGKILL while a tab runs `sleep 1000`
- **THEN** the `sleep` keeps running, detached

#### Scenario: Command tab
- **WHEN** a tab runs `brindle -e sleep 1000`'s command and the user quits
- **THEN** that `sleep` is gone within a few seconds and no session is listed for it

### Requirement: Tab strip keyboard navigation
The `focus_tab_strip` action SHALL move focus to the active tab in the strip. With the strip focused, left, right, home and end SHALL change the active tab as a tab list does and the terminal below SHALL show the new tab, without taking focus. Enter and escape SHALL return focus to the active tab's terminal. A pointer press on a tab SHALL activate it and leave focus in its terminal, not in the strip.

#### Scenario: Arrow through tabs
- **WHEN** the strip has focus on the first of three tabs and the user presses right twice
- **THEN** the third tab is active and the strip still has focus

#### Scenario: Escape returns
- **WHEN** the strip has focus and the user presses escape
- **THEN** focus is in the active tab's terminal

#### Scenario: Click keeps focus in the terminal
- **WHEN** the user clicks a background tab
- **THEN** that tab becomes active and typing goes to its terminal

### Requirement: Strip button tooltips
The new-tab button, the profile chevron and each tab's close button SHALL show a tooltip with their name and shortcut, when they have one: "New tab", "New tab from profile…" and "Close tab".

#### Scenario: New-tab tooltip
- **WHEN** the pointer rests on the new-tab button
- **THEN** a tooltip reads "New tab" followed by its shortcut

### Requirement: Tab faces
The strip's tab faces SHALL each be drawn on its terminal's background in its terminal's foreground, with its icons and close button in that foreground, with 4 pixels between neighbouring faces and no background behind them. The strip SHALL have no background of its own beyond the window's, no border under it, and a 6-pixel margin above and at the left of the faces, matching the margin round the zone cards below. The strip's buttons SHALL use the global tokens.

#### Scenario: Mixed themes
- **WHEN** one tab's profile sets a theme other than the global one
- **THEN** that tab's face is drawn on its own theme's background and foreground, whether active or not, and the other faces are drawn in the global theme's

#### Scenario: Faces are apart
- **WHEN** a window has two tabs
- **THEN** 4 pixels of the window background show between their faces, and none shows a shared pill behind them

#### Scenario: Strip margins
- **WHEN** a window is drawn
- **THEN** the first face starts 6 pixels from the window's left edge, and 6 pixels of window background are above the faces and between them and the zone cards

#### Scenario: Active tab marker
- **WHEN** a tab is active
- **THEN** its face is raised with a shadow and its text is at full opacity, and no line runs along its top edge

#### Scenario: Inactive tab
- **WHEN** a tab is inactive and not hovered
- **THEN** its text is its foreground at 60% opacity, and at full opacity while hovered

### Requirement: Strip icons are legible
The icons of the strip's buttons (new tab, the profile chevron, the window controls) and of a tab's close button SHALL be drawn at least 16px high with a stroke of at least two pixels at the default scale.

#### Scenario: New-tab icon
- **WHEN** the strip is drawn at the default scale
- **THEN** the `+` is at least 16px across with strokes at least two pixels wide

### Requirement: Rename Tab command
The user SHALL be able to set a manual title for the active tab with a `Rename Tab` command. The command SHALL prompt for one line of text, pre-filled with the tab's current displayed title and selected for replacement.

Brindle SHALL trim the submitted text before applying it. A non-empty trimmed value SHALL become the tab's manual title. An empty trimmed value SHALL clear the tab's manual title and restore automatic title updates.

#### Scenario: Rename active tab
- **WHEN** the active tab is titled `bash`, the user runs `Rename Tab`, enters `  server  `, and submits it
- **THEN** the active tab is titled `server`

#### Scenario: Whitespace clears manual title
- **WHEN** a tab has the manual title `server`, the user runs `Rename Tab`, enters only whitespace, and submits it
- **THEN** the manual title is cleared and the tab follows the automatic title rules

### Requirement: Bell dot
A bell from a tab that is not active, or from any tab while the window is inactive, SHALL mark that tab until the user activates it. This SHALL include bells from any zone of a tab.

#### Scenario: Background bell
- **WHEN** a background tab rings the bell
- **THEN** a dot appears on that tab and disappears when the tab is activated

#### Scenario: Bell in a background zone
- **WHEN** an unfocused zone in a background tab rings the bell
- **THEN** a dot appears on that tab and disappears when the tab is activated

### Requirement: Program status dot
A tab SHALL show a status dot while any of its zones holds a program status record (see the `program-status` capability) other than `idle`. The dot's colour SHALL follow the most pressing state across those records, in the order `blocked` (the theme's yellow, ANSI 3), `error` (red, ANSI 1), `done` (green, ANSI 2), `working` (the muted foreground). It SHALL sit beside the bell dot when both apply.

#### Scenario: Blocked in a background tab
- **WHEN** a program in a background tab reports `state=blocked:kind=permission`
- **THEN** that tab shows a yellow dot

#### Scenario: Zones in a tab
- **WHEN** a program in one zone of a tab reports `state=error`, and another zone in the same tab reports `state=working`
- **THEN** that tab shows a red dot

### Requirement: Tab context menu
Right-clicking a tab SHALL activate that tab and open a menu beside the pointer listing: Rename Tab, Move Tab Left, Move Tab Right, and Close Tab. Move Tab Left SHALL be disabled on the first tab and Move Tab Right on the last. Each item SHALL run the action of the same name on that tab, and show that action's shortcut when it has one.

#### Scenario: Menu on a background tab
- **WHEN** the user right-clicks the third of four tabs while the first is active
- **THEN** the third tab becomes active and the menu opens with Move Tab Right enabled

#### Scenario: Move left
- **WHEN** the menu of the second tab is open and the user selects Move Tab Left
- **THEN** that tab becomes the first and stays active

#### Scenario: Keyboard path
- **WHEN** the config binds a key to `open_tab_menu`
- **THEN** the key opens the active tab's menu beside the tab

#### Scenario: Activated while blocked
- **WHEN** a background tab shows the yellow dot and the user activates it
- **THEN** the dot remains until the program reports again

### Requirement: New tab shortcut
ctrl-shift-t (or the `+` button) SHALL open a tab with the default profile.

#### Scenario: Default profile
- **WHEN** a tab is active and the user presses ctrl-shift-t
- **THEN** a tab with the default profile opens and becomes active
