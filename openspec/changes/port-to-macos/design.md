# Design

## Context

See proposal.md (Why) for the Linux-only spots. Facts from the GPUI 0.2.2 and alacritty 0.26 sources that shape the approach:

- **Shaders.** On macOS, GPUI's `build.rs` runs `xcrun -sdk macosx metal … -mmacosx-version-min=10.15.7` and `metallib` to precompile `shaders.metal`, unless the `runtime_shaders` feature is on. The `macos-blade` feature switches to Blade instead of Metal. Its own build script marks both features for deprecation (`//TODO: deprecate "runtime-shaders" and "macos-blade"`). The build also runs bindgen (dispatch bindings) and cbindgen (shader header), and both are already in `Cargo.lock`.
- **Decorations.** The macOS window never overrides `window_decorations()`, so it reports `Decorations::Server`. `start_window_move`, `show_window_menu` and `start_window_resize` are the trait's no-op defaults there, and `request_decorations(WindowDecorations::Client)` is ignored. `TitlebarOptions { appears_transparent: true }` gives a full-size content view with a transparent, title-less title bar. `traffic_light_position` moves the close, minimize and zoom buttons, and GPUI re-applies the position on resize and fullscreen changes. `titlebar_double_click()` reads `AppleActionOnDoubleClick` and minimizes, zooms or does nothing.
- **app_id.** `set_app_id` is a no-op on macOS. The Dock name and icon come from the bundle's `Info.plist`.
- **Keys.** Keybinding strings accept `cmd` (macOS ⌘), and `secondary` means cmd on macOS and ctrl elsewhere. For option-letter, GPUI reports `key = "f"`, `modifiers.alt = true` and `key_char = "ƒ"`, which is the character the layout produces. Brindle's `keys::to_esc_str` returns `None` for any `platform` (cmd) combination. Its Alt branch prefixes ESC to `key_char` when there is one, so on macOS today option-f would send `ESC ƒ`.
- **PTY.** alacritty's `tty::unix` supports macOS. When `Options.shell` is `None`, its macOS `default_shell_command` runs `/usr/bin/login -flp <user> /bin/zsh -fc "exec -a -<shell> <shell>"`, with `-q` when `~/.hushlogin` exists. The shell comes from the passwd entry (`pw_shell`), and this is how Terminal.app starts shells. Brindle always passes `Some(Shell)` today.
- **Process info.** `libc` 0.2.177 (locked) exposes `proc_name`, `proc_pidinfo`, `PROC_PIDVNODEPATHINFO` and `proc_vnodepathinfo` for Apple targets. `tcgetpgrp` is POSIX and works unchanged.
- **Icons.** `resvg` 0.45.1 and `tiny-skia` 0.11.4 (with PNG encoding on by default) are in `Cargo.lock` as GPUI dependencies, so Brindle can use them without changing the lockfile.

## Goals / Non-Goals

**Goals:**
- One codebase. Platform differences are behind `cfg(target_os = "macos")` in a few small modules, and Linux behaviour and tests are unchanged.
- An installed app that is indistinguishable whether it came from the `.dmg` or from `--install-desktop`.
- Every macOS claim checked in CI or on hardware before the change is archived.

**Non-Goals:**
- Native macOS tabs (`NSWindow` tabbing), or Brindle's tabs following the system's tab preference.
- Keeping the app running with no windows open. Closing the last window still quits (tabs spec), and Dock reopen is not handled.
- Secure Keyboard Entry, Services menu, `open -a Brindle <dir>` folder handling, and Touch Bar.
- Universal (fat) binaries. Releases ship one `.dmg` per architecture.
- Homebrew cask and Mac App Store distribution. A cask can follow once releases are notarized.

## Decisions

### 1. Metal with precompiled shaders; no GPUI feature changes

Use GPUI's default macOS path: Metal, with shaders compiled at build time by `xcrun metal`. The build then needs Xcode or the Command Line Tools with the Metal toolchain, and CI's macOS runners have it.

*Alternatives:*
- **`runtime_shaders`:** avoids the Xcode requirement but compiles shaders on every launch, and GPUI plans to deprecate it.
- **`macos-blade`:** goes through Blade and MoltenVK-style paths that GPUI also plans to deprecate.

Both alternatives would also change `Cargo.toml` features for every platform build.

### 2. Process info behind `terminal/process.rs`

Move the `/proc` code out of `Terminal` into a `process` module with two functions, `process_name(pid) -> Option<String>` and `process_cwd(pid) -> Option<PathBuf>`:
- **Linux:** today's `/proc/<pid>/comm` and `/proc/<pid>/cwd` reads.
- **macOS:** `proc_name` into a 2×`MAXCOMLEN` buffer for the name, and `proc_pidinfo(pid, PROC_PIDVNODEPATHINFO, …)` for the working directory, read from `pvi_cdir.vip_path`.

`foreground_pid` (`tcgetpgrp`) stays shared. Unit tests call both functions on the test process itself: the name is the test binary's, and the cwd equals `std::env::current_dir()`. So the macOS implementation is checked in CI without a GUI.

*Alternative:* `sysctl(KERN_PROC_PID)` gives the name but not the cwd, and the `libproc` crate adds a dependency for two calls.

### 3. Title bar: system traffic lights in Brindle's strip

On macOS, `open_window` sets `traffic_light_position: Some(point(px(12), px((TAB_BAR_HEIGHT - 14) / 2)))`, which centres the 14 px buttons in the 38 px strip. `WindowDecorations::Client` stays: it's ignored on macOS and still wanted on Linux.

`Workspace::render_tab_strip` adds a left inset of `TRAFFIC_LIGHT_INSET` (78 px) on macOS so tabs start to the right of the buttons. The inset goes to 0 in fullscreen (`window.is_fullscreen()`), where the buttons are hidden. Today the strip's mouse handlers sit behind `csd`, which is false on macOS. The macOS branch adds:
- **double-click on empty space:** calls `window.titlebar_double_click()`, so the system preference decides what happens;
- **drag to move:** relies on macOS's own handling of the transparent title-bar region, because GPUI has no window-move call on macOS.

The custom window buttons stay `csd`-only, so none are drawn.

*Alternative:* a server-drawn title bar above the tab strip. That's simpler, but it wastes about 28 px and looks unlike every other Mac terminal.

### 4. Keybindings: a mechanical cmd mapping applied to one table

`default_bindings()` stays the single table. On macOS each entry goes through `mac_key(&str) -> Cow<str>` before `KeyBinding::new`:
- `ctrl-shift-<k>` becomes `cmd-<k>`;
- `ctrl-<punct|0|insert>` becomes `cmd-<…>`;
- `alt-<1-9>` becomes `cmd-<1-9>`;
- `ctrl-alt-<1-9>` becomes `cmd-alt-<1-9>`;
- in `TmuxWindow`, `alt-<arrow>` becomes `cmd-alt-<arrow>`;
- everything else is unchanged: `ctrl-tab`, `shift-pageup` and so on, and the picker's ctrl-p and ctrl-n.

Three cases are handled outside the mapping: `ctrl-shift-space` becomes `cmd-shift-space` (cmd-space is Spotlight), `ctrl-shift-tab` is left alone (cmd-tab is the app switcher), and macOS also gets `cmd-shift-[` and `cmd-shift-]` for previous and next tab. The mapping is a pure function with unit tests that run on every platform. Applying it to the whole table means new actions get Mac keys automatically.

The resulting macOS keys:

| Action | macOS |
|---|---|
| New / close tab, new window, quit | cmd-t / cmd-w, cmd-n, cmd-q |
| Copy / paste / select all | cmd-c / cmd-v / cmd-a (also cmd-insert, shift-insert) |
| Command palette, new-tab palette | cmd-p, cmd-shift-space |
| Reload config, open config | cmd-r, cmd-, |
| Clear scrollback | cmd-k |
| Font size | cmd-= / cmd-+ / cmd-- / cmd-0 |
| Prev / next tab | cmd-shift-[ / cmd-shift-], ctrl-tab / ctrl-shift-tab, ctrl-pageup/down |
| Move tab | cmd-pageup / cmd-pagedown |
| Jump to tab / profile N | cmd-N / cmd-alt-N |
| tmux split right / down, close, zoom, detach | cmd-e / cmd-o, cmd-x, cmd-z, cmd-d |
| tmux pane focus | cmd-alt-arrows |
| Scroll line | cmd-up / cmd-down |

`Swallow` on cmd-d is harmless, because cmd keys never reach the PTY (`to_esc_str` bails on `platform`), so it's kept for uniformity. User `[keybindings]` are unchanged: users write `cmd-…` themselves.

*Alternative:* keep ctrl-shift on macOS (like kitty's default). Rejected because cmd-c and cmd-v are the one thing every Mac user tries first.

### 5. Menu bar from the same actions

At startup on macOS, `cx.set_menus(...)` builds the menus listed in the spec from existing actions, and GPUI shows each item's binding. About is the system about panel, with name and version from the bundle. Quit is the existing `Quit` action. Menu items dispatch through the focused window, so tmux-only items aren't in the menu. They stay in the command palette.

### 6. Option key: pass through or Meta, chosen by config

Add `macos_option_as_meta: bool` (default `false`) to `Config`. It is accepted and ignored on other platforms, so a shared config doesn't produce errors. In `keys::to_esc_str`, the Alt branch on macOS is:
- **off:** return `None` for Alt with a printable key. The keystroke falls through to the text-input path and the layout's character (`ƒ`, `@`, …) is inserted, exactly as plain typing is.
- **on:** encode ESC plus the unmodified `key` (shifted when shift is held), not `key_char`. Otherwise option-f would send `ESC ƒ`.

Special keys (arrows, Home/End, F-keys, enter, backspace) keep the Alt modifier encoding either way. The encoder gains an `option_as_meta` parameter, so this stays a pure, unit-tested function on all platforms.

*Alternative:* left Option as Meta and right Option as compose (iTerm2-style). GPUI's `Modifiers` doesn't distinguish left from right Option, so that would need GPUI changes.

### 7. Config path: XDG layout on macOS too

`Config::path` uses `$XDG_CONFIG_HOME` or `~/.config` on macOS instead of `dirs::config_dir()`. Reasons:
- the configuration spec already says XDG;
- dotfile repos can share one file between Linux and Mac;
- alacritty, kitty and wezterm all read `~/.config` on macOS.

Linux is unchanged, and Windows is out of scope here.

### 8. Shell startup and PATH

- **Login shell:** on macOS, `spawn_pty` passes `shell: None` when the profile sets no command and there's no `-e`. alacritty then uses `login -flp`, so the shell is a login shell, `.zprofile` runs, and the session gets a proper tty and utmp entry. Profiles with an explicit `command` behave as on Linux.
- **PATH import:** this runs only when `getppid() == 1`, which is how a Finder, Dock, Spotlight or `open` launch looks: launchd is the parent and the environment is launchd's minimal one. Before `Application::new()`, Brindle runs `<pw_shell> -l -c 'printf %s "$PATH"'` with stdin closed and a 2 s timeout. If the output is non-empty, it becomes `PATH` for the process, so the PTY children and `tmux -C` inherit it. On failure or timeout, `/opt/homebrew/bin` and `/usr/local/bin` are appended if missing, and a warning is logged. A non-interactive `-l` avoids prompts and `.zshrc` noise, and the cost is one shell start, only on GUI launches.

*Alternative:* `-l -i` would also pick up PATH changes in `.zshrc`, but interactive startup can print output or block on prompts (oh-my-zsh updates). Users who set PATH only in `.zshrc` get the fallback.

### 9. Fonts

On macOS, `resolve_family` tries `SF Mono` and then `Menlo` before its Linux fallback list. `FALLBACK_FAMILIES` gains a macOS list: `Symbols Nerd Font Mono`, `Menlo`, `Apple Symbols`, `Apple Color Emoji`, `PingFang SC` and `Hiragino Sans`. The default config's `font.family` comment notes that Menlo is the Mac fallback, and the list itself stays as it is.

### 10. Bundle layout and one shared assembler

```
Brindle.app/Contents/
  Info.plist          CFBundleName=Brindle, CFBundleDisplayName=Brindle,
                      CFBundleIdentifier=io.github.scmccart.brindle,
                      CFBundleExecutable=brindle, CFBundleIconFile=brindle,
                      CFBundleShortVersionString/CFBundleVersion=<crate version>,
                      LSMinimumSystemVersion=10.15.7, NSHighResolutionCapable=true,
                      LSApplicationCategoryType=public.app-category.developer-tools
  MacOS/brindle
  Resources/brindle.icns
```

`src/macos_app.rs` owns this layout, the `Info.plist` template and an `.icns` writer. `--install-desktop` and the release script both use it: `scripts/macos/bundle.sh` runs `target/release/brindle --install-desktop --prefix <stage>` and takes `<stage>/Applications/Brindle.app`. So there's exactly one implementation of the bundle, and no hidden options.

The bundle identifier uses the repo owner's GitHub namespace. It can change before the first public release, and only `Info.plist` and the ownership check depend on it.

### 11. Icon rendered from the SVG at runtime, no committed binaries

The `.icns` is generated from `include_bytes!("../assets/brindle.svg")`:
- `resvg` renders PNGs at 16, 32, 64, 128, 256, 512 and 1024 px;
- they're wrapped in the ICNS container: an `icns` magic and big-endian length, then `icp4`, `icp5`, `ic12`, `ic07`, `ic08`, `ic09` and `ic10` chunks of PNG data.

The format is a few dozen lines to write, so neither `iconutil` nor a committed `.icns` is needed. The SVG stays the only source of truth, the Linux-side icon work shares it, and the writer can be unit-tested on Linux by parsing the chunk table back. Brindle adds `resvg = { version = "0.45", default-features = false }` and reaches `tiny_skia` through `resvg::tiny_skia`. The SVG has no text or raster images, so the disabled features aren't needed.

*Alternatives:*
- **Committed `.icns`:** a binary that drifts from the SVG.
- **`iconutil` plus `rsvg-convert` in a script:** `cargo install` users wouldn't have `rsvg-convert`.

### 12. `--install-desktop` on macOS copies and re-signs

On macOS, `--install-desktop` and `--uninstall-desktop`, the options `add-linux-desktop-install` adds, act on `<prefix>/Applications/Brindle.app`. `<prefix>` defaults to `$HOME`, which mirrors Linux's `<prefix>/share`. Reusing the Linux option names gives one documented install command on every platform.

`--install-desktop`:
1. builds the bundle in a temp directory next to the target (`<prefix>/Applications/.Brindle.app.tmp`);
2. copies `std::env::current_exe()` (canonicalized) to `MacOS/brindle`;
3. ad-hoc signs the bundle with `/usr/bin/codesign --force --sign - <bundle>`, which ships with macOS;
4. renames it over the old bundle, after the ownership check (spec: never replace an app Brindle did not create);
5. finally runs `lsregister -f` (from LaunchServices.framework) on the bundle if it exists, so Spotlight and Launchpad pick it up immediately.

Signing is needed because Apple Silicon refuses to run a modified or unsigned executable. The copied linker signature doesn't cover the bundle's `Info.plist` and resources.

`<prefix>/Applications` is created if missing.

*Alternatives:*
- **A symlink to the cargo binary:** breaks the bundle's code signature and LaunchServices' executable lookup.
- **A shell-script launcher:** the process wouldn't be the bundle's executable, so the Dock shows a generic icon named "brindle".
- **Separate `--install-app` / `--uninstall-app` options:** users and docs would need a different install command per platform for the same intent.

### 13. Disk image and CI

`scripts/macos/dmg.sh` stages `Brindle.app` plus an `Applications -> /Applications` symlink and runs `hdiutil create -volname Brindle -format UDZO -srcfolder <stage> Brindle-<ver>-<arch>.dmg`.

A new `.github/workflows/macos.yml` runs on `macos-14` (arm64) and `macos-13` (x86_64):
- `cargo build --release --locked`;
- `cargo test --locked`;
- `bundle.sh`, `dmg.sh`, and uploading the `.dmg` as an artifact;
- the same steps on `v*` tags, which also attach the `.dmg` to the release.

`--locked` makes CI fail rather than silently change `Cargo.lock`.

## Risks / Trade-offs

- **[Dragging the transparent title-bar region may not move the window when GPUI's content view handles the mouse-down]** GPUI 0.2.2 has no macOS window-move call. → Mitigation: this is verified first on hardware (tasks 2.4). If dragging fails, fall back to a server-drawn title bar (`appears_transparent: false`) with the tab strip below it, and record that here.
- **[With Option as compose, the text-input path might not deliver option-letter characters after `key_down` declines them]** → Mitigation: verify on hardware with US and German layouts. If needed, insert `key_char` directly from `key_down` for Alt with a printable key and no IME composition.
- **[Unsigned or ad-hoc-signed `.dmg` downloads are quarantined by Gatekeeper]** ("cannot be opened because the developer cannot be verified"). → Mitigation: README documents right-click → Open, or `xattr -d com.apple.quarantine`. Notarization is an open question. `--install-desktop` builds are local, so they aren't quarantined.
- **[cmd-x, cmd-z and cmd-p differ from their usual Mac meanings]** (cut, undo, print). → Accepted: a terminal has no cut, undo or print, and these mirror Brindle's existing letters. Users can rebind them.
- **[PATH import adds one login-shell start to GUI launches]** → Bounded by the 2 s timeout, and skipped entirely when launched from a terminal.
- **[`lsregister` lives at a private framework path that could move]** → It's called only if present and only as a nudge; LaunchServices finds the app on its own eventually.

## Migration Plan

None for Linux users. On macOS, `--install-desktop` is idempotent, and `--uninstall-desktop` removes only the bundle. Rollback means deleting the bundle.

## Open Questions

- **Signing and notarization:** buy an Apple Developer ID ($99/year) to sign and notarize release `.dmg`s? That only adds a CI signing step and secrets, and changes no spec.
- **Bundle identifier:** confirm `io.github.scmccart.brindle` before the first public release, or choose a domain-based one.
- **Universal binary:** decide later whether per-architecture `.dmg`s or one universal (`lipo`) `.dmg` is preferable.
- **`--prefix` meaning across platforms:** it is `<dir>/share` on Linux (`add-linux-desktop-install`), `<dir>/Applications` here, and rejected with exit 2 by `port-to-windows`. Decide whether to keep one flag with per-platform meaning or rename it, before either port is applied.
- **Primary selection:** GPUI 0.2.2 defines `write_to_primary` / `read_from_primary` only for Linux and FreeBSD, and Brindle calls them in three places. This change doesn't yet say how macOS handles them. `port-to-windows` uses an in-app buffer shared by all windows. Decide whether macOS does the same or drops the primary selection, and add the matching spec and tasks.
- **Title bar drag:** GPUI's `start_window_move` and `show_window_menu` do nothing on macOS. Whether dragging the transparent title-bar area moves the window natively is unverified, and needs checking on a Mac before the fallback in this design is chosen.
- **Archive order:** this change adds to `desktop-integration`, so `add-linux-desktop-install` must be archived first. MODIFIED blocks shared with `port-to-windows` need rebasing onto whichever change archives first.
