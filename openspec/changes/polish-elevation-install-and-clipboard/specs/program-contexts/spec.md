# Spec Delta

## ADDED Requirements

### Requirement: Detected elevation on Windows
A local Windows terminal SHALL be detected as elevated while its foreground program, the process its tab title names, runs elevated (high integrity or above), with no OSC 3008 needed. Its notable context SHALL then be privileged, labelled `Administrator`, unless the terminal has a notable context from OSC 3008, which SHALL win. Detection SHALL NOT block the UI thread. While a process's elevation can't be read, it SHALL count as not elevated.

#### Scenario: Elevated Brindle
- **WHEN** Brindle runs elevated and opens a local PowerShell tab
- **THEN** the terminal's notable context is privileged, labelled `Administrator`, and the tab face shows the warning badge with the tooltip "Running as Administrator"

#### Scenario: sudo inline
- **WHEN** in an unelevated Brindle a user runs `sudo pwsh` with Windows sudo in inline mode, approves the prompt, and then exits that shell
- **THEN** the tab shows the privileged badge while the elevated shell runs, and none once it has exited

#### Scenario: Announced context wins
- **WHEN** an elevated tab's program prints `ESC ] 3008 ; start=c1 ; type=container ; container=web ESC \`
- **THEN** the terminal's notable context is the container, labelled `web`

#### Scenario: Unelevated tab
- **WHEN** an unelevated Brindle opens a local tab running `cmd.exe`
- **THEN** the terminal has no notable context and its tab face shows no badge

#### Scenario: Remote and Linux tabs
- **WHEN** a tab runs on a remote host, or Brindle runs on Linux
- **THEN** its notable context comes from OSC 3008 alone
