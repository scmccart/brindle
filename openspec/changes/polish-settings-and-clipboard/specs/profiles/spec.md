## MODIFIED Requirements

### Requirement: Profile menu
The strip's `⌄` button SHALL open a dropdown menu listing every profile in configuration order, each item showing the profile's name, the picker's description under it and, for the first nine, its ctrl-alt-N shortcut. After the profiles come a separator and a "Settings" item showing the settings dialog's shortcut. Pressing `⌄` again, escape or a click outside SHALL close the menu. Typing SHALL jump to the item whose label starts with the text; filtering is the picker's job.

#### Scenario: Shortcuts shown
- **WHEN** the menu opens with three profiles
- **THEN** its items show ctrl-alt-1, ctrl-alt-2 and ctrl-alt-3 at their right edge

#### Scenario: Settings last
- **WHEN** the menu opens with two profiles
- **THEN** it lists the two profiles, a separator, then "Settings" with the shortcut the command palette shows for Open Settings

## ADDED Requirements

### Requirement: Settings from the profile menu
Choosing "Settings" in the profile menu SHALL open the settings dialog as its shortcut does, with keyboard focus in the dialog, and SHALL open no tab. When the dialog closes, keyboard focus SHALL return to the tab that had it before the menu opened.

#### Scenario: Open settings from the menu
- **WHEN** the user presses `⌄` and chooses "Settings"
- **THEN** the menu closes, the settings dialog opens on its first section with keyboard focus, and the window has as many tabs as before

#### Scenario: Keyboard path to Settings
- **WHEN** the profile menu is open and the user presses end, then enter
- **THEN** the settings dialog opens

#### Scenario: Focus comes back
- **WHEN** the user opens the dialog from the profile menu and presses escape in its first list
- **THEN** the dialog closes and keyboard focus is on the tab's terminal
