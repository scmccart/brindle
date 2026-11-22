# Spec Delta

## ADDED Requirements

### Requirement: Detected elevation on Windows
A local Windows terminal SHALL be detected as elevated while its foreground program, the process its tab title names, runs elevated (high integrity or above), with no OSC 3008 needed. Its notable context SHALL then be privileged, labelled `Administrator`, whatever OSC 3008 context a program announces, which decides only while the terminal isn't elevated. Detection SHALL NOT block the UI thread. While a process's elevation can't be read, it SHALL count as not elevated.

#### Scenario: Elevated Brindle
- **WHEN** Brindle runs elevated and opens a local PowerShell tab
- **THEN** the terminal's notable context is privileged, labelled `Administrator`, and the tab face shows the warning badge with the tooltip "Running as Administrator"

#### Scenario: Elevated program from the tab's shell
- **WHEN** in an unelevated Brindle a user runs a program that stays the tab's descendant and runs elevated, and then it exits
- **THEN** the tab shows the privileged badge while that program runs, and none once it has exited

#### Scenario: sudo inline not detected
- **WHEN** in an unelevated Brindle a user runs `sudo pwsh` with Windows sudo in inline mode and approves the prompt
- **THEN** the badge may show for about a second while sudo's elevated helper runs, and none once the elevated shell's prompt shows, since that shell is no longer a descendant of the tab's program

#### Scenario: Detected elevation can't be hidden
- **WHEN** an elevated tab's program prints `ESC ] 3008 ; start=c1 ; type=container ; container=web ESC \`, or an `elevate` context with `targetuser=guest`
- **THEN** the terminal's notable context stays privileged, labelled `Administrator`

#### Scenario: Announced context in an unelevated tab
- **WHEN** an unelevated tab's program prints `ESC ] 3008 ; start=c1 ; type=container ; container=web ESC \`
- **THEN** the terminal's notable context is the container, labelled `web`

#### Scenario: Unelevated tab
- **WHEN** an unelevated Brindle opens a local tab running `cmd.exe`
- **THEN** the terminal has no notable context and its tab face shows no badge

#### Scenario: Remote and Linux tabs
- **WHEN** a tab runs on a remote host, or Brindle runs on Linux
- **THEN** its notable context comes from OSC 3008 alone
