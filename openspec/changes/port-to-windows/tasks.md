# Tasks

Prerequisite: `add-linux-desktop-install` must be applied first, because group 5 extends its `src/desktop.rs` and its `--install-desktop` options. Nothing in this change can be built or run on the Linux dev machine for Windows. Windows verification goes through the CI job from 1.1, plus the manual steps in group 6 on a real Windows 11 machine. Keep `Cargo.lock` stable: never run `cargo update`, and use `--locked` in CI.

## 1. Windows CI and a compiling build

- [ ] 1.1 Add `.github/workflows/windows.yml` (design.md §12). For now it runs `cargo build --release --locked` and `cargo test --locked` on `windows-latest`, and leaves out the MSI steps. Verify that the workflow parses (`gh workflow view` after pushing a branch) and that the job runs. It is expected to fail to compile at this point; record the errors.
- [ ] 1.2 Create `src/platform/{mod,unix,windows}.rs` (design.md §1). Move the PTY handle, foreground process name, working directory, default program and editor command into `unix.rs` without changes, and make `libc` a `cfg(unix)` dependency. Verify with `cargo test` and `cargo build --release` on Linux, with no behaviour change: `--send 'sleep 3\r' --dump-screen-after 2` still titles the tab `sleep`. Warn the user before the GUI run.
- [ ] 1.3 Implement the Windows `pty_handle`, using `child_watcher().pid()` and setting `escape_args: true`. Add stubs: `foreground_process_name` returns the child's name or `None`, and `working_directory` returns `None`. Add `windows` 0.61 as a `cfg(windows)` dependency, with only the features that are needed. Verify that `git diff Cargo.lock` adds only dependency edges and no package versions, and that the CI job compiles. Fix any further Unix-only code the job reports.
- [ ] 1.4 Add the primary-selection wrapper (design.md §4): an app-global buffer on Windows that forwards to GPUI on Unix. Route OSC 52 selection reads and writes, `finish_selection` and `paste_primary_selection` through it. Unit-test the Windows buffer's logic behind a cfg-free inner type, so it runs on Linux too. Verify with `cargo test` on Linux, and on CI with a compiling Windows build and passing tests.

## 2. Windows behaviour

- [ ] 2.1 Write the Toolhelp tab-title walk (design.md §2) as a pure function over `(pid, parent_pid, exe)` rows. Unit-test it with no children, a chain, siblings picking the highest pid, and `.exe` stripping. Wire it in with one snapshot per title-poll tick shared by all tabs. Verify with `cargo test` (Linux and CI).
- [ ] 2.2 Write the Windows `default_program` (design.md §5) as a pure function of a `PATH` value and a `COMSPEC` value. Unit-test the four fallback steps. Update the doc comment on `Profile::command` and the default config's comments. Verify with `cargo test`.
- [ ] 2.3 Write the Windows `editor_command`: `cmd.exe /C` with `%VISUAL%`, then `%EDITOR%`, then `notepad.exe`, and correct quoting for paths with spaces. Unit-test the generated argument vectors. Verify with `cargo test`.
- [ ] 2.4 Make the `tmux::command_line` change for WSL (design.md §7): omit `-c` when the program's stem is `wsl`, compared case-insensitively. Unit-test `wsl`, `WSL.EXE`, `tmux` and a full path. Add a commented WSL control-mode profile to `assets/default-config.toml`. Verify with `cargo test tmux`.
- [ ] 2.5 Add the font fallbacks (design.md §6): `Cascadia Mono` and `Consolas` at the end of the terminal fallbacks and of the default `font.family`, and `Segoe UI Variable Text` and `Segoe UI` in the UI list. Verify with `cargo test`, including the config defaults test, and confirm that Linux font resolution is unchanged.
- [ ] 2.6 Add Windows caption controls to the tab strip (design.md §3). Show the minimize, maximize and close buttons under `cfg!(windows)`, and mark the empty strip and the buttons with `window_control_area`. Verify that the Linux build is unchanged visually (warn before the GUI run, then dump), and that the CI build compiles.

## 3. Icons

- [ ] 3.1 Add `scripts/render-icons.sh` (design.md §9). It renders `assets/brindle.svg` at 16, 20, 24, 32, 40, 48, 64 and 256 px with whichever of `rsvg-convert`, `resvg` or `magick` is present, and packs them into `assets/windows/brindle.ico`, with the 256 px image PNG-compressed. If no rasterizer is installed, ask the user to install one; don't add a Rust dependency. Commit the `.ico`. Verify that `file assets/windows/brindle.ico` reports 8 icons, and view the 16 and 32 px PNGs.
- [ ] 3.2 Add a CI check that fails when `assets/brindle.svg` is newer in git history than `assets/windows/brindle.ico`. Verify by temporarily touching the SVG on a branch.

## 4. Executable resources

- [ ] 4.1 Add `assets/windows/brindle.rc` with `1 ICON` and a `VERSIONINFO` block fed by `/D` version macros. Add a `build.rs` that compiles it with `embed-resource` only when `CARGO_CFG_TARGET_OS == "windows"`, and declare `embed-resource = "3"` under the Windows target's build-dependencies (design.md §8). Verify that the Linux build is unaffected and `Cargo.lock` gains no new versions.
- [ ] 4.2 On CI, check that `brindle.exe` contains icon resource 1 and a version resource. A PowerShell step can read `(Get-Item brindle.exe).VersionInfo` and use `[System.Drawing.Icon]::ExtractAssociatedIcon`, or the job can use `rcedit`/`ResourceHacker`. Verify that the step passes.
- [ ] 4.3 On CI, check that GPUI's manifest is embedded in `brindle.exe` (`mt.exe -inputresource:brindle.exe;#1 -out:m.xml` contains `PerMonitorV2`). If it is missing, apply the fallback in design.md §8 (disable `windows-manifest`, and put the manifest in `brindle.rc`), then verify again.

## 5. Install experience

- [ ] 5.1 Add the Windows `--install-desktop` / `--uninstall-desktop` to `src/desktop.rs` (design.md §11). It writes the `IShellLinkW` shortcut to `FOLDERID_Programs`. Reject `--prefix` with exit status 2 and a message pointing to the MSI. Unit-test the argument handling in a cfg-free way. On CI, run both commands and check with PowerShell that the `.lnk` appears and disappears, including uninstalling when it is already absent.
- [ ] 5.2 Add `packaging/windows/brindle.wxs` (design.md §10). It needs a per-machine install, a stable `UpgradeCode`, `MajorUpgrade`, a Start Menu shortcut, `ARPPRODUCTICON`, an RTF licence, and the optional `PathFeature`. Add MSI build, quiet install and uninstall to the CI job, and upload the MSI as an artifact. Verify on CI after install that the file exists, the shortcut exists and the uninstall registry key exists, and after uninstall that all three are gone and `%APPDATA%\brindle` is untouched.
- [ ] 5.3 Add an upgrade check to CI. Build the MSI twice with versions `0.0.1-ci` and the real version, install the first and then the second, and assert that exactly one Brindle uninstall key remains. Verify that the step passes.
- [ ] 5.4 Add a Windows section to the README covering:
  - build prerequisites: MSVC, the Windows SDK, `GPUI_FXC_PATH`, and WiX for the MSI;
  - installing from the MSI;
  - `cargo install` followed by `brindle --install-desktop`, and not to use both routes;
  - the SmartScreen warning for unsigned builds;
  - the WSL tmux profile.

  Verify that every command in the section matches the code and the CI workflow.

## 6. Manual verification on Windows 11

Run these on real hardware, using the CI MSI artifact. Report the results to the user; they can't be automated here.

- [ ] 6.1 Install the MSI, then launch from the Start Menu. Check that:
  - the taskbar, Alt-Tab and title area show the Brindle icon;
  - the window is sharp at 150% scaling;
  - the default tab runs `pwsh` or `powershell`.
- [ ] 6.2 Check the title bar against the `tabs` spec:
  - drag the strip to move the window, and double-click it to maximize;
  - right-click for the system menu;
  - drag to a screen edge to Snap, and hover maximize for Snap Layouts;
  - the minimize, maximize and close buttons work, and closing the last window exits;
  - edge resizing works.
- [ ] 6.3 Tab titles and selection:
  - running `python` from `cmd` titles the tab `python`;
  - middle-click pastes the in-app selection without changing the clipboard;
  - ctrl-shift-c and ctrl-shift-v use the system clipboard;
  - AltGr characters (German layout, AltGr+7 = `{`) type, and don't open profile 7.
- [ ] 6.4 tmux through WSL: with WSL and tmux installed, a `command = "wsl"`, `tmux_args = ["tmux", "-L", "brindle-e2e", "-f", "/dev/null"]` control profile attaches, with windows as tabs and panes drawn natively. Kill that server afterwards.
- [ ] 6.5 Run `cargo install --path .` and then `brindle --install-desktop`. The Start Menu shows Brindle, and it launches `%USERPROFILE%\.cargo\bin\brindle.exe`. `--uninstall-desktop` removes it. Uninstalling the MSI leaves `%APPDATA%\brindle\config.toml` in place.
- [ ] 6.6 Run `openspec validate port-to-windows --strict`, and `cargo test` and `cargo build --release` on Linux, to confirm that Linux didn't regress.
