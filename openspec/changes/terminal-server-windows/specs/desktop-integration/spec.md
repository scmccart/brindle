## MODIFIED Requirements

### Requirement: Windows release archive
Brindle SHALL be distributed on Windows as a zip, `brindle-<version>-x86_64-pc-windows-msvc.zip`, holding `brindle.exe`, `brindle-remote.exe`, the Linux remote helper, and the licence texts of Brindle and of the ConPTY host. `brindle-remote.exe` SHALL carry the ConPTY host it ships, so the zip's files run from any directory with nothing else beside them.

#### Scenario: Run from the zip
- **WHEN** the user unpacks the zip into any directory and starts `brindle.exe`
- **THEN** a window opens, and its tabs run in the terminal server through the ConPTY host Brindle ships

#### Scenario: Helper missing from the directory
- **WHEN** the user copies only `brindle.exe` out of the zip into another directory and starts it
- **THEN** a window opens, and its tab shows an error saying the terminal server helper was not found

### Requirement: Windows upgrade and uninstall
Upgrading Brindle with WinGet SHALL replace the installed version in place, leaving one entry in Installed apps, also while local sessions are running. Uninstalling with WinGet SHALL remove `brindle.exe`, `brindle-remote.exe` and the `brindle` command. It SHALL leave the user's `%APPDATA%\brindle` configuration and `%LOCALAPPDATA%\brindle` state in place.

#### Scenario: Upgrade
- **WHEN** WinGet upgrades an installed Brindle 0.1.0 to 0.2.0
- **THEN** only 0.2.0 is listed in Installed apps, and `brindle --version` prints 0.2.0

#### Scenario: Upgrade with sessions running
- **WHEN** Brindle has quit with local sessions left running detached, and WinGet upgrades it
- **THEN** the upgrade succeeds, and the next launch reattaches those sessions through the new version's server

#### Scenario: Uninstall keeps config
- **WHEN** the user uninstalls Brindle with WinGet
- **THEN** the `brindle` command is gone, and `%APPDATA%\brindle\config.toml` remains
