# Tasks

## 1. Desktop module

- [x] 1.1 Create `src/desktop.rs` with the embedded icon (`include_bytes!("../assets/brindle.svg")`) and a pure `quote_exec(&Path) -> String` (design.md §4). Unit-test it with a plain path, a path with a space, a path containing `"`, `$` and `\`, and a path with `%`. Verify with `cargo test desktop`.
- [x] 1.2 Add a pure `desktop_entry(exec: &str) -> String` that renders the entry in design.md §4. Unit-test that it contains `Icon=brindle`, `StartupWMClass=brindle`, `StartupNotify=false` and the given `Exec`. Write one rendered entry to a scratch file and verify that `desktop-file-validate` reports no errors.
- [x] 1.3 Add path resolution, `data_dir(prefix: Option<&Path>) -> PathBuf`, plus the entry and icon paths derived from it (design.md §3). Unit-test the prefix and no-prefix cases. Verify with `cargo test desktop`.
- [x] 1.4 Implement `install(prefix)` and `uninstall(prefix)`:
  - atomic write (temp file plus rename) and `create_dir_all`;
  - print each path written or removed;
  - treat already-missing files as success on uninstall;
  - return errors that name the failing path.

  Unit-test both against a temp directory passed as the prefix: install, reinstall, uninstall, uninstall again. Verify with `cargo test desktop`.
- [x] 1.5 Add the conservative cache refresh (design.md §5): look up the tools on `PATH`, rebuild the icon cache only if `icon-theme.cache` already exists, and turn failures into warnings. Unit-test that installing into a temp prefix with no cache doesn't create `icon-theme.cache`. Verify with `cargo test desktop`.

## 2. Command-line options

- [x] 2.1 Parse `--install-desktop`, `--uninstall-desktop` and `--prefix <DIR>` in `src/main.rs` into `Args`. Factor the validation into a pure function, so that `--prefix` without a command, or with both commands, is a usage error. Add the options to `HELP`. Unit-test the valid combinations and the error cases. Verify with `cargo test`.
- [x] 2.2 In `main`, dispatch the desktop command before `Application::new()`: exit 0 on success, print the error and exit 1 on failure, and exit 2 on a usage error. Check with no display at all: `env -u WAYLAND_DISPLAY -u DISPLAY XDG_DATA_HOME=<scratch> ./target/debug/brindle --install-desktop` exits 0. Then confirm that `<scratch>/applications/brindle.desktop` passes `desktop-file-validate`, and that its `Exec` is the debug binary's absolute path.
- [x] 2.3 Check the error paths. `--install-desktop --prefix /proc/brindle` must exit non-zero and name the path. A bare `--prefix /tmp` must print usage and exit 2. Then `--uninstall-desktop` on the scratch dir from 2.2 must leave both files gone, and running it again must still exit 0.
- [x] 2.4 Update `CLAUDE.md`'s architecture notes with a short bullet for `desktop.rs`: it embeds the entry and icon, and its options run before GPUI starts. Verify the note matches the code.

## 3. Source install scripts and docs

- [x] 3.1 Write `scripts/install.sh` (POSIX `sh`, executable) per design.md §7: repo-root `cd`, refuse to run as root, build as the user, `sudo` only when the target isn't writable, run the installed binary's `--install-desktop --prefix`, and a `PATH` hint. Verify with `shellcheck` if available, and with `PREFIX=<scratch> scripts/install.sh`: confirm `<scratch>/bin/brindle` exists and the entry's `Exec` points to it.
- [x] 3.2 Write `scripts/uninstall.sh` per design.md §7, including the fallback for when the binary is gone. Verify with `PREFIX=<scratch> scripts/uninstall.sh` after 3.1: the binary, entry and icon are removed. Then rerun it with the binary already deleted and confirm it exits 0.
- [x] 3.3 Add an "Installing" section to `README.md`, after Building, covering:
  - `scripts/install.sh`, including `PREFIX=/usr/local`;
  - `cargo install --path .` followed by `brindle --install-desktop`;
  - re-running `--install-desktop` after moving or upgrading the binary;
  - `scripts/uninstall.sh` and `brindle --uninstall-desktop`;
  - the fact that config in `~/.config/brindle` is kept.

  Add the new options to the Usage block. Verify that each documented command matches the script and option names.

## 4. Integration on the real desktop

These steps write to the user's real `~/.local` and change their launcher. Warn the user and wait for their go-ahead. The visual checks are theirs to do.

- [x] 4.1 With the user's go-ahead, run `scripts/install.sh` with the default prefix. Confirm that `~/.local/bin/brindle`, `~/.local/share/applications/brindle.desktop` and `~/.local/share/icons/hicolor/scalable/apps/brindle.svg` exist, and that no `icon-theme.cache` was created. Then ask the user to check that Brindle shows in the launcher with its icon, and that a launched window shows the icon in the taskbar or dock on Wayland (and on Xwayland/X11 if they use it).
- [x] 4.2 Run `cargo test`, `cargo build --release` and `openspec validate add-linux-desktop-install --strict`.
