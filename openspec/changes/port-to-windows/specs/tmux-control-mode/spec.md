# Spec Delta

## ADDED Requirements

### Requirement: Control mode through WSL on Windows
On Windows, a control-mode profile whose command is `wsl` or `wsl.exe`, with `tmux` among its tmux arguments, SHALL attach to tmux inside WSL. All control-mode behaviour SHALL be the same as on Linux. Brindle SHALL NOT pass the tab's Windows working directory to tmux as a start directory for such a profile.

#### Scenario: Attach through WSL
- **WHEN** a Windows profile sets `tmux = "control"`, `command = "wsl"` and `tmux_args = ["tmux"]`, and the user opens it
- **THEN** each window of the WSL tmux session appears as a native tab with its panes drawn natively

#### Scenario: No Windows path reaches tmux
- **WHEN** such a profile is opened from a tab whose directory is `C:\src`
- **THEN** the tmux command line contains no `-c` argument
