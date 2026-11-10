# Spec Delta

## MODIFIED Requirements

### Requirement: Profile picker
ctrl-shift-space SHALL open the new-tab palette, a picker listing every profile with a short description of what it runs. For a control-mode profile the description SHALL name its session and, when the window shows that session at the moment the picker opens, add "open in this window". Pressing ctrl-shift-space again while it is open SHALL close it. The list SHALL be filtered by a case-insensitive subsequence match on the typed text. The picker SHALL be operable with:
- up/down (also ctrl-p / ctrl-n) to move;
- enter to open the profile;
- escape or a click outside to dismiss.

Profiles SHALL NOT be listed in the command palette. The `open_profile_picker` action name SHALL keep opening this picker from user keybindings.

#### Scenario: Filter and open
- **WHEN** the user opens the picker, types `tc` and presses enter
- **THEN** a tab opens with the first profile whose name contains `t` then `c`, such as `tmux (classic)`

#### Scenario: Existing keybinding keeps working
- **WHEN** the config binds `ctrl-shift-p` to `open_profile_picker`
- **THEN** ctrl-shift-p opens the new-tab palette instead of the command palette

#### Scenario: Session shown elsewhere
- **WHEN** the window shows session `main` of profile `tmux` in a background tab, and the user opens the picker
- **THEN** the `tmux` row's description reads `tmux control mode · main · open in this window`

#### Scenario: Pick a session shown elsewhere
- **WHEN** the window shows session `main` of profile `tmux` in a background tab, and the user picks `tmux` in the picker
- **THEN** no palette asks, a tab of `main` becomes active, and no second tmux client attaches

#### Scenario: Session not open
- **WHEN** the window doesn't show session `main`, and the user opens the picker
- **THEN** the `tmux` row's description reads `tmux control mode · main`

## ADDED Requirements

### Requirement: Profile menu
The strip's `⌄` button SHALL open a dropdown menu listing every profile in configuration order, each item showing the profile's name, the picker's description under it and, for the first nine, its ctrl-alt-N shortcut. Pressing `⌄` again, escape or a click outside SHALL close the menu. Typing SHALL jump to the profile whose name starts with the text; filtering is the picker's job.

#### Scenario: Shortcuts shown
- **WHEN** the menu opens with three profiles
- **THEN** its items show ctrl-alt-1, ctrl-alt-2 and ctrl-alt-3 at their right edge

### Requirement: Choosing from the profile menu
Choosing an item in the profile menu SHALL open that profile exactly as choosing it in the picker does, including a control-mode session that the window or another Brindle already shows.

#### Scenario: Open a profile
- **WHEN** the user presses `⌄` and chooses the second profile
- **THEN** a tab opens with the second profile, as ctrl-alt-2 would open it

#### Scenario: Description shown
- **WHEN** the window shows session `main` of profile `tmux` in a background tab and the user opens the menu
- **THEN** the `tmux` item's second line reads `tmux control mode · main · open in this window`

#### Scenario: Session shown elsewhere
- **WHEN** the window shows session `main` of profile `tmux` in a background tab and the user chooses `tmux` in the menu
- **THEN** a tab of `main` becomes active and no second tmux client attaches

#### Scenario: Toggle
- **WHEN** the menu is open and the user presses `⌄`
- **THEN** the menu closes

#### Scenario: Keyboard path
- **WHEN** the config binds a key to `open_profile_menu`
- **THEN** the key opens the menu below the `⌄` button
