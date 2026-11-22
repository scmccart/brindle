# Spec Delta

## ADDED Requirements

### Requirement: One-command install on Linux
A release's `install.sh`, piped to `sh` or `bash`, SHALL install that release on Linux x86-64 as the invoking user: download the Linux archive, check it against the release's `SHA256SUMS`, install `brindle` and `brindle-remote-linux-x86_64` in `$PREFIX/bin` (default `~/.local`), and run `brindle --install-desktop --prefix "$PREFIX"`. It SHALL use `sudo` only to write an unwritable `$PREFIX`, and SHALL warn when `$PREFIX/bin` isn't on `PATH`.

#### Scenario: Install the latest release
- **WHEN** a user runs `curl -fsSL https://github.com/scmccart/brindle/releases/latest/download/install.sh | sh`
- **THEN** `~/.local/bin/brindle --version` prints that release's version, the helper is beside it, and the launcher entry runs it

#### Scenario: Pick a version
- **WHEN** a user pipes the script to `env BRINDLE_VERSION=0.4.0 sh`
- **THEN** release 0.4.0 is installed, whichever release the script came from

#### Scenario: Re-run to update
- **WHEN** a user runs a newer release's script over an install while Brindle is running
- **THEN** the files are replaced, and the running Brindle keeps its old build until it quits

### Requirement: One-command install on Windows
A release's `install.ps1`, run with `irm … | iex` in PowerShell 5.1 or 7, SHALL install that release for the current user: download the Windows zip, check it against `SHA256SUMS`, install its executables and Linux helper in `%LOCALAPPDATA%\Programs\brindle`, add that directory to the user `PATH`, and run `brindle --install-desktop`. It SHALL replace a running install's files as `scripts/install.ps1` does, and SHALL NOT need elevation.

#### Scenario: Install the latest release
- **WHEN** a user runs `irm https://github.com/scmccart/brindle/releases/latest/download/install.ps1 | iex`
- **THEN** `brindle --version` in a new terminal prints that release's version, and the Start Menu has a Brindle shortcut

#### Scenario: Update while sessions run
- **WHEN** Brindle is running with local sessions, and the user runs a newer release's script
- **THEN** the install succeeds, and the next launch reattaches those sessions through the new version's server

### Requirement: One-command install failures
Either script SHALL stop before installing anything, saying why, when the platform or architecture is unsupported, a download fails, or a checksum doesn't match. A failure SHALL leave an existing install unchanged. Run through `iex`, the Windows script SHALL report a failure as an error without closing the user's PowerShell. A script cut off mid-download SHALL run nothing.

#### Scenario: Checksum mismatch
- **WHEN** the downloaded archive doesn't match its `SHA256SUMS` line
- **THEN** the script prints that the checksum failed, exits non-zero (or throws, under `iex`), and installs nothing

#### Scenario: Unsupported machine
- **WHEN** `install.sh` runs on an `aarch64` machine or on macOS
- **THEN** it says that only Linux x86-64 has a release archive, and installs nothing

#### Scenario: Truncated download
- **WHEN** the connection drops while `sh` is reading the script
- **THEN** no command of the script runs

### Requirement: One-command uninstall
Each script SHALL take an uninstall option (`--uninstall` through `sh -s -- --uninstall`, `$env:BRINDLE_UNINSTALL = 1` or `-Uninstall` on Windows) that removes what it installed, as `scripts/uninstall.sh` and `scripts/uninstall.ps1` do, and keeps the user's config and state.

#### Scenario: Uninstall on Linux
- **WHEN** a user runs `curl -fsSL …/install.sh | sh -s -- --uninstall`
- **THEN** `brindle`, the helper, the launcher entry and the icon are removed, and `~/.config/brindle/` remains

#### Scenario: Uninstall on Windows
- **WHEN** a user runs `$env:BRINDLE_UNINSTALL = 1; irm …/install.ps1 | iex`
- **THEN** the executables, the helper, the shortcut and the `PATH` entry are removed, and `%APPDATA%\brindle\config.toml` remains

## MODIFIED Requirements

### Requirement: Uninstall from source
On Linux, `scripts/uninstall.sh` SHALL remove the desktop entry and icon for `$PREFIX`, then `$PREFIX/bin/brindle` and the helper beside it (`brindle-remote` or `brindle-remote-linux-x86_64`). It SHALL work even when the binary is already gone, and it SHALL leave user configuration in place.

#### Scenario: Uninstall after script install
- **WHEN** a user runs `scripts/uninstall.sh` after `scripts/install.sh`
- **THEN** the binary, entry and icon are removed, and `~/.config/brindle/` remains

#### Scenario: Uninstall after a one-command install
- **WHEN** a user runs `scripts/uninstall.sh` after a release's `install.sh`
- **THEN** `brindle`, `brindle-remote-linux-x86_64`, the entry and the icon are removed
