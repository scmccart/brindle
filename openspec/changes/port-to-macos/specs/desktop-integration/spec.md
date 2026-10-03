# Spec Delta

## ADDED Requirements

### Requirement: macOS application bundle
On macOS, Brindle SHALL be packaged as `Brindle.app`. This is a standard application bundle containing the `brindle` executable, the Brindle icon rendered from the project's icon artwork, and bundle metadata: the display name "Brindle", the bundle identifier `io.github.scmccart.brindle`, the crate version, and the minimum macOS version. Opening the app SHALL open a window with the default profile.

#### Scenario: Open from Finder
- **WHEN** the user double-clicks `Brindle.app` in `/Applications`
- **THEN** a Brindle window opens, and the Dock shows the Brindle icon with the name "Brindle"

#### Scenario: Found by Spotlight
- **WHEN** `Brindle.app` is in `/Applications` or `~/Applications` and the user searches Spotlight for "Brindle"
- **THEN** Brindle is offered and opens when chosen

### Requirement: macOS disk image
Each release SHALL provide a macOS `.dmg` disk image containing `Brindle.app` and a link to `/Applications`, so the user installs by dragging the app onto the link. The image name SHALL include the version and CPU architecture.

#### Scenario: Install from the disk image
- **WHEN** the user opens `Brindle-0.1.0-arm64.dmg` and drags Brindle onto Applications
- **THEN** `/Applications/Brindle.app` exists and opens a Brindle window

### Requirement: Install the macOS app from the binary
On macOS, `brindle --install-desktop` SHALL create `<prefix>/Applications/Brindle.app` from the running executable, where `<prefix>` is the home directory by default or the `--prefix` directory. The bundle SHALL match the released app in layout, name, identifier and icon, and hold a copy of the executable. Each path written SHALL be printed. Running the command again SHALL replace the bundle, so re-running it after an upgrade updates the app.

#### Scenario: Install after cargo install
- **WHEN** the user runs `cargo install --path .` and then `brindle --install-desktop`
- **THEN** `~/Applications/Brindle.app` exists, Brindle is listed in Launchpad and Spotlight, and opening it starts the newly built version

#### Scenario: Reinstall after upgrade
- **WHEN** `~/Applications/Brindle.app` exists from an earlier `--install-desktop` and the user runs `brindle --install-desktop` from a newer build
- **THEN** the bundle now runs the newer build

#### Scenario: System-wide install
- **WHEN** `brindle --install-desktop --prefix /` runs with write access to `/Applications`
- **THEN** the app is written to `/Applications/Brindle.app`

### Requirement: Never replace a macOS app Brindle did not create
On macOS, `--install-desktop` SHALL replace an existing `Brindle.app` at its target only when that bundle's identifier is Brindle's. Otherwise it SHALL leave the existing app untouched, print an error naming it, and exit with a non-zero status.

#### Scenario: Unrelated app in the way
- **WHEN** `~/Applications/Brindle.app` exists with a different bundle identifier and the user runs `brindle --install-desktop`
- **THEN** the existing app is unchanged, an error names it, and the exit status is non-zero

### Requirement: Uninstall the macOS app
On macOS, `brindle --uninstall-desktop` SHALL remove `<prefix>/Applications/Brindle.app` if its bundle identifier is Brindle's, and print what it removed. If there is no such app, it SHALL say so and succeed. It SHALL NOT remove the user's config.

#### Scenario: Uninstall
- **WHEN** `~/Applications/Brindle.app` was created by `--install-desktop` and the user runs `brindle --uninstall-desktop`
- **THEN** the bundle is gone, `~/.config/brindle/config.toml` is untouched, and the exit status is 0

#### Scenario: Nothing to uninstall
- **WHEN** there is no `~/Applications/Brindle.app` and the user runs `brindle --uninstall-desktop`
- **THEN** Brindle reports that nothing was installed and exits with status 0
