# Brindle

A GPU-accelerated terminal emulator for Linux, written in Rust on
[GPUI](https://crates.io/crates/gpui), the toolkit Zed is built with.
Terminal emulation comes from `alacritty_terminal`; rendering goes through
GPUI's Vulkan renderer.

- **Tabs** in a client-side-decorated title bar: drag to reorder, middle-click to close.
- **Profiles**: command, args, working directory, environment, theme and font size per profile.
- **tmux**, both ways:
  - *Classic*: tmux runs inside the terminal with SGR mouse, focus events (1004),
    bracketed paste, OSC 52 clipboard, true color, styled underlines, cursor
    shapes and synchronized output. Box-drawing characters are drawn as shapes, so
    pane borders join up at any line height.
  - *Control mode* (`tmux = "control"`): Brindle talks to tmux through `tmux -C`.
    Each tmux window is a native tab and each pane is drawn natively on tmux's
    layout grid. Existing panes are restored on attach, including their alternate
    screen and mouse modes. Detaching leaves the session running.

## Building

On Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev \
  libxcb1-dev libfreetype-dev tmux
cargo build --release
./target/release/brindle
```

Wayland, fontconfig and Vulkan are loaded at runtime (dlopen), so only their
runtime libraries are needed, and every desktop install has them. `tmux` is
only needed for the tmux profiles.

`Cargo.lock` starts from gpui 0.2.2's own lockfile; newer transitive versions
(e.g. `libc`) break its dependencies.

## Usage

```
brindle [-p PROFILE] [-d DIR] [-e COMMAND ARGS...]
brindle --list-actions | --list-themes
```

On first launch Brindle writes a commented config to
`~/.config/brindle/config.toml`. The file is reloaded automatically when it changes.

| Keys | Action |
|---|---|
| ctrl-shift-t / ctrl-shift-w | new / close tab |
| ctrl-tab, ctrl-pageup/down | switch tab |
| alt-1 … alt-9 | go to tab (9 = last) |
| ctrl-shift-pageup/down | move tab |
| ctrl-shift-p | profile picker |
| ctrl-alt-1 … ctrl-alt-9 | new tab with profile N |
| ctrl-shift-c / ctrl-shift-v | copy / paste (middle click pastes the primary selection) |
| shift-pageup/down, shift-home/end | scrollback |
| ctrl-= / ctrl-- / ctrl-0 | font size |
| ctrl-shift-n | new window |
| ctrl-, | edit config |

Plain `ctrl-<letter>` always goes to the program, so tmux's prefix works as usual.

### tmux control mode

In control mode, keys reach panes through `send-keys`, which skips tmux's own key
bindings. **Your tmux prefix does not work in control-mode tabs**; use Brindle's
bindings instead:

| Keys | Action |
|---|---|
| ctrl-shift-t | new tmux window (as a tab) |
| ctrl-shift-w | close tab (kills the tmux window) |
| ctrl-shift-e / ctrl-shift-o | split right / down |
| ctrl-shift-x | close pane |
| ctrl-shift-z | zoom pane |
| alt-arrows | move between panes |
| ctrl-shift-d | detach (the session keeps running) |

Clicking a pane selects it in tmux. If the program in a pane has turned on mouse
reporting, clicks go to it; otherwise you get normal text selection.
`tmux_args` lets a profile use its own server, e.g. `["-L", "work"]`.

## License

MIT or Apache-2.0, at your option.
