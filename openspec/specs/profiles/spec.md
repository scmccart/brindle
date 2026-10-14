# profiles Specification

## Purpose
Let users define named launch configurations (command, directory, environment, look and tmux mode) and open a tab from any of them quickly.

## Requirements

### Requirement: Profile definition
A profile SHALL be able to set:
- a name;
- a command and its arguments (default: `$SHELL`, then `/bin/sh`);
- a working directory, with `~` expanded;
- environment variables;
- a theme;
- a font size;
- a tmux mode (`none`, `plain` or `control`), with a session name (default `main`) and extra tmux arguments.

There SHALL always be at least one profile; a "Shell" profile is created when the config defines none. Profiles without a name SHALL be named `Profile N`.

#### Scenario: No profiles configured
- **WHEN** the config defines no profiles
- **THEN** a single "Shell" profile running the user's shell is available

### Requirement: Default profile
New tabs and windows SHALL use the profile named by `default_profile`, matched case-insensitively. If no profile has that name, the first profile SHALL be used. When that profile is a control-mode profile whose session is already shown in another window (or, for a new window, in any window), Brindle SHALL ask what to open instead, as defined by `tabs`.

#### Scenario: Case-insensitive default
- **WHEN** `default_profile = "work"` and a profile is named `Work`
- **THEN** ctrl-shift-t opens a `Work` tab

### Requirement: Profile picker
ctrl-shift-space (or the strip's `⌄` button) SHALL open the new-tab palette, a picker listing every profile with a short description of what it runs. For a control-mode profile the description SHALL name its session and, when that session is shown in a window at the moment the picker opens, add "open in this window" or "in another window (switches)". Picking a profile whose session is open in another window SHALL go to that session without asking, as defined by "Attach to a session" in `tmux-control-mode`. Pressing ctrl-shift-space again while it is open SHALL close it. The list SHALL be filtered by a case-insensitive subsequence match on the typed text. The picker SHALL be operable with:
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
- **THEN** the `tmux` row's description reads `tmux control mode · main · in another window (switches)`

#### Scenario: Pick a session shown elsewhere
- **WHEN** window A shows session `main` of profile `tmux`, and the user picks `tmux` in window B's picker
- **THEN** no palette asks, window A's tab of `main` becomes active, and Brindle asks the desktop to raise window A

#### Scenario: Session not open
- **WHEN** no window shows session `main`, and the user opens the picker
- **THEN** the `tmux` row's description reads `tmux control mode · main`

### Requirement: Direct profile shortcuts
ctrl-alt-N SHALL open a tab with the Nth profile, for N from 1 to 9.

#### Scenario: Second profile
- **WHEN** the user presses ctrl-alt-2
- **THEN** a tab opens using the second profile

### Requirement: Classic tmux profiles
A profile with `tmux = "plain"` SHALL run `tmux [tmux_args] new-session -A -s <session>` in an ordinary tab. This attaches to the session if it already exists.

#### Scenario: Reattach classic session
- **WHEN** the user opens a plain tmux profile while that session already exists
- **THEN** the tab attaches to the existing session instead of creating a new one

### Requirement: Per-profile appearance
A profile's theme and font size SHALL apply to its tabs only, and SHALL update when the config is reloaded.

#### Scenario: Themed profile
- **WHEN** a profile sets `theme = "tokyo-night"`
- **THEN** its tabs use Tokyo Night colors and other tabs keep the default theme

### Requirement: Renamed profiles keep their tabs
When a profile is renamed from the settings dialog, every open tab of that profile, in every window, SHALL follow the new name. Later changes to the profile SHALL reach those tabs on reload. A rename made by editing the config file directly SHALL leave such tabs on their previous settings until they are closed.

#### Scenario: Edit after rename
- **WHEN** a tab uses profile `Work`, the user renames it to `Job` in the dialog, and then sets `Job`'s theme to `tokyo-night` and saves
- **THEN** the tab shows `tokyo-night`
