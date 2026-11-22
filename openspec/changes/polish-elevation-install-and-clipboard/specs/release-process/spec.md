# Spec Delta

## ADDED Requirements

### Requirement: Installer assets
Each release SHALL also carry `install.sh` and `install.ps1`, the one-command installers the desktop-integration capability describes, each installing that release's version unless told another, and `SHA256SUMS` SHALL list them. The latest release's installers SHALL be served at `https://github.com/scmccart/brindle/releases/latest/download/install.sh` and `…/install.ps1`.

#### Scenario: Installers name their release
- **WHEN** a user downloads release 0.5.0's `install.sh` and runs it without `BRINDLE_VERSION`
- **THEN** it installs 0.5.0, even after a newer release has been published

#### Scenario: Installers at the latest URL
- **WHEN** a release is published
- **THEN** `https://github.com/scmccart/brindle/releases/latest/download/install.sh` and `…/install.ps1` serve that release's installers

#### Scenario: Installers checksummed
- **WHEN** a user downloads a release's `install.sh` and `SHA256SUMS` and runs `sha256sum -c --ignore-missing SHA256SUMS`
- **THEN** `install.sh` checks out

### Requirement: Archives work out of the box
Each release archive SHALL, unpacked alone with no other download, build, Docker image or environment variable, run local tabs, SSH profiles and container profiles. The Windows zip SHALL carry `brindle-remote.exe` with the pinned ConPTY embedded and the static Linux helper. The Linux archive SHALL carry the static Linux helper, which serves both local tabs and remote hosts.

#### Scenario: Windows zip, local tab
- **WHEN** a user unpacks the Windows zip on a Windows 11 machine with no Brindle installed before and runs `brindle.exe`
- **THEN** a local PowerShell tab opens through the packaged ConPTY

#### Scenario: Windows zip, SSH and container
- **WHEN** that user opens an SSH profile to a Linux host, and a container profile for a running Linux container
- **THEN** both tabs open, using the Linux helper from the zip

#### Scenario: Linux archive, local and remote
- **WHEN** a user unpacks the Linux archive and runs `brindle`, then opens an SSH profile and a container profile
- **THEN** the local tab and both remote tabs open, using the helper from the archive

#### Scenario: One-command install keeps it
- **WHEN** a user installs with `install.sh` or `install.ps1`
- **THEN** the same local and remote tabs open from the installed copy
