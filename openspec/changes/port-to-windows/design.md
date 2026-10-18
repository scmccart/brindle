# Design

## Context

See proposal.md for the motivation. These facts were checked on this Windows 11 machine (build 26200, SDK 10.0.26100.0, rustc 1.99.1 msvc). They shape the approach:

- **What doesn't compile today.** A `cargo check --all-targets` of a scratch copy with the Linux-only crates gated stops on the following. More type errors will surface once these resolve, for example the missing `escape_args` in the `tty::Options` literal.
  - `src/terminal/mod.rs`: `pty.child()`, `std::os::fd` and `libc::tcgetpgrp`.
  - `src/terminal/pty.rs`: `pty.file()`.
  - `src/session.rs`: `flock`, Unix modes, `std::os::unix`.
  - `src/terminal/links.rs`: `process_group`.
  - `src/notify/{dbus,activation}.rs`: zbus, wayland-*, `raw_window_handle`, `gpui::guess_compositor`.
  - Unix-only tests in `scan.rs`, `links.rs`, `config_edit.rs`, `session.rs`, `tests/e2e` and `tests/perf`.
- **alacritty_terminal 0.26 on Windows.**
  - Its `conpty.rs` calls `CreatePseudoConsole(…, dwFlags = 0, …)` and opens two anonymous pipes.
  - It looks for `conpty.dll` with a bare `LoadLibraryW` (the standard search path, `PATH` included), and then asks for the unprefixed export names.
  - Its pipe reader/writer (`blocking.rs`) and `ChildExitWatcher` are in private modules.
  - What is public: the traits `EventedReadWrite`, `EventedPty` and `OnResize`, `tty::PTY_READ_WRITE_TOKEN`/`PTY_CHILD_EVENT_TOKEN`, and `EventLoop`. The loop is woken through `polling`'s public IOCP extension, `PollerIocpExt::post(CompletionPacket)`.
  - `Pty` has no `file()`, so `MarkedPty` (`src/terminal/pty.rs`), which clones the Unix master fd to parse output itself, can't be built on top of it.
- **ConPTY behaviour.** A spike in scratch hosted a pseudoconsole and had its child write test sequences, recording the raw bytes that came out.
  - **Inbox ConPTY** (`conhost.exe`):
    - repaints the screen and moves OSC 0/7/9/9;9/777/52 ahead of their text;
    - drops `?1000h`/`?1006h`, emits `?2026h…l` empty with the content outside it, and swallows the OSC 11 query (answering it itself);
    - rewrites OSC 8 with an `id=`.
  - **Packaged ConPTY** (NuGet `Microsoft.Windows.Console.ConPTY` 1.25.260930003, MIT; `conpty.dll` 119,608 bytes and `OpenConsole.exe` 1,089,376 bytes, x64) passed every sequence byte for byte and in order. It begins with `\e[1t\e[c\e[?1004h\e[?9001h` (it asks for DA1) and ends with `\e[?1004l\e[?9001l`.
  - **Both** request win32-input-mode (`?9001h`) and focus events (`?1004h`) at startup. Both accept the flags `0x10` (`PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH`) and `0x20`. Both exit at once after `ReleasePseudoConsole` followed by `ClosePseudoConsole`.
  - **Input on both hosts:** the terminal wrote an SGR click and release, `ESC [ I`, a bracketed paste, `ESC [ 1 ; 5 A` and `x`.
    - A child reading with `ENABLE_VIRTUAL_TERMINAL_INPUT` got those exact bytes.
    - A child reading `ReadConsoleInputW` got mouse press and release records at (4,9), a focus event, the paste as key events, and ctrl+up with `LEFT_CTRL_PRESSED | ENHANCED_KEY`.
    - Input therefore needs no special handling. The inbox host's loss is on the output side: a program's `?1000h` never reaches Brindle, so Brindle never starts reporting.
  - **Glyph widths:** a child wrote strings with `WriteConsoleW` and read back its cursor column.
    - The inbox host ignores the flags and gives e+U+0301 two cells.
    - The packaged host's default (graphemes) gives ❤️ (U+2764 U+FE0F) two cells.
    - Only the packaged host with `0x10` gives 表 2, 😀 2, e+U+0301 1 and ❤️ 1. That matches alacritty's per-code-point `unicode-width` grid.
- **Exports and flags.**
  - The package's `inc/conpty.h` defines `INHERIT_CURSOR 0x1`, `GLYPH_WIDTH_GRAPHEMES 0x08`, `GLYPH_WIDTH_WCSWIDTH 0x10`, `GLYPH_WIDTH_CONSOLE 0x18` and `AMBIGUOUS_IS_WIDE 0x20`. There is no resize-quirk, passthrough or win32-input flag.
  - `conpty.dll` exports the unprefixed names plus `Conpty{Create,Resize,Close,Clear,ShowHide,Reparent,Release,Pack}PseudoConsole`.
  - Inbox `kernel32.dll` on build 26200 exports `Create/Resize/Close/ReleasePseudoConsole`.
  - `conpty.dll` finds `OpenConsole.exe` either next to itself or in an `x64\` subdirectory; both layouts were tested.
- **GPUI 0.2.2 on Windows.**
  - It renders with Direct3D 11. In release builds `build.rs` compiles the HLSL shaders with `fxc.exe`, found via `GPUI_FXC_PATH`, then `PATH`, then SDK 10.0.26100.0.
  - The `windows-manifest` feature (a default) embeds PerMonitorV2 DPI awareness with `embed-resource`.
  - The window class loads icon resource 1 from the `.exe`.
  - `window_decorations()` returns `Server`. With `appears_transparent` set, `WM_NCHITTEST` maps `WindowControlArea::{Drag,Min,Max,Close}` to the native caption hit codes.
  - A press on the caption or a caption button reaches GPUI's listeners first, as a mouse-down at that point. Windows drags, double-clicks, resizes or presses the button only when no listener stops propagation or prevents default. GPUI's own focus transfer prevents default whenever a focusable element is under the mouse. So the tab strip occludes the focusable workspace behind it, and the caption buttons get no handlers on Windows.
  - `zoom()` only ever maximizes, and `start_window_move` and `show_window_menu` do nothing.
  - `write_to_primary`/`read_from_primary` exist only on Linux and FreeBSD.
  - `KeyDownEvent` carries only a `Keystroke` (key name, `key_char`, modifiers), with no virtual-key code or scan code.
  - It sets no AppUserModelID.
  - `HasWindowHandle` gives the HWND.
- **Lockfile.** Adding `windows` 0.61 (with the features below) and `embed-resource` 3 to a scratch copy changed `Cargo.lock` by exactly two dependency edges in `brindle`'s entry, with no new packages or versions.
- **Linux side.**
  - A Claude session on the Linux machine (`/home/scmccart/src/brindle`, Ubuntu, rustc 1.99.1, tmux 3.6) runs checks when its user approves them. It fetches from origin.
  - `scripts/with-xvfb` runs there headless.
- **`port-to-macos`** is active and puts process info behind `terminal/process.rs`. This design uses the same seam, so the two ports don't diverge.

## Goals / Non-Goals

**Goals:**
- One codebase that builds and tests on `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`, with Linux behaviour unchanged.
- Local tabs on Windows get the same VT fidelity as on Linux, through a ConPTY Brindle controls.
- Platform code sits in a few `cfg`-split modules shared with the macOS port. Pure logic (process-tree walks, command lines, path decoding, argument quoting) is cfg-free and unit-tested on both platforms.

**Non-Goals:**
- **win32-input-mode encoding.** GPUI doesn't expose virtual-key codes, scan codes or key-ups. Brindle ignores `?9001h`, and ConPTY then falls back to translating VT input, which is the documented behaviour when the terminal doesn't answer.
- **`ConptyShowHidePseudoConsole` and `ConptyClearPseudoConsole`.** No requirement needs them: with passthrough, the host has no screen of its own to repaint.
- Translating paths between Windows and WSL, and reading other processes' working directories through the PEB.
- Isolating Windows GUI e2e runs from the desktop the way `scripts/with-xvfb` does. Local GUI runs ask first, and CI runs them on its own desktop.

## Decisions

### 1. Seams: `cfg` by module, shared with the macOS port

- **Dependencies:**
  - `zbus`, `wayland-*` and dev-dependency `x11rb` move to `[target.'cfg(target_os = "linux")'.…]`.
  - `libc` moves to `cfg(unix)`.
  - `raw-window-handle` stays unconditional, because Windows needs the HWND.
  - `windows` 0.61 goes under `cfg(windows)` with `Win32_Foundation`, `Win32_Security`, `Win32_Storage_FileSystem`, `Win32_System_{Com,Console,Diagnostics_ToolHelp,LibraryLoader,Pipes,Threading,IO,Registry}`, `Win32_UI_{Shell,Shell_PropertiesSystem,WindowsAndMessaging}`, `Win32_Storage_EnhancedStorage`, `Foundation`, `Data_Xml_Dom` and `UI_Notifications`. The exact list is trimmed to what compiles.
- **Modules:**
  - `notify/{dbus,activation}.rs` become `cfg(target_os = "linux")`.
  - `terminal/process.rs` (the macOS seam) holds process name and cwd: `/proc` on Linux, Toolhelp on Windows (§4).
  - `terminal/conpty.rs` is the Windows PTY (§2).
  - `src/primary.rs` is the primary selection (§6).
  - `notify/toast.rs` is the Windows sink (§9).
  - `src/win.rs` collects small Windows helpers: console attach, `CREATE_NO_WINDOW` spawns, AUMID, known folders.
- **Tests:** Unix-only tests get `#[cfg(unix)]`. Where the property matters on both platforms, they get a portable twin; for example, `scan.rs`'s split-read test uses an in-memory reader instead of `UnixStream`.

*Alternative:* a `src/platform/{unix,windows}.rs` facade, which the old version of this change proposed. Rejected, because `port-to-macos` already chose `terminal/process.rs`. Two parallel seams for the same thing would conflict when both changes land.

### 2. Brindle owns the Windows pseudoconsole (`terminal/conpty.rs`)

Brindle implements `EventedReadWrite + EventedPty + OnResize` itself and hands that to alacritty's `EventLoop`. alacritty's Windows `Pty` is not used.

- **Which ConPTY.** `ConptyApi` resolves once per process. Every candidate is compared byte for byte with the copy embedded in the binary (§3), so only that exact build is ever loaded. A build without an embedded copy goes straight to the inbox functions.
  1. `<exe dir>\conpty.dll` and `OpenConsole.exe`, when both match. Dev builds copy them there. A `cargo install`ed exe in `~\.cargo\bin` (user-writable, and on `PATH`) is protected by the same check.
  2. Else the embedded copy, extracted to `%LOCALAPPDATA%\brindle\conpty\<version>\`. The extraction is reused when the bytes on disk match, and rewritten when they don't.
  3. Else the inbox `kernel32` functions, logging one warning that names the paths tried.
  - `BRINDLE_CONPTY=inbox` forces step 3. It is a test hook in the same pattern as `BRINDLE_TMUX_STATE`.
  - The DLL is loaded with `LoadLibraryExW(<absolute path>, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)`. It never goes through `PATH` or the current directory.
  - From the DLL Brindle takes the `Conpty*` exports, which are the names `conpty.h` documents. From kernel32 it takes the unprefixed ones.
  - `ReleasePseudoConsole` is resolved as optional. It was confirmed only on build 26200, and the Windows 11 floor includes older builds. When it is missing, close relies on the detached-thread `Close` alone.
- **Creation.**
  - Two anonymous pipes.
  - `CreatePseudoConsole(size, in_read, out_write, PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH, &hpc)`. The flag makes the packaged host count cells per code point, as alacritty's `unicode-width` grid does; the measurements are in Context. The host's grapheme default would put ❤️ in two cells where Brindle draws one. The inbox host accepts the flag and ignores it.
  - Then the pseudoconsole side of each pipe is closed.
  - `STARTUPINFOEXW` with `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, `STARTF_USESTDHANDLES` with null handles so nothing leaks, and `EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT`.
  - The environment block merges the profile's env over Brindle's environment case-insensitively, then sorts it case-insensitively, as `CreateProcessW` requires.
- **Command line.** Built with the MSVC `CommandLineToArgvW` rules, written fresh and unit-tested on all platforms, so TOML `args` arrive as written. A raw tail is allowed for the `cmd.exe /S /C` editor launch (§7), because `cmd` doesn't understand backslash escapes.
- **I/O threads.**
  - A reader thread does blocking `ReadFile` into a bounded buffer. When data arrives while the loop has read interest, it posts `CompletionPacket(PTY_READ_WRITE_TOKEN)`.
  - `Read` on the Brindle side drains the buffer or returns `WouldBlock`.
  - A writer thread does the same for writes, so a full input pipe never blocks the loop.
  - The child exit is watched with `RegisterWaitForSingleObject`, which sends `ChildEvent::Exited(code)` and posts `PTY_CHILD_EVENT_TOKEN`.
  - This is the contract alacritty's own Windows pipes meet. It is written from the trait docs and `polling`'s API, not copied: alacritty_terminal is Apache-2.0, Brindle is MIT OR Apache-2.0.
- **Shell integration.** `ParsingReader` becomes `ParsingReader<R: Read>`. On Unix it wraps `ScanReader<File>` (the cloned master, unchanged). On Windows it wraps `ScanReader<PipeReader>` and owns the reader end directly, so no clone is needed. `MarkedPty` becomes cfg-split: on Unix it is today's wrapper; on Windows it is the ConPTY plus a `ParsingReader`.
- **Close.**
  - `Drop` calls `ReleasePseudoConsole` and then `ClosePseudoConsole` on a short-lived detached thread.
  - The reader thread keeps draining until EOF, so even an older host that blocks in `Close` until output is drained finishes, and neither the event-loop thread nor the UI thread waits.
  - Field order puts the pseudoconsole before the pipes.
- **Reparenting.** With the packaged host, `ConptyReparentPseudoConsole(hpc, hwnd)` is called when the tab's view is first placed in a window, and again if it moves to another window. Console programs that parent dialogs to `GetConsoleWindow()` (credential prompts, `git` GUI helpers) then center on Brindle.
- **Startup requests from the host.**
  - DA1 is answered by alacritty's `Term` through `Terminal::reply()`, which writes to the PTY for local tabs.
  - `?1004h` turns on focus reports, which ConPTY forwards to programs that ask for them.
  - `?9001h` is ignored, since alacritty treats unknown private modes as no-ops (see Non-Goals).

*Alternatives:*
- Ship `conpty.dll` next to the exe and keep alacritty's backend: no flags, `PATH` lookup, no Release.
- Patch alacritty_terminal: Brindle avoids forks, and the change would still need a release.
- Inbox ConPTY only: the spike shows it loses mouse modes and reorders OSCs.

### 3. Getting the packaged ConPTY into every build

- **Fetch:** `scripts/fetch-conpty.ps1` downloads `Microsoft.Windows.Console.ConPTY` at a pinned version, checks the `.nupkg` against a pinned sha256, and extracts the x64 `conpty.dll` and `OpenConsole.exe` plus the licence into `.conpty/<version>/` (gitignored). The version and hash are moved by hand, like the other pins. Today they are `1.25.260930003` and `02B07B349AF66D801159BDF9E440D4A1CE78BB951F37FC8609731665AFDAE7EE`.
- **Build:** on Windows, `build.rs` looks for `.conpty/<version>/` via `CARGO_MANIFEST_DIR`, which also works for `cargo install --path .` with its temporary target dir.
  - When the files are present, it `include_bytes!`s them through a generated file, and copies them next to the build's output exe so `cargo run` uses path 1.
  - When they are absent, it prints a `cargo:warning` that names the script, and the binary falls back as in §2.
- **Install:** the release zip holds only `brindle.exe`, which embeds both files. Run from wherever it was unpacked, or from WinGet's package directory, it uses path 2. `cargo install` users get path 2 too if they ran the script first.

*Alternatives:*
- Commit the binaries (2.3 MB of third-party executables in git history).
- Download from `build.rs` (no network in builds).
- Rely on `PATH` (planting risk, and it is the thing the spec forbids).

### 4. Tab titles from one Toolhelp snapshot per tick

- `process::Snapshot::take()` returns, on Windows, the `(pid, parent_pid, exe)` rows from one `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)`. On Linux it is empty, and the `/proc` path stays as it is.
- The workspace's 1 s title poll takes one snapshot per tick and passes it to each tab. A pure `newest_descendant(rows, root)` then:
  1. walks down from the tab's child pid, picking the highest pid at each level, since pids only approximate creation order;
  2. skips `conhost.exe` and `OpenConsole.exe`;
  3. strips `.exe`.
- An OSC title still takes precedence, and most Windows TUIs set one.
- **The console's default title.** The console host starts every program with its full path as its title (`C:\Program Files\PowerShell\7\pwsh.exe`), and ConPTY reports it like any other, so it would always win over the process name: the tab showed the path, and `python` run from it never named the tab. A pure `cmdline::console_title` treats a title that is just an absolute `.exe` path as no title, and in a title that starts with one followed by ` - ` (cmd's `C:\WINDOWS\system32\cmd.exe - python`) replaces the path with the program's stem (`cmd - python`). Any other title, including elevated PowerShell's `Administrator: …`, is shown as it is.

### 5. Working directory from OSC 7 and OSC 9;9

- **Parser:** Brindle's `Parser` already scans OSC sequences for OSC 133 before alacritty sees them. It gains a cwd decoder (pure and unit-tested):
  - `7;file://<host>/<path>` is percent-decoded. On Windows a leading `/C:/` becomes `C:\`.
  - `9;9;<path>` has optional double quotes, which are stripped.
- **Rule:** a path that isn't `Path::is_absolute()` for the build's platform is ignored. A Linux path from a WSL shell in a Windows tab is ignored that way.
- **Storage:** the result lands in a `Arc<Mutex<Option<PathBuf>>>` beside `Marks`. It is locked after `Marks`, which keeps the lock order "parser, then term, then marks".
- **Use:**
  - `Terminal::working_directory()` on Windows returns the report when it `is_dir()`, and otherwise `None` (the home directory).
  - On Linux it is unchanged (`/proc`), and the report is recorded but unused.
  - Session restore already records `working_directory()`, so restore follows without extra code.
- **No collision with notifications:** `notify/osc.rs` already treats ConEmu `9;1`–`9;12` as non-notifications.

### 6. Primary selection

`src/primary.rs` provides `write_primary(cx, item)` and `read_primary(cx)`:
- on Linux and FreeBSD they forward to GPUI;
- elsewhere they read and write a GPUI global holding a `PrimaryBuffer`, a cfg-free type that is unit-tested everywhere.

The three call sites go through it: OSC 52 `Selection` in both directions, `finish_selection`, and middle click / `PastePrimary`.

### 7. Default program, editor and environment on Windows

- **`default_program(path, comspec)`** is pure:
  1. `pwsh.exe` on `PATH`, with `-NoLogo`;
  2. else `powershell.exe` on `PATH`, with `-NoLogo`;
  3. else `%COMSPEC%`;
  4. else `cmd.exe`.
- **`editor_command(visual, editor, path)`** gives `cmd.exe /D /S /C "<editor> "<path>""`, the raw tail from §2, using `%VISUAL%`, then `%EDITOR%`, then `notepad.exe`. This way values like `code -w` work, and paths with spaces stay quoted. It runs in a new tab, as the configuration spec says.
- **Environment:** `child_env` keeps `TERM` and the other variables, which helps WSL and ssh started from the tab.

### 8. tmux through WSL

- **Splitting the arguments:** `tmux_command(profile)` returns the program and an argument prefix. For a program whose file stem is `wsl` (case-insensitive), `tmux_args` is split at the first element equal to `tmux` into WSL options and tmux options.
- **The prefix:** WSL options, then `--exec` unless they already end in `--exec`/`-e`, then `tmux`. `-u` follows only on attach command lines.
- **Callers:** `command_line` and `has_session_command` both use it, so `-u` is never handed to `wsl.exe`, which would read it as `--user`.
- **No `-c`:** `command_line` drops `-c` for WSL profiles. Other paths already drop it, since a tmux pane directory like `/home/x` fails `is_dir()` on Windows.
- **`--exec`** runs tmux without a Linux shell, so session names pass verbatim.
- **Spawning:** every tmux spawn gets `CREATE_NO_WINDOW`, so `wsl.exe` (a console program) opens no console window.

### 9. Toast notifications

- **The sink:** `Notifier` gains `Sink::Toast`, which is `notify/toast.rs` running on its own thread with `RoInitialize(MTA)`.
  - Each request becomes a `ToastGeneric` toast whose title and body are text nodes (no XML injection, since they are set as node text).
  - The `Tag` is Brindle's notification id, and bells reuse their tab's id, which is how the "replaces" rule maps onto toasts.
  - `Activated` sends `Reply::Action { id, "default" }`, and `Dismissed` sends `Reply::Closed`.
  - If the notifier's `Setting` isn't `Enabled`, the request is dropped with a warning.
- **The identity:**
  - The AUMID is `Brindle.Terminal`. `main` calls `SetCurrentProcessExplicitAppUserModelID` before any window opens.
  - Brindle registers the AUMID under `HKCU\Software\Classes\AppUserModelId\Brindle.Terminal` (`DisplayName`, `IconUri` pointing to a PNG extracted beside the ConPTY copy). That way toasts work for `cargo install` and dev builds, which have no shortcut.
  - The `--install-desktop` shortcut sets `System.AppUserModel.ID` to the same value, which gives the taskbar grouping the spec requires.
- **Click handling:** a click runs `Workspace::show_tab`, then a Windows `activation::raise`. That restores the window if it is minimized and calls `SetForegroundWindow`. If Windows refuses foreground, it calls `FlashWindowEx` so the taskbar button flashes.
- **Unchanged:** the rate limits, the focus rule and `BRINDLE_NOTIFY_LOG` (which wins over every sink).

### 10. Title bar

- **Show controls:** `csd` becomes `matches!(decorations, Decorations::Client { .. }) || cfg!(windows)`.
- **Mark areas:** on Windows, the strip's empty space gets `window_control_area(Drag)`, and the three buttons get `Min`, `Max` and `Close`. Tabs, the new-tab button and the chevron stay unmarked, so they are `HTCLIENT`.
- **Actions:** GPUI performs Min/Max/Close on release. Close posts `WM_CLOSE` into the existing close path.
- **Linux-only:** the `start_window_move`, `zoom_window` and `show_window_menu` handlers are no-ops on Windows and stay for Linux. So does the `resize_edge` code, since Windows uses GPUI's default border hit-testing.

### 11. Process, console and paths

- **GUI subsystem:** `#![cfg_attr(windows, windows_subsystem = "windows")]`.
- **Console attach:** Brindle attaches to its parent's console only around writing output, and only when stdout is not already a valid handle (a redirect or pipe gives one).
  - It uses `AttachConsole(ATTACH_PARENT_PROCESS)`, writes, then calls `FreeConsole`.
  - This covers informational options, usage errors, `--install-desktop`/`--uninstall-desktop` messages and the `--dump-screen-after` dump.
  - A plain GUI launch never stays attached. If it did, ctrl-c in that console (`CTRL_C_EVENT`) or closing it (`CTRL_CLOSE_EVENT`) would end Brindle.
- **No console windows:** spawns outside tabs get `CREATE_NO_WINDOW`: tmux, `has-session` and `BRINDLE_LINK_OPENER`.
- **Links:** without an override, links are opened with `ShellExecuteW("open", uri)` on the existing opener thread, which keeps `&` and spaces intact. `process_group(0)` stays Unix-only.
- **Session state:**
  - The directory is `dirs::data_local_dir()` (`%LOCALAPPDATA%`) on Windows.
  - The lock moves from `libc::flock` to `std::fs::File::try_lock` on every platform. On Linux that is `flock(LOCK_EX | LOCK_NB)`, the same semantics; on Windows it is `LockFileEx`.
  - The Unix `0o700`/`0o600` modes stay `cfg(unix)`; `%LOCALAPPDATA%` is per-user by ACL.
  - The write stays temp file plus `rename` (`MoveFileExW` with replace on Windows).
- **Fonts:** `Cascadia Mono` and `Consolas` are appended to the terminal fallbacks and to the default `font.family`; `Segoe UI Variable Text` and `Segoe UI` go into the UI list. Linux resolution is unchanged, because they come last.

### 12. Executable resources and icons

- **`build.rs`** (Windows target only) compiles `assets/windows/brindle.rc` with `embed-resource`. The `.rc` holds `1 ICON "brindle.ico"` and a `VERSIONINFO` block fed from `CARGO_PKG_VERSION`. It also handles the ConPTY embedding from §3.
- **No manifest in our `.rc`:** GPUI's `windows-manifest` already supplies one, and CI checks that it reaches the exe. If it doesn't, the fallback is to turn off that feature, list the remaining defaults (`font-kit`, `wayland`, `x11`) explicitly, and put GPUI's Apache-2.0 manifest XML in our `.rc`.
- **Icons:** `scripts/render-icons.sh` renders `assets/brindle.svg` to 16–256 px PNGs with `rsvg-convert`, `resvg` or `magick`, packs `assets/windows/brindle.ico` (the 256 px image PNG-compressed), and also writes `assets/windows/brindle-256.png` for the toast `IconUri`. Both are committed. CI fails if the SVG is newer in git history than the `.ico`.
- **Where it runs:** the script runs under Git Bash on Windows, or on Linux.

### 13. Release zip, WinGet and `--install-desktop`

- **No installer:** `brindle.exe` is self-contained (§3), so it needs no install step and no files beside it.
- **The zip:** `scripts/package-windows.ps1` writes `target/dist/brindle-<version>-x86_64-pc-windows-msvc.zip`.
  - It holds `brindle.exe`, `LICENSE-MIT`, `LICENSE-APACHE` and `THIRD-PARTY-NOTICES.txt`, which carries the ConPTY's MIT notice (committed in `packaging/windows/`).
  - It refuses to package when `.conpty/<version>/` is missing, so a release never falls back to the inbox ConPTY.
  - The CI `windows` job runs it after the release build and uploads the zip as an artifact.
- **WinGet:** `packaging/winget/` holds the manifest templates (version, installer and default-locale files) at the newest schema the runner's `winget` validates.
  - The identifier is `scmccart.brindle`.
  - The installer is `InstallerType: zip` with `NestedInstallerType: portable`, `brindle.exe` as the nested file and `PortableCommandAlias: brindle`, for x64 and Windows 11 (`MinimumOSVersion: 10.0.22000.0`).
  - `scripts/winget-manifest.ps1` fills in the version, the GitHub Release asset URL for it and the zip's sha256, writing to `target/dist/winget/`. CI runs it on the built zip and runs `winget validate` on the result.
  - WinGet installs portable packages per user, links `brindle` into a directory on `PATH`, and adds an Installed apps entry. Its uninstall removes those and nothing in `%APPDATA%\brindle` or `%LOCALAPPDATA%\brindle`.
  - Publishing the zip to a GitHub Release and submitting the manifest to `microsoft/winget-pkgs` happen at the first release, outside this change.
- *Alternatives:*
  - An MSI built with WiX: per-machine installs, its own shortcut and Group Policy deployment, none of which are needed yet. WiX v6 and later also carry the Open Source Maintenance Fee EULA. It can be added later around the same exe.
  - Inno Setup, which needs wrappers for enterprise deploys.
  - MSIX, where signing is mandatory, plus filesystem virtualisation inside a terminal.
- **`--install-desktop`:** it finds `FOLDERID_Programs`, then creates `Brindle.lnk` through `IShellLinkW`, with the path set to `current_exe` and the icon to the file it resolves to, `IPropertyStore` setting `PKEY_AppUserModel_ID`, and `IPersistFile::Save`.
- **`--uninstall-desktop`:** it deletes the shortcut and the HKCU AUMID key, and treats both being missing as success.
- **The shortcut and WinGet:** WinGet doesn't create Start Menu shortcuts for portable packages, so the README points to `brindle --install-desktop`, and to `brindle --uninstall-desktop` before `winget uninstall`, which doesn't know about the shortcut. The shortcut has to keep working across `winget upgrade`. Run as the installed command, `current_exe` is WinGet's `%LOCALAPPDATA%\Microsoft\WinGet\Links\brindle.exe` symlink, which an upgrade re-points at the new exe, so the shortcut follows it (checked with a local manifest, 0.1.0 to 0.1.1). The shell reads no icon through that symlink: a shortcut whose icon location is the link shows the blank one, and the taskbar, which looks up the shortcut carrying the windows' AUMID, showed none either. The icon location is therefore the exe in WinGet's package directory, whose path doesn't change across upgrades.
- **`--prefix`:** rejected with exit status 2, with a message pointing to WinGet. The argument handling is cfg-free and unit-tested.

### 14. Verification

- **This Windows machine:**
  - `cargo build`, `cargo test` and the Windows e2e cases.
  - GUI runs here steal focus like on Linux `:0`, so they are announced and asked for first.
- **Windows e2e cases:**
  - `tests/e2e` compiles on Windows. X11 captures and tmux cases are `cfg(target_os = "linux")`.
  - New `windows-*` cases drive Brindle with the existing `--send`/`--dump-screen-after` hooks, `BRINDLE_NOTIFY_LOG` and a Toolhelp check. They cover the ConPTY host, passthrough order, titles, the OSC 7 new tab and the PowerShell marks.
  - `windows-wsl-tmux` skips itself when WSL or tmux is missing, and uses the `-L brindle-e2e-…` throwaway server rule.
- **The Linux machine:**
  - Each group that touches shared code is pushed to the `port-to-windows` branch on origin.
  - The Linux Claude session runs `cargo test --locked`, `cargo build --release --locked` and `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e`, each when its user approves.
  - The Linux CI jobs run on the same push.
- **CI:**
  - A `windows` job is added to `ci.yml`, sharing `RUST_VERSION`, on `windows-latest`.
  - It runs fetch-conpty, `cargo build --release --locked`, `cargo test --locked`, the resource and manifest checks, the Windows e2e cases, the zip, and the WinGet manifest with `winget validate`.
  - It uploads the zip.
  - Like `e2e`, the Windows e2e step is advisory.

## Risks / Trade-offs

- **[Writing our own ConPTY pipes and threads]** Event-loop wake-ups or drop order could be wrong (hangs on close, lost output). → Unit tests run the pipe reader against a pipe pair. A Windows e2e case closes a tab mid-output. The spike already exercises Release→Close.
- **[Packaged ConPTY absent in some builds]** Users of a plain `cargo install`, without the fetch script, get the inbox fallback and its fidelity loss. → A logged warning, the README, and release zips always including it (the packaging script refuses otherwise).
- **[Extracted DLL in `%LOCALAPPDATA%`]** It is writable by the user who runs Brindle, the same trust level as `~/.cargo/bin\brindle.exe` itself. It is rewritten whenever its bytes don't match the embedded copy, so tampering is undone at the next launch. Release zips and WinGet installs use it too: their `brindle.exe` sits in a per-user directory at the same trust level.
- **[Toast foreground activation]** Windows may refuse `SetForegroundWindow` from a toast `Activated` callback. → `FlashWindowEx` fallback, and a manual check in group 9.
- **[Toasts from an AUMID registered only in HKCU]** The registry route, with no shortcut, is what the Windows App SDK uses for unpackaged apps, but it hasn't been probed here, and `IconUri` support varies by build. → Task 7.1 starts with a probe, after asking, since it shows a toast. If toasts need a shortcut, the delta's "Toasts without the installer" scenario changes to "after `brindle --install-desktop`", and Brindle logs a hint pointing to it.
- **[GUI subsystem and stdio]** If `AttachConsole` behaves differently under some hosts (Windows Terminal versus conhost), CLI output could be lost. → An e2e test uses redirection, and the CLI options are covered by a manual check from PowerShell in Windows Terminal.
- **[No installer]** WinGet installs give no Start Menu entry, and `winget uninstall` leaves a shortcut made by `--install-desktop` behind. → The README covers both commands. An installer can be added later without changing the exe.
- **[Newest-pid title heuristic]** It can pick a background job. → OSC titles win, and this is specified as "most recently started".
- **[CI runs Windows Server 2025, not 11]** This affects the inbox ConPTY build and Snap. → Interactive checks are manual on this Windows 11 machine. The packaged ConPTY is the same binary everywhere.
- **[Shared MODIFIED blocks with `port-to-macos`]** configuration, profiles, tabs (titles) and terminal-rendering are modified by both changes. → Whichever change archives second rebases its blocks onto the first's.

## Migration Plan

- **Linux:** code moves behind `cfg` without behaviour changes, except that `flock` moves to `File::try_lock`, which has the same semantics. `cargo test` and the Linux e2e suite must pass before and after, on the Linux machine.
- **Windows:** new. Releases attach the zip to GitHub Releases, and the manifest goes to `microsoft/winget-pkgs`; both start with the first release.
- **Rollback:** don't publish the zip. The Windows code is inert on Linux.

## Open Questions

- **Code signing** (an OV certificate, EV, or Azure Trusted Signing): this affects only a post-build release step.
- **winget:** who submits the manifest at the first release and keeps it current, by hand or with a release workflow (wingetcreate or Komac).
- **ARM64:** GPUI's D3D11 path and the package's ARM64 binaries should work. This is a follow-up once x64 is stable.
- **`--prefix` meaning across platforms:** `<dir>/share` on Linux, `<dir>/Applications` in `port-to-macos`, and rejected here. Settle it before both ports are archived.
