# Proposal

## Why

Brindle only builds and runs on Linux. GPUI 0.2.2 and `alacritty_terminal` 0.26 both support macOS: GPUI has a Metal renderer and a Cocoa platform layer, alacritty has a macOS PTY, and `Cargo.lock` already contains their macOS dependencies (`cocoa`, `metal`, `core-text`). What stops Brindle running there is a handful of Linux assumptions in Brindle itself, plus the lack of any way to install it as a Mac app. We want to dogfood Brindle on macOS too, so it needs both a port and an install experience that feels native: a `Brindle.app` in Applications with the Brindle icon, opened from the Dock or Spotlight.

The Linux-only spots found in the code:
- **Process info:** tab titles and working-directory inheritance read `/proc/<pid>/comm` and `/proc/<pid>/cwd` (`src/terminal/mod.rs`). macOS has `tcgetpgrp` but no `/proc`.
- **Title bar:** the tab strip is drawn as a client-side title bar. On macOS GPUI always reports server decorations (`Decorations::Server`) and `start_window_move` does nothing, so today the strip would lose its drag and double-click handling. The strip's left end would also sit under the window's traffic-light buttons.
- **Keybindings:** the defaults are ctrl-shift-* (GNOME Terminal conventions). Mac users expect cmd-t, cmd-c, cmd-v and so on, and cmd never reaches the shell, so the ctrl-shift workaround isn't needed there.
- **Config location:** `Config::path` uses `dirs::config_dir()`, which is `~/Library/Application Support` on macOS. The configuration spec says `$XDG_CONFIG_HOME/brindle/config.toml`.
- **Shell and environment:** Brindle always passes an explicit shell to alacritty (`$SHELL`, then `/bin/sh`), which skips alacritty's macOS login-shell path. Apps opened from Finder or the Dock also get launchd's minimal `PATH`, so `tmux` from Homebrew wouldn't be found.
- **Fonts:** the default family list and glyph fallbacks name Linux fonts (DejaVu, Noto).
- **Option key:** on macOS the Option key composes characters (option-2 is `@` on a German layout), so treating Alt as Meta unconditionally would break typing for many layouts.

## What Changes

- **Build and run on macOS** (Apple Silicon and Intel, macOS 10.15.7 and later, the minimum GPUI's shader build targets). The Metal renderer is used with GPUI's default precompiled shaders, so building needs Xcode's `metal` compiler. No GPUI features change and `Cargo.lock` is not regenerated.
- **Process info on macOS** through `tcgetpgrp` plus libproc (`proc_name`, `proc_pidinfo` with `PROC_PIDVNODEPATHINFO`), behind a small per-platform module, so tab titles and working-directory inheritance behave as on Linux.
- **Native title bar on macOS:** a transparent title bar with the traffic lights inset into the tab strip, the tabs starting to their right, and double-click handled by the system's "double-click a window's title bar" preference. No custom minimize, maximize or close buttons are drawn.
- **Mac keybindings:** on macOS the default app shortcuts use cmd in place of ctrl-shift (cmd-t, cmd-w, cmd-c, cmd-v, cmd-k, cmd-, …), cmd-1…9 jumps to a tab, and a standard menu bar (Brindle, Shell, Edit, View, Window) exposes the common actions.
- **Option as Meta is opt-in on macOS:** new config key `macos_option_as_meta` (default `false`). When off, Option produces the layout's character; when on, Option acts as Meta as on Linux.
- **Config at `~/.config/brindle/config.toml`** on macOS (or `$XDG_CONFIG_HOME/brindle/`), matching Linux and other terminals, so one dotfile works on both.
- **Login shell and PATH:** with no command configured, macOS tabs start the user's login shell the way Terminal.app does. When Brindle starts outside a terminal, it imports `PATH` from the user's login shell, so profile commands and tmux resolve as they do in a shell.
- **Mac font defaults:** Menlo and SF Mono join the default family list, and the glyph fallbacks gain Apple Color Emoji, Apple Symbols and PingFang.
- **Packaging:**
  - a `Brindle.app` bundle (`Info.plist`, `brindle.icns` rendered from `assets/brindle.svg`), assembled by `scripts/macos/bundle.sh`;
  - a drag-to-Applications `.dmg` built by `scripts/macos/dmg.sh`, from CI on tagged releases;
  - for `cargo install` users, `brindle --install-desktop` and `--uninstall-desktop`, the cross-platform options from `add-linux-desktop-install`. On macOS they wrap the running binary in `~/Applications/Brindle.app`, or in `<prefix>/Applications/Brindle.app` with `--prefix`.
- **CI:** a GitHub Actions job on a macOS runner builds, runs `cargo test`, and produces the `.app` and `.dmg` as artifacts.

## Capabilities

### New Capabilities
- `macos-platform`: how Brindle behaves on macOS — Metal rendering, process info, the native title bar, cmd keybindings and menu bar, the Option key, config location, login shell and PATH import, and font defaults.

### Modified Capabilities
- `desktop-integration`: adds the macOS requirements — the `.app` bundle and its icon, the `.dmg`, and what `--install-desktop` / `--uninstall-desktop` do on macOS. `add-linux-desktop-install` introduces this capability and reserves it for the platform ports, so this change must be archived after that one. The `command-line` requirement for those options, which that change also adds, already covers exit codes and running without a window, so `command-line` needs no delta here.

## Impact

- **Code:**
  - `src/terminal/mod.rs`: `foreground_process_name` and `working_directory` move into a `src/terminal/process.rs` with Linux and macOS implementations. `spawn_pty` passes no shell to alacritty on macOS when the profile sets no command.
  - `src/workspace.rs`: macOS branch of the tab strip (traffic-light inset, `titlebar_double_click`).
  - `src/main.rs`: window options on macOS (`traffic_light_position`), menu bar, login-shell `PATH` import. On macOS `--install-desktop` / `--uninstall-desktop` dispatch to the bundle code instead of the Linux desktop-entry code.
  - `src/actions.rs`: macOS default bindings and the menu definition.
  - `src/terminal_view.rs` / `src/terminal/keys.rs`: Option handling.
  - `src/config.rs`, `src/settings.rs`, `assets/default-config.toml`: config path, `macos_option_as_meta`, font defaults.
  - New `src/macos_app.rs` (bundle layout, `Info.plist` template, `.icns` writer) shared by `--install-desktop` and `scripts/macos/bundle.sh`.
  - The existing Linux-only items are gated with `cfg(target_os = …)`; Linux behaviour is unchanged.
- **Dependencies:** `resvg` 0.45 (already in `Cargo.lock` through GPUI, so no lockfile churn) to render the icon. No other new crates. macOS libproc calls come from `libc`, which already has them at the locked version (0.2.177).
- **Build requirements on macOS:** Xcode, or the Command Line Tools plus the Metal toolchain, for `xcrun metal` (GPUI's `build.rs` compiles `shaders.metal`). `hdiutil` and `codesign` ship with macOS.
- **Verification:** none of this can run on the Linux dev machine. Checks run in the macOS CI job and on real Mac hardware, as listed in tasks.md.
- **Open costs:** signing with a Developer ID and notarization need an Apple Developer account (see design.md, Open Questions). Until then, the `.dmg` is ad-hoc signed and first launch needs right-click → Open.
