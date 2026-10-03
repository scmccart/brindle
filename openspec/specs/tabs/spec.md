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
1. the title the program set (OSC 0/2), when there is one;
2. otherwise the name of the terminal's foreground process;
3. otherwise the profile name.

The window title SHALL match the active tab's title.

#### Scenario: Foreground process
- **WHEN** the user starts `htop` in a tab whose program sets no title
- **THEN** within about a second the tab is titled `htop`

### Requirement: Working directory inheritance
A new tab whose profile sets no working directory SHALL start in the working directory of the active tab's foreground process.

#### Scenario: New tab in same directory
- **WHEN** the active tab's shell is in `~/src/brindle` and the user opens a new tab
- **THEN** the new shell starts in `~/src/brindle`

### Requirement: Bell indicator
A bell from a tab that is not active, or from any tab while the window is inactive, SHALL mark that tab until the user activates it.

#### Scenario: Background bell
- **WHEN** a background tab rings the bell
- **THEN** a dot appears on that tab and disappears when the tab is activated

### Requirement: Title bar behaviour
When the window draws its own decorations:
- empty space in the tab strip SHALL move the window when dragged;
- double-clicking empty space SHALL maximize or restore the window;
- right-clicking SHALL open the window menu;
- minimize, maximize and close buttons SHALL be shown;
- the window edges SHALL resize the window.

#### Scenario: Drag to move
- **WHEN** the user drags empty space in the tab strip
- **THEN** the window moves

### Requirement: Multiple windows
The user SHALL be able to open additional windows (ctrl-shift-n), each with its own tabs, and quit everything with ctrl-shift-q.

#### Scenario: New window
- **WHEN** the user presses ctrl-shift-n
- **THEN** a second window opens with one tab using the default profile
