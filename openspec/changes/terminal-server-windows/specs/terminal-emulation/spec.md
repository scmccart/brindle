## REMOVED Requirements

### Requirement: Local Linux terminals run in the terminal server
**Reason**: Its text is phase 1's (the `kill` policy, quitting and crashes ending every program), which phase 2 replaced in `tabs` without updating it here, and it covers Linux only.
**Migration**: See "Local terminals run in the terminal server", which covers Linux and Windows with phase 2's close rule.

## ADDED Requirements

### Requirement: Local terminals run in the terminal server
On Linux and Windows, each local tab and zone SHALL run its program in a session of the terminal server (`terminal-server`), with the close rule of `tabs`: closing it ends the program, quitting or a crash leaves it detached, and `-e` command terminals end with Brindle. Brindle SHALL start the server when none is running. Every other requirement here, and titles, directories, shell integration, status and notifications, SHALL behave as with a PTY owned by Brindle.

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
- **WHEN** Brindle is killed while a `brindle -e sleep 1000` tab is open
- **THEN** that `sleep` is gone within a few seconds

#### Scenario: Titles and directories
- **WHEN** the user runs `cd /var/log` and then `htop` in a local tab, and splits the zone while `htop` runs
- **THEN** the tab title shows `htop`, and the new zone starts in `/var/log`

#### Scenario: Titles and directories on Windows
- **WHEN** the user runs `cd C:\Windows` in a local PowerShell tab with Brindle's integration script loaded, then `python`, and splits the zone while Python runs
- **THEN** the tab title shows `python`, and the new zone starts in `C:\Windows`

## MODIFIED Requirements

### Requirement: Sessions get the launching Brindle's environment
A local tab's program SHALL see the environment of the Brindle process that opened it, with the defaults and profile entries of "Program launch environment" applied, never the environment of the Brindle that started the server.

#### Scenario: Second Brindle with a different environment
- **WHEN** a Brindle started with `FOO=1` is running, and a second Brindle started with `FOO=2` opens a tab
- **THEN** `echo $FOO` in the second Brindle's tab prints `2`

#### Scenario: Case-insensitive override through the server
- **WHEN** on Windows a profile sets `env = { path = "C:\\tools" }` and the server was started by another Brindle
- **THEN** programs in that profile's tabs see a single `Path` variable whose value is `C:\tools`

### Requirement: An unusable terminal server is reported in the tab
Local tabs SHALL always run in the terminal server; Brindle SHALL NOT run their programs on PTYs or pseudoconsoles of its own. When Brindle can't find the `brindle-remote` binary, can't start or reach the server, or shares no protocol version with the running server, each local tab it opens SHALL show an error naming the reason, as for a program that can't start. Brindle SHALL try the server again for the next tab.

#### Scenario: Missing helper
- **WHEN** no `brindle-remote` is found beside Brindle, `BRINDLE_SERVER_HELPER` is unset (and on Linux `BRINDLE_REMOTE_AGENT` too), and the user opens a local tab
- **THEN** the tab shows an error saying the terminal server helper was not found, with how to point Brindle at it

#### Scenario: No common protocol version
- **WHEN** the running server speaks none of Brindle's protocol versions and refuses the upgrade
- **THEN** the tab shows an error naming both versions, and the running server and its sessions are untouched

#### Scenario: Recovery
- **WHEN** a tab showed a missing-helper error, the helper is then installed, and the user opens another local tab
- **THEN** the new tab runs in the server

### Requirement: Pseudoconsole host on Windows
On Windows, each local tab's program SHALL run on the ConPTY build that ships with Brindle (`conpty.dll` and `OpenConsole.exe`), hosted by the terminal server. Only that exact build SHALL be loaded, never a different `conpty.dll` found beside an executable, on `PATH` or in the current directory. When it can't be, the built-in ConPTY SHALL be used, and the server's log SHALL say what was missing. `BRINDLE_CONPTY=inbox` in Brindle's environment SHALL force the built-in ConPTY for the tabs that Brindle opens, for tests.

#### Scenario: Shipped host
- **WHEN** a tab opens in Brindle run from the release zip or installed with WinGet
- **THEN** the tab's console host process is the shipped `OpenConsole.exe`, not `conhost.exe`

#### Scenario: Planted DLL is ignored
- **WHEN** a directory on `PATH`, the directory Brindle was started from, or the directory of a `cargo install`ed `brindle.exe` or `brindle-remote.exe` contains a `conpty.dll` that differs from the shipped one
- **THEN** it isn't loaded

#### Scenario: Fallback
- **WHEN** the shipped ConPTY can't be loaded, or `BRINDLE_CONPTY=inbox` is set
- **THEN** tabs still open through the built-in ConPTY, and a warning is logged once

### Requirement: Closing a tab never blocks on Windows
Closing a Windows tab, or its program exiting, SHALL release its pseudoconsole without blocking Brindle's window or the terminal server's other sessions. That holds even while the program is still writing output.

#### Scenario: Close during output
- **WHEN** a Windows tab is running a command that prints continuously and the user closes the tab
- **THEN** the tab closes at once and the window stays responsive

#### Scenario: Other sessions keep streaming
- **WHEN** two Windows tabs print continuously and the user closes one of them
- **THEN** the other tab keeps receiving output without a pause
