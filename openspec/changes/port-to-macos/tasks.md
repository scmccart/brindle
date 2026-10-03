# Tasks

None of this can run on the Linux dev machine. "CI" means the macOS job from group 1, and "on a Mac" means a manual check on real hardware, done by the user or on a Mac they provide. Pure helpers stay platform-independent so their unit tests also run on Linux. Every group must leave `cargo test` and `cargo build --release` passing on Linux.

## 1. macOS builds in CI

- [ ] 1.1 Add `.github/workflows/macos.yml` (design.md §13) with `macos-14` and `macos-13` jobs running `cargo build --release --locked` and `cargo test --locked`. Fix whatever fails to compile on macOS with the smallest `cfg` gates possible, leaving behaviour changes to later groups. Verify both jobs are green and `git diff Cargo.lock` is empty.
- [ ] 1.2 On a Mac, run `cargo run --release` from Terminal.app. Verify that a window opens, a shell prompt appears, `echo hi` works, and closing the window quits. Record the macOS version and chip in the PR description.
- [ ] 1.3 Add a "Building on macOS" subsection to README.md covering Xcode or the Command Line Tools plus the Metal toolchain (`xcrun metal` must work), and note in CLAUDE.md that platform code lives behind `cfg(target_os)`. Verify the README steps match what CI runs.

## 2. Process info and title bar

- [ ] 2.1 Create `src/terminal/process.rs` with `process_name` and `process_cwd`: the Linux `/proc` versions moved from `src/terminal/mod.rs`, plus the macOS libproc versions (design.md §2). Unit-test both on the current process (name equals the test binary's name, cwd equals `current_dir()`). Verify with `cargo test terminal::process` on Linux and in CI.
- [ ] 2.2 On macOS, set `traffic_light_position` in `open_window` and add the left inset `TRAFFIC_LIGHT_INSET` to the tab strip, 0 in fullscreen (design.md §3). Verify with a clean CI build.
- [ ] 2.3 Add the macOS strip handler: double-click on empty space calls `window.titlebar_double_click()`. Keep the custom window buttons `csd`-only. Verify with a clean CI build.
- [ ] 2.4 On a Mac, check:
  - the traffic lights sit centred in the strip, with no tab under them;
  - dragging empty strip space moves the window;
  - double-click follows the System Settings › Desktop & Dock "Double-click a window's title bar" choice (try Zoom and Minimize);
  - fullscreen removes the inset.

  If dragging doesn't move the window, apply the fallback in design.md's Risks and update design.md §3.
- [ ] 2.5 On a Mac, verify the tab-title and working-directory scenarios from specs/macos-platform ("Foreground process title", "New tab in the same directory").

## 3. Keybindings and menu bar

- [ ] 3.1 Add the pure `mac_key` mapping (design.md §4), covering the exceptions (`ctrl-shift-space` and `ctrl-shift-tab`) and the extra cmd-shift-[ and cmd-shift-] bindings. Apply it in `default_bindings()` under `cfg(target_os = "macos")`. Unit-test every rule and exception on all platforms, plus one test asserting that no mapped default binding contains `ctrl-shift`. Verify with `cargo test actions`.
- [ ] 3.2 Build the menu bar with `cx.set_menus` from existing actions (design.md §5). Verify with a clean CI build.
- [ ] 3.3 On a Mac, verify the spec scenarios: "Mac copy and paste", "Open a tab", "Control keys still reach the shell" (`cat -v`, ctrl-c), "Jump to a tab", "Next tab", "Quit from the menu" and "Settings opens the config". Also check that the menus show their shortcuts and that the command palette lists cmd shortcuts.
- [ ] 3.4 Add the macOS column to README.md's key table (or a separate macOS table). Verify every key in it against `mac_key`'s tests.

## 4. Option key

- [ ] 4.1 Add `macos_option_as_meta` to `Config` (accepted on all platforms, used on macOS) and a commented entry in `assets/default-config.toml`. Verify with a config unit test that the key parses and an unknown-key test still fails for typos.
- [ ] 4.2 Give `keys::to_esc_str` an `option_as_meta` parameter (design.md §6). When off on macOS, return `None` for Alt with a printable key. When on, encode ESC plus the unmodified key. Special keys keep their Alt encoding. Unit-test option-f both ways, shift-option-f, option-left (`ESC [ 1 ; 3 D`) and option-enter. Verify with `cargo test terminal::keys`.
- [ ] 4.3 On a Mac, with US and German layouts, verify the "Option key on macOS" scenarios, including live reload of the setting. If option-letter text doesn't arrive through the input path, apply the mitigation in design.md's Risks and add a test for it.

## 5. Config path, shell, PATH and fonts

- [ ] 5.1 Make `Config::path` use `$XDG_CONFIG_HOME` or `~/.config` on macOS (design.md §7). Factor the decision into a pure function of (os, env, home) and unit-test it on all platforms. Verify with `cargo test config`.
- [ ] 5.2 On macOS, pass `shell: None` to alacritty when the profile has no command and there is no `-e` (design.md §8). Verify with a clean CI build, and on a Mac that `echo $0` prints `-zsh` (or the user's shell with a leading `-`) and that a `~/.zprofile` variable is set.
- [ ] 5.3 Add the macOS PATH import (design.md §8): only when `getppid() == 1`, with a 2 s timeout and the Homebrew fallback. Put the output parsing and fallback merge in a pure function, unit-tested on all platforms (empty output, timeout, already-present dirs). Verify with `cargo test`. The GUI-launch check comes in group 6, once the app can be opened from Finder.
- [ ] 5.4 Add the macOS font fallbacks and default family (design.md §9). Verify on a Mac that, with `font.family = ["Nonexistent"]`, text is drawn in Menlo, and that `printf '🐕\n'` shows a color emoji.
- [ ] 5.5 Document the macOS config path, the login-shell behaviour and `macos_option_as_meta` in README.md, and the macOS config location in `assets/default-config.toml`'s header comment. Verify the README matches the unit-tested behaviour.

## 6. App bundle, install options and disk image

- [ ] 6.1 Add `resvg = { version = "0.45", default-features = false }` (design.md §11). Verify `cargo build --locked` passes and `git diff Cargo.lock` adds only the new direct-dependency line for `brindle`, with no version changes.
- [ ] 6.2 Create `src/macos_app.rs` with the `Info.plist` template, the bundle layout and the `.icns` writer (design.md §10–11). Compile the platform-independent parts (plist, icns) on every OS. Unit-test that the plist has the identifier, version and executable keys, and that the generated `.icns` parses back into the expected chunk types, with PNG dimensions matching each chunk. Verify with `cargo test macos_app` on Linux.
- [ ] 6.3 Implement `--install-desktop` / `--uninstall-desktop`, with `--prefix`, on macOS against `src/macos_app.rs` (design.md §12). Include the ownership check, the temp-dir-then-rename install, `codesign --force --sign -` and the optional `lsregister`. This needs `add-linux-desktop-install`'s option parsing in place. Unit-test, on all platforms, target-path resolution (default `$HOME/Applications`; `--prefix /` gives `/Applications`) and the ownership check against a fixture `Info.plist`. Verify with `cargo test`.
- [ ] 6.4 Add `scripts/macos/bundle.sh` and `scripts/macos/dmg.sh` (design.md §13). Extend the CI job to run them and upload `Brindle-<ver>-<arch>.dmg`, and attach the `.dmg` to releases on `v*` tags. Verify the CI artifact mounts, contains `Brindle.app` and an Applications link, and that `codesign --verify --deep Brindle.app` passes in CI.
- [ ] 6.5 On a Mac, verify the desktop-integration macOS scenarios:
  - "Open from Finder", "Found by Spotlight" and "Install from the disk image", from the CI `.dmg`, using right-click → Open for the quarantine prompt;
  - "Install after cargo install", "Reinstall after upgrade", "System-wide install", "Unrelated app in the way", "Uninstall" and "Nothing to uninstall".

  Then check "Homebrew tmux from the Dock" and "Started from a terminal" from specs/macos-platform.
- [ ] 6.6 Add a "Installing on macOS" section to README.md covering the `.dmg` (including the Gatekeeper right-click → Open step) and `cargo install --path . && brindle --install-desktop`. Verify the commands as written on a Mac.

## 7. Integration

- [ ] 7.1 On a Mac, run a tmux control-mode session (Homebrew tmux, throwaway server `-L brindle-e2e -f /dev/null`) from the installed app. Check attach, splits (cmd-e, cmd-o), pane focus (cmd-alt-arrows), detach (cmd-d) and reattach, then kill the server.
- [ ] 7.2 Confirm Linux is unchanged: run `cargo test`, `cargo build --release`, and a `--dump-screen-after` smoke run on Linux, warning the user before opening a window. The default bindings on Linux must still be ctrl-shift-*: check this with `--list-actions` and the existing tests.
- [ ] 7.3 Run `openspec validate port-to-macos --strict` and confirm both CI jobs are green on the final commit.
