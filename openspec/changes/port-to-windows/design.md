# Design

## Context

See proposal.md for the motivation. These facts from the dependency sources shape the approach:

- **GPUI 0.2.2 on Windows**:
  - **Renderer:** Direct3D 11, accepting feature level 10.1 or higher. It supports Windows 10 and 11.
  - **Shaders:** `build.rs` compiles the HLSL shaders with `fxc.exe` in release builds. It finds `fxc.exe` through `GPUI_FXC_PATH`, then `PATH`, then the Windows SDK 10.0.26100.0 directory, and panics if none has it.
  - **Manifest:** with the default `windows-manifest` feature, GPUI embeds a manifest for PerMonitorV2 DPI awareness and Common Controls v6. It does this with `embed-resource`. Because the GPUI package has no binaries, `embed-resource` emits `rustc-link-lib` for the compiled resource rather than `rustc-link-arg-bins`.
  - **Icon:** `register_window_class` uses `load_icon()`, which is `LoadImageW(GetModuleHandleW(None), MAKEINTRESOURCE(1), IMAGE_ICON)`. **An icon resource with id 1 in our `.exe` therefore becomes the window and taskbar icon without any API call.**
  - **Decorations:** the Windows window doesn't override `window_decorations()`, so it returns `Decorations::Server`. `start_window_move`, `show_window_menu` and `start_window_resize` are the trait's no-op defaults.
  - **Hidden caption:** when `TitlebarOptions::appears_transparent` is set, which Brindle does, GPUI hides the native caption. Its `WM_NCHITTEST` handler then asks the element tree for a `WindowControlArea` and maps it as follows: `Drag` to `HTCAPTION`, `Min`/`Max`/`Close` to the matching `HT*BUTTON`. It falls back to the native resize borders. On button release, GPUI itself minimizes, maximizes or restores, or posts `WM_CLOSE`.
  - **Primary selection:** `App::write_to_primary` and `App::read_from_primary` exist only with `cfg(any(target_os = "linux", target_os = "freebsd"))`.
  - **AltGr:** GPUI treats right Alt plus left Ctrl as AltGr, not as ctrl-alt, on layouts that use AltGr. So Brindle's `ctrl-alt-1…9` bindings don't swallow AltGr characters.
- **alacritty_terminal 0.26 on Windows**:
  - The PTY is ConPTY, and `Pty::child_watcher().pid()` gives the child's process id.
  - There is no `child()`/`file()` and no notion of a foreground process.
  - `tty::Options` has a Windows-only `escape_args` field.
  - vte 0.15 doesn't handle OSC 7; it logs it as unhandled.
- **Brindle's Unix-only code**:
  - `Backend::Pty { child_pid, master_fd }` is filled from `pty.child().id()` and `AsRawFd`, in `src/terminal/mod.rs` around line 218.
  - `foreground_pid` uses `libc::tcgetpgrp`, and the name and working directory come from `/proc/<pid>/{comm,cwd}`, around lines 511–540.
  - Selection, OSC 52 and middle click call the primary-selection API.
  - `Workspace::open_config` runs `/bin/sh -c "$EDITOR <path>"`.
  - The default program falls back from `$SHELL` to `/bin/sh`.
  - Font fallbacks list only Linux fonts.
- **The lockfile already has the Windows crates**: `windows` 0.57 and 0.61.3, `windows-sys` 0.48 to 0.61, and `embed-resource` 3.0.6. Per CLAUDE.md, `Cargo.lock` must not be regenerated.
- **This machine** has only the `x86_64-unknown-linux-gnu` Rust target. Brindle can't be built, type-checked or run for Windows here.

## Goals / Non-Goals

**Goals:**
- Build with `cargo build --release` on `x86_64-pc-windows-msvc`, and keep the Linux build and tests unchanged.
- Keep platform differences in a small number of `cfg` modules, so the rest of the code stays platform-neutral.
- Install the way Windows apps install: an MSI, the icon in the `.exe`, a Start Menu shortcut and an uninstall entry. Also give `cargo install` users a one-command shortcut.

**Non-Goals:**
- ARM64 Windows, Windows 7/8, and the MinGW (`-gnu`) target as a supported release target.
- Reading another process's working directory through `NtQueryInformationProcess` and the PEB, which is undocumented and fragile.
- OSC 7 directory tracking. It would need Brindle to see PTY output before alacritty parses it, which is a separate change for all platforms.
- Running tmux natively on Windows, or translating paths for WSL.

## Decisions

### 1. Platform code lives in `src/platform/{unix,windows}.rs`

A new `src/platform/mod.rs` re-exports one implementation through `#[cfg(unix)]` or `#[cfg(windows)]`:
- `struct PtyHandle` replaces `child_pid` and `master_fd`. On Unix it holds the pid and fd, and on Windows it holds the pid.
- `fn pty_handle(&Pty) -> PtyHandle`.
- `fn foreground_process_name(&PtyHandle) -> Option<String>`.
- `fn working_directory(&PtyHandle) -> Option<PathBuf>`, which always returns `None` on Windows.
- `fn default_program() -> (String, Vec<String>)`.
- `fn editor_command(editor: Option<String>, path: &Path) -> (String, Vec<String>)`.

The Unix module is the current code, moved without changes. `libc` becomes a `cfg(unix)` dependency.

*Alternative:* `cfg` attributes scattered through `terminal/mod.rs`, `workspace.rs` and `config.rs`. That was rejected because the next platform, macOS, needs the same seams, and the macOS port can add `platform/macos.rs` alongside.

### 2. Windows tab titles come from a Toolhelp process snapshot

ConPTY has no foreground process group. `foreground_process_name` therefore:
1. takes one `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)`;
2. builds a parent-to-children map;
3. walks down from the tab's pid, choosing the child with the highest pid at each level as a proxy for "most recently started";
4. returns that process's `szExeFile` without `.exe`.

The 1 s title poll would take one snapshot per tab. To avoid that, `Workspace`'s poll takes **one snapshot per tick** and passes it to every tab, so the cost is about one syscall per second no matter how many tabs are open.

The tree walk is a pure function over `(pid, parent_pid, name)` rows and is unit-tested on every platform. Using the highest pid as "newest" is a heuristic, because pids are reused. Process creation times, read with `GetProcessTimes`, would be exact but need an `OpenProcess` per candidate. The heuristic is good enough for a title, and the tab's OSC title still takes precedence.

Uses `windows` 0.61, `Win32_System_Diagnostics_ToolHelp`.

*Alternatives:*
- Only the shell's name, which would always say `pwsh` and is useless.
- `conhost`-style console process lists (`GetConsoleProcessList`), which need Brindle to attach to the pseudoconsole. Rejected as fragile.

### 3. The title bar uses GPUI's window-control areas on Windows

The tab strip's render code in `src/workspace.rs` currently gates its controls on `Decorations::Client`, which GPUI never reports on Windows. The change:
- **Show controls:** `csd` becomes `matches!(decorations, Decorations::Client { .. }) || cfg!(windows)`.
- **Mark areas:** on Windows, the strip's empty-space div gets `.window_control_area(WindowControlArea::Drag)`, and the three buttons get `Min`, `Max` and `Close`. The tabs, new-tab button and profile chevron are not marked, so they get `HTCLIENT` and keep their own handlers.
- **Native caption behaviour:** because Windows hit-tests these regions, it provides drag-to-move, double-click maximize, the right-click system menu, Aero Snap and Snap Layouts natively. The existing `on_mouse_down` handlers that call `start_window_move`, `zoom_window` and `show_window_menu` are harmless no-ops on Windows.
- **Button clicks:** GPUI performs the button action on release, so the buttons' `on_click` handlers won't fire on Windows. Close posts `WM_CLOSE`, which ends in the existing `on_window_closed` hook, and that quits after the last window.
- **Resizing:** borders come from GPUI's default hit-test, so the Linux `resize_edge` code stays Linux-only.

*Alternative:* turn `appears_transparent` off on Windows and keep the native caption above the tab strip. That's simpler but costs a second 30 px bar and breaks the "tab strip is the title bar" design in the `tabs` spec.

### 4. The primary selection is an app-global buffer on Windows

`src/platform/windows.rs` provides `write_primary(cx, item)` and `read_primary(cx)`, backed by a GPUI global `PrimarySelection(Option<ClipboardItem>)`. On Unix these forward to `cx.write_to_primary` and `cx.read_from_primary`. All three call sites (OSC 52 `ClipboardType::Selection` in both directions, `finish_selection`, and middle click / `PastePrimary`) go through the wrapper. The behaviour matches X11 within Brindle, and other applications never see the buffer. That's acceptable, because Windows has no such convention.

*Alternative:* on Windows, treat the primary selection as the clipboard. Rejected, because every selection would overwrite the clipboard, which is what the `copy_on_select = false` default is there to prevent.

### 5. Default shell and editor

`default_program()` on Windows searches `PATH` for `pwsh.exe`, then `powershell.exe`, then uses `%COMSPEC%`, then `cmd.exe`. The search is a plain loop over `std::env::split_paths`, with no new dependency. The `-NoLogo` argument is added for the two PowerShells. `escape_args: true` is set, so profile `args` are quoted with the C runtime rules, which is what users writing TOML arrays expect.

`editor_command` on Windows uses `%VISUAL%` or `%EDITOR%` through `cmd.exe /C`, so values like `code -w` work, and defaults to `notepad.exe`. On Unix it is the current `/bin/sh -c` with `nano`.

`child_env` keeps setting `TERM`, `COLORTERM` and so on. The values are harmless on Windows, and they help WSL and ssh sessions started from the tab.

### 6. Fonts

The terminal fallback list in `resolve_family` gains `Cascadia Mono` and `Consolas` after the Linux entries. `Cascadia Mono` ships with Windows Terminal and is standard on Windows 11. The config's default `font.family` gains `Cascadia Mono` and `Consolas` at the end, after the existing entries, so nothing changes on Linux machines that have one of those. The UI font list gains `Segoe UI Variable Text` and `Segoe UI`. DirectWrite does the glyph fallback for symbols, emoji and CJK, so the font-selection requirement holds without more code.

### 7. tmux through WSL

`tmux::command_line` already runs `profile.command` with `tmux_args` placed before `-C new-session -A -s <name>`. So `command = "wsl"` with `tmux_args = ["tmux"]` produces `wsl tmux -C new-session -A -s main`. Control mode runs over stdin and stdout pipes, which `wsl.exe` passes through.

The only change is that `command_line` omits `-c <dir>` when the program's file stem is `wsl`, compared case-insensitively. The directory would be a Windows path, which tmux inside WSL can't use. This is unit-tested on every platform. The default config gains a commented WSL control-mode profile.

### 8. Icon resource and version info: `build.rs` with `embed-resource`

The new `build.rs` acts only when `CARGO_CFG_TARGET_OS == "windows"`. It compiles `assets/windows/brindle.rc`, which holds:
- `1 ICON "brindle.ico"`, at id 1 because that's the id GPUI loads;
- a `VERSIONINFO` block whose `ProductVersion` and `FileVersion` come from `CARGO_PKG_VERSION`, passed as `/D` macros.

It uses `embed-resource` 3, declared under `[target.'cfg(target_os = "windows")'.build-dependencies]`, which is the same form GPUI uses. That version is already locked, so `winresource`, a new crate, isn't needed.

**No manifest in our `.rc`.** GPUI's `windows-manifest` feature already supplies RT_MANIFEST id 1, and a second one fails linking with a duplicate-resource error. Task 4.3 checks that GPUI's manifest really reaches `brindle.exe`. If it doesn't, Brindle would turn off GPUI's `windows-manifest` feature and put GPUI's manifest XML into `brindle.rc`. GPUI is Apache-2.0, so copying that XML is allowed. Turning the feature off means listing the other default features (`font-kit`, `wayland`, `x11`) explicitly. Feature changes don't change locked versions.

### 9. Icons are rendered by a script and committed

There is no SVG rasterizer in the dependency tree, and adding `resvg` as a build dependency would pull in a dozen new crates. Instead, `scripts/render-icons.sh` renders `assets/brindle.svg` to PNGs at 16, 20, 24, 32, 40, 48, 64 and 256 px with `rsvg-convert`, or with `resvg` or `magick` if that's what is installed. It then packs them into `assets/windows/brindle.ico`. The 256 px image is PNG-compressed inside the ICO, and the smaller ones are BMP for compatibility with older shells. The `.ico` is committed and rebuilt only when the SVG changes. A CI step checks that it is newer than the SVG. The macOS port can reuse the script's PNG output for its `.icns`.

### 10. Installer: MSI built with the WiX Toolset CLI

`packaging/windows/brindle.wxs` is authored by hand for the WiX v4+ `wix build` CLI and builds `Brindle-<version>-x86_64.msi`. It defines:
- a per-machine `Package` in `ProgramFiles64Folder\Brindle`;
- a stable `UpgradeCode` and a `MajorUpgrade` element, so newer versions replace older ones;
- a Start Menu shortcut;
- an optional `PathFeature` (Level 2, off by default) that adds the install directory to the system `PATH` through an `Environment` element;
- `ARPPRODUCTICON` set to `brindle.ico`;
- the MIT and Apache-2.0 licence text in the `LICENSE` dialog, as RTF.

Uninstall removes exactly what the MSI installed. `%APPDATA%\brindle` is never touched.

Alternatives compared:

| | MSI (WiX CLI) | Inno Setup | MSIX |
|---|---|---|---|
| Enterprise deploy (Intune, GPO, `msiexec /qn`) | native | needs wrappers | yes, but signing is mandatory |
| Upgrade and uninstall bookkeeping | Windows Installer does it | script-defined | OS-managed |
| Unsigned sideloading | works, with a SmartScreen warning | works, with a SmartScreen warning | **impossible**: the package must be signed by a trusted certificate |
| Effect on a terminal | none | none | File-system and registry virtualization of `%APPDATA%` writes, and a container identity that leaks into the shells it starts |
| winget support | yes | yes | yes |
| Toolchain | `dotnet tool install wix`, scriptable in CI | Windows-only `ISCC.exe` | `makeappx`, signing |
| Precedent | Alacritty ships an MSI built with WiX | – | Windows Terminal, which is Microsoft-signed |

MSI wins on deployability and on needing no signing. *cargo-wix* was considered, but its generated template and driver target the WiX v3 `candle`/`light` toolset. A hand-written `.wxs` for the current CLI is small, about 80 lines, and keeps the toolchain current.

### 11. `--install-desktop` on Windows creates a Start Menu shortcut

`src/desktop.rs`, from the Linux change, gains a `#[cfg(windows)]` implementation:
1. find the per-user Programs folder with `SHGetKnownFolderPath(FOLDERID_Programs)`;
2. create `Brindle.lnk` through COM, using `IShellLinkW` with `SetPath(current_exe)`, `SetIconLocation(current_exe, 0)` and `SetDescription`, saved with `IPersistFile::Save`;
3. print the path.

`--uninstall-desktop` deletes that file and treats a missing file as success. `--prefix` is rejected with exit status 2, as `desktop-integration` specifies. The code uses `windows` 0.61 with the `Win32_UI_Shell` and `Win32_System_Com` features, which GPUI already enables on the same locked version. No AppUserModelID is set on the shortcut or the process, so Windows groups taskbar buttons by executable path, which matches the shortcut.

The MSI doesn't call `--install-desktop`, because it owns its own shortcut. If a user has both, the per-user `.lnk` and the MSI's all-users shortcut show up as two Start Menu entries. The README says to use one or the other.

### 12. CI is the Windows verification

`.github/workflows/windows.yml` runs on `windows-latest`, which has MSVC and the Windows SDK, so `fxc.exe` and `rc.exe` are available. The job:
1. installs the WiX CLI with `dotnet tool install --global wix` (version pinned) and adds the UI extension;
2. runs `cargo build --release --locked` and `cargo test --locked`;
3. builds the MSI;
4. installs and uninstalls the MSI quietly with `msiexec /i … /qn` and `/x`, checking the installed file, the shortcut and the uninstall registry key;
5. uploads the MSI as an artifact.

`--locked` enforces the "never regenerate `Cargo.lock`" rule. Interactive checks (title bar, Snap Layouts, icon, IME, fonts) are manual steps on a real Windows 11 machine, listed in tasks.md.

## Risks / Trade-offs

- **[Code that has never been compiled for Windows]** The first CI run may find more Unix-only code than the review above found. → Task 1.1 makes the Windows job the first thing landed, before any behaviour work, and the remaining errors are fixed in groups 1–2.
- **[GPUI's manifest may not reach the `.exe`]** If it doesn't, the window isn't DPI-aware and renders blurry at scales above 100%. → Task 4.3 checks for it explicitly. The fallback is decision 8.
- **[Tab titles from the newest-pid heuristic can be wrong]** For example, with a background job started after the foreground one. → It only applies when the program sets no title. PowerShell and most TUIs set one.
- **[No working directory inheritance on Windows]** This is a visible gap compared with Linux. → It is specified as such. OSC 7 support is a future cross-platform change.
- **[Unsigned binaries]** SmartScreen warns on the first download, and some antivirus tools flag new unsigned executables. → Signing is an open question. Until it's decided, the README explains the warning.
- **[A `fxc.exe` path tied to SDK 10.0.26100.0]** GPUI's build script panics if neither `PATH` nor that SDK path has it. → CI runners have it, and the README tells local builders to set `GPUI_FXC_PATH`.
- **[Lockfile drift]** Adding direct dependencies on locked crates updates `Cargo.lock`'s dependency lists. → Run plain `cargo build` once to add the edges, without `cargo update`. Check that `git diff Cargo.lock` only adds dependency edges and no new versions, then use `--locked` from then on.

## Migration Plan

Nothing changes on Linux: the code moves into `platform/unix.rs` without behaviour changes, and `cargo test` must pass before and after the move. Windows is new. Releases attach the MSI to GitHub Releases. The winget manifest is submitted once there is a stable download URL. Rolling back means not publishing the MSI. The Windows code is inert on Linux.

## Open Questions

- **Code signing.** Should Brindle buy an Authenticode certificate? An OV certificate costs roughly $200–400 a year and still builds SmartScreen reputation slowly. EV is more expensive and needs hardware tokens. Azure Trusted Signing is cheaper, but its eligibility is limited. This affects only the release pipeline, not the specs or tasks, since the MSI and the `.exe` are signed as a post-build step.
- **winget publishing ownership.** Who submits and maintains the `microsoft/winget-pkgs` manifest? `wingetcreate` can automate it from the release workflow once releases exist.
- **ARM64.** GPUI's D3D11 path should work on ARM64. Adding `aarch64-pc-windows-msvc` to CI and the MSI is a follow-up once x64 is stable.
- **`--prefix` meaning across platforms:** it is `<dir>/share` on Linux (`add-linux-desktop-install`), `<dir>/Applications` in `port-to-macos`, and rejected with exit 2 here. Decide whether to keep one flag with per-platform meaning or rename it, before either port is applied.
- **Archive order:** this change adds to `desktop-integration`, so `add-linux-desktop-install` must be archived first. MODIFIED blocks shared with `port-to-macos` (configuration, profiles, tabs, terminal-rendering) need rebasing onto whichever change archives first.
