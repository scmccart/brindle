# Spec Delta

## MODIFIED Requirements

### Requirement: Tab titles
A tab's title SHALL be:
1. the title the program set (OSC 0/2), when there is one;
2. otherwise the name of the terminal's foreground process. Windows has no foreground process group, so there this is the most recently started descendant of the tab's program, or the program itself when it has none, without the `.exe` suffix;
3. otherwise the profile name.

The window title SHALL match the active tab's title.

#### Scenario: Foreground process
- **WHEN** the user starts `htop` in a tab whose program sets no title
- **THEN** within about a second the tab is titled `htop`

#### Scenario: Child process on Windows
- **WHEN** the user runs `python` from `cmd.exe`, which sets no title, in a Windows tab
- **THEN** within about a second the tab is titled `python`, and after Python exits it is titled `cmd`

### Requirement: Working directory inheritance
On Linux, a new tab whose profile sets no working directory SHALL start in the working directory of the active tab's foreground process. On Windows, where another process's working directory can't be read, such a tab SHALL start in the home directory.

#### Scenario: New tab in same directory
- **WHEN** the active tab's shell is in `~/src/brindle` and the user opens a new tab
- **THEN** the new shell starts in `~/src/brindle`

#### Scenario: New tab on Windows
- **WHEN** the active tab's shell on Windows has changed to `C:\src` and the user opens a new tab with a profile that sets no directory
- **THEN** the new shell starts in the user's home directory

### Requirement: Title bar behaviour
When the window draws its own decorations, as it always does on Windows:
- empty space in the tab strip SHALL move the window when dragged;
- double-clicking empty space SHALL maximize or restore the window;
- right-clicking SHALL open the window menu;
- minimize, maximize and close buttons SHALL be shown;
- the window edges SHALL resize the window.

On Windows, these SHALL behave like a native caption, including Snap when the window is dragged to a screen edge and the Snap Layouts flyout when the pointer hovers the maximize button.

#### Scenario: Drag to move
- **WHEN** the user drags empty space in the tab strip
- **THEN** the window moves

#### Scenario: Snap Layouts on Windows 11
- **WHEN** the user hovers the maximize button on Windows 11
- **THEN** the Snap Layouts flyout appears

#### Scenario: Close button on Windows
- **WHEN** the user clicks the close button of the only window
- **THEN** the window closes and Brindle exits
