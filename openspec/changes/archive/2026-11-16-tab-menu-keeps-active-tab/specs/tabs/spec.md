# Spec Delta

## MODIFIED Requirements

### Requirement: Tab context menu
Right-clicking a tab SHALL open a menu for that tab beside the pointer, listing Rename Tab, Move Tab Left, Move Tab Right and Close Tab. It SHALL NOT activate the tab: the active tab and the focus stay as they were, and closing the menu returns focus to the active tab. Move Tab Left SHALL be disabled on the first tab and Move Tab Right on the last. Each item SHALL do what the action of the same name does, but to the right-clicked tab rather than the active one, without activating it. Each item SHALL show that action's shortcut when it has one.

#### Scenario: Menu on a background tab
- **WHEN** the user right-clicks the third of four tabs while the first is active
- **THEN** the menu opens with Move Tab Right enabled, and the first tab stays active and keeps focus

#### Scenario: Dismissed without a choice
- **WHEN** the user right-clicks a background tab and presses escape
- **THEN** the menu closes and the tab that was active is still active, with focus in its terminal

#### Scenario: Move left
- **WHEN** the menu of the second tab is open and the user selects Move Tab Left
- **THEN** that tab becomes the first and stays active

#### Scenario: Move a background tab
- **WHEN** the first of three tabs is active and the user selects Move Tab Left from the third tab's menu
- **THEN** the third tab becomes the second, and the first tab is still active

#### Scenario: Close a background tab
- **WHEN** the second of three tabs is active and the user selects Close Tab from the third tab's menu
- **THEN** the third tab closes and the second tab is still active

#### Scenario: Rename a background tab
- **WHEN** the first tab is active and the user selects Rename Tab from the second tab's menu and submits "logs"
- **THEN** the second tab is titled "logs", the first tab's title is unchanged, and the first tab is still active

#### Scenario: Keyboard path
- **WHEN** the config binds a key to `open_tab_menu`
- **THEN** the key opens the active tab's menu beside the tab

#### Scenario: Activated while blocked
- **WHEN** a background tab shows the yellow dot and the user activates it
- **THEN** the dot remains until the program reports again
