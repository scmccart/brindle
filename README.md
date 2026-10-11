# Brindle

A GPU-accelerated terminal emulator for Linux, written in Rust on
[GPUI](https://crates.io/crates/gpui), the toolkit Zed is built with.
Terminal emulation comes from `alacritty_terminal`; rendering goes through
GPUI's Vulkan renderer.

Brindle treats tmux as a native windowing layer, not just a program running
inside the terminal: tmux windows become tabs and panes are drawn natively,
while sessions stay in tmux, persistent and detachable.

- **Tabs** in a client-side-decorated title bar: drag to reorder, middle-click to close.
- **Profiles**: command, args, working directory, environment, theme and font size per
  profile, editable in the settings dialog.
- **Shell integration** (OSC 133): jump between prompts, select or copy the last
  command's output, and see failed commands marked in the gutter.
- **Desktop notifications** from OSC 9, OSC 777 and bells in tabs you can't see;
  clicking one takes you to its tab.
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

## Control mode or classic?

Control mode gives each pane its own scrollback (the wheel scrolls it, with no
copy mode) and selection that stays inside the pane. tmux windows become
draggable tabs, native shortcuts and the command palette replace the prefix,
and dividers and title lines are painted in your tmux styles. Attaching
doesn't change tmux's current window or active pane.

Classic mode runs your tmux config unchanged, including the prefix and custom
bindings, which control mode can't use. Pick control mode when tmux is your
main workspace, and classic mode when you depend on the prefix.

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

### Performance

```sh
cargo build --release                               # the suite times the release build
BRINDLE_PERF=1 cargo test --test perf               # every case (opens windows)
BRINDLE_PERF=1 cargo test --test perf -- latency    # only cases matching "latency"
BRINDLE_PERF=1 cargo test --test perf -- --list     # list the cases
BRINDLE_PERF_BASELINE=target/tmp/perf/latest BRINDLE_PERF=1 cargo test --test perf
                                                    # show changes against the last run
```

The performance suite in `tests/perf/` measures two things, each in a local
shell tab and in a tmux control-mode tab on a throwaway server:

- **Throughput:** how fast Brindle drains output, measured with
  [vtebench](https://github.com/alacritty/vtebench), the same method other
  terminals publish numbers with. On first use the suite clones a pinned commit
  into `target/perf-tools/vtebench` and builds it there, which needs network
  access. Set `BRINDLE_PERF_VTEBENCH` to use your own checkout instead.
- **Keystroke latency:** `brindle --latency-bench N` types N keystrokes through
  GPUI's own key handling (no synthetic OS input), then prints percentiles for
  three stages, each measured from when Brindle received the key:
  - `output`: the program's echo arrived;
  - `paint`: the terminal painted it;
  - `frame`: the next frame began.

  This is not key-to-photon latency: GPU, compositor and display time are not
  included. `brindle --latency-log FILE` records the same timings for real
  typing, as JSON lines, and prints a summary on exit.

The suite reports numbers and never fails on them. Run it on an idle machine and
compare runs made on the same machine. Each run is kept in
`target/tmp/perf/<UTC time>/` (`latest` points at the newest) with a
`summary.toml`, and the CPU governor and display are recorded with it.
`BRINDLE_PERF_SAMPLES` changes the keystrokes per latency case (500),
`BRINDLE_PERF_BIN` the binary, and `BRINDLE_PERF_DISPLAY` the X display (`:0`).

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
brindle [-p PROFILE] [-d DIR] [--no-restore] [-e COMMAND ARGS...]
brindle --list-actions | --list-themes | --shell-integration bash|zsh|fish
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
| ctrl-shift-up / ctrl-shift-down | previous / next prompt ([shell integration](#shell-integration)); one line without it |
| ctrl-= / ctrl-- / ctrl-0 | font size |
| ctrl-shift-n | new window |
| ctrl-shift-, | settings (theme picker with live preview, custom theme editor, profiles) |
| ctrl-, | edit config |

In the settings dialog, up/down previews a theme and enter applies it as the
default; escape puts the previous theme back. `n` makes a new theme (a copy of
the selected one), `e` edits a custom theme and `d` deletes one. Built-in themes
can't be changed, so editing one starts from a copy. In the editor, tab moves
between fields and every valid color shows on the tabs as you type. Saving
updates `config.toml` in place: your comments and the rest of the file are kept.

ctrl-tab switches to the Profiles section, which lists your profiles with their
ctrl-alt-N numbers. Enter or `e` edits one, `n` makes a new one, `c` duplicates
the selected one, `d` deletes it, and alt-up/alt-down reorder the list. The
editor covers every profile setting; arguments, environment (`KEY=value`) and
tmux arguments are written as shell words. A profile's theme and font size show
on its open tabs while you edit; the other settings apply to new tabs. Renaming
a profile keeps its open tabs linked to it and updates `default_profile`.

Plain `ctrl-<letter>` always goes to the program, so tmux's prefix works as usual.

### Links

Hold ctrl over a URL (`http`, `https`, `file`, `mailto`, `ftp`, `ssh`, `git`, …)
or over text a program marked as an OSC 8 hyperlink: the link is underlined
and the pointer becomes a hand. Ctrl-click opens it with `xdg-open`, even in
programs that use the mouse themselves (vim, htop, tmux with `mouse on`).
Ctrl-clicks elsewhere still reach those programs. Trailing punctuation and
enclosing brackets aren't part of a link, and a URL that wraps onto the next
row is still one link. Ctrl-drag no longer block-selects when it starts on a
link; alt-drag still does.

To open links with something other than `xdg-open`, set
`BRINDLE_LINK_OPENER` to a program; it gets the link as its only argument.
"Copy Link Under Mouse" (`copy_link`) in the command palette copies the link
under the mouse; it has no default key, so bind one if you want it.

Links work the same in control-mode panes. In classic tmux, tmux passes OSC 8
hyperlinks on only if it knows the terminal supports them:
`set -as terminal-features ',xterm-256color:hyperlinks'`.

**Breaking change:** ctrl-shift-p used to open the profile list and now opens the
command palette. The profile list is on ctrl-shift-space and the `⌄` button; to
get the old key back, bind `"ctrl-shift-p" = "open_profile_picker"`.

### Session restore

With `restore_session = true` in the config (it is off by default), launching
`brindle` with none of `-p`, `-d` or `-e` reopens the windows and tabs you had
when it last quit:

- each window's size, its tabs in order, and its active tab;
- each tab's profile, in its shell's last directory;
- classic tmux tabs, which reattach to their session;
- control-mode sessions, attached once each. Their windows come back as tabs
  where they were, and tmux's current window and panes are left as they are.

Scrollback and running programs are not restored: shells start fresh in
their directory. `brindle --no-restore` skips restoring once.

The state is written to `~/.local/state/brindle/session.toml`
(`$XDG_STATE_HOME/brindle/`, or `$BRINDLE_STATE_DIR` when it is set) a second
or two after anything changes, and again on quit, so a crash or logout loses
at most the last couple of seconds. If a profile or directory has gone
since then, the default profile or the home directory is used. A tmux session
that no longer exists is created again. Closing a window's last tab forgets
that window, and quitting keeps everything that is open. While one Brindle is
running, launching another one opens a normal window and doesn't touch the
saved session.

### Shell integration

Shells can mark their prompts and command output with OSC 133 sequences. Brindle
ships scripts that do this for bash, zsh and fish. Load one at the **end** of
your shell's startup file, after your prompt theme and anything that sets
`PROMPT_COMMAND`:

```sh
eval "$(brindle --shell-integration bash)"    # ~/.bashrc
eval "$(brindle --shell-integration zsh)"     # ~/.zshrc
brindle --shell-integration fish | source     # ~/.config/fish/config.fish
```

The same scripts are in [`shell-integration/`](shell-integration/) if you'd rather
`source` a file. Brindle never edits your startup files or injects the scripts
itself. They only emit escape sequences, and terminals that don't understand
them ignore them. With a script loaded:

- **ctrl-shift-up / ctrl-shift-down** scroll so the previous or next prompt is at
  the top of the view. In a terminal without marks they still scroll one line.
  To keep line scrolling on those keys, bind `"ctrl-shift-up" = "scroll_line_up"`
  and `"ctrl-shift-down" = "scroll_line_down"`.
- **Select Last Command Output** and **Copy Last Command Output** are in the
  command palette. They have no default key; bind `select_last_output` or
  `copy_last_output`. Copy leaves your selection alone.
- A thin red bar (the theme's red) marks prompts whose command exited non-zero.
  Turn it off with `mark_failed_commands = false`.

Marks follow their text as it scrolls, reflows on resize, and is cleared. They
work the same in tmux control-mode panes, because tmux passes pane output to
control clients unchanged; output restored on attach has no marks. Classic tmux
keeps OSC 133 to itself, so Brindle sees no marks there. bash needs 4.4 or newer
for output marks. fish's script doesn't mark where the command line starts, so a
failure bar covers only the first line of a multi-line fish prompt.

### Desktop notifications

Programs can raise a desktop notification with OSC 9 or OSC 777, and a bell
raises one too:

```sh
printf '\033]9;Build finished\a'                  # OSC 9: a message
printf '\033]777;notify;Tests;12 passed\a'        # OSC 777: title and body
sleep 3; printf '\a'                              # a bell; switch tabs meanwhile
```

By default a tab notifies only when it isn't in front of you: it's a background
tab, or its window isn't focused. The notification's title names the tab, and
clicking it raises the window and switches to that tab. ConEmu's numbered
`OSC 9;N` commands (progress `9;4`, working directory `9;9`, …) never notify.
Bells notify at most once every 5 seconds per tab, and programs can send a
burst of three, then one every 2 seconds, so a bell storm can't flood the
desktop. Configure it in `config.toml`:

```toml
[notifications]
bell = true          # bells
osc9 = true          # OSC 9
osc777 = true        # OSC 777
when = "unfocused"   # or "always"
```

In tmux control mode, bells and notifications from any pane, in any tmux window,
work the same way. In classic tmux, tmux itself swallows OSC 9 and 777 unless the
program wraps them in tmux's passthrough (`set -g allow-passthrough on`); bells
get through.

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
