## MODIFIED Requirements

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

## ADDED Requirements

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

## MODIFIED Requirements

### Requirement: One window
Brindle SHALL show a single window. No command SHALL open another window; opening a profile or an attached session SHALL add a tab to that window.

#### Scenario: No new window
- **WHEN** the user presses ctrl-shift-n
- **THEN** no window opens

#### Scenario: Not in the command palette
- **WHEN** the user opens the command palette and types `window`
- **THEN** "Close Window" is listed and "New Window" is not

## ADDED Requirements

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

## REMOVED Requirements

### Requirement: Manual tab title command
**Reason**: Rewritten without the tmux-tab paragraph and scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Rename Tab command".

### Requirement: Bell indicator
**Reason**: Rewritten without tmux control-mode panes and the tmux scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Bell dot".

### Requirement: Program status indicator
**Reason**: Rewritten without tmux control-mode panes and the tmux scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Program status dot".

### Requirement: Tab menu
**Reason**: Rewritten without the tmux tab variant and its scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Tab context menu".

### Requirement: New tab with the default profile
**Reason**: Rewritten without the control-mode session rule and its scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "New tab shortcut".
