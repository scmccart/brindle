# Spec Delta

## MODIFIED Requirements

### Requirement: Attach to a session
Opening a profile with `tmux = "control"` SHALL attach to the profile's session through `tmux -u [tmux_args] -C new-session -A -s <session>`, creating the session if needed. Each of the session's windows SHALL appear as a tab, in tmux's window order, with the tmux window name as the tab title. A session SHALL be shown by at most one Brindle window. Opening a control profile whose session is already attached in the current window SHALL make one of its tabs active. When it is attached in another window, Brindle SHALL NOT attach it again. Picking the profile in the new-tab palette, which says beforehand that the session is open in another window, SHALL go to that session as "Go to <session>" does, without asking. The other ways of opening it SHALL ask what to do, as defined by "Choosing when a session is open elsewhere". Two profiles that run the same tmux program with the same `tmux_args` and session name SHALL count as the same session.

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

## ADDED Requirements

### Requirement: Control clients are UTF-8 in any locale
Brindle's control-mode clients SHALL tell tmux they handle UTF-8, whatever locale Brindle runs in. Attaching, the tab titles and every reply Brindle parses SHALL be the same with no locale set (`LANG`, `LC_ALL` and `LC_CTYPE` all unset, or `C`) as under a UTF-8 locale.

#### Scenario: No locale
- **WHEN** Brindle runs with `LANG`, `LC_ALL`, `LC_CTYPE` and `TMUX` unset and opens a control profile whose session has windows named `café` and `logs`
- **THEN** two tabs appear, titled `café` and `logs`

#### Scenario: The user's own flags still apply
- **WHEN** a control profile has `tmux_args = ["-L", "work"]`
- **THEN** Brindle runs `tmux -u -L work -C new-session -A -s <session>`, attaching on the `work` server
