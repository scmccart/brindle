# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Brindle is a GPU-accelerated terminal emulator for Linux and Windows 11, in Rust. It is built on GPUI 0.2.2 (Zed's toolkit, from crates.io) and `alacritty_terminal` 0.26, with tabs, profiles, and tmux in two modes: classic, and control mode (`tmux -C`). See README.md for user-facing behaviour and keybindings.

## Build and test

Build dependencies are the system packages listed in README.md. Wayland, fontconfig and Vulkan are dlopen'd at runtime, so they have no `-dev` packages.

```sh
cargo build                          # deps built at opt-level 2 (Blade is unusable unoptimized)
cargo build --release                # stripped, thin LTO
cargo test                           # all unit tests (no GUI needed; the e2e suite skips itself)
cargo test tmux::layout              # one module
cargo test collector_matches_the_spike_transcript   # one test
```

- **`Cargo.lock` was seeded from gpui 0.2.2's own lockfile.** Do not regenerate it, and avoid broad `cargo update`: a newer `libc` breaks `xattr`. Add new dependencies with care.
- The GPUI and alacritty sources to read for API questions are in `~/.cargo/registry/src/index.crates.io-*/gpui-0.2.2` and `.../alacritty_terminal-0.26.0`.
- Zed's `terminal`/`terminal_view` crates are GPL-3.0. Use them only to learn the API shape and never copy code from them: Brindle is MIT OR Apache-2.0.

### Windows

The Windows port (OpenSpec change `port-to-windows`) builds on `x86_64-pc-windows-msvc` with the MSVC toolchain and the Windows 11 SDK.
- The same `cargo build`, `cargo build --release` and `cargo test` commands work in PowerShell or Git Bash. The release build needs the SDK's `fxc.exe`: on `PATH`, in SDK 10.0.26100.0, or named by `GPUI_FXC_PATH`.
- Platform code is behind `cfg`: `target_os = "linux"` for D-Bus, Wayland and X11 captures, `unix` for `libc`, `windows` for the `windows` crate. `src/win.rs` holds small Windows helpers. Process info is in `terminal/process.rs` (the seam `port-to-macos` uses), and the primary selection is in `src/primary.rs`.
- `.gitattributes` keeps every text file LF, because `include_str!` embeds assets and shell scripts.
- The Windows icons (`assets/windows/brindle.ico` and `brindle-256.png`, for toasts) are rendered from `assets/brindle.svg` by `scripts/render-icons.sh` and committed; re-run it after changing the SVG.
- On Windows `BRINDLE_E2E=1 cargo test --test e2e` lists only the Windows cases (`tests/e2e/cases/win.rs`: the ConPTY host and fallback, passthrough, glyph widths, closing mid-output, titles, PowerShell integration, the caption hit-test, WSL tmux), and `tests/perf` runs only its self-test.
- Pitfalls found on the way:
  - `--send` turns `\t`, `\e`, `\r` and `\n` into controls, so Windows paths in steps use `/`.
  - `brindle.exe` is a GUI program: PowerShell doesn't wait for it unless its output is piped, or it is run with `Start-Process -Wait`.
  - Elevated PowerShell (as on CI) sets its own console title.
  - GPUI answers `WM_NCHITTEST` for the last mouse position, so probes send a `WM_MOUSEMOVE` to the window first. That is a message, not input.
  - Windows acts on a caption press (drag, double-click, the caption buttons) only when GPUI leaves it unhandled, and GPUI's focus transfer counts as handling it. So the tab strip occludes the focusable workspace, and the caption buttons have no handlers there.
  - The shell reads no icon through a symlink, such as WinGet's `Links\brindle.exe`, so the shortcut's icon location is the resolved exe.
  - Test processes aren't DPI-aware, so Windows scales down the window rectangles they read. Probes call `SetThreadDpiAwarenessContext(PER_MONITOR_AWARE_V2)` first to work in Brindle's physical pixels.
  - Pure helpers that read Windows paths and are tested on Linux can't use `Path` (it splits only on `/` there); see `cmdline::program_stem`.
  - Commit subjects must start lowercase, so not with "Windows" or "PowerShell".
- **Linux checks from a Windows machine:** push the branch, then ask the Linux machine's Claude session, reachable over Remote Control, to fetch it and run `cargo test --locked`, `cargo build --release --locked` and `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e`. Its user approves each run. The Linux CI jobs run the same commands on every push.
- GUI runs on a Windows desktop (any Brindle window, `--dump-screen-after` included) steal focus as on Linux `:0`, so ask first. There is no `scripts/with-xvfb` equivalent on Windows.

## Commit messages

Commits follow Conventional Commits, checked by commitlint (`commitlint.config.mjs`):
- Subject is `type(scope)!: subject`. Types: `feat`, `fix`, `docs`, `test`, `ci`, `build`, `refactor`, `perf`, `style`, `chore`, `revert`.
- Scopes are lower-case and name the area: `tmux`, `e2e`, `settings`, `shell-integration`, `deps`.
- The subject starts lowercase, except for an acronym (`docs: README, …`). Headers are at most 100 characters.
- `!` marks a breaking change (a changed key, config or default).
- OpenSpec propose, archive and check-off commits are `docs(openspec): …`. Merges keep `Merge …`, which the linter skips.
- `git config core.hooksPath scripts/hooks` (set in this clone) runs `scripts/lint-commits --edit` as a `commit-msg` hook. `scripts/lint-commits` alone checks the commits not on `origin/main`; `scripts/lint-commits BASE` checks `BASE..HEAD`.
- It runs commitlint through `npx` (Node ≥ 22.12), with `COMMITLINT_VERSION` pinned in the script and moved by hand. To vet a bump, run `scripts/lint-commits 7157fba` (the root commit) over the whole history.

## Verifying GUI behaviour

**Never inject synthetic input (XTest, xdotool) on this desktop.** It goes to the user's focused window, not Brindle. Opening a window can also steal focus while the user is typing, so ask before GUI runs unless they've said they're away. Use the built-in debug hooks instead:

```sh
BRINDLE_CONFIG=/tmp/test-config.toml ./target/release/brindle -p tmux \
  --send 'echo hi\r' --action tmux_split_right --dump-screen-after 5
```

- `--send` and `--action` steps run in order, one second apart (`--step-interval S` changes that), against the active tab. Action names are listed by `--list-actions`. While a palette or the settings dialog has a text input focused, `--send` types into that input (control characters dropped) instead of the terminal; use `--action` (`picker_confirm`, `text_input_left`, …) for keys.
- `BRINDLE_NOTIFY_LOG=<file>` appends each desktop notification to the file (`source<TAB>title<TAB>body`) instead of sending it over D-Bus.
- `BRINDLE_TMUX_STATE=poll|subscribe` overrides how control-mode sessions follow pane state (see "Subscriptions or polling" below). The e2e suite passes it through, so `BRINDLE_TMUX_STATE=subscribe BRINDLE_E2E=1 cargo test --test e2e` runs the subscription path on any tmux.
- `--dump-screen-after N` prints the tab titles, the active terminal's size and `TermMode`, its grid geometry (`--- grid origin=X,Y cell=WxH scale=S`, logical window pixels), and its screen text, then exits. An open overlay adds its `--- palette` or `--- settings` line (palette rows print as `label  detail  [shortcut]`), and a focused text input adds `--- input text=… cursor=N selection=A..B`. Steps and the dump act on the first window; each other window adds `--- window N tabs=[…]`.
- For pixels, run under Xwayland (`env -u WAYLAND_DISPLAY DISPLAY=:0`) and grab the window with x11rb `GetImage` (`tests/e2e/capture.rs`).
- For tmux tests, use a throwaway server (profile `tmux_args = ["-L", "brindle-e2e", "-f", "/dev/null"]`), never the user's default server or `main` session, and kill it afterwards.

### End-to-end suite

Repeatable GUI checks live in `tests/e2e/` as named cases. It's an opt-in `harness = false` test target, so plain `cargo test` skips it:

```sh
BRINDLE_E2E=1 cargo test --test e2e                  # every case (opens windows: ask first)
BRINDLE_E2E=1 cargo test --test e2e -- titles tmux-  # cases whose names contain a filter
BRINDLE_E2E=1 cargo test --test e2e -- --list        # case names; "(no display)" ones open no windows
BRINDLE_E2E=1 cargo test --test e2e -- --self-test   # the harness's own parser tests
BRINDLE_E2E=1 cargo test --test e2e -- --shard 2/4   # every 4th selected case, from the 2nd
```

- Each case starts its own throwaway tmux server (`brindle-e2e-<pid>-<n>`) and runs Brindle against it with the debug hooks. Cases compare the dump with `capture-pane`, sample colors in window captures, or read OSC 10/11 replies. `tmux-*` cases check tmux behaviour Brindle relies on, over a raw `tmux -C` client, with no display.
- Expected colors come from the `e2e` theme in the generated test config (`tests/e2e/brindle.rs`), not from built-in themes.
- `BRINDLE_E2E_BIN` tests another binary (default: this build's). `BRINDLE_E2E_SLOW=2` doubles every delay, Brindle's step spacing included (through `--step-interval`). `BRINDLE_E2E_DISPLAY` picks the X display (default `:0`).
- Each case's config, dumps, logs, captures (`.ppm`) and transcripts are kept in `target/tmp/e2e/<case>/`.
- When a change adds a GUI check, add it as a case here instead of a one-off script.
- `scripts/with-xvfb <command>` runs it headless: a private Xvfb on a free display, openbox, lavapipe (`VK_DRIVER_FILES`), `LC_ALL=C.UTF-8` and a private session bus (`dbus-run-session`). Runs under it open no windows on the desktop, so they don't need asking first, unlike runs on `:0`. The suite needs the window manager (palette and multi-window cases). The locale keeps programs in panes consistent; tmux doesn't need it, since control clients run with `-u`.
- CI (`.github/workflows/ci.yml`, `ubuntu-26.04` runners, no containers) runs on every pull request and on pushes to every branch except Dependabot's (those always have a PR): `commits` (`scripts/lint-commits` from the PR's base or the push's `before`, else from `origin/main`, and a check that the `.ico` is no older than the SVG), `test` (`cargo test` and both harness self-tests), `release`, `windows` (on `windows-latest`: `scripts/fetch-conpty.ps1`, the release build, `cargo test`, both self-tests, the exe's version resource, icon and PerMonitorV2 manifest, `--install-desktop`/`--uninstall-desktop`, and the release zip (`scripts/package-windows.ps1`, uploaded as `brindle-windows-x64`) with its WinGet manifest (`scripts/winget-manifest.ps1`, checked by `winget validate`), with `GPUI_FXC_PATH` set from the runner's newest SDK), `windows-e2e` (the suite's Windows cases on the runner's desktop, advisory, uploading `target/tmp/e2e/` as `e2e-windows` on failure; the runner has no WSL distribution, so `windows-wsl-tmux` skips), and `e2e` (the suite under `scripts/with-xvfb` with `BRINDLE_E2E_SLOW=2`) per tmux version in 4 shards (`--shard K/4`, round-robin after filters; the count is in the matrix, the job name and the command). e2e is advisory: red on failure, not a required check. A failed shard uploads `target/tmp/e2e/` as `e2e-tmux-<v>-shard-<k>`; rerun it locally with the same `--shard`. Move the pins by hand: `COMMITLINT_VERSION` in `scripts/lint-commits`, and in the workflow `RUST_VERSION`, and the e2e matrix (a tmux version, what `tmux -V` reports, and the tarball's sha256 when it's built from source). `RUST_VERSION` matches the desktop's toolchain. Dependabot (`.github/dependabot.yml`) opens monthly PRs for the actions; Cargo gets security PRs only (its entry sets `open-pull-requests-limit: 0`), which are lockfile bumps to review against the `Cargo.lock` rule above. Both entries set `commit-message` so subjects read `chore(deps): bump …`.

### Manual checks

The suite never sends real keys, so these stay manual. Hand them to the user:
- ctrl-shift-d in a local tab does nothing and sends no `^D` (the `Swallow` binding; `commands-detach-beside-local` only checks the action);
- IME composition: with IBus, ctrl-shift-u `e9` space gives `é`, in a terminal and in a palette;
- wheel scrolling in local tabs and tmux panes;
- classic (`plain`) tmux profiles with the prefix keys;
- dragging tabs to reorder them;
- the palette shortcuts toggling (ctrl-shift-p, ctrl-shift-space and the `⌄` button).

### Performance suite

`tests/perf/` is a separate opt-in target that reports numbers rather than pass/fail, so it isn't part of e2e:

```sh
cargo build --release                              # it times target/release/brindle
BRINDLE_PERF=1 cargo test --test perf              # every case (opens windows: ask first)
BRINDLE_PERF=1 cargo test --test perf -- --self-test
```

- Cases: `latency-shell` and `latency-tmux` (`--latency-bench`), and `throughput-brindle` and `throughput-tmux` (vtebench at a pinned commit, fetched into `target/perf-tools/vtebench`).
- Each run goes to `target/tmp/perf/<UTC time>/` with a `summary.toml`. `BRINDLE_PERF_BASELINE=<run dir>` prints changes against an earlier run.
- `tests/common/` holds what both suites share (`Result`, `ensure!`, `TmuxServer`).
- Latency probes live in `src/latency.rs`. They are a GPUI global that exists only with `--latency-log` or `--latency-bench`. A probe completes on the paint whose grid includes output newer than the key, tracked by each `Terminal`'s `OutputMarker`, so repaints the key itself causes don't count.

## Architecture

Data flows **PTY / tmux → `Terminal` (model) → `TerminalView` (input) → `TerminalElement` (paint)**, with `Workspace` owning the tabs of one window.

- **`terminal/`** — the `Terminal` entity wraps an `Arc<FairMutex<alacritty Term>>` and a `Backend`:
  - `Pty`: alacritty's `EventLoop` runs on its own thread.
  - `Remote`: bytes are pushed in with `feed()` and input goes to a closure. Used for tmux panes and for error-message tabs (`Terminal::message`).

  `links.rs` finds URLs in a line of chars and runs the opener (`xdg-open`, or `BRINDLE_LINK_OPENER`) on a background thread. `link_at` returns the link at a grid point: the cell's OSC 8 hyperlink, or else a URL found in the logical line, where rows are joined by `WRAPLINE`. `TerminalView` looks links up only in event handlers while ctrl is held (hover) or on a ctrl-click, never in `prepaint`, which just underlines the stored spans.
  - **Parsing and shell integration (`shell_integration.rs`, `pty.rs`):** both backends parse through Brindle's own `Parser` (vte processor + OSC 133 scanner). For `Pty`, `MarkedPty` wraps alacritty's `Pty` (Unix) or Brindle's pseudoconsole (Windows) so the event loop's reader (`ParsingReader<R>`) parses under the term lock and returns `Ok(0)`; alacritty's own processor never sees bytes. Lock order is parser → term (`lock_unfair` while holding the parser) → marks. Marks are absolute lines (`base + Line`); `base` is kept by finding the row that was at `Line(0)` after each processor call (rows rotate, their cells never move), and resizes remap marks through logical lines. Synchronized-update timeouts are flushed from the GPUI side on `Wakeup`.

  - **Windows pseudoconsole (`conpty.rs`, `cmdline.rs`):** Brindle creates the pseudoconsole itself; alacritty's ConPTY backend isn't used.
    - The ConPTY functions are resolved once. They come from the packaged `conpty.dll` that `build.rs` embeds (`scripts/fetch-conpty.ps1` fetches it). It is loaded by absolute path from beside the exe, else from a copy extracted to `%LOCALAPPDATA%\brindle\conpty\<version>`, and only when the bytes there match the embedded ones. Otherwise the inbox kernel32 functions are used, with one warning. `BRINDLE_CONPTY=inbox` forces those.
    - It is created with `PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH` (0x10), which matches the grid's per-code-point widths; the host's grapheme default makes ❤️ two cells.
    - `PipeReader`/`PipeWriter` do blocking I/O on threads and wake the event loop with IOCP `post`s on `tty::PTY_READ_WRITE_TOKEN`. The loop polls level-triggered and registers once, so the reader posts again whenever a read leaves data behind, and the writer posts when room frees up. `ChildWatcher` uses `RegisterWaitForSingleObject`.
    - The `Pseudoconsole` is an `Arc` shared by `MarkedPty` (resizes) and the tab (`parent_console_to`, reparenting to the window's HWND). The last drop calls `ReleasePseudoConsole` (optional export), then `ClosePseudoConsole` on a short-lived thread, while the reader keeps draining to EOF. Closing never blocks a thread Brindle cares about.
    - Arguments are quoted with the MSVC rules. The environment block is merged and sorted case-insensitively, with profile entries winning.

  Alacritty events reach GPUI through an unbounded channel drained by a spawned task. Replies the emulator generates go through `reply()`, which drops them for `Remote` terminals, because tmux answers terminal queries itself. `keys.rs` and `mouse.rs` are pure xterm encoders with unit tests.
- **`terminal_view.rs` / `terminal_element.rs`**
  - Key handling: GPUI resolves keybindings *before* `on_key_down`. `key_down` encodes special keys and modifier combos and calls `stop_propagation`. Plain text (no modifiers or shift only) instead arrives through the `EntityInputHandler` path, which is what makes IME work. That's why app shortcuts are ctrl-shift-*.
  - Painting: the element batches each row into same-style runs, shapes each batch once, and places it at its exact column. Non-ASCII glyphs and the cursor cell get their own batch. Box drawing (U+2500–U+259F) is painted as rectangles, not glyphs.
  - Mouse: listeners are registered in `paint` with hitbox checks, and move/up are tracked globally while dragging.
- **Grid geometry** — `cell_metrics()` and `fit_grid()` are the single source of cell size; anything that positions cells (tmux panes!) must use them. `fixed_size` views (tmux panes) never resize their `Terminal` and get no padding, because the container positions them.
- **`workspace.rs`** — each tab is either a local terminal (`TabContent::Terminal`) or a tmux window (`TabContent::Tmux`).
  - The tab strip is the client-side-decorated title bar.
  - Tab titles are cached and refreshed on `TitleChanged` plus a 1 s poll of `/proc`.
  - Removing the last tab closes the window, and closing the last window quits.
  - A tmux session is shown by one window at most, keyed by `SessionKey` (program, `tmux_args`, session name). `find_session` scans this window, then the other windows' `Workspace`s (`cx.windows()`; the window being updated can't be read, so it's checked through `self`). Picking a profile in the new-tab palette whose session another window shows goes there (`go_to_session`; the row says so). Other ways of opening it (ctrl-alt-N, ctrl-shift-t, ctrl-shift-n) go through `open_profile`, which asks with a palette instead of attaching or switching windows.
- **`settings.rs`** — a GPUI global holding the `Config`, the resolved font (built once) and the zoom level. Live reload (`main.rs` polls the file mtime) rebuilds it, rebinds keys, and calls `Workspace::apply_config`.
- **`notify/`** — desktop notifications.
  - `osc.rs` scans output for OSC 9/777, beside alacritty, which drops OSCs it doesn't know. It keeps state across reads. ConEmu's `9;1`–`9;12` commands are not messages.
  - Local tabs scan on alacritty's I/O thread: the shell-integration reader (`pty.rs`) reads the PTY through `terminal/scan.rs`'s `ScanReader`, before parsing. Notices travel on the terminal's event channel (`ChannelEvent`). Remote terminals scan in `feed()`.
  - `TmuxWindowView` re-emits its panes' `Bell`/`Notify`, so `Workspace::on_attention` handles local and tmux tabs alike (bell dot, focus rule, settings).
  - `Notifier` (a GPUI global) rate-limits (`policy.rs`), then sends to a zbus worker on the background executor, or to the `BRINDLE_NOTIFY_LOG` file (written synchronously, since `--dump-screen-after` quits right after). It maps notification ids to window and tab for clicks.
  - A click activates the tab and raises the window. On Wayland, `activation.rs` uses the server's `ActivationToken` on GPUI's own `wl_surface` (foreign display, own event queue), because GPUI's own activation request is refused when unfocused. To raise another Brindle window, `request_token` asks for a token from the focused window, with the latest key or button press serial: GPUI keeps its serials private, so `track_input` keeps our own `wl_keyboard`/`wl_pointer` on that connection (the compositor sends each event to all of them) and drains its queue every second.
- **`desktop.rs`** — `--install-desktop` / `--uninstall-desktop` register Brindle with freedesktop launchers. They write `brindle.desktop` (its `Exec` is the running binary's absolute path) and the embedded `assets/brindle.svg` icon, and they run in `main` before GPUI starts, so no display is needed. `scripts/install.sh` delegates to them. Linux icons come only from the desktop entry and the icon theme, matched through the `brindle` app_id. The icon cache is refreshed only where one already exists.
- **`actions.rs`** — one `ACTIONS` table drives config-name lookup, `--list-actions` and the command palette (an entry's label puts it in the palette). Default bindings use the key contexts `Workspace`, `Terminal`, `TmuxWindow`, `Palette`, `SettingsDialog` and `TextInput`. User bindings go in the `Terminal` context, so they win over the defaults.
  - tmux actions (detach included) are handled only in `TmuxWindow`. ctrl-shift-d is also bound to the hidden `Swallow` action in `Workspace`, so outside tmux tabs it never reaches a shell as `^D`.
- **`picker.rs`** — `Palette`, the overlay list behind both the new-tab palette (profiles) and the command palette, plus a one-line prompt mode.
  - The command palette lists the labelled actions available from the tab that had focus, snapshotted before the palette takes focus. It needs both `available_actions` (for global listeners) and `is_action_available` (for `no_json` actions). Running a command refocuses the tab and calls `window.dispatch_action`, so it takes the keybinding's path.
  - Prompted commands (`TmuxRenameWindow`, `TmuxCommand`) are ordinary actions. Their handlers emit a prompt request that `Workspace` opens, so a keybinding and the palette behave the same.
  - The filter and the prompt are `TextInput`s, and the palette's focus is the input's, so the palette's actions bubble up from it.
- **`text_input.rs`** — the single-line `TextInput` used by every overlay text field. `EditState` (text, selection, IME composition, UTF-16 offsets) is pure and unit-tested; the entity adds focus, the `TextInput` key context, mouse, clipboard and `EntityInputHandler`, so typed text and IME arrive through the platform input handler as in terminals.
  - The `TextInput` context binds no enter, escape, tab, up/down or ctrl-p/ctrl-n, so those reach the `Palette` and `SettingsDialog` contexts around it.
  - Owners keep plain strings: they subscribe to `TextInputEvent::Changed` and call `set_text` (which emits nothing) for their own changes. The settings dialog's inputs are tab stops, and tab moves real focus with `Window::focus_next`.
- **`tmux/`** — control mode over plain pipes (`tmux -C`, not `-CC`):
  - **UTF-8:** clients start as `tmux -u [tmux_args] -C …`. Without `-u`, a client in a non-UTF-8 locale gets `_` for the tabs in our `-F` formats and for non-ASCII text, and no tab appears.
  - **Framing (`protocol.rs`):** `Collector` groups output into `%begin…%end/%error` responses and notifications. Responses to our commands (flags bit 0) are matched FIFO against `Io.pending`, because the `%begin` number is tmux's global counter, not a request id.
  - **Windows:** `%window-add` carries no layout and isn't followed by `%layout-change`, so window state always comes from a full `list-windows`. Its `#{pane_id}` is each window's active pane; it only fills in panes we don't know yet, and `%window-pane-changed` is authoritative after that.
  - **Never write tmux's selection back:** attaching must not change tmux's current window or active panes. tmux tabs are inserted inactive and come to the front only on `WindowActivated`. `active_window` is set before the tabs are announced, and `select_pane` sends nothing when the pane is already active or tmux hasn't reported one yet.
  - **Layout:** we render `#{window_visible_layout}`, the third field of `%layout-change`, which reflects zoom.
  - **Pane geometry:** layout cells drive dividers, but a pane can be smaller than its cell: `pane-border-status` takes a row for a title line without any `%layout-change` or change to the layout string. Each pane's `Inset` comes from tmux's reported geometry (`PANE_STATE_FORMAT` on restore, then the `brindle-geometry` subscription or poll) and carries over to new cells. A size change that arrives outside a layout change re-snapshots the pane. A report that doesn't fit its cell means the layout is stale (`rotate-window` sends no `%layout-change`), so the window list is fetched again.
  - **Title lines:** tmux draws `pane-border-status` lines only for terminal clients. Brindle follows each pane's `#{T:pane-border-format}` (already expanded, with `#[...]` style runs that `tmux/style.rs` parses) and each window's `#{pane-border-status}`, and `TmuxWindowView` paints them in the reserved rows, caching the shaped runs.
  - **Pane styles:** tmux applies `window-style`/`window-active-style` and the border styles only for terminal clients. Brindle follows them and sets each pane `Terminal`'s `default_colors`, and uses the border colors for dividers and title text. tmux's built-in border values (the active border's green) count as unset. Color reports to tmux carry each pane's effective colors, because tmux answers OSC 10/11 from them before looking at `window-style`.
  - **Pane restore:** three commands are sent back to back — `display-message` (`PANE_STATE_FORMAT`), `capture-pane -a`, `capture-pane`. `%output` for the pane is dropped until the last capture arrives, then `restore_bytes` replays the snapshot (after `ESC c` when re-snapshotting a pane). Mode 1003 is `#{mouse_all_flag}`; `mouse_any_flag` means *any* mouse mode.
  - **Input:** `send-keys -H`. Size: `refresh-client -C` from the tab's area.
  - **Color queries:** a control client has no tty, so tmux answers OSC 10/11 in panes with black unless told otherwise. Each pane's theme foreground and background are reported with `refresh-client -r` when the pane is created and again on config reload.
  - **Palette queries:** tmux 3.6 ignores OSC 4 in `refresh-client -r` reports, but passes OSC 4 queries through in `%output`, so Brindle answers them itself: the pane `Terminal` hands the reply to the session's `PaletteHandler`, which sends it with `send-keys -H`. It stays silent when tmux answers instead: the program set the index itself, `#{pane-colours[N]}` is set, or a terminal client is attached to the session (`list-clients`; tmux asks that client's terminal).
  - **Subscriptions or polling:** tmux before 3.7 can segfault when a control client with format subscriptions leaves or loses its session (`control_check_subs_timer` reads through a NULL session; 3.7 checks for it). So the session asks for `#{version}` before its first window list (`subscriptions_safe`), and on older or unrecognised tmux registers no subscriptions at all. It polls with one `list-panes -s` a second instead, built from the same `SUBSCRIPTIONS` table, and `PollState` turns each reply into the `SubscriptionChanged` notifications for values that changed, so both paths share the handlers. One poll is in flight at a time, and one is sent right after a user-typed command.
  - **Leaving a session:** with subscriptions, `release()` removes them before `detach-client`, from `detach`, `Drop` and an app-quit hook that waits for the writer to flush, and `kill_window`/`kill_pane` unsubscribe before ending the session's last window or pane. A session ended from inside tmux (the last shell exiting) can't be covered that way, which is why old tmux polls.
  - **Events:** `Failed` (tmux exited before ever attaching) is distinct from `Detached`.
  - The tmux prefix can't work in control mode, so native `Tmux*` actions replace it.
- **`tmux_view.rs`** — `TmuxWindowView` mirrors one tmux window. It creates pane views and follows tmux's active pane in a session observer (`sync`), not during render, and draws the dividers.

GPUI 0.2.2 doesn't cache views by default, so any `cx.notify()` re-renders the whole window. Keep I/O, allocation-heavy work and lock contention out of `render` and `prepaint`.

## Specs

Behaviour is specified with OpenSpec:
- Main specs: `openspec/specs/<capability>/spec.md`.
- History and design rationale: `openspec/changes/archive/`.

Read the relevant spec before changing behaviour, and propose spec changes through an OpenSpec change with the `openspec` CLI.

<!-- rtk-instructions v2 -->
# Command output

Command output here is condensed to save tokens, keeping every signal and
dropping costly noise. Treat it as the complete result: run commands
normally, and batch related commands into one call to avoid extra turns.
Truncated results state their recovery path in their own output. Re-run a
command as `rtk proxy <cmd>` only when its result is unusable: empty when
output was clearly expected, contradicting its exit code, or garbled.
<!-- /rtk-instructions -->
