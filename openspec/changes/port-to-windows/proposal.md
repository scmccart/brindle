# Proposal

## Why

Brindle only builds on Linux, but its foundations already run on Windows. GPUI 0.2.2 has a Direct3D 11 renderer and native window handling for Windows 10 and 11, alacritty_terminal 0.26 has a ConPTY backend, and `Cargo.lock` already contains the Windows-only crates (`windows` 0.61, `windows-sys` 0.59, `embed-resource` 3). What stops Brindle is a handful of Unix-only code paths in Brindle itself, plus the lack of any way to install it. Windows users expect an installer, a Start Menu entry, an uninstall entry and an icon in the `.exe`.

## What Changes

- **Build on Windows (`x86_64-pc-windows-msvc`).** Move the Unix-only code behind `cfg`:
  - PTY spawn: today it reads `pty.child().id()` and `pty.file()`. On Windows, the child pid comes from `pty.child_watcher().pid()`, and there is no master fd. The `tty::Options` struct also gains the Windows-only `escape_args` field.
  - Foreground process and working directory: today they use `/proc` and `libc::tcgetpgrp`.
  - Primary selection: GPUI only defines `write_to_primary`/`read_from_primary` on Linux and FreeBSD.
  - `open_config`: today it runs the editor through `/bin/sh -c`.
- **Tab titles on Windows.** ConPTY has no foreground process group. The process-name step of the title order uses the newest descendant of the tab's shell, found with a Toolhelp snapshot, and falls back to the shell's own name.
- **Working directory inheritance is unavailable on Windows.** No supported API reads another process's current directory, and alacritty's parser ignores OSC 7. New tabs start in the profile's directory or the home directory.
- **Title bar on Windows.** GPUI reports server-side decorations on Windows, and `start_window_move`/`show_window_menu` do nothing there. Meanwhile `appears_transparent` hides the native caption. So today a Windows window would have no way to move, minimize or close it with the mouse. Brindle will draw minimize, maximize and close buttons itself and mark them, and the empty strip, with GPUI's `window_control_area`. Windows then provides native drag, double-click-to-maximize, the system menu, Snap, and Snap Layouts on the maximize button.
- **Default shell on Windows:** `pwsh.exe` if it is on `PATH`, otherwise `powershell.exe`, otherwise `%COMSPEC%`, otherwise `cmd.exe`.
- **Config location:** `%APPDATA%\brindle\config.toml`, which is `dirs::config_dir()` and so is what the code already resolves to.
- **Primary selection is emulated inside Brindle on Windows.** Finishing a selection stores it in an app-wide buffer. Middle click and OSC 52 `p` use that buffer.
- **tmux on Windows goes through WSL.** A control-mode profile with `command = "wsl"` and `tmux_args = ["tmux"]` runs `wsl tmux -C new-session …` over pipes, unchanged. Brindle omits the Windows-only `-c <dir>` argument for that command. A missing tmux is already explained by the existing attach-failure tab.
- **Icon in the executable.** A `build.rs` step compiles `assets/windows/brindle.rc`, which holds the icon as resource 1 plus version info, with the already-locked `embed-resource`. GPUI's window class loads icon resource 1 from the `.exe`, so the title bar, taskbar and Alt-Tab all show it without further code. The multi-size `brindle.ico` is generated from `assets/brindle.svg` by a script and committed.
- **Installer: an MSI built with the WiX Toolset CLI from `packaging/windows/brindle.wxs`.** It installs per-machine to `Program Files\Brindle`, adds a Start Menu shortcut and an Apps & Features uninstall entry, and can optionally add Brindle to `PATH`. A stable UpgradeCode makes newer versions upgrade in place. A winget manifest is an optional follow-up once releases are published. design.md compares this with Inno Setup and MSIX.
- **Parity for `cargo install` users.** `brindle --install-desktop` and `--uninstall-desktop`, which the Linux change introduces, create and remove a per-user Start Menu shortcut to the running executable on Windows. `--prefix` is rejected on Windows, because machine-wide installs are the MSI's job.
- **CI.** Add a GitHub Actions `windows-latest` job that runs `cargo build --release` and `cargo test` and builds the MSI. This is the only automated Windows verification. Interactive checks run on a real Windows machine.

## Capabilities

### New Capabilities

None. The Windows install requirements go into `desktop-integration`, which `add-linux-desktop-install` introduces and which its proposal reserves for platform ports. This change should therefore be archived after that one.

### Modified Capabilities

- `configuration`: the config location names the platform's config directory, not only `$XDG_CONFIG_HOME`.
- `profiles`: the default program on Windows is PowerShell or `%COMSPEC%`, not `$SHELL` and `/bin/sh`.
- `tabs`:
  - The tab title's process-name step is defined for Windows.
  - Working directory inheritance is Linux-only.
  - The title bar behaviour covers Windows' native caption hit-testing.
- `terminal-emulation`: Selection and copy and OSC 52 define a primary selection on platforms that have none.
- `terminal-rendering`: GPU rendering names Direct3D 11 on Windows.
- `tmux-control-mode`: a requirement for attaching through WSL on Windows.
- `desktop-integration` (added by `add-linux-desktop-install`): Windows icon resource, MSI installer, and `--install-desktop` / `--uninstall-desktop` on Windows.

## Impact

- **Code:**
  - `src/terminal/mod.rs`: `Backend::Pty` fields are behind `cfg`, and the process info code moves into a per-platform module.
  - `src/workspace.rs`: the title bar gets caption controls on Windows, and `open_config` gets a Windows editor command.
  - `src/config.rs`: the default shell.
  - `src/settings.rs`: Windows font fallbacks, Cascadia Mono, Consolas and Segoe UI.
  - `src/tmux/mod.rs`: the `-c` rule for `wsl`.
  - `src/terminal_view.rs` and `src/terminal/mod.rs`: the primary-selection shim.
  - `src/main.rs` and the Linux change's `src/desktop.rs`: the Windows `--install-desktop` path.
- **New files:**
  - `build.rs`;
  - `assets/windows/brindle.rc` and `assets/windows/brindle.ico`;
  - `scripts/render-icons.sh`;
  - `packaging/windows/brindle.wxs`;
  - `.github/workflows/windows.yml`;
  - a README section on Windows.
- **Dependencies:** each of these crates is already in `Cargo.lock` at the version shown, so the lockfile only gains edges to them and no crate versions change.
  - `windows` 0.61 as a Windows-only dependency, for Toolhelp, the `IShellLinkW` shortcut and `SHGetKnownFolderPath`.
  - `embed-resource` 3 as a Windows-only build dependency.
- **Build prerequisites on Windows:**
  - the MSVC toolchain;
  - the Windows 10/11 SDK, because GPUI's build script needs `fxc.exe` for release builds and `rc.exe` compiles resources;
  - the WiX Toolset CLI, to build the MSI.
- **Not verifiable on this Linux machine:** there is no Windows target or SDK here. Verification is the CI job plus manual steps on Windows hardware.
- **Out of scope:**
  - ARM64 Windows;
  - code signing, which is an open question in design.md;
  - the Microsoft Store and MSIX;
  - OSC 7 working-directory tracking;
  - native (non-WSL) tmux.
