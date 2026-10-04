# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Brindle is a GPU-accelerated Linux terminal emulator in Rust. It is built on GPUI 0.2.2 (Zed's toolkit, from crates.io) and `alacritty_terminal` 0.26, with tabs, profiles, and tmux in two modes: classic, and control mode (`tmux -C`). See README.md for user-facing behaviour and keybindings.

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

## Verifying GUI behaviour

**Never inject synthetic input (XTest, xdotool) on this desktop.** It goes to the user's focused window, not Brindle. Opening a window can also steal focus while the user is typing, so ask before GUI runs unless they've said they're away. Use the built-in debug hooks instead:

```sh
BRINDLE_CONFIG=/tmp/test-config.toml ./target/release/brindle -p tmux \
  --send 'echo hi\r' --action tmux_split_right --dump-screen-after 5
```

- `--send` and `--action` steps run in order, one second apart, against the active tab. Action names are listed by `--list-actions`.
- `--dump-screen-after N` prints the tab titles, the active terminal's size and `TermMode`, its grid geometry (`--- grid origin=X,Y cell=WxH scale=S`, logical window pixels), and its screen text, then exits.
- For pixels, run under Xwayland (`env -u WAYLAND_DISPLAY DISPLAY=:0`) and grab the window with x11rb `GetImage` (`tests/e2e/capture.rs`).
- For tmux tests, use a throwaway server (profile `tmux_args = ["-L", "brindle-e2e", "-f", "/dev/null"]`), never the user's default server or `main` session, and kill it afterwards.

### End-to-end suite

Repeatable GUI checks live in `tests/e2e/` as named cases. It's an opt-in `harness = false` test target, so plain `cargo test` skips it:

```sh
BRINDLE_E2E=1 cargo test --test e2e                  # every case (opens windows: ask first)
BRINDLE_E2E=1 cargo test --test e2e -- titles tmux-  # cases whose names contain a filter
BRINDLE_E2E=1 cargo test --test e2e -- --list        # case names; "(no display)" ones open no windows
BRINDLE_E2E=1 cargo test --test e2e -- --self-test   # the harness's own parser tests
```

- Each case starts its own throwaway tmux server (`brindle-e2e-<pid>-<n>`) and runs Brindle against it with the debug hooks. Cases compare the dump with `capture-pane`, sample colors in window captures, or read OSC 10/11 replies. `tmux-*` cases check tmux behaviour Brindle relies on, over a raw `tmux -C` client, with no display.
- Expected colors come from the `e2e` theme in the generated test config (`tests/e2e/brindle.rs`), not from built-in themes.
- `BRINDLE_E2E_BIN` tests another binary (default: this build's). `BRINDLE_E2E_SLOW=2` doubles every delay. `BRINDLE_E2E_DISPLAY` picks the X display (default `:0`).
- Each case's config, dumps, logs, captures (`.ppm`) and transcripts are kept in `target/tmp/e2e/<case>/`.
- When a change adds a GUI check, add it as a case here instead of a one-off script.

## Architecture

Data flows **PTY / tmux → `Terminal` (model) → `TerminalView` (input) → `TerminalElement` (paint)**, with `Workspace` owning the tabs of one window.

- **`terminal/`** — the `Terminal` entity wraps an `Arc<FairMutex<alacritty Term>>` and a `Backend`:
  - `Pty`: alacritty's `EventLoop` runs on its own thread.
  - `Remote`: bytes are pushed in with `feed()` and input goes to a closure. Used for tmux panes and for error-message tabs (`Terminal::message`).

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
- **`settings.rs`** — a GPUI global holding the `Config`, the resolved font (built once) and the zoom level. Live reload (`main.rs` polls the file mtime) rebuilds it, rebinds keys, and calls `Workspace::apply_config`.
- **`desktop.rs`** — `--install-desktop` / `--uninstall-desktop` register Brindle with freedesktop launchers. They write `brindle.desktop` (its `Exec` is the running binary's absolute path) and the embedded `assets/brindle.svg` icon, and they run in `main` before GPUI starts, so no display is needed. `scripts/install.sh` delegates to them. Linux icons come only from the desktop entry and the icon theme, matched through the `brindle` app_id. The icon cache is refreshed only where one already exists.
- **`actions.rs`** — one `ACTIONS` table drives config-name lookup, `--list-actions` and the command palette (an entry's label puts it in the palette). Default bindings use the key contexts `Workspace`, `Terminal`, `TmuxWindow` and `Palette`. User bindings go in the `Terminal` context, so they win over the defaults.
  - tmux actions (detach included) are handled only in `TmuxWindow`. ctrl-shift-d is also bound to the hidden `Swallow` action in `Workspace`, so outside tmux tabs it never reaches a shell as `^D`.
- **`picker.rs`** — `Palette`, the overlay list behind both the new-tab palette (profiles) and the command palette, plus a one-line prompt mode.
  - The command palette lists the labelled actions available from the tab that had focus, snapshotted before the palette takes focus. It needs both `available_actions` (for global listeners) and `is_action_available` (for `no_json` actions). Running a command refocuses the tab and calls `window.dispatch_action`, so it takes the keybinding's path.
  - Prompted commands (`TmuxRenameWindow`, `TmuxCommand`) are ordinary actions. Their handlers emit a prompt request that `Workspace` opens, so a keybinding and the palette behave the same.
- **`tmux/`** — control mode over plain pipes (`tmux -C`, not `-CC`):
  - **Framing (`protocol.rs`):** `Collector` groups output into `%begin…%end/%error` responses and notifications. Responses to our commands (flags bit 0) are matched FIFO against `Io.pending`, because the `%begin` number is tmux's global counter, not a request id.
  - **Windows:** `%window-add` carries no layout and isn't followed by `%layout-change`, so window state always comes from a full `list-windows`. Its `#{pane_id}` is each window's active pane; it only fills in panes we don't know yet, and `%window-pane-changed` is authoritative after that.
  - **Never write tmux's selection back:** attaching must not change tmux's current window or active panes. tmux tabs are inserted inactive and come to the front only on `WindowActivated`. `active_window` is set before the tabs are announced, and `select_pane` sends nothing when the pane is already active or tmux hasn't reported one yet.
  - **Layout:** we render `#{window_visible_layout}`, the third field of `%layout-change`, which reflects zoom.
  - **Pane geometry:** layout cells drive dividers, but a pane can be smaller than its cell: `pane-border-status` takes a row for a title line without any `%layout-change` or change to the layout string. Each pane's `Inset` comes from tmux's reported geometry (`PANE_STATE_FORMAT` on restore, then the `brindle-geometry` format subscription) and carries over to new cells. A size change that arrives outside a layout change re-snapshots the pane. A report that doesn't fit its cell means the layout is stale (`rotate-window` sends no `%layout-change`), so the window list is fetched again.
  - **Title lines:** tmux draws `pane-border-status` lines only for terminal clients. Brindle subscribes to each pane's `#{T:pane-border-format}` (already expanded, with `#[...]` style runs that `tmux/style.rs` parses) and each window's `#{pane-border-status}`, and `TmuxWindowView` paints them in the reserved rows, caching the shaped runs.
  - **Pane styles:** tmux applies `window-style`/`window-active-style` and the border styles only for terminal clients. Brindle subscribes to them and sets each pane `Terminal`'s `default_colors`, and uses the border colors for dividers and title text. tmux's built-in border values (the active border's green) count as unset. Color reports to tmux carry each pane's effective colors, because tmux answers OSC 10/11 from them before looking at `window-style`.
  - **Pane restore:** three commands are sent back to back — `display-message` (`PANE_STATE_FORMAT`), `capture-pane -a`, `capture-pane`. `%output` for the pane is dropped until the last capture arrives, then `restore_bytes` replays the snapshot (after `ESC c` when re-snapshotting a pane). Mode 1003 is `#{mouse_all_flag}`; `mouse_any_flag` means *any* mouse mode.
  - **Input:** `send-keys -H`. Size: `refresh-client -C` from the tab's area.
  - **Color queries:** a control client has no tty, so tmux answers OSC 10/11 in panes with black unless told otherwise. Each pane's theme foreground and background are reported with `refresh-client -r` when the pane is created and again on config reload.
  - **Leaving a session:** tmux 3.6 can segfault when a control client with format subscriptions leaves or loses its session (fixed upstream, unreleased). `release()` removes the subscriptions before `detach-client`, from `detach`, `Drop` and an app-quit hook that waits for the writer to flush, and `kill_window`/`kill_pane` unsubscribe before ending the session's last window or pane. A session ended from inside tmux (the last shell exiting) is still exposed.
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
