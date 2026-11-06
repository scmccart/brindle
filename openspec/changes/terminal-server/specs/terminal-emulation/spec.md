# Spec Delta

## ADDED Requirements

### Requirement: Local Linux terminals run in the terminal server
On Linux, each local tab and zone SHALL run its program in a session of the terminal server, as defined by the `terminal-server` capability, with the `kill` close policy. Brindle SHALL start the server when none is running. Every other requirement of this capability, and titles, directories, shell integration, program status and notifications, SHALL behave as they do with a PTY owned by Brindle.

#### Scenario: Tab program runs under the server
- **WHEN** a local Linux tab starts the user's shell
- **THEN** the shell's parent process is the terminal server, not Brindle, and `brindle-remote list` lists its session as attached

#### Scenario: Closing a tab ends its program
- **WHEN** the user closes a local tab that is running `sleep 1000`
- **THEN** `sleep` is gone and the session is no longer listed

#### Scenario: Quitting ends every program
- **WHEN** the user quits Brindle with local tabs open
- **THEN** their programs are gone and their sessions are no longer listed

#### Scenario: A crash ends every program
- **WHEN** Brindle is killed with SIGKILL while local tabs are open
- **THEN** their programs are gone within a few seconds

#### Scenario: Titles and directories
- **WHEN** the user runs `cd /var/log` and then `htop` in a local tab, and splits the zone while `htop` runs
- **THEN** the tab title shows `htop`, and the new zone starts in `/var/log`

### Requirement: Sessions get the launching Brindle's environment
A local Linux tab's program SHALL see the environment of the Brindle process that opened it, with the defaults and profile entries of "Program launch environment" applied, never the environment of the Brindle that started the server.

#### Scenario: Second Brindle with a different environment
- **WHEN** a Brindle started with `FOO=1` is running, and a second Brindle started with `FOO=2` opens a tab
- **THEN** `echo $FOO` in the second Brindle's tab prints `2`

### Requirement: Losing the terminal server is reported in the tab
When Brindle's connection to the terminal server ends unexpectedly, each affected tab SHALL keep its screen and show a message saying the terminal server went away, instead of closing.

#### Scenario: Server killed
- **WHEN** the terminal server is killed with SIGKILL while a local tab is open
- **THEN** the tab stays open with its last screen and a line saying the terminal server went away

### Requirement: An unusable terminal server is reported in the tab
Local Linux tabs SHALL always run in the terminal server; Brindle SHALL NOT run their programs on PTYs of its own. When Brindle can't find the `brindle-remote` binary, can't start or reach the server, or shares no protocol version with the running server, each local tab it opens SHALL show an error naming the reason, as for a program that can't start. Brindle SHALL try the server again for the next tab.

#### Scenario: Missing helper
- **WHEN** no `brindle-remote` is found beside Brindle and `BRINDLE_REMOTE_AGENT` is unset, and the user opens a local tab
- **THEN** the tab shows an error saying the terminal server helper was not found, with how to point Brindle at it

#### Scenario: No common protocol version
- **WHEN** the running server speaks none of Brindle's protocol versions and refuses the upgrade
- **THEN** the tab shows an error naming both versions, and the running server and its sessions are untouched

#### Scenario: Recovery
- **WHEN** a tab showed a missing-helper error, the helper is then installed, and the user opens another local tab
- **THEN** the new tab runs in the server
