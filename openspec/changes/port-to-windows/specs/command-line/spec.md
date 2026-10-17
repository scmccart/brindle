# Spec Delta

## MODIFIED Requirements

### Requirement: Informational options
`--help`, `--version`, `--list-actions` (every bindable action name), `--list-themes` (the built-in themes) and `--shell-integration <bash|zsh|fish|pwsh>` (that shell's integration script) SHALL print their output and exit without opening a window. `--shell-integration` with a missing or unsupported shell SHALL print an error naming the supported shells and exit with status 2.

#### Scenario: List actions
- **WHEN** the user runs `brindle --list-actions`
- **THEN** each bindable action name is printed, including `activate_tab_<N>` and `new_tab_with_profile_<N>`

#### Scenario: Print a shell script
- **WHEN** the user runs `brindle --shell-integration zsh`
- **THEN** the zsh integration script is printed and the process exits 0 without opening a window

#### Scenario: Print the PowerShell script
- **WHEN** the user runs `brindle --shell-integration pwsh`
- **THEN** the PowerShell integration script is printed and the process exits 0 without opening a window

## ADDED Requirements

### Requirement: No console window on Windows
On Windows, starting Brindle from the Start Menu, Explorer or a shortcut SHALL NOT open a console window. Programs Brindle starts outside a tab, such as tmux through WSL, the link opener and tmux session checks, SHALL NOT open console windows either.

#### Scenario: Start Menu launch
- **WHEN** the user starts Brindle from the Start Menu
- **THEN** only Brindle's window opens, with no console window beside it

#### Scenario: WSL attach opens no console
- **WHEN** the user opens a control-mode profile whose command is `wsl`
- **THEN** no console window appears while the session is attached

### Requirement: Console output on Windows
When Brindle is started from a console on Windows, its informational output, usage errors and `--dump-screen-after` output SHALL appear in that console, or in the file or pipe its output is redirected to. Closing that console, or pressing ctrl-c in it, SHALL NOT close Brindle's windows.

#### Scenario: Version from PowerShell
- **WHEN** the user runs `brindle --version` in PowerShell
- **THEN** the version is printed in that PowerShell window

#### Scenario: Launching console closes
- **WHEN** the user runs `brindle` from PowerShell, then presses ctrl-c in that PowerShell and closes its window
- **THEN** Brindle's window stays open and keeps working

#### Scenario: Redirected output
- **WHEN** a test runs `brindle --dump-screen-after 2 > dump.txt` on Windows
- **THEN** `dump.txt` contains the screen dump
