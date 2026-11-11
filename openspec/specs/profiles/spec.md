# profiles Specification

## Purpose
Let users define named launch configurations (command, directory, environment and look) and open a tab from any of them quickly.

## Requirements

### Requirement: Profile definition
A profile SHALL be able to set:
- a name;
- a command and its arguments. The default on Linux is `$SHELL`, then `/bin/sh`. On Windows it is `pwsh.exe` when it is on `PATH`, then `powershell.exe`, then `%COMSPEC%`, then `cmd.exe`;
- a working directory, with `~` expanded;
- environment variables;
- a theme;
- a font size.

There SHALL always be at least one profile; a "Shell" profile is created when the config defines none. Profiles without a name SHALL be named `Profile N`. A profile that still sets `tmux`, `tmux_session` or `tmux_args` (settings of earlier versions) SHALL load, with those keys ignored and a warning logged for each.

#### Scenario: No profiles configured
- **WHEN** the config defines no profiles
- **THEN** a single "Shell" profile running the user's shell is available

#### Scenario: Default shell on Windows
- **WHEN** a profile sets no command on a Windows machine that has PowerShell 7 installed
- **THEN** its tabs run `pwsh.exe`

#### Scenario: Windows without PowerShell 7
- **WHEN** a profile sets no command and `pwsh.exe` is not on `PATH`
- **THEN** its tabs run `powershell.exe`

#### Scenario: Arguments with spaces on Windows
- **WHEN** a Windows profile sets `command = "pwsh.exe"` and `args = ["-NoExit", "-Command", "Set-Location 'C:\My Files'"]`
- **THEN** the program receives exactly three arguments, the last one with its spaces intact

#### Scenario: Old tmux settings are ignored
- **WHEN** a profile `work` sets `tmux = "plain"`, `tmux_session = "work"` and `tmux_args = ["-L", "w"]`, and no command
- **THEN** the config loads, the log has a warning for each of the three keys, and a tab of `work` runs the default shell, not tmux

#### Scenario: Running tmux as a command
- **WHEN** a profile sets `command = "tmux"` and `args = ["new-session", "-A", "-s", "main"]`
- **THEN** its tabs run that command in an ordinary terminal, as they would any program

### Requirement: Default profile
New tabs SHALL use the profile named by `default_profile`, matched case-insensitively. If no profile has that name, the first profile SHALL be used.

#### Scenario: Case-insensitive default
- **WHEN** `default_profile = "work"` and a profile is named `Work`
- **THEN** ctrl-shift-t opens a `Work` tab

### Requirement: Direct profile shortcuts
ctrl-alt-N SHALL open a tab with the Nth profile, for N from 1 to 9.

#### Scenario: Second profile
- **WHEN** the user presses ctrl-alt-2
- **THEN** a tab opens using the second profile

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

### Requirement: Remote target
A profile MAY define one remote Linux endpoint. An SSH endpoint SHALL accept `host` and optional `user`, `port`, `identity_file`, and `ssh_args`. A container endpoint SHALL accept `container`, optional `user`, and `engine` (`docker` by default, or `podman`). The profile's command, arguments, working directory, environment, theme, and font size SHALL otherwise retain their local meanings on that endpoint.

#### Scenario: SSH config alias
- **WHEN** a profile sets `remote = { type = "ssh", host = "A8" }`
- **THEN** Brindle invokes OpenSSH using `A8`, so the user's SSH configuration, agent, proxy, and host-key policy apply

#### Scenario: Existing container
- **WHEN** a profile sets `remote = { type = "container", container = "devbox" }`
- **THEN** Brindle opens the profile in the already-running `devbox` through `docker exec` and does not create or start a container

### Requirement: Profile menu
The strip's `⌄` button SHALL open a dropdown menu listing every profile in configuration order, each item showing the profile's name, the picker's description under it and, for the first nine, its ctrl-alt-N shortcut. Pressing `⌄` again, escape or a click outside SHALL close the menu. Typing SHALL jump to the profile whose name starts with the text; filtering is the picker's job.

#### Scenario: Shortcuts shown
- **WHEN** the menu opens with three profiles
- **THEN** its items show ctrl-alt-1, ctrl-alt-2 and ctrl-alt-3 at their right edge

### Requirement: New-tab profile picker
ctrl-shift-space SHALL open the new-tab palette, a picker listing every profile with a short description of what it runs. Pressing ctrl-shift-space again while it is open SHALL close it. The list SHALL be filtered by a case-insensitive subsequence match on the typed text. The picker SHALL be operable with:
- up/down (also ctrl-p / ctrl-n) to move;
- enter to open the profile;
- escape or a click outside to dismiss.

Profiles SHALL NOT be listed in the command palette. The `open_profile_picker` action name SHALL keep opening this picker from user keybindings.

#### Scenario: Filter and open
- **WHEN** the user opens the picker, types `wk` and presses enter
- **THEN** a tab opens with the first profile whose name contains `w` then `k`, such as `Work`

#### Scenario: Existing keybinding keeps working
- **WHEN** the config binds `ctrl-shift-p` to `open_profile_picker`
- **THEN** ctrl-shift-p opens the new-tab palette instead of the command palette

### Requirement: Opening a profile from the menu
Choosing an item in the profile menu SHALL open that profile exactly as choosing it in the picker does.

#### Scenario: Open a profile
- **WHEN** the user presses `⌄` and chooses the second profile
- **THEN** a tab opens with the second profile, as ctrl-alt-2 would open it

#### Scenario: Toggle
- **WHEN** the menu is open and the user presses `⌄`
- **THEN** the menu closes

#### Scenario: Keyboard path
- **WHEN** the config binds a key to `open_profile_menu`
- **THEN** the key opens the menu below the `⌄` button
