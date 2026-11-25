## MODIFIED Requirements

### Requirement: Local terminals run in the terminal server
On Linux and Windows, each local tab and zone SHALL run its program in a session of the terminal server (`terminal-server`), with the close rule of `tabs`: closing it ends the program, quitting or a crash leaves it detached, and `-e` command terminals (`brindle terminal -e`) end with Brindle. Brindle SHALL start the server when none is running. Every other requirement here, and titles, directories, shell integration, status and notifications, SHALL behave as with a PTY owned by Brindle.

#### Scenario: Tab program runs under the server
- **WHEN** a local Linux tab starts the user's shell
- **THEN** the shell's parent process is the terminal server, not Brindle, and `brindle-remote list` lists its session as attached

#### Scenario: Windows tab program runs under the server
- **WHEN** a local Windows tab starts PowerShell
- **THEN** PowerShell's parent process is a terminal server holder, not Brindle, and `brindle-remote list` lists its session as attached

#### Scenario: Closing a tab ends its program
- **WHEN** the user closes a local tab that is running `sleep 1000`
- **THEN** `sleep` is gone and the session is no longer listed

#### Scenario: Quitting keeps every program
- **WHEN** the user quits Brindle with local tabs open
- **THEN** their programs keep running and their sessions are listed as detached

#### Scenario: Command tab ends with Brindle
- **WHEN** Brindle is killed while a `brindle terminal -e sleep 1000` tab is open
- **THEN** that `sleep` is gone within a few seconds

#### Scenario: Titles and directories
- **WHEN** the user runs `cd /var/log` and then `htop` in a local tab, and splits the zone while `htop` runs
- **THEN** the tab title shows `htop`, and the new zone starts in `/var/log`

#### Scenario: Titles and directories on Windows
- **WHEN** the user runs `cd C:\Windows` in a local PowerShell tab with Brindle's integration script loaded, then `python`, and splits the zone while Python runs
- **THEN** the tab title shows `python`, and the new zone starts in `C:\Windows`
