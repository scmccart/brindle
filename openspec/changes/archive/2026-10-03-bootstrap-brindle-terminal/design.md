# Design

> Backfilled from the 2026-10-03 build session. Decisions record what was
> chosen and why, including problems found along the way.

## Context

See proposal.md for the motivation. Constraints at the time:

- The build machine had no C compiler, no `pkg-config`, no `-dev` headers,
  no tmux, and no passwordless sudo. Runtime libraries (Vulkan, Wayland,
  xkbcommon, fontconfig) were installed.
- GPUI is published on crates.io, but only as 0.2.2 (October 2025). Zed's own
  repository moves much faster.
- The desktop is GNOME on Wayland. It does not support server-side
  decorations, so GPUI windows draw their own.

## Goals / Non-Goals

**Goals:**
- Use GPUI's GPU renderer (Blade/Vulkan) for everything drawn on screen.
- Make tmux a first-class workflow: protocol-correct inside a tab, and
  native when attached through control mode.
- Build reproducibly without root.

**Non-Goals:**
- Native split panes outside tmux (tmux covers splitting).
- Search, hyperlinks, ligatures, image protocols, kitty keyboard protocol.
- macOS and Windows.

## Decisions

### Toolkit and emulation
- **GPUI 0.2.2 from crates.io, not a git dependency on Zed.** It is a stable
  version with a published lockfile, and it avoids cloning all of Zed. The
  trade-off is an older API.
- **Seed `Cargo.lock` from gpui 0.2.2's own `Cargo.lock`.** With freshly
  resolved versions, `libc` 0.2.190 breaks `xattr` 0.2.3 (`ENOATTR` is
  missing). Seeding keeps versions known to work together.
- **`alacritty_terminal` 0.26 for emulation, with Brindle's own encoders for
  keys and mouse.** The library parses output and manages the grid; input
  encoding is the frontend's job. Zed's `terminal`/`terminal_view` crates
  were read only to learn the API shape. They are GPL-3.0 and no code was
  copied, so Brindle can be MIT OR Apache-2.0.

### Build environment
- **Rootless toolchain (`.toolchain/bootstrap.sh`).** It uses
  `apt-get download` and `dpkg -x` to unpack gcc, pkgconf, the `-dev`
  headers and tmux into `.toolchain/root`.
  - Dangling `.so` dev symlinks are pointed at the system runtime libraries.
  - `cc1` and the other compiler helpers are symlinked in, because a
    relocated gcc only looks next to itself.
  - `cc` and `c++` symlinks are added, since rustc's default linker is `cc`.
  - `env.sh` exports `PATH`, `CPATH`, `LIBRARY_PATH`, `PKG_CONFIG_*` and
    `RUSTFLAGS`. Nothing machine-specific goes into `.cargo/config.toml`.

### Window and rendering
- **Client-side decorations, with the tab strip as the title bar.**
  - Empty strip space moves the window, double-click maximizes, and
    right-click opens the window menu.
  - A transparent inset around the window shows a shadow and carries the
    resize handles.
- **Rows are split into batches of same-style cells.** Each batch is shaped
  once (GPUI caches shaped lines across frames) and placed at its exact
  column.
  - Non-ASCII glyphs, the cursor cell and wide characters get batches of
    their own, so an odd glyph advance can't shift the rest of the row.
  - Background runs are merged and snapped to whole pixels.
- **Box-drawing and block characters (U+2500–U+259F) are drawn as
  rectangles**, not font glyphs. Line height is 1.25× by default, so glyph
  borders would leave gaps. Drawing them keeps tmux and TUI borders joined.
- **Ligatures are disabled**, which keeps one glyph per cell.

### Input
- Keystrokes are encoded in a key-down handler, which then stops further
  handling. Plain text (no modifiers, or shift only) goes through GPUI's
  platform input handler instead, which makes IME and dead keys work.
  Bindings resolve before key-down, so app shortcuts use ctrl-shift
  combinations and plain ctrl+letter always reaches the program.

### tmux control mode
- **`tmux -C` over plain pipes, not `-CC` on a PTY.** A protocol spike
  against tmux 3.6 confirmed that `-C` works without a tty: there is no DCS
  wrapper and no echo.
- **Response framing.**
  - Every command we send gets one `%begin`…`%end` or `%error` block with
    flag bit 0 set.
  - The block's number is tmux's global counter, not a request id. Responses
    are therefore matched first-in, first-out against a pending queue.
  - Blocks we didn't send (such as the attach) have flags 0 and are ignored.
  - Notifications never appear inside a block, and a block closes only on an
    `%end` or `%error` with the same number.
- **Windows are discovered through `list-windows`.** `%window-add` carries
  no layout, and no `%layout-change` follows it, so every add, session
  change or unknown layout triggers a full `list-windows`.
- **Rendering uses the visible layout**, the third field of
  `%layout-change`, so a zoomed window looks zoomed.
- **Restoring a pane on attach.** Three commands are sent back to back:
  1. `display-message` with the cursor and mode flags
  2. `capture-pane -a` (the saved normal screen)
  3. `capture-pane -e -S -` (the current screen plus history)

  `%output` for that pane is dropped until the last capture arrives, because
  the snapshot already contains it. The capture is then replayed as escape
  sequences: lines, alternate screen, cursor position, and DECSET modes.
  - **Gotcha:** tmux's `mouse_any_flag` means "any mouse mode is on". Mode
    1003 is `mouse_all_flag`. Mixing them up restored button-drag apps
    (1002) as any-motion (1003).
- **Input goes through `send-keys -H`** in 256-byte chunks. The terminal's
  own replies (device attributes, color queries) are dropped for tmux panes,
  because tmux answers those queries itself.
- **The client size is set with `refresh-client -C WxH`**, computed from the
  tab's content area, and only sent when it changes.
- **The tmux prefix can't work in control mode** (`send-keys` skips tmux's
  key bindings). Native bindings cover new window, split, close, zoom and
  pane focus, and this is documented in the README.
- **Separate `Failed` and `Detached` events.** The session tracks whether a
  window list ever arrived. This avoids guessing from "no tabs left", which
  misreported a session whose windows had all been closed.

### Verification approach
- Pure modules are unit-tested: keys, mouse, protocol, layout, restore,
  config, themes, box drawing and actions.
- `--send`, `--action` and `--dump-screen-after` drive the real app without
  synthetic OS input.
- Screenshots were taken by running under Xwayland and capturing the window
  with an x11rb `GetImage` helper.
- **Do not inject XTest keystrokes on the user's desktop.** Injected keys went
  to the focused window, which was the user's terminal, not Brindle's.

## Risks / Trade-offs

- [gpui 0.2.2 is old and its API differs from Zed main] → Pinned through the
  lockfile. Upgrading is a separate change.
- [Debug builds of Blade are slow] → Dependencies are built at opt-level 2
  in the dev profile. Release builds use thin LTO and are stripped (21 MB).
- [In control mode, the user's tmux prefix doesn't work] → Native bindings
  plus documentation.
- [Restore replays a text snapshot, so some state is lost (scroll regions,
  charset, saved cursor)] → The most visible state is restored: alternate
  screen, cursor, keypad and mouse modes.
- [The process name in tab titles is polled once a second from `/proc`] →
  Titles are cached; there is no per-frame I/O.

## Open Questions

- On Wayland, the first `refresh-client -C` was seen about 3 s after attach,
  so tmux briefly uses 80×24. This is probably frame throttling before the
  window is shown. Not investigated.
