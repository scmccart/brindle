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
