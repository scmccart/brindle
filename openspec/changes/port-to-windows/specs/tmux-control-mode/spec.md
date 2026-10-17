# Spec Delta

## MODIFIED Requirements

### Requirement: Attach to a session
Opening a profile with `tmux = "control"` SHALL attach to the profile's session through `tmux -u [tmux_args] -C new-session -A -s <session>`, creating the session if needed. For a WSL profile on Windows, the command line is the one defined by "Control mode through WSL on Windows". Each of the session's windows SHALL appear as a tab, in tmux's window order, with the tmux window name as the tab title. A session SHALL be shown by at most one Brindle window. Opening a control profile whose session is already attached in the current window SHALL make one of its tabs active. When it is attached in another window, Brindle SHALL NOT attach it again. Picking the profile in the new-tab palette, which says beforehand that the session is open in another window, SHALL go to that session as "Go to <session>" does, without asking. The other ways of opening it SHALL ask what to do, as defined by "Choosing when a session is open elsewhere". Two profiles that run the same tmux program with the same `tmux_args` and session name SHALL count as the same session.

#### Scenario: Existing session with two windows
- **WHEN** the user opens a control profile whose session has windows 0 and 1
- **THEN** two tabs appear, named after the two windows, and tmux's current window is active

#### Scenario: Session open in this window
- **WHEN** the window shows a local tab (active) and the tabs of session `main`, and the user picks the profile for `main` in the new-tab palette
- **THEN** a tab of `main` becomes active, nothing is asked, and no second tmux client attaches

#### Scenario: Session open in another window
- **WHEN** window A shows session `main`, and the user picks the profile for `main` from the new-tab palette in window B
- **THEN** nothing is asked, window A's tab for tmux's current window of `main` becomes active, Brindle asks the desktop to raise window A, and `tmux list-clients` shows one Brindle client for `main`

#### Scenario: Session open in another window, opened by shortcut
- **WHEN** window A shows session `main`, and the user presses the ctrl-alt-N shortcut of the profile for `main` in window B
- **THEN** window B asks what to do, window A is not raised, and `tmux list-clients` shows one Brindle client for `main`

### Requirement: Create a new session
A "tmux: New Session" command SHALL prompt for a session name, then create that session and attach it in the current window, beside the window's other tabs. An empty name SHALL let tmux choose the name. The new session SHALL start in the active tab's directory, except with a WSL profile on Windows, as defined by "No Windows directory reaches tmux in WSL". It SHALL use, in this order, the active tab's control-mode profile, the default profile if it is a control-mode profile, or the first control-mode profile. The command SHALL be available from every tab when the config has at least one control-mode profile, and SHALL NOT be listed otherwise. It has no default keybinding; its action name is `tmux_new_session`.

#### Scenario: Named session
- **WHEN** the user runs "tmux: New Session" from a local tab and enters `work`
- **THEN** `tmux ls` lists a session named `work`, its window appears as a tab in the current window, and the prompt closes

#### Scenario: Name left empty
- **WHEN** the user runs "tmux: New Session" and submits an empty name
- **THEN** a session with a name tmux chose is created and shown, and its tabs are recorded for session restore under that name

#### Scenario: From a tmux tab
- **WHEN** the active tab belongs to a session of profile `remote`, whose `tmux_args` select another server, and the user creates session `logs`
- **THEN** `logs` is created on the server `remote` uses

#### Scenario: Name already taken
- **WHEN** the user enters the name of a session that already exists
- **THEN** the prompt stays open showing tmux's error (`duplicate session: <name>`), no tab is added, and no attach-failure tab opens

#### Scenario: No control-mode profile
- **WHEN** no profile in the config uses `tmux = "control"`
- **THEN** "tmux: New Session" is not listed in the command palette

## ADDED Requirements

### Requirement: Control mode through WSL on Windows
On Windows, a control-mode profile whose command is `wsl` or `wsl.exe` SHALL run tmux inside WSL, with all control-mode behaviour the same as on Linux. Its `tmux_args` are WSL's options, then `tmux`, then tmux's options. If `tmux` is absent, all of them are WSL's options. Brindle SHALL run `wsl <WSL options> --exec tmux -u <tmux options> -C …`, adding `--exec` only when the WSL options don't end with `--exec` or `-e`.

#### Scenario: Attach through WSL
- **WHEN** a Windows profile sets `tmux = "control"`, `command = "wsl"` and `tmux_args = ["tmux"]`, and the user opens it
- **THEN** Brindle runs `wsl --exec tmux -u -C new-session -A -s main`, and each window of that WSL session appears as a native tab with its panes drawn natively

#### Scenario: Distribution and server
- **WHEN** the profile's `tmux_args` are `["-d", "Ubuntu", "tmux", "-L", "work"]`
- **THEN** Brindle runs `wsl -d Ubuntu --exec tmux -u -L work -C new-session -A -s main`

#### Scenario: No tmux element
- **WHEN** the profile's `tmux_args` are `["-d", "Debian"]`
- **THEN** Brindle runs `wsl -d Debian --exec tmux -u -C new-session -A -s main`

#### Scenario: Session checks go through WSL too
- **WHEN** the user runs "tmux: New Session" with such a profile and enters `logs`
- **THEN** the name check runs `wsl --exec tmux has-session -t =logs`, and `logs` is created inside WSL

### Requirement: No Windows directory reaches tmux in WSL
For a WSL control-mode profile, Brindle SHALL NOT pass a start directory to tmux (`-c`), whichever command attaches or creates the session. New sessions start in tmux's default directory inside WSL.

#### Scenario: No Windows path reaches tmux
- **WHEN** such a profile is opened from a local tab whose reported directory is `C:\src`
- **THEN** the tmux command line contains no `-c` argument
