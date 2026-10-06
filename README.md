# Brindle

A GPU-accelerated terminal emulator for Linux, written in Rust on
[GPUI](https://crates.io/crates/gpui), the toolkit Zed is built with.
Terminal emulation comes from `alacritty_terminal`; rendering goes through
GPUI's Vulkan renderer.

- **Tabs** in a client-side-decorated title bar: drag to reorder, middle-click to close.
- **Profiles**: command, args, working directory, environment, theme and font size per profile.
- **Themes**: five built in, plus your own. The settings dialog (ctrl-shift-,)
  previews themes live on your open tabs and has an editor for custom themes.
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

## Testing

```sh
cargo test                                      # unit tests; no display needed
BRINDLE_E2E=1 cargo test --test e2e             # end-to-end suite
BRINDLE_E2E=1 cargo test --test e2e -- titles   # only cases whose names contain "titles"
BRINDLE_E2E=1 cargo test --test e2e -- --list   # list the cases
```

The end-to-end suite in `tests/e2e/` runs Brindle against throwaway tmux
servers and compares what it shows with tmux: screen text, window pixels and
replies to color queries. It is opt-in because it opens Brindle windows (under
Xwayland, display `:0` unless `BRINDLE_E2E_DISPLAY` says otherwise) and needs
`tmux`, `bash` and `less`. Without `BRINDLE_E2E=1`, `cargo test` skips it.
Each case keeps its logs, screen dumps and window captures in
`target/tmp/e2e/<case>/`.

## Installing

From a checkout, after installing the build dependencies above:

```sh
scripts/install.sh                     # binary to ~/.local/bin, plus launcher entry and icon
PREFIX=/usr/local scripts/install.sh   # system-wide; builds as you, uses sudo only to copy
```

The script always installs under `$PREFIX/share`, even if `XDG_DATA_HOME` is
set; `brindle --install-desktop` on its own follows `XDG_DATA_HOME`. To update,
re-run the script. Windows that are already open keep running the
old build; new windows get the new one.

With cargo, the binary registers itself:

```sh
cargo install --locked --path .
brindle --install-desktop
```

The launcher entry runs the binary by its absolute path, so it works even when
`~/.cargo/bin` isn't on the desktop session's `PATH`. After moving or upgrading
the binary, run `brindle --install-desktop` again so the entry stays current.

To uninstall, run `scripts/uninstall.sh` (with the same `PREFIX`), or
`brindle --uninstall-desktop` followed by `cargo uninstall brindle`. Your config
in `~/.config/brindle` is kept either way.

## Usage

```
brindle [-p PROFILE] [-d DIR] [-e COMMAND ARGS...]
brindle --list-actions | --list-themes
brindle --install-desktop | --uninstall-desktop [--prefix DIR]
```

On first launch Brindle writes a commented config to
`~/.config/brindle/config.toml`. The file is reloaded automatically when it changes.

| Keys | Action |
|---|---|
| ctrl-shift-t / ctrl-shift-w | new / close tab |
| ctrl-tab, ctrl-pageup/down | switch tab |
| alt-1 … alt-9 | go to tab (9 = last) |
| ctrl-shift-pageup/down | move tab |
| ctrl-shift-p | command palette (every command, with its shortcut) |
| ctrl-shift-space, or the strip's `⌄` | new tab with a profile |
| ctrl-alt-1 … ctrl-alt-9 | new tab with profile N |
| ctrl-shift-c / ctrl-shift-v | copy / paste (middle click pastes the primary selection) |
| shift-pageup/down, shift-home/end | scrollback |
| ctrl-= / ctrl-- / ctrl-0 | font size |
| ctrl-shift-n | new window |
| ctrl-shift-, | settings (theme picker with live preview, custom theme editor) |
| ctrl-, | edit config |

In the settings dialog, up/down previews a theme and enter applies it as the
default; escape puts the previous theme back. `n` makes a new theme (a copy of
the selected one), `e` edits a custom theme and `d` deletes one. Built-in themes
can't be changed, so editing one starts from a copy. In the editor, tab moves
between fields and every valid color shows on the tabs as you type. Saving
updates `config.toml` in place: your comments and the rest of the file are kept.

Plain `ctrl-<letter>` always goes to the program, so tmux's prefix works as usual.

**Breaking change:** ctrl-shift-p used to open the profile list and now opens the
command palette. The profile list is on ctrl-shift-space and the `⌄` button; to
get the old key back, bind `"ctrl-shift-p" = "open_profile_picker"`.

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

More tmux commands have no default key and are run from the command palette
(ctrl-shift-p): layouts (even horizontal/vertical, main vertical, tiled, next),
rotate panes, swap a pane with the previous or next one, break a pane out into a
new tab, **rename window**, and **run command**, which takes any tmux command line.
Run command shows tmux's error if the line fails, and new windows or panes it
creates start in the current pane's directory. Each command can also be bound to
a key; see `brindle --list-actions`.

Clicking a pane selects it in tmux. If the program in a pane has turned on mouse
reporting, clicks go to it; otherwise you get normal text selection.
`tmux_args` lets a profile use its own server, e.g. `["-L", "work"]`.

## License

MIT or Apache-2.0, at your option.
