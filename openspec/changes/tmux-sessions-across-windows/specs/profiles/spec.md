# Spec Delta

## MODIFIED Requirements

### Requirement: Default profile
New tabs and windows SHALL use the profile named by `default_profile`, matched case-insensitively. If no profile has that name, the first profile SHALL be used. When that profile is a control-mode profile whose session is already shown in another window (or, for a new window, in any window), Brindle SHALL ask what to open instead, as defined by `tabs`.

#### Scenario: Case-insensitive default
- **WHEN** `default_profile = "work"` and a profile is named `Work`
- **THEN** ctrl-shift-t opens a `Work` tab

### Requirement: Profile picker
ctrl-shift-space (or the strip's `⌄` button) SHALL open the new-tab palette, a picker listing every profile with a short description of what it runs. For a control-mode profile the description SHALL name its session and, when that session is shown in a window at the moment the picker opens, add "open in this window" or "open in another window". Pressing ctrl-shift-space again while it is open SHALL close it. The list SHALL be filtered by a case-insensitive subsequence match on the typed text. The picker SHALL be operable with:
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
- **WHEN** another window shows session `main` of profile `tmux`, and the user opens the picker
- **THEN** the `tmux` row's description reads `tmux control mode · main · open in another window`

#### Scenario: Session not open
- **WHEN** no window shows session `main`, and the user opens the picker
- **THEN** the `tmux` row's description reads `tmux control mode · main`
