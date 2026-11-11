## MODIFIED Requirements

### Requirement: Quitting keeps programs running
Quitting Brindle (`Quit`, ctrl-shift-q, closing the window) SHALL leave every open local terminal's program running detached, on Linux and on Windows, so the next launch brings it back (see `session-restore`), except `-e` command terminals, which end. Brindle SHALL NOT ask. When Brindle ends without quitting (a crash, a signal, the desktop stopping it), the same terminals SHALL keep running.

#### Scenario: Quit
- **WHEN** the window holds three local tabs and the user presses ctrl-shift-q
- **THEN** Brindle exits without asking, and the three sessions are listed as detached with their pids unchanged

#### Scenario: Crash
- **WHEN** Brindle is killed with SIGKILL while a tab runs `sleep 1000`
- **THEN** the `sleep` keeps running, detached

#### Scenario: Crash on Windows
- **WHEN** Brindle is ended with `taskkill /F` while a tab runs `ping -t localhost`
- **THEN** the `ping` keeps running, detached

#### Scenario: Command tab
- **WHEN** a tab runs `brindle -e sleep 1000`'s command and the user quits
- **THEN** that `sleep` is gone within a few seconds and no session is listed for it
