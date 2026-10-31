# Spec Delta

## MODIFIED Requirements

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
