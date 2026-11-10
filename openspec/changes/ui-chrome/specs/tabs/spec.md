# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: Tab menu
Right-clicking a tab SHALL activate that tab and open a menu beside the pointer listing: Rename Tab (Rename Window on a tmux tab), Move Tab Left, Move Tab Right, and Close Tab. Move Tab Left SHALL be disabled on the first tab and Move Tab Right on the last. Each item SHALL run the action of the same name on that tab, and show that action's shortcut when it has one.

#### Scenario: Menu on a background tab
- **WHEN** the user right-clicks the third of four tabs while the first is active
- **THEN** the third tab becomes active and the menu opens with Move Tab Right enabled

#### Scenario: Move left
- **WHEN** the menu of the second tab is open and the user selects Move Tab Left
- **THEN** that tab becomes the first and stays active

#### Scenario: Tmux tab
- **WHEN** the user right-clicks a tmux window's tab
- **THEN** the menu offers Rename Window in place of Rename Tab, and Close Tab closes that tmux window

#### Scenario: Keyboard path
- **WHEN** the config binds a key to `open_tab_menu`
- **THEN** the key opens the active tab's menu beside the tab

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
Each tab's face in the strip SHALL be drawn on its terminal's background in its terminal's foreground, in shadcn's `line` geometry (a 36px list, square list corners, 4px between faces). An inactive tab's text SHALL be the foreground at 60% opacity until hovered. The active tab SHALL be marked by full-opacity text and a 2px line along its top edge in that tab's theme accent. The strip's own background and its buttons SHALL use the global tokens.

#### Scenario: Mixed themes
- **WHEN** one tab's profile sets a theme other than the global one
- **THEN** that tab's face is drawn on its own theme's background and foreground, whether active or not, and the other faces are drawn in the global theme's

#### Scenario: Active tab marker
- **WHEN** a tab is active
- **THEN** its text is at full opacity and a 2px line in its theme's accent runs along its top edge; inactive tabs have no line

### Requirement: Active-tab action buttons
The strip SHALL show Split Right, Split Down and Find buttons, between its empty space and the window controls, acting on the active tab through the same actions as the keymap (the tmux split actions on a tmux tab). A split button SHALL be disabled when the active zone cannot be split along its axis, and a press on any of them SHALL NOT move the window or take focus from the terminal.

#### Scenario: Split from the strip
- **WHEN** the user presses Split Right in a local tab with one zone
- **THEN** the tab has two zones side by side and keyboard focus is in the new zone's terminal

#### Scenario: Tmux tab
- **WHEN** the active tab is a tmux window and the user presses Split Down
- **THEN** tmux receives a down split, as for the `tmux_split_down` action

#### Scenario: Find from the strip
- **WHEN** the user presses Find
- **THEN** the active terminal's find bar opens with its query focused

#### Scenario: Button tooltip
- **WHEN** the pointer rests on Split Right
- **THEN** a tooltip reads "Split right" with the key bound to the action
