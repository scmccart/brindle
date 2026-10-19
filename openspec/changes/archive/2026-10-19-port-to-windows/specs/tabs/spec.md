# Spec Delta

## MODIFIED Requirements

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
