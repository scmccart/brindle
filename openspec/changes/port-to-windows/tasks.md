# Tasks

Work happens on the Windows machine, on a `port-to-windows` branch pushed to origin.

- **Linux checks** go to the Linux Claude session (`/home/scmccart/src/brindle`), which fetches the branch and runs commands when its user approves: `cargo test --locked`, `cargo build --release --locked`, and `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e`. The Linux CI jobs run on the same push.
- **GUI runs on this Windows desktop** (anything that opens a Brindle window, including `--dump-screen-after` and the Windows e2e cases) steal focus, so ask the user before each one.
- **`Cargo.lock`:** never run `cargo update`. Use `--locked` once 1.2 has added the two edges.

## 1. A Windows build that compiles and runs

- [x] 1.1 Gate the Linux-only code (design §1):
  - `zbus`, `wayland-*` and dev-dependency `x11rb` move to `cfg(target_os = "linux")`, and `libc` to `cfg(unix)`;
  - `notify/{dbus,activation}.rs` become Linux-only, and on other platforms `Notifier` drops requests with one logged warning until group 7;
  - `tests/e2e` captures and `tests/perf` become Linux-only.

  Verify that `cargo check --all-targets` on Windows no longer reports unresolved crates, and that the Linux session's `cargo test --locked` passes.
- [x] 1.2 Add `windows` 0.61 (`cfg(windows)`, the features from design §1) and `embed-resource` 3 (Windows build-dependency). Run one plain `cargo check`. Verify that `git diff Cargo.lock` shows only the two new edges in `brindle`'s entry, and no added or changed `[[package]]` blocks.
- [x] 1.3 Move process info into `terminal/process.rs` (design §1, the `port-to-macos` seam). The Linux `/proc` and `tcgetpgrp` code moves unchanged, and Windows returns `None` for now.
  - `session.rs`: the state dir becomes `dirs::data_local_dir()` on Windows, the lock moves to `File::try_lock` on all platforms, and the Unix modes become `cfg(unix)`.
  - `links.rs`: `process_group` becomes `cfg(unix)`.
  - Unix-only tests get `#[cfg(unix)]`, with a portable twin where the property is cross-platform. `scan.rs`'s split read uses an in-memory reader.

  Verify `cargo test` on Windows for the touched modules, and the Linux session's `cargo test --locked` (the lock tests included).
- [x] 1.4 Temporary scaffolding, removed in 2.4: on Windows, spawn local tabs through alacritty's `tty::Pty` (`escape_args: true`, the pid from `child_watcher().pid()`), without `MarkedPty`. Verify that `cargo build` and `cargo test` pass on Windows.
- [ ] 1.5 Switch to the GUI subsystem, attaching to the parent console only around output (design §11). Verify:
  - `brindle --version` and `brindle --list-actions` print in PowerShell inside Windows Terminal;
  - `brindle --bogus` prints usage with exit status 2;
  - after asking the user, `brindle --send "echo hi\r" --dump-screen-after 3 > dump.txt` produces a dump containing `hi`, and no console window opens;
  - after asking, `brindle` started from a PowerShell window survives ctrl-c in that window and its closing.
- [x] 1.6 Add a `windows` job to `.github/workflows/ci.yml` on `windows-latest`, using the shared `RUST_VERSION`, that runs `cargo build --release --locked` and `cargo test --locked`. Verify that the job is green on the pushed branch.
- [x] 1.7 Document the Windows build in README.md (MSVC, Windows SDK, `GPUI_FXC_PATH`, Windows 11 only) and in CLAUDE.md (the build and test commands on Windows; the Linux session and `scripts/with-xvfb` for Linux checks; "Linux terminal" becomes "Linux and Windows"). Verify that each documented command runs as written on this machine.

## 2. Brindle's own pseudoconsole

- [x] 2.1 Add `scripts/fetch-conpty.ps1` (design §3), pinned to `1.25.260930003` and sha256 `02B07B34…AE7EE`, extracting into `.conpty/<version>/`, which is gitignored. Make `build.rs` embed the files and copy them next to the output exe when they are present, and print a `cargo:warning` otherwise. Verify:
  - the script fails on a wrong hash;
  - a build with the files embeds them (the exe grows by about 1.2 MB, and `target\debug\conpty.dll` exists);
  - a build without them warns and still builds.
- [x] 2.2 Write the pure helpers, cfg-free and unit-tested on both platforms:
  - MSVC command-line quoting (the spaces, quotes and trailing-backslash cases), and the raw tail;
  - the case-insensitive, sorted environment block, where the profile entry wins and `path` replaces `Path`.

  Verify with `cargo test` on Windows and on the Linux session.
- [x] 2.3 Implement `terminal/conpty.rs` (design §2):
  - the `ConptyApi` resolution order: exe dir and then the extracted copy, both only when byte-identical to the embedded copy, then kernel32. Use `LoadLibraryExW` with the safe search flags, and honour `BRINDLE_CONPTY=inbox`;
  - `PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH`;
  - the pipe reader and writer threads posting IOCP completions, and the child watcher;
  - an optional `ReleasePseudoConsole`, then a non-blocking `ClosePseudoConsole` on drop.

  Unit-test the reader and writer against an anonymous pipe pair. Test the resolution order with a temp directory: a mismatched `conpty.dll` beside the exe is ignored, a stale extraction is rewritten, and a `conpty.dll` in the current directory is never considered. Verify with `cargo test` on Windows.
- [x] 2.4 Make `ParsingReader` generic over its reader. Add the Windows `MarkedPty` over `conpty.rs` and remove 1.4's scaffolding. Add Windows e2e cases:
  - `windows-conpty-host`: a Toolhelp check that the tab's host is the shipped `OpenConsole.exe`;
  - `windows-passthrough`: PowerShell writes OSC 9, OSC 133, an SGR mouse mode and OSC 11, and the notify log, the dump and the reply show them in order;
  - `windows-glyph-width`: PowerShell writes `❤️`, and then e plus U+0301, each at column 0, and `[Console]::CursorLeft` prints 1 for each;
  - `windows-conpty-fallback`: with `BRINDLE_CONPTY=inbox`, `echo hi` prints `hi`, the host is `conhost.exe`, and the log has exactly one fallback warning;
  - `windows-close-mid-output`: a tab printing in a loop closes at once.

  Verify that the cases pass locally (after asking) and in CI, and that the Linux session's `cargo test --locked` and with-xvfb e2e pass.
- [x] 2.5 Call `ConptyReparentPseudoConsole` with the window's HWND when a tab's view is placed in a window, and when it moves to another one, with the packaged host only. Verify with a debug log line in a local run (after asking), and with the manual check in 10.3.
- [x] 2.6 Document the ConPTY host: in README.md, the fetch script before `cargo build`/`cargo install` and the inbox fallback warning; in CLAUDE.md, the architecture entry for `terminal/conpty.rs`, the resolution order and the drop order. Verify that the README commands work from a clean clone on this machine.

## 3. Tab titles on Windows

- [x] 3.1 Write `newest_descendant(rows, root)` (design §4), cfg-free. Unit-test no children, a chain, siblings picking the highest pid, skipping `conhost.exe`/`OpenConsole.exe`, and stripping `.exe`. Wire `process::Snapshot::take()` into the 1 s title poll, once per tick for all tabs. Add the e2e case `windows-title-child`: in `pwsh` (which sets no title, unlike `cmd.exe`), `ping -n 6 127.0.0.1` titles the tab `PING` (its executable name as on disk), and afterwards it no longer is. Verify with `cargo test` on Windows and Linux, and with the case locally (after asking) and in CI.

## 4. Working directory and PowerShell integration

- [x] 4.1 Add the OSC 7 and OSC 9;9 decoder to `Parser` (design §5), cfg-free. Unit-test percent-decoding, `file://host/C:/x`, quoted and unquoted 9;9, a relative path ignored, and a POSIX path ignored on Windows. Store the report beside `Marks` (lock order parser, term, marks, cwd), and return it from `working_directory()` on Windows. Verify with `cargo test` on both platforms, and the Linux session's e2e `restore` and `shell` cases.
- [x] 4.2 Add `shell-integration/brindle.ps1` and `brindle --shell-integration pwsh`. The script emits A, B, C, D;status and OSC 7. It keeps a user's `prompt`, and `$?`/`$LASTEXITCODE`. It does nothing when non-interactive and is safe to load twice. The error for an unknown shell lists `pwsh`. Add the e2e case `windows-pwsh-integration`, run under `pwsh` and `powershell.exe`:
  - `cmd /c exit 3` records status 3;
  - after `cd C:\Windows`, a new tab starts in `C:\Windows`;
  - session restore brings that directory back.

  Verify that the cases pass locally (after asking) and in CI, and that `cargo test` covers the shell list.
- [x] 4.3 Document the PowerShell opt-in line in README.md's shell integration section, and the Windows directory rule in the tabs docs. Verify that the documented `$PROFILE` line works in `pwsh` and Windows PowerShell 5.1 on this machine.

## 5. Windows defaults and chrome

- [x] 5.1 Add `src/primary.rs` (design §6), with the cfg-free `PrimaryBuffer` unit-tested. Route OSC 52 `Selection` in both directions, `finish_selection` and `PastePrimary` through it. Verify with `cargo test` on both platforms, and the Linux session's `selection` e2e cases.
- [ ] 5.2 Write `default_program(path, comspec)` (design §7) as a pure function. Unit-test the four steps and `-NoLogo`. Update the `Profile::command` doc and the default config comments. Verify with `cargo test`, and the config-defaults test.
- [ ] 5.3 Write `editor_command` (the `cmd.exe /D /S /C` raw tail; `%VISUAL%`, then `%EDITOR%`, then Notepad). Unit-test the generated command line for `code -w` and for a config path with spaces. Verify with `cargo test`, and by opening the config with ctrl-, locally (after asking).
- [ ] 5.4 Add the font fallbacks (design §11). Verify with `cargo test` (the config defaults), and that the Linux session's e2e cases are unchanged.
- [ ] 5.5 Open links with `ShellExecuteW` on Windows, with `BRINDLE_LINK_OPENER` spawned with `CREATE_NO_WINDOW`. Unit-test that the override gets the URI as one argument on Windows, using a test helper that records its argv. Verify with `cargo test` on Windows, and the manual check in 10.3.
- [ ] 5.6 Add the Windows caption controls and `window_control_area` marks (design §10). Verify that the CI build compiles, and that the Linux session's with-xvfb `titles`, `windows` and `palette` e2e cases still pass. The Windows behaviour is checked in 10.2.

## 6. tmux through WSL

- [ ] 6.1 Split WSL arguments in a `tmux_command` used by `command_line` and `has_session_command` (design §8). `-u` and `--exec` go in the right places, there is no `-c` for WSL, and tmux spawns get `CREATE_NO_WINDOW`. Unit-test every scenario in the `tmux-control-mode` delta (`wsl`, `WSL.EXE`, `-d Ubuntu tmux -L work`, no `tmux` element, an explicit `-e`, has-session, no `-c`), and that non-WSL command lines are unchanged. Verify with `cargo test tmux` on both platforms.
- [ ] 6.2 Add a commented WSL control-mode profile to `assets/default-config.toml`, and a README section. Add the e2e case `windows-wsl-tmux` with `tmux_args = ["tmux", "-L", "brindle-e2e-<pid>", "-f", "/dev/null"]`, which skips when `wsl --exec tmux -V` fails, and kill its server afterwards. Verify locally (after asking; first check that a WSL distribution with tmux is installed, and ask the user to install one if it isn't).

## 7. Toast notifications

- [ ] 7.1 First, probe in scratch, after asking the user, because a toast appears on screen: register `HKCU\Software\Classes\AppUserModelId\Brindle.Terminal` with no shortcut, then show a toast through `CreateToastNotifierWithId`. Record whether it appears and whether its icon is shown. If it doesn't appear, update the `desktop-notifications` delta's "Toasts without the installer" scenario and design §9 as the design's risk entry says, before going on. Then set the process AUMID before any window opens, and register the key with `DisplayName` and an `IconUri` pointing to the extracted `brindle-256.png`. Verify with `reg query` after a local run (after asking).
- [ ] 7.2 Add `notify/toast.rs` (design §9):
  - an MTA worker;
  - `ToastGeneric` XML built from text nodes, with a cfg-free unit test that markup in titles and bodies stays literal;
  - the tag and replace rule for bells;
  - `Activated` and `Dismissed` replies;
  - the setting check, warning when disabled.

  Verify with `cargo test`, and that the existing `notify` e2e cases pass on Windows with `BRINDLE_NOTIFY_LOG`.
- [ ] 7.3 Add the Windows `activation::raise`: restore if minimized, `SetForegroundWindow`, and `FlashWindowEx` when refused. Verify with the manual check in 10.3.
- [ ] 7.4 Document toasts in README.md: the AUMID registration and the Windows Settings switch. Verify that the text matches 7.1–7.3.

## 8. Icons, resources and installers

- [ ] 8.1 Add `scripts/render-icons.sh` (design §12). It writes `assets/windows/brindle.ico`, with 8 sizes and a PNG-compressed 256 px image, and `assets/windows/brindle-256.png`. If neither `rsvg-convert`, `resvg` nor `magick` is installed, ask the user to install one. Commit the outputs. Verify that the `.ico` header lists 8 images, and view the 16 and 32 px renders.
- [ ] 8.2 Add a CI check that fails when `assets/brindle.svg` is newer in git history than `assets/windows/brindle.ico`. Verify by touching the SVG on a scratch branch, and then reverting.
- [ ] 8.3 Add `assets/windows/brindle.rc` (`1 ICON`, `VERSIONINFO` from `/D` macros) to `build.rs`. Verify locally that `(Get-Item target\release\brindle.exe).VersionInfo` shows Brindle and the crate version, that `[System.Drawing.Icon]::ExtractAssociatedIcon` returns the icon, and that `mt.exe -inputresource:brindle.exe;#1 -out:m.xml` contains `PerMonitorV2`. If it doesn't, apply the design §12 fallback and check again. Add the same checks to the CI job, and verify that they pass there.
- [ ] 8.4 Add the Windows `--install-desktop` / `--uninstall-desktop` (design §13): the `.lnk` with the AUMID property; removing the shortcut and the HKCU key; `--prefix` rejected with exit status 2. Unit-test the argument handling, cfg-free. Verify locally (no window opens): the `.lnk` exists, and its `System.AppUserModel.ID` reads `Brindle.Terminal` through `Shell.Application`; uninstall removes both the shortcut and the key; uninstalling twice exits 0; `--prefix C:\Tools` exits 2. Run the same checks in CI.
- [ ] 8.5 Add `packaging/windows/brindle.wxs` (design §13), with the WiX CLI version pinned. Installing the WiX dotnet tool on this machine changes it, so ask the user first. Build the MSI locally and in CI, and upload it from CI. In CI, after a quiet install, check that `brindle.exe`, `conpty.dll`, `OpenConsole.exe`, the shortcut (with the AUMID) and the uninstall key exist. After uninstall, check that they are all gone and `%APPDATA%\brindle` is untouched. Verify that the CI step passes.
- [ ] 8.6 Add a CI upgrade check: build `0.0.1-ci` and the real version, install the first and then the second, and assert that exactly one Brindle uninstall key remains and `brindle --version` prints the real version. Verify that the step passes.
- [ ] 8.7 Document installing in README.md: the MSI; `cargo install` plus `--install-desktop` (and not to use both); the SmartScreen warning for unsigned builds; uninstall leaving config. Verify that each command matches the code and the CI workflow.

## 9. CI completeness

- [ ] 9.1 Extend the CI `windows` job with `scripts/fetch-conpty.ps1` before the build, the Windows e2e cases (advisory, like `e2e`, uploading `target/tmp/e2e/` when they fail), and the steps from group 8. Update CLAUDE.md's CI paragraph, covering the job, its pins and the advisory e2e step. Verify that the job is green on the branch, and that a deliberately failing case uploads its artifacts.

## 10. Manual verification on Windows 11 (this machine)

These are integration checks on real hardware, using the CI MSI artifact. Ask the user before each GUI step, and report the results. Add the steps that stay manual to CLAUDE.md's "Manual checks".

- [ ] 10.1 Install the MSI and launch Brindle from the Start Menu. Check that:
  - the taskbar, Alt-Tab and title area show the icon;
  - pinning to the taskbar and then launching from the pin gives one button;
  - the window is sharp at 150% scaling and when moved to a monitor with another scale;
  - the default tab runs `pwsh`;
  - no console window appears;
  - started from a PowerShell window instead, Brindle survives ctrl-c in that window and its closing.
- [ ] 10.2 Title bar against the `tabs` spec:
  - dragging the strip moves the window, and double-clicking maximizes;
  - right-click opens the system menu;
  - Snap works at the screen edges, and hovering maximize shows Snap Layouts;
  - minimize, maximize and close work, and closing the last window exits;
  - edge resizing works;
  - tabs, the new-tab button and the chevron still respond to clicks.
- [ ] 10.3 Behaviour:
  - `python` from `pwsh` titles the tab `python` (and stops once it exits), and from `cmd` the tab shows cmd's own title;
  - middle-click pastes the in-app selection without changing the clipboard, and ctrl-shift-c/v use the clipboard;
  - AltGr+7 on a German layout types `{` and doesn't open profile 7;
  - ctrl-clicking `https://example.com/a?b=1&c=2` opens that exact URL;
  - a background tab's OSC 9 shows a toast with Brindle's icon, and clicking it brings that tab's window to the front with the tab active;
  - a credential prompt from `git` in a tab centers on Brindle;
  - IME composition works in a terminal and in a palette;
  - wheel scrolling works in a local tab and in a WSL tmux pane.
- [ ] 10.4 Without the MSI: `scripts/fetch-conpty.ps1`, `cargo install --path .` and `brindle --install-desktop`. Check that the Start Menu entry launches `%USERPROFILE%\.cargo\bin\brindle.exe`, the tab host is the extracted `OpenConsole.exe`, toasts appear, and `--uninstall-desktop` removes the entry. Uninstalling the MSI leaves `%APPDATA%\brindle\config.toml` in place.
- [ ] 10.5 Run `openspec validate port-to-windows --strict` on Windows. Have the Linux session run `cargo test --locked`, `cargo build --release --locked` and the full with-xvfb e2e suite on the branch head. Check that both CI workflows are green. Report each result.

## Workflow follow-up

- Archive after review. `port-to-macos` also modifies `configuration`, `profiles`, `tabs` and `terminal-rendering`, so whichever change archives second rebases those MODIFIED blocks onto the first's.
- Code signing, winget and ARM64 stay open (design.md, Open Questions).
