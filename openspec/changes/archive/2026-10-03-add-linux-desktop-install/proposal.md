# Proposal

## Why

Brindle is ready to dogfood, but it only runs from `./target/release/brindle`. It has no launcher entry, and the taskbar shows a generic icon, because Linux desktops find an app's icon through a `.desktop` file and the icon theme, not through the executable. `cargo install` can't help: it only copies the binary into `~/.cargo/bin` and has no post-install step. Brindle needs a supported way to install itself, using the new icon at `assets/brindle.svg`.

## What Changes

- New `brindle --install-desktop` command. It writes a desktop entry, `brindle.desktop`, and the scalable icon, `brindle.svg`, into the XDG data directory, then exits without opening a window. Both files are embedded in the binary, so this works for `cargo install` users with no source checkout. The entry's `Exec` points at the running binary's absolute path, because `~/.cargo/bin` is often missing from the desktop session's `PATH`.
- New `brindle --uninstall-desktop` command, which removes exactly those two files.
- Both commands take `--prefix <DIR>` to target `<DIR>/share` instead of the user's data directory, for system-wide installs.
- New `scripts/install.sh` and `scripts/uninstall.sh`, for installing from source:
  - `install.sh` builds the release binary, copies it to `$PREFIX/bin` (default `~/.local`), and runs the *installed* binary's `--install-desktop`, so `Exec` points at the installed copy. It uses `sudo` only for those copy and register steps, and only when `$PREFIX` isn't writable.
  - `uninstall.sh` reverses this. User config in `~/.config/brindle` is left alone.
- README gains an Installing section covering both routes.

**Assumption to confirm:** the user asked whether the self-install was needed for `cargo install` rather than picking an option. This proposal keeps both routes, with the script delegating to the binary so the install logic lives in one place. Dropping the scripts would leave only `cargo install` + `--install-desktop`; that is a cut to tasks group 3, not a redesign.

## Capabilities

### New Capabilities

- `desktop-integration`: how Brindle registers with the desktop. This covers which files are installed and where, the contents of the desktop entry, how uninstalling works, and the source install scripts. The macOS and Windows ports can later add their own platform requirements here.

### Modified Capabilities

- `command-line`: adds the `--install-desktop` / `--uninstall-desktop` / `--prefix` options. Like the informational options, they do their work and exit without opening a window.

## Impact

- **Code:**
  - `src/main.rs` parses the new options and dispatches them before `Application::new()`, so no GPUI or display connection is needed.
  - A new `src/desktop.rs` holds the embedded assets, desktop-entry rendering, path resolution, and install/uninstall.
  - `assets/brindle.svg` already exists and is embedded with `include_bytes!`.
- **Scripts and docs:** new `scripts/install.sh` and `scripts/uninstall.sh`, plus a README section.
- **Dependencies:** none new. `dirs` (already a dependency) resolves the XDG data directory.
- **User system:** writes under `~/.local/share` by default. It refreshes the icon cache only if that hicolor directory already has one; see design.md.
