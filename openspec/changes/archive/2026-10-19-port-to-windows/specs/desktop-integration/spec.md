# Spec Delta

## ADDED Requirements

### Requirement: Icon in the Windows executable
The Windows `brindle.exe` SHALL contain Brindle's icon as icon resource 1, in sizes from 16 to 256 pixels, along with version information that matches the crate version. Explorer, the window's title bar, the taskbar and Alt-Tab SHALL show this icon.

#### Scenario: Taskbar icon on Windows
- **WHEN** `brindle.exe` is launched on Windows
- **THEN** the taskbar button and Alt-Tab show Brindle's icon, not a generic one

#### Scenario: Explorer shows icon and version
- **WHEN** the user views `brindle.exe` in Explorer and opens its properties
- **THEN** the file shows Brindle's icon, and the Details tab shows the product name Brindle and the crate version

### Requirement: Windows release archive
Brindle SHALL be distributed on Windows as a zip, `brindle-<version>-x86_64-pc-windows-msvc.zip`, holding `brindle.exe` and the licence texts of Brindle and of the ConPTY host. `brindle.exe` SHALL carry the ConPTY host it ships, so it runs from any directory with no other files beside it.

#### Scenario: Run from the zip
- **WHEN** the user unpacks the zip into any directory and starts `brindle.exe`
- **THEN** a window opens, and its tabs run through the ConPTY host Brindle ships

### Requirement: WinGet package
Brindle SHALL provide a WinGet manifest that installs the release zip's `brindle.exe` as a portable command named `brindle`. A WinGet install SHALL put `brindle` on the user's `PATH` and list Brindle in Installed apps.

#### Scenario: Install with WinGet
- **WHEN** the user installs Brindle with WinGet
- **THEN** a new console can run `brindle --version`, and Brindle is listed in Installed apps

### Requirement: Windows taskbar identity
On Windows, Brindle's windows, its Start Menu shortcuts and its toasts SHALL share one application identity. A window opened from a shortcut SHALL group with that shortcut's pinned taskbar button instead of adding a second button.

#### Scenario: Pinned shortcut
- **WHEN** the user pins Brindle to the taskbar from the Start Menu and then starts it from the pin
- **THEN** its window appears under the pinned button, with no second Brindle button

### Requirement: Windows upgrade and uninstall
Upgrading Brindle with WinGet SHALL replace the installed version in place, leaving one entry in Installed apps. Uninstalling with WinGet SHALL remove `brindle.exe` and the `brindle` command. It SHALL leave the user's `%APPDATA%\brindle` configuration and `%LOCALAPPDATA%\brindle` state in place.

#### Scenario: Upgrade
- **WHEN** WinGet upgrades an installed Brindle 0.1.0 to 0.2.0
- **THEN** only 0.2.0 is listed in Installed apps, and `brindle --version` prints 0.2.0

#### Scenario: Uninstall keeps config
- **WHEN** the user uninstalls Brindle with WinGet
- **THEN** the `brindle` command is gone, and `%APPDATA%\brindle\config.toml` remains

### Requirement: Start Menu shortcut on Windows
On Windows, `brindle --install-desktop` SHALL create a per-user Start Menu shortcut `Brindle.lnk` in the user's Start Menu Programs folder, and `brindle --uninstall-desktop` SHALL remove it. Both SHALL exit without opening a window. The shortcut SHALL target the absolute path of the executable that ran the command, and SHALL use that executable's icon and Brindle's application identity. This replaces the desktop entry and icon files that these options install on Linux.

#### Scenario: Shortcut after cargo install
- **WHEN** a user who ran `cargo install` runs `brindle --install-desktop` on Windows
- **THEN** Brindle appears in the Start Menu with its icon, and the shortcut launches `%USERPROFILE%\.cargo\bin\brindle.exe`

#### Scenario: Shortcut after a WinGet install
- **WHEN** a user who installed Brindle with WinGet runs `brindle --install-desktop`, and WinGet later upgrades Brindle
- **THEN** the Start Menu shortcut still launches Brindle, now the upgraded version

#### Scenario: Remove the shortcut
- **WHEN** the user runs `brindle --uninstall-desktop` on Windows, whether or not the shortcut exists
- **THEN** no Brindle shortcut remains in the user's Start Menu, and the command exits with status 0

### Requirement: No prefix on Windows
On Windows, `--prefix` SHALL be rejected with a message pointing to WinGet, and an exit status of 2.

#### Scenario: Prefix on Windows
- **WHEN** the user runs `brindle --install-desktop --prefix C:\Tools` on Windows
- **THEN** a message pointing to WinGet is printed, nothing is installed, and the exit status is 2
