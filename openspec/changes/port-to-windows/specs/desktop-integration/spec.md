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

### Requirement: Windows installer
Brindle SHALL be distributed on Windows as an MSI that installs per-machine to `%ProgramFiles%\Brindle`, with the ConPTY host it ships beside `brindle.exe`. It SHALL add a Start Menu shortcut named Brindle and an entry in Installed apps. An installer feature, off by default, SHALL add the install directory to the system `PATH`. The installer SHALL show the licences of Brindle and of the ConPTY host.

#### Scenario: Install with the MSI
- **WHEN** the user runs the Brindle MSI and accepts the defaults
- **THEN** `%ProgramFiles%\Brindle\brindle.exe`, `conpty.dll` and `OpenConsole.exe` exist, Brindle appears in the Start Menu with its icon, and Brindle is listed in Installed apps

#### Scenario: Add to PATH
- **WHEN** the user enables the PATH feature during install
- **THEN** a new console can run `brindle --version`

### Requirement: Windows taskbar identity
On Windows, Brindle's windows, its Start Menu shortcuts and its toasts SHALL share one application identity. A window opened from a shortcut SHALL group with that shortcut's pinned taskbar button instead of adding a second button.

#### Scenario: Pinned shortcut
- **WHEN** the user pins Brindle to the taskbar from the Start Menu and then starts it from the pin
- **THEN** its window appears under the pinned button, with no second Brindle button

### Requirement: Windows upgrade and uninstall
Installing a newer Brindle MSI SHALL replace an installed older version in place, leaving one entry in Installed apps. Uninstalling SHALL remove the installed files, the Start Menu shortcut and any `PATH` entry it added. It SHALL leave the user's `%APPDATA%\brindle` configuration and `%LOCALAPPDATA%\brindle` state in place.

#### Scenario: Upgrade
- **WHEN** version 0.2.0's MSI is run where 0.1.0 is installed
- **THEN** only 0.2.0 is listed in Installed apps, and `brindle --version` prints 0.2.0

#### Scenario: Uninstall keeps config
- **WHEN** the user uninstalls Brindle from Installed apps
- **THEN** the program files and Start Menu shortcut are gone, and `%APPDATA%\brindle\config.toml` remains

### Requirement: Start Menu shortcut without the installer
On Windows, `brindle --install-desktop` SHALL create a per-user Start Menu shortcut `Brindle.lnk` in the user's Start Menu Programs folder, and `brindle --uninstall-desktop` SHALL remove it. Both SHALL exit without opening a window. The shortcut SHALL target the absolute path of the executable that ran the command, and SHALL use that executable's icon and Brindle's application identity. This replaces the desktop entry and icon files that these options install on Linux.

#### Scenario: Shortcut after cargo install
- **WHEN** a user who ran `cargo install` runs `brindle --install-desktop` on Windows
- **THEN** Brindle appears in the Start Menu with its icon, and the shortcut launches `%USERPROFILE%\.cargo\bin\brindle.exe`

#### Scenario: Remove the shortcut
- **WHEN** the user runs `brindle --uninstall-desktop` on Windows, whether or not the shortcut exists
- **THEN** no Brindle shortcut remains in the user's Start Menu, and the command exits with status 0

### Requirement: No prefix on Windows
On Windows, `--prefix` SHALL be rejected with a message saying that machine-wide installs use the MSI, and an exit status of 2.

#### Scenario: Prefix on Windows
- **WHEN** the user runs `brindle --install-desktop --prefix C:\Tools` on Windows
- **THEN** a message pointing to the MSI is printed, nothing is installed, and the exit status is 2
