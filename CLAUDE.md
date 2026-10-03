# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Brindle is a GPU-accelerated Linux terminal emulator in Rust. It is built on GPUI 0.2.2 (Zed's toolkit, from crates.io) and `alacritty_terminal` 0.26, with tabs, profiles, and tmux in two modes: classic, and control mode (`tmux -C`). See README.md for user-facing behaviour and keybindings.

## Build and test

Build dependencies are the system packages listed in README.md. Wayland, fontconfig and Vulkan are dlopen'd at runtime, so they have no `-dev` packages.

```sh
cargo build                          # deps built at opt-level 2 (Blade is unusable unoptimized)
cargo build --release                # stripped, thin LTO
cargo test                           # all unit tests (no GUI needed)
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
- `--dump-screen-after N` prints the tab titles, the active terminal's size and `TermMode`, and its screen text, then exits.
- For pixels, run under Xwayland (`env -u WAYLAND_DISPLAY DISPLAY=:0`) and grab the window with an x11rb `GetImage` helper.
- For tmux tests, use a throwaway server (profile `tmux_args = ["-L", "brindle-e2e", "-f", "/dev/null"]`), never the user's default server or `main` session, and kill it afterwards.

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
- **`actions.rs`** — one `ACTIONS` table drives both config-name lookup and `--list-actions`. Default bindings use the key contexts `Workspace`, `Terminal`, `TmuxWindow` and `ProfilePicker`. User bindings go in the `Terminal` context, so they win over the defaults.
- **`tmux/`** — control mode over plain pipes (`tmux -C`, not `-CC`):
  - **Framing (`protocol.rs`):** `Collector` groups output into `%begin…%end/%error` responses and notifications. Responses to our commands (flags bit 0) are matched FIFO against `Io.pending`, because the `%begin` number is tmux's global counter, not a request id.
  - **Windows:** `%window-add` carries no layout and isn't followed by `%layout-change`, so window state always comes from a full `list-windows`.
  - **Layout:** we render `#{window_visible_layout}`, the third field of `%layout-change`, which reflects zoom.
  - **Pane restore:** three commands are sent back to back — `display-message` (`PANE_STATE_FORMAT`), `capture-pane -a`, `capture-pane`. `%output` for the pane is dropped until the last capture arrives, then `restore_bytes` replays the snapshot. Mode 1003 is `#{mouse_all_flag}`; `mouse_any_flag` means *any* mouse mode.
  - **Input:** `send-keys -H`. Size: `refresh-client -C` from the tab's area.
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
