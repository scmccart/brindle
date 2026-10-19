# desktop-integration Specification

## Purpose
Register Brindle with the desktop so it appears in launchers and taskbars with its own name and icon, and remove that registration cleanly. Covers what gets installed and where, for the binary's self-install and for the source install scripts.

## Requirements

### Requirement: Install desktop entry and icon
On Linux, `brindle --install-desktop` SHALL write a desktop entry named `brindle.desktop` to `<data>/applications/`. It SHALL also write Brindle's scalable icon to `<data>/icons/hicolor/scalable/apps/brindle.svg`. Both files come from data built into the binary, so no source checkout is needed. Missing directories SHALL be created, and existing files SHALL be replaced. Each path written SHALL be printed.

#### Scenario: Install after cargo install
- **WHEN** a user who installed with `cargo install` runs `brindle --install-desktop`
- **THEN** `~/.local/share/applications/brindle.desktop` and `~/.local/share/icons/hicolor/scalable/apps/brindle.svg` exist, and Brindle appears in the desktop's application launcher with its icon

#### Scenario: Reinstall
- **WHEN** `--install-desktop` runs a second time
- **THEN** both files are overwritten with current content, and the command succeeds

### Requirement: Install location
On Linux, by default, `<data>` SHALL be the user's XDG data directory: `$XDG_DATA_HOME`, or `~/.local/share` when that is unset. With `--prefix <DIR>`, `<data>` SHALL be `<DIR>/share`.

#### Scenario: Custom XDG data home
- **WHEN** `XDG_DATA_HOME=/tmp/xdg brindle --install-desktop` runs
- **THEN** the files are written under `/tmp/xdg/applications/` and `/tmp/xdg/icons/hicolor/scalable/apps/`

#### Scenario: System-wide prefix
- **WHEN** `brindle --install-desktop --prefix /usr/local` runs with write access
- **THEN** the files are written under `/usr/local/share/applications/` and `/usr/local/share/icons/hicolor/scalable/apps/`

### Requirement: Desktop entry contents
On Linux, the entry SHALL be a valid freedesktop Desktop Entry of `Type=Application` with:
- `Name=Brindle`;
- `Icon=brindle`;
- `Terminal=false`;
- `Categories=System;TerminalEmulator;`;
- `StartupWMClass=brindle`, which matches the window's Wayland app_id and X11 `WM_CLASS`;
- `Exec` set to the absolute path of the binary that ran the command, quoted as the Desktop Entry specification requires.

#### Scenario: Exec follows the installed binary
- **WHEN** `/home/u/.cargo/bin/brindle --install-desktop` runs
- **THEN** the entry's `Exec` runs `/home/u/.cargo/bin/brindle`, so launching works even when `~/.cargo/bin` is not on the desktop session's `PATH`

#### Scenario: Path needing quotes
- **WHEN** the binary's path contains a space
- **THEN** `Exec` quotes the path so that it launches the binary, and `desktop-file-validate` accepts the entry

#### Scenario: Taskbar icon
- **WHEN** Brindle is launched after installing
- **THEN** its windows are matched to the entry, and the taskbar or dock shows Brindle's icon

### Requirement: Icon cache refresh
On Linux, after installing or uninstalling, Brindle SHALL refresh caches without ever making icon lookup worse:
- The hicolor icon cache SHALL be rebuilt only if that hicolor directory already has a cache file.
- The applications directory's cache SHALL be updated when the tool for it is available.
- A missing tool or a failed refresh SHALL produce a warning and SHALL NOT fail the command.

#### Scenario: No existing icon cache
- **WHEN** `~/.local/share/icons/hicolor/` has no icon cache and `--install-desktop` runs
- **THEN** no icon cache is created there

#### Scenario: Missing tools
- **WHEN** neither cache tool is installed
- **THEN** the install still succeeds, with at most a warning

### Requirement: Uninstall desktop entry and icon
On Linux, `brindle --uninstall-desktop` SHALL remove the two files that `--install-desktop` writes for the same `<data>` directory, and only those. It SHALL succeed when they are already absent, and it SHALL NOT remove directories, the binary, or user configuration.

#### Scenario: Uninstall
- **WHEN** the user runs `brindle --uninstall-desktop` after installing
- **THEN** both files are gone, Brindle no longer appears in the launcher, and `~/.config/brindle/` is untouched

#### Scenario: Nothing to remove
- **WHEN** `--uninstall-desktop` runs and the files don't exist
- **THEN** the command exits with status 0

### Requirement: Install from source
On Linux, `scripts/install.sh` SHALL build the release binary as the invoking user. It SHALL then copy the binary to `$PREFIX/bin/brindle`, with `$PREFIX` defaulting to `~/.local`, and run that installed copy with `--install-desktop --prefix "$PREFIX"`. When `$PREFIX` is not writable, only the copy and register steps SHALL run under `sudo`. If `$PREFIX/bin` is not on `PATH`, the script SHALL warn.

#### Scenario: Default install
- **WHEN** a user runs `scripts/install.sh` from a checkout
- **THEN** `~/.local/bin/brindle` exists, the desktop entry's `Exec` points to it, and the build output is owned by the user

#### Scenario: System-wide install
- **WHEN** a user runs `PREFIX=/usr/local scripts/install.sh`
- **THEN** cargo runs without `sudo`, and the binary, entry and icon are installed under `/usr/local`

### Requirement: Uninstall from source
On Linux, `scripts/uninstall.sh` SHALL remove the desktop entry and icon for `$PREFIX`, then `$PREFIX/bin/brindle`. It SHALL work even when the binary is already gone, and it SHALL leave user configuration in place.

#### Scenario: Uninstall after script install
- **WHEN** a user runs `scripts/uninstall.sh` after `scripts/install.sh`
- **THEN** the binary, entry and icon are removed, and `~/.config/brindle/` remains

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
