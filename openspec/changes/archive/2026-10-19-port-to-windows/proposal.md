# Proposal

## Why

Brindle only builds and runs on Linux. GPUI 0.2.2 already renders with Direct3D 11 on Windows, and alacritty_terminal 0.26 has a ConPTY backend. That backend, though, uses only the original 2018 pseudoconsole API. Windows' built-in ConPTY re-renders program output instead of passing it through. In a spike on Windows 11 build 26200 it dropped mouse-mode requests, emptied synchronized updates, answered OSC 11 itself, and moved OSC 0/7/9/52 ahead of the text they belong to. Microsoft's current ConPTY (the one Windows Terminal ships, MIT-licensed on NuGet) passed every test sequence through byte for byte. Windows 11 users also expect to install with WinGet, a Start Menu entry, toasts and a native-feeling title bar. Brindle itself has Unix-only code in about a dozen places (PTY spawn, `/proc`, `flock`, D-Bus, Wayland, `xdg-open`, `/bin/sh`), and a Windows `cargo check` stops on all of them.

## What Changes

- **Build and run on Windows 11 (`x86_64-pc-windows-msvc`).**
  - Linux-only crates (zbus, wayland-*, x11rb) and `libc` move behind `cfg`.
  - Unix-only code paths get Windows implementations next to the macOS port's seams.
  - The executable uses the GUI subsystem, so launching it opens no console window, and the child processes it starts don't flash console windows either. Informational options still print to the console they were run from.
- **Brindle hosts its own pseudoconsole on Windows.**
  - A new Windows PTY backend creates the pseudoconsole itself. alacritty's event loop drives it through alacritty's public PTY traits.
  - It loads Microsoft's packaged ConPTY (`conpty.dll` with `OpenConsole.exe`, pinned version) from Brindle's own install, never from `PATH`. If that isn't present, it falls back to the inbox ConPTY, logs a warning and accepts the documented loss of fidelity.
  - It requests wcwidth glyph widths, so ConPTY's idea of character width matches Brindle's grid.
  - It releases the pseudoconsole before closing it, so closing a tab never deadlocks or blocks the UI.
  - It parents the console window to Brindle's window.
  - With the packaged host, mouse reporting, synchronized updates, the alternate screen, OSC 8/52/133/9/777 and color queries reach Brindle exactly as the program wrote them.
- **Working directory on Windows comes from the shell.**
  - Brindle recognizes OSC 7 (`file://` URIs) and ConEmu's OSC 9;9.
  - On Windows, a new tab and a restored tab start in the directory the active tab last reported, and in the home directory when it reported none.
  - Linux keeps reading `/proc`.
- **PowerShell shell integration:** `brindle --shell-integration pwsh` prints a script for PowerShell 7 and Windows PowerShell 5.1. It emits OSC 133 marks and OSC 7.
- **Tab titles on Windows.** ConPTY has no foreground process group, so the process-name step uses the newest descendant of the tab's program. It is found with one Toolhelp snapshot per title tick, shared by all tabs.
- **Title bar on Windows.** Brindle draws minimize, maximize and close buttons and marks them, and the empty strip, as caption areas. Windows then provides drag, double-click maximize, the system menu, Snap and Snap Layouts.
- **Windows toasts.** OSC 9/777 and bell notifications become toast notifications under Brindle's AppUserModelID. Clicking one activates its tab and window. The focus rule, rate limits and the `BRINDLE_NOTIFY_LOG` test sink are unchanged.
- **Other Windows behaviour:**
  - **Primary selection:** an in-app buffer shared by Brindle's windows.
  - **Links:** opened with the shell's default handler. `BRINDLE_LINK_OPENER` still overrides it.
  - **Default program:** `pwsh.exe`, then `powershell.exe`, then `%COMSPEC%`, then `cmd.exe`.
  - **Config editor:** `%VISUAL%`, then `%EDITOR%`, then Notepad.
  - **Fonts:** Cascadia Mono, Consolas and Segoe UI fallbacks.
  - **Paths:** config in `%APPDATA%\brindle`, session state in `%LOCALAPPDATA%\brindle`.
  - **Single owner of the session state:** a file lock instead of `flock`.
- **tmux through WSL.**
  - A control-mode profile with `command = "wsl"` runs `wsl [wsl options] --exec tmux -u [tmux options] -C …`.
  - **This fixes the old plan:** the `-u` that the UTF-8 change put first would have reached `wsl.exe` as `--user`.
  - No Windows directory is passed as `-c`.
- **Install experience:**
  - Icon and version resources in `brindle.exe`.
  - No installer. `brindle.exe` embeds the packaged ConPTY and runs from any directory, so a release is a zip: `brindle.exe` and the licence texts, built and uploaded by CI.
  - A WinGet manifest for that zip, installing `brindle.exe` as a portable `brindle` command. WinGet gives install, upgrade, uninstall, an Installed apps entry and the command on `PATH`, and leaves `%APPDATA%\brindle` alone.
  - `--install-desktop` / `--uninstall-desktop` create and remove a per-user Start Menu shortcut, which WinGet's portable installs don't create. `--prefix` is rejected on Windows.
- **Verification.**
  - Windows builds, unit tests and Windows e2e cases run on this Windows machine and in a Windows CI job.
  - Linux regressions are checked on the Linux machine (via its Claude session) and by the existing Linux CI.
  - Interactive checks are listed as manual steps.
- **Supported Windows:** Windows 11 only.

## Capabilities

### New Capabilities

None. Windows behaviour extends existing capabilities.

### Modified Capabilities

- `configuration`: the config location names `%APPDATA%` on Windows.
- `profiles`: the default program on Windows.
- `tabs`: the process-name title step on Windows, directory inheritance from shell reports on Windows, and native caption behaviour on Windows.
- `terminal-emulation`: the pseudoconsole host on Windows and what passes through it, the launch environment wording, and the in-app primary selection for OSC 52 and selection.
- `terminal-rendering`: Direct3D 11 on Windows 11.
- `shell-integration`: the PowerShell script, and recognising OSC 7 and OSC 9;9 working-directory reports.
- `command-line`: `pwsh` for `--shell-integration`, and no console window on Windows.
- `session-restore`: the state directory on Windows.
- `links`: the Windows opener.
- `desktop-notifications`: toast delivery and click-to-tab on Windows.
- `desktop-integration`: the Windows icon resource, the release zip, the WinGet package with its upgrade and uninstall, the per-user shortcut and the `--prefix` rejection.
- `tmux-control-mode`: attaching through WSL.

## Impact

- **Code:**
  - `src/terminal/` gains a Windows PTY backend (`pty_windows.rs`). `ParsingReader` becomes generic over its reader, and process info moves into `terminal/process.rs`, the seam `port-to-macos` also uses.
  - Also changed: `src/terminal/{mod,links,scan}.rs`, `src/terminal/shell_integration.rs` (OSC 7 / 9;9), `src/session.rs`, `src/notify/` (a toast sink beside D-Bus), `src/workspace.rs` (caption controls, `open_config`), `src/config.rs`, `src/settings.rs`, `src/tmux/mod.rs`, `src/desktop.rs` and `src/main.rs`.
- **New files:**
  - `build.rs`;
  - `assets/windows/{brindle.rc,brindle.ico,brindle-256.png}`;
  - `scripts/render-icons.sh`, `scripts/fetch-conpty.ps1`, `scripts/package-windows.ps1` and `scripts/winget-manifest.ps1`;
  - `shell-integration/brindle.ps1`;
  - `packaging/windows/THIRD-PARTY-NOTICES.txt` and the WinGet manifest templates in `packaging/winget/`;
  - Windows e2e cases;
  - Windows sections in README.md and CLAUDE.md.
- **Dependencies:**
  - `windows` 0.61 (Windows-only) and `embed-resource` 3 (Windows-only build dependency). Both are already locked, so `Cargo.lock` gains only the two dependency edges, as checked in a scratch copy.
  - The packaged ConPTY is fetched by a pinned script (NuGet `Microsoft.Windows.Console.ConPTY`, version and sha256 pinned) and embedded at build time. It isn't committed to git.
- **Build prerequisites on Windows:** MSVC and the Windows 11 SDK (`fxc.exe`, `rc.exe`).
- **Out of scope:**
  - Windows 10;
  - ARM64;
  - code signing;
  - an MSI or other installer, MSIX and the Store;
  - publishing GitHub Releases and submitting the manifest to `microsoft/winget-pkgs`, which come with the first release;
  - win32-input-mode key encoding, because GPUI 0.2.2 doesn't expose virtual-key codes, scan codes or key-ups;
  - native (non-WSL) tmux;
  - translating paths between Windows and WSL.
