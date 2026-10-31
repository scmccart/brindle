# Spec Delta

## MODIFIED Requirements

### Requirement: Tab titles
A tab's title SHALL be the manual title the user set for that tab, when one is set. Otherwise a tab's title SHALL be:
1. the title the program set (OSC 0/2), when there is one. On Windows the console host starts every program with the program's full path as its title, and cmd builds its title while running a command from that path plus ` - <command>`. A title that is such a path, alone or followed by ` - `, counts as no title;
2. otherwise the name of the terminal's foreground process. Windows has no foreground process group, so there this is the most recently started descendant of the tab's program, or the program itself when it has none, without the `.exe` suffix;
3. otherwise the profile name.

The window title SHALL match the active tab's title.

A manual title SHALL stay in effect until the user changes or clears it. Program title changes, foreground process changes, profile name changes, and tmux window name changes SHALL NOT replace a manual title.

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

#### Scenario: Manual title overrides automatic changes
- **WHEN** the user renames a tab to `prod logs` and that tab's program later changes its OSC title or foreground process
- **THEN** the tab remains titled `prod logs`

#### Scenario: Clearing a manual title restores automatic titles
- **WHEN** the user clears a manual title from a tab whose foreground process is `bash`
- **THEN** the tab is titled `bash` by the automatic title rules

## ADDED Requirements

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
