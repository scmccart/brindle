# Spec Delta

## ADDED Requirements

### Requirement: Desktop install options
Brindle SHALL accept `--install-desktop` and `--uninstall-desktop`, with an optional `--prefix <DIR>`, as defined by the `desktop-integration` capability. Like the informational options, they SHALL do their work and exit without opening a window or connecting to a display. They SHALL exit with status 0 on success. A file that cannot be written or removed SHALL produce an error naming the path and a non-zero exit. `--prefix` without one of these options SHALL be rejected as a usage error.

#### Scenario: Headless install
- **WHEN** `brindle --install-desktop` runs over SSH with no display
- **THEN** the files are installed and the process exits 0 without trying to open a window

#### Scenario: Unwritable target
- **WHEN** `brindle --install-desktop --prefix /usr` runs without write access
- **THEN** an error naming the path that could not be written is printed, and the exit status is non-zero

#### Scenario: Stray prefix
- **WHEN** the user runs `brindle --prefix /usr/local`
- **THEN** usage is printed and the exit status is 2
