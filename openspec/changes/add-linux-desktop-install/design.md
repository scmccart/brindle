# Design

## Context

See proposal.md for the problem. The relevant facts:

- **Window identity.** `open_window` sets `app_id: Some("brindle")` (`src/main.rs`). GPUI forwards it to `xdg_toplevel.set_app_id` on Wayland. On X11 it becomes `WM_CLASS`, with both instance and class set to `brindle` (`gpui-0.2.2/src/platform/linux/x11/window.rs`, `set_app_id`). So the desktop entry's basename must be `brindle.desktop`, and `StartupWMClass=brindle` matches on both.
- **No window icon API.** GPUI 0.2.2 doesn't set `_NET_WM_ICON` or use `xdg-toplevel-icon-v1`. Icon lookup therefore goes entirely through the desktop entry and the icon theme.
- **No startup notification.** GPUI only *passes* `XDG_ACTIVATION_TOKEN` to processes it spawns (`platform/linux/platform.rs`). It never consumes `DESKTOP_STARTUP_ID` or a launch token.
- **Argument parsing.** `parse_args` in `src/main.rs` is a hand-written loop. Informational options call `process::exit` from inside it, before `Application::new()`.
- **Icon asset.** `assets/brindle.svg` already exists (512×512 viewBox, self-contained, no external refs).
- **Tools on the dev machine.** `desktop-file-validate`, `update-desktop-database` and `gtk-update-icon-cache` are installed. `~/.local/share/icons/hicolor/` exists and has **no** `icon-theme.cache`, which is the common case.

## Goals / Non-Goals

**Goals:**
- One implementation of "register with the desktop", in the binary, shared by `cargo install` users and the source scripts.
- Correct `Exec` regardless of where the binary lives or whether it's on `PATH`.
- Never leave the user's icon lookup worse than before. In particular, never create a stale icon cache.

**Non-Goals:**
- Distro packages (`.deb`, `.rpm`, AUR), Flatpak, AppImage. These are later work; the same entry and icon would be reused.
- Raster icon sizes. Every freedesktop icon-theme implementation in use renders `scalable/apps/*.svg`.
- A `--new-window` desktop action. Each launch already opens a window in a new process.
- An AppStream metainfo file. It only matters for software centres, so it comes with distro packaging.

## Decisions

### 1. Install logic lives in the binary; scripts delegate to it

`src/desktop.rs` embeds the icon with `include_bytes!("../assets/brindle.svg")` and renders the entry from a template, filling in `Exec` at run time. The scripts only build, copy the binary, and call `--install-desktop` on the **installed** copy, so `Exec` resolves to that copy.

*Alternative considered:* scripts that write the files with `install -Dm644` and `sed` the template. Rejected: that duplicates the logic, and `cargo install` users would be left without it.

### 2. Run before GPUI starts

`--install-desktop`, `--uninstall-desktop` and `--prefix` are parsed into an `Args` field. `main` handles them before `Application::new()`, so they work over SSH or in a TTY with no display.

`--prefix` is only meaningful alongside one of the two commands. A bare `--prefix` is a usage error (exit 2), consistent with unknown arguments. Keeping the decision in a small pure function (`args → Result<DesktopCommand, UsageError>`) makes it unit-testable without exiting the process.

### 3. Paths

`data = match prefix { Some(p) => p/share, None => dirs::data_dir() }`. On Linux, `dirs::data_dir()` returns `$XDG_DATA_HOME` or `~/.local/share`. The entry goes to `data/applications/brindle.desktop`, and the icon to `data/icons/hicolor/scalable/apps/brindle.svg`. Parent directories are created with `create_dir_all`.

Each file is written to a temporary sibling, then renamed into place. A desktop environment watching the directory therefore never sees a half-written entry.

### 4. Desktop entry

```ini
[Desktop Entry]
Type=Application
Name=Brindle
GenericName=Terminal
Comment=GPU-accelerated terminal emulator
Exec=<exec>
Icon=brindle
Terminal=false
Categories=System;TerminalEmulator;
Keywords=shell;prompt;command;commandline;cmd;terminal;tmux;
StartupWMClass=brindle
StartupNotify=false
```

- **`Exec`** is `std::env::current_exe()`, canonicalised.
  - If the path contains any character the Desktop Entry spec reserves (space, tab, `"`, `'`, `\`, `>`, `<`, `~`, `|`, `&`, `;`, `$`, `*`, `?`, `#`, `(`, `)`, `` ` ``, newline), it is wrapped in double quotes. Inside the quotes, `"`, `` ` ``, `$` and `\` are backslash-escaped.
  - The whole value then gets the general string escaping, where `\` becomes `\\`.
  - A path containing `%` has it doubled to `%%`, since field codes start with `%`.
  - This is a pure function, `quote_exec(&Path) -> String`, with unit tests.
- **`Icon=brindle`** is a theme name, not a path, so themes can override it and HiDPI lookup works.
- **`StartupNotify=false`**: Brindle doesn't consume startup tokens (see Context). Setting it to `true` would show a busy cursor on X11 until the launcher's timeout.
- **`Terminal=false`**: Brindle *is* the terminal. `Categories` includes `TerminalEmulator`, so desktop "default terminal" pickers can find it.

### 5. Cache refresh is conservative

- **Applications directory:** if `update-desktop-database` is on `PATH`, run `update-desktop-database -q <data>/applications`. This cache only maps MIME types and is safe to create.
- **Icon cache:** if `<data>/icons/hicolor/icon-theme.cache` already exists and `gtk-update-icon-cache` is on `PATH`, run `gtk-update-icon-cache -q -t -f <data>/icons/hicolor`.
  - *Never create the icon cache.* A cache that exists but goes stale hides icons that other apps install later. That is why `gtk-update-icon-cache -t` on a cache-less user directory is worse than doing nothing. GNOME and KDE both find `~/.local/share/icons` entries without a cache, through a directory scan.
- Failures and missing tools are logged as warnings and never change the exit status. Distro installs under `/usr` have their own triggers, so this only matters for `--prefix /usr/local`.

### 6. Uninstall removes exactly what install writes

It removes only the two files for the same `<data>`, then runs the same cache refresh. It doesn't remove directories, because they may be shared, and an already-missing file is not an error. Brindle doesn't track what it installed; the paths are deterministic.

### 7. Scripts

`scripts/install.sh` (POSIX `sh`, `set -eu`):

1. `cd` to the repo root (the script's parent directory), so it works from anywhere.
2. `cargo build --release`, as the invoking user. When run as root, exit with a message to run it as a normal user, because the script calls `sudo` itself where it's needed. Otherwise `sudo ./install.sh` would leave a root-owned `target/` behind and install into root's `~/.local`.
3. `PREFIX=${PREFIX:-$HOME/.local}`. If `$PREFIX` (or its nearest existing parent) isn't writable, prefix the next two steps with `sudo`.
4. Run `install -Dm755 target/release/brindle "$PREFIX/bin/brindle"`, then `"$PREFIX/bin/brindle" --install-desktop --prefix "$PREFIX"`.
5. If `$PREFIX/bin` isn't in `$PATH`, print a hint.

`scripts/uninstall.sh` runs `"$PREFIX/bin/brindle" --uninstall-desktop --prefix "$PREFIX"` if that binary exists. Otherwise it removes the two files directly, the one place their paths are repeated. Then it removes the binary, using the same `sudo` rule.

With the default prefix, `--prefix ~/.local` targets `~/.local/share`. That matches `dirs::data_dir()` unless `XDG_DATA_HOME` is customised, in which case the script's explicit prefix wins. Documented in the README.

## Risks / Trade-offs

- **[Risk] Moving the binary breaks `Exec`.** If `cargo install --root` changes, or the user moves `~/.local/bin/brindle`, the absolute `Exec` goes stale. → Mitigation: re-running `--install-desktop` fixes it, and the README says so. A bare `Exec=brindle` would avoid this but fails for `~/.cargo/bin`, which is the reason for this change.
- **[Risk] Old `--install-desktop` runs leave an outdated entry after an upgrade.** → Mitigation: `install.sh` always re-runs it. For `cargo install` users, the README pairs upgrading with re-running it.
- **[Trade-off] No raster icons.** Some minimal panels (older tint2, some X11 docks) only read PNGs. → Mitigation: acceptable for dogfooding. Rasterising at build time is a follow-up if needed, and the macOS and Windows ports need a rasteriser anyway.
- **[Risk] Writing into the real `~/.local/share` during verification.** → Mitigation: automated checks use `XDG_DATA_HOME=<scratch>` or `--prefix <scratch>`. The real install, plus the launcher and taskbar check, is handed to the user.
