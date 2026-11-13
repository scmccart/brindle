# Brindle

[![CI](https://github.com/scmccart/brindle/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/scmccart/brindle/actions/workflows/ci.yml)

A GPU-accelerated terminal emulator for Linux and Windows 11, written in Rust
on [GPUI](https://crates.io/crates/gpui), the toolkit Zed is built with.
Terminal emulation comes from `alacritty_terminal`; rendering goes through
GPUI's Vulkan renderer on Linux and its Direct3D 11 renderer on Windows.

Brindle treats tmux as a native windowing layer, not just a program running
inside the terminal: tmux windows become tabs and panes are drawn natively,
while sessions stay in tmux, persistent and detachable.

- **Tabs** in a client-side-decorated title bar: drag to reorder, middle-click to close.
  A new tab starts in the active tab's directory: on Linux the foreground
  program's, on Windows the one its shell last reported (see
  [Shell integration](#shell-integration)).
- **Zones**: split any local tab horizontally (ctrl-shift-e) or vertically (ctrl-shift-o)
  into a binary tree of zones with native GPUI header bars and controls, move between
  them with alt-arrows, and close a zone with ctrl-shift-x.
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

## Remote hosts and containers

Profiles can run on a Linux host over SSH or inside an existing Docker/Podman
container. Brindle keeps drawing the terminal locally; a small headless
`brindle-remote` helper owns the remote PTY, resize and `/proc` process queries.
It is copied to a versioned, user-only cache on first connection and reused
while its version matches. Output remains the terminal's original byte stream,
so OSC 7 directories, OSC 9/777 notifications, OSC 7501 program status,
clipboard/color replies and shell-integration marks work as in a local tab.
Zone splits stay on the same endpoint and inherit its remote directory.

SSH uses your normal OpenSSH configuration, agent and host aliases:

```toml
[[profiles]]
name = "A8"
remote = { type = "ssh", host = "A8" }

[[profiles]]
name = "A8 tmux"
remote = { type = "ssh", host = "A8", user = "dev", port = 22 }
tmux = "control"
tmux_session = "main"
```

`identity_file` and `ssh_args` are optional SSH fields. Brindle never stores
credentials and leaves host-key verification to OpenSSH.

For an already-running container:

```toml
[[profiles]]
name = "dev container"
remote = { type = "container", container = "my-devbox" }

[[profiles]]
name = "dev container tmux"
remote = { type = "container", engine = "podman", container = "my-devbox", user = "1000" }
tmux = "control"
```

The engine defaults to `docker`. Brindle copies the helper to a unique,
root-owned path under `/tmp`, starts it with `exec -i` as the profile user, and
removes it when the connection ends. It does not create or start containers.
Remote control-mode profiles run `tmux -C` on the endpoint, so remote tmux
windows and panes are native Brindle tabs and panes.
If the connection drops, Brindle notices within about 30 seconds (it sends a
no-op command when tmux is quiet, and any reply or output counts), ends the
session as a detach ("connection lost") and, in the background, asks the
server to detach its client. The remote tmux session keeps running.
Official Windows archives include the static x86-64 Linux helper beside
`brindle.exe`. For a Windows source build, build that helper in Docker and
point Brindle at it:

```powershell
docker run --rm -v "${PWD}:/src" -w /src rust:1.99.1 sh -c `
  "apt-get update -qq && apt-get install -y -qq musl-tools && rustup target add x86_64-unknown-linux-musl && cargo build --locked --release -p brindle-remote-agent --target x86_64-unknown-linux-musl"
$env:BRINDLE_REMOTE_AGENT = "$PWD/target/x86_64-unknown-linux-musl/release/brindle-remote"
```

### Terminal server

On Linux and Windows, every local tab runs in the terminal server, which is
`brindle-remote` in two roles. Holders (`brindle-remote hold`) own the PTYs
and are the programs' parents; the core (`brindle-remote serve`) runs an
emulator per session, so a client that attaches or reconnects gets the
screen replayed. Brindle starts them when the first local tab opens, and they
exit when nothing is left to serve. The core can be restarted or upgraded
without ending a session: a newer core takes the holders over and rebuilds
every screen, and holders of an older build drain, taking no new sessions.
Their sockets, locks and logs (`core.log`, `holder-*.log`) are in
`$BRINDLE_SERVER_DIR`, else `$XDG_RUNTIME_DIR/brindle`, else
`~/.cache/brindle/run`. A socket path must fit in 108 bytes, so keep a custom
`BRINDLE_SERVER_DIR` short. On Windows the directory is
`%LOCALAPPDATA%rindleun` (`run-elevated` for an elevated Brindle), or
`%BRINDLE_SERVER_DIR%`, and the roles run from a per-build copy of
`brindle-remote.exe`, so a rebuild or an upgrade never needs a running one
replaced.

The helper is therefore required, found through `BRINDLE_SERVER_HELPER`, else
on Linux `BRINDLE_REMOTE_AGENT`, else `brindle-remote-linux-x86_64` or
`brindle-remote` beside `brindle`, and on Windows `brindle-remote.exe` beside
`brindle.exe`. (On Windows `BRINDLE_REMOTE_AGENT` still names the Linux helper
for remote profiles.) Without it, a new tab says so and tries again for the
next one. The Linux release archive and `scripts/install.sh` ship it.

`brindle-remote list`, `kill`, `status` and `stop` manage the server from a
shell on both platforms, with the same `BRINDLE_SERVER_DIR`; none of them
starts one.

Closing a zone or a tab ends its programs. Quitting Brindle (ctrl-shift-q,
closing its window, a crash, or the desktop stopping it at logout) leaves
every local tab's programs running in the server, detached, and the next
launch attaches them again. A command run with `brindle -e` ends at quit instead. A session
also ends when its program exits, and if Brindle's connection drops (say the
core crashes) the tabs reconnect and get their screens back; a session the
server no longer has shows a red "session lost" line and the tab stays open.

Every launch except `--no-restore` attaches the detached sessions, quietly (a
replayed bell or notification doesn't fire again): restored zones take back
their own sessions ([session restore](#session-restore)), and any others
(sessions a `--no-restore` launch left, say) open as tabs after them, oldest
first. To pick one by hand, "Attach Session…" in the command palette lists
the detached sessions, newest first, with their directory and age: enter
attaches one into a new tab, ctrl-shift-delete ends the selected one.
`brindle --attach ID` opens session `ID` (from `brindle-remote list`) as a
tab, or a tab saying why it can't.

The server's roles run in systemd scopes of their own
(`brindle-serve-*.scope`, `brindle-hold-*.scope`) when there is a systemd
user manager, so logging out of a desktop that stops Brindle's app scope
doesn't take the sessions with it. `BRINDLE_NO_SCOPE=1` starts them without
one.

When a Brindle whose `brindle-remote` is newer than the running core (after a
reinstall, say) opens its first local tab, it hands the sessions over to a core
of the new build, and their programs carry on. `brindle-remote status` then
shows the old holders draining until their sessions end.

These commands manage the server of that directory, and never start one:

```sh
brindle-remote list [--json]     # id, pid, attached/detached, policy, age, title, command, directory
brindle-remote kill ID           # hang one session up (kill --all for every one)
brindle-remote status [--json]   # the core's version, pid and uptime, and its holders
brindle-remote stop              # hang every session up; the core and holders exit
brindle-remote check --holder-protocols 1,2   # whether this binary can take those holders over
```

With no server running, `list` prints nothing and `status` says so.

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

`cargo build` also builds the `brindle-remote` helper into the same directory,
where `brindle` finds it for the [terminal server](#terminal-server).

`ui/` is `brindle-ui`, the shared component crate (see [`ui/README.md`](ui/README.md)).
Its gallery shows every component, in light and dark:

```sh
cargo run -p brindle-ui --example gallery              # opens the gallery window
cargo run -p brindle-ui --example gallery -- --check   # drives the components' spec scenarios, prints ok/FAIL/SKIP per check
```

Both open a window. `--check` exits non-zero when a check fails; its pointer
and key-release checks run on Windows and are skipped elsewhere.

`rust-toolchain.toml` pins the Rust version that CI and releases build with.
rustup installs it on the first `cargo` run in the checkout.

On Windows 11 (Windows 10 isn't supported), install the MSVC toolchain and the
Windows 11 SDK (Visual Studio Build Tools with "Desktop development with C++"),
then, in PowerShell:

```powershell
scripts/fetch-conpty.ps1
cargo build --release
.\target\release\brindle.exe
```

`scripts/fetch-conpty.ps1` downloads Microsoft's packaged ConPTY (the console
host Windows Terminal ships, MIT-licensed), checked against a pinned hash, and
the build embeds it in `brindle-remote.exe`, the terminal server's helper,
which hosts each tab with it, so what
programs write reaches the terminal unchanged: mouse modes, synchronized
updates, hyperlinks, notifications and color queries. A build made without it
warns, and its tabs fall back to the ConPTY built into Windows, which repaints
output instead of passing it through and drops mouse modes. The helper logs a
warning when it falls back. `BRINDLE_CONPTY=inbox` in a profile's environment
picks the inbox host for that session. Run the script before
`cargo install --path .` too.

GPUI's release build compiles its shaders with the SDK's `fxc.exe`, which it
looks for on `PATH` and in SDK 10.0.26100.0. With another SDK version, point
`GPUI_FXC_PATH` at it, for example
`$env:GPUI_FXC_PATH = "${env:ProgramFiles(x86)}\Windows Kits\10\bin\<version>\x64\fxc.exe"`.

`Cargo.lock` starts from gpui 0.2.2's own lockfile; newer transitive versions
(e.g. `libc`) break its dependencies.

## Testing

```sh
cargo test                                      # unit tests; no display needed
BRINDLE_E2E=1 cargo test --test e2e             # end-to-end suite
BRINDLE_E2E=1 cargo test --test e2e -- titles   # only cases whose names contain "titles"
BRINDLE_E2E=1 cargo test --test e2e -- --list   # list the cases
BRINDLE_E2E=1 cargo test --test e2e -- --shard 2/4  # every 4th case, from the 2nd
```

The end-to-end suite in `tests/e2e/` runs Brindle against throwaway tmux
servers and compares what it shows with tmux: screen text, window pixels and
replies to color queries. It is opt-in because it opens Brindle windows (under
Xwayland, display `:0` unless `BRINDLE_E2E_DISPLAY` says otherwise) and needs
`tmux`, `bash` and `less`. Without `BRINDLE_E2E=1`, `cargo test` skips it.
Each case keeps its logs, screen dumps and window captures in
`target/tmp/e2e/<case>/`.

To run the suite headless, the way CI does, wrap it in `scripts/with-xvfb`
(after `sudo apt install xvfb x11-utils openbox mesa-vulkan-drivers dbus`):

```sh
BRINDLE_E2E=1 scripts/with-xvfb cargo test --test e2e
```

It starts a private X server with a window manager, renders with Mesa's
software Vulkan driver, sets a UTF-8 locale and gives the run its own session
bus, so nothing opens on the desktop.

CI (`.github/workflows/ci.yml`) checks the new commits' messages, and runs the
unit tests, a release build, and the end-to-end suite that way on Ubuntu
26.04, against tmux 3.8 (built from source), for pushes to `main` and every
pull request. The suite also runs against tmux 3.6a (Ubuntu's) when the
workflow is started by hand with the `old-tmux` input
(`gh workflow run ci.yml --ref <branch> -f old-tmux=true`). Each tmux
version's suite is split into 4 shards that run as separate jobs. A failed
shard uploads its `target/tmp/e2e/` as an artifact. A Windows job runs the
release build, the unit tests and the harness self-tests on `windows-latest`.

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

## Commit messages

Commits follow [Conventional Commits](https://www.conventionalcommits.org/):
`type(scope): subject`, with a lower-case scope, a subject that starts
lowercase (acronyms such as README are fine), and `!` after the scope for a
breaking change, as in `feat(tmux): draw pane title lines`. Merges keep their
`Merge …` subjects.

[commitlint](https://commitlint.js.org/) checks them, with the rules in
`commitlint.config.mjs`. It needs Node 22.12 or later:

```sh
git config core.hooksPath scripts/hooks   # once per clone: check each new commit
scripts/lint-commits                      # check the commits not on origin/main
```

## Installing

Each [GitHub release](https://github.com/scmccart/brindle/releases) has a Linux
x86-64 archive, `brindle-<version>-x86_64-unknown-linux-gnu.tar.gz`, holding
`brindle`, the static remote helper `brindle-remote-linux-x86_64` and the
licences, plus a `SHA256SUMS` file. It's built on Ubuntu 26.04, so it needs a
glibc at least that new, along with the runtime libraries of the build
dependencies above (xkbcommon, xcb, FreeType), which desktop installs have.
Keep the helper beside `brindle`, where local tabs (the terminal server) and
remote profiles look for it:

```sh
sha256sum -c --ignore-missing SHA256SUMS
tar -xzf brindle-<version>-x86_64-unknown-linux-gnu.tar.gz
install -Dm755 -t ~/.local/bin brindle-<version>-x86_64-unknown-linux-gnu/brindle \
  brindle-<version>-x86_64-unknown-linux-gnu/brindle-remote-linux-x86_64
~/.local/bin/brindle --install-desktop   # launcher entry and icon
```

To update, install the new release's files the same way.

From a checkout, after installing the build dependencies above:

```sh
scripts/install.sh                     # binary to ~/.local/bin, plus launcher entry and icon
PREFIX=/usr/local scripts/install.sh   # system-wide; builds as you, uses sudo only to copy
```

The script always installs under `$PREFIX/share`, even if `XDG_DATA_HOME` is
set; `brindle --install-desktop` on its own follows `XDG_DATA_HOME`. To update,
re-run the script. A Brindle that is already running keeps running the
old build until it quits; launching it again only raises it.

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

### On Windows

There's no installer. The terminal server's helper, `brindle-remote.exe`,
carries the packaged ConPTY and unpacks it to
`%LOCALAPPDATA%\brindle\conpty\<version>` on first run; it must sit beside
`brindle.exe`. Each [GitHub release](https://github.com/scmccart/brindle/releases) has a
zip, `brindle-<version>-x86_64-pc-windows-msvc.zip`, with `brindle.exe`,
`brindle-remote.exe`, the Linux remote helper and the licences (`SHA256SUMS`
beside it lists its hash); unpack it anywhere, keeping the two exes together,
and run `brindle.exe`. Once releases are published to WinGet, it installs as
the `brindle` command on `PATH`, with the terminal server's `brindle-remote`
(`brindle-remote status`, `list`, `kill`, `stop`) beside it:

```powershell
winget install scmccart.brindle
winget upgrade scmccart.brindle
```

WinGet doesn't add Start Menu entries for programs like this, so add one
yourself, and remove it before uninstalling (WinGet doesn't know about it):

```powershell
brindle --install-desktop     # a Start Menu shortcut for this brindle.exe
brindle --uninstall-desktop
winget uninstall scmccart.brindle
```

From a checkout, fetch the packaged ConPTY first, so the build embeds it, and
put `brindle-remote.exe` (`cargo build --release` builds it) beside the
installed `brindle.exe`, or point `BRINDLE_SERVER_HELPER` at it:

```powershell
scripts/fetch-conpty.ps1
cargo install --locked --path .
brindle --install-desktop
```

Releases aren't code-signed yet, so SmartScreen may stop the first start of a
downloaded `brindle.exe` ("Windows protected your PC"): choose More info, then
Run anyway. Uninstalling either way keeps your config in `%APPDATA%\brindle`
and session state in `%LOCALAPPDATA%\brindle`. `scripts/package-windows.ps1`
builds the release zip from a release build, and `scripts/winget-manifest.ps1`
writes its WinGet manifest.

## Usage

```
brindle [-p PROFILE] [-d DIR] [--no-restore] [-e COMMAND ARGS...]
brindle --attach ID                 # Linux: a detached terminal server session
brindle --list-actions | --list-themes | --shell-integration bash|zsh|fish
brindle --install-desktop | --uninstall-desktop [--prefix DIR]
```

Brindle has one window. Launching it again while it runs (from the desktop or
a shell) hands the launch to the running Brindle and exits: the window comes
to the front, and `-p`, `-d`, `-e` or `--attach` open their tab there as the
active one, with `-e` starting in the directory it was typed in. A launch with
the debug options (`--send`, `--action`, `--dump-screen-after`, …) runs on its
own instead, restoring nothing and recording nothing. Brindles with different
`BRINDLE_STATE_DIR`s are separate.

On first launch Brindle writes a commented config to
`~/.config/brindle/config.toml`. The file is reloaded automatically when it changes.

| Keys | Action |
|---|---|
| ctrl-shift-t / ctrl-shift-w | new / close tab |
| ctrl-tab, ctrl-pageup/down | switch tab |
| alt-1 … alt-9 | go to tab (9 = last) |
| ctrl-shift-pageup/down | move tab |
| ctrl-shift-e / ctrl-shift-o | split zone right / down (in a local tab) |
| ctrl-shift-x | close zone (in a local tab) |
| alt-arrows | move between zones (in a local tab) |
| ctrl-shift-p | command palette (every command, with its shortcut) |
| ctrl-shift-space | new tab with a profile (a list you can filter by typing) |
| the strip's `⌄` | profile menu (see below) |
| right-click a tab / a zone header | tab menu / zone menu (see below) |
| ctrl-alt-1 … ctrl-alt-9 | new tab with profile N |
| ctrl-shift-c / ctrl-shift-v | copy / paste (middle click pastes the primary selection) |
| shift-pageup/down, shift-home/end | scrollback |
| ctrl-shift-up / ctrl-shift-down | previous / next prompt ([shell integration](#shell-integration)); one line without it |
| ctrl-shift-f | find in the scrollback (see below) |
| ctrl-= / ctrl-- / ctrl-0 | font size |
| ctrl-shift-, | settings (general options, profiles, theme picker with live preview, custom theme editor) |
| ctrl-, | edit config |

The `⌄` button at the end of the tab strip opens the profile menu: every profile
in config order, each with what it runs and its ctrl-alt-N key. Up/down move,
enter opens the profile in a new tab, typing jumps to a profile whose name
starts with the text, and escape, a click outside or `⌄` again closes the menu.
Choosing a control-mode profile whose session the window already shows switches
to that tab instead (the menu says "open in this window"), just as the
ctrl-shift-space list does; that list is the one to use to filter by name.

Right-clicking a tab makes it the active tab and opens its menu at the pointer:
Rename Tab (Rename Window on a tmux window's tab), Move Tab Left, Move Tab
Right (each disabled at its end of the strip) and Close Tab, which on a tmux tab
closes that tmux window. Right-clicking a zone header focuses that zone and
opens its menu: Split Zone Right, Split Zone Down (each disabled where the zone
is too small to split) and Close Zone. Each item shows its key and does what
the key does; the menus take the profile menu's keys.

Four actions have no default key, for working without a mouse: bind
`open_profile_menu` to open the profile menu below `⌄`, `open_tab_menu` to open
the active tab's menu below it, `open_zone_menu` to open the active zone's menu
below its header (at the top-left of a tab with one zone, which has no header),
and `focus_tab_strip` to put keyboard focus on the tab strip, where left/right
(wrapping), home and end switch tabs and enter or escape go back to the
terminal.

The settings dialog has three sections: General, Profiles and Theme. The section
list is the dialog's first Tab stop (shift-tab from where the dialog opens):
there up/down change the section at once (wrapping) and home/end go to the
first or last, then tab goes into the section. ctrl-tab and ctrl-pagedown
(next) and ctrl-shift-tab and ctrl-pageup (previous) change it from anywhere in
the dialog. An editor with unsaved changes keeps its section until you save or
discard. It opens on General, which holds what a window does when
its tmux session ends ([tmux control mode](#tmux-control-mode)); pick it from
the select's menu (enter, space, up or down opens it), and the change is saved
at once.

In the Theme section, the theme list has focus: up/down (and home/end) preview a
theme and enter applies it as the default; escape puts the previous theme back.
Tab moves from the list to its buttons and back, as in the profile list. `n` makes a new theme (a copy of
the selected one), `e` edits a custom theme and `d` deletes one. Built-in themes
can't be changed, so editing one starts from a copy. In the editor, every valid
color shows on the tabs as you type. Saving
updates `config.toml` in place: your comments and the rest of the file are kept.

The Profiles section lists your profiles with their ctrl-alt-N numbers. Enter or `e` edits one, `n` makes a new one, `c` duplicates
the selected one, `d` deletes it, and alt-up/alt-down reorder the list. The
editor covers every profile setting, including the remote target (`none`, `ssh` or
`container`, with the fields of the chosen kind); arguments, environment (`KEY=value`) and
tmux arguments are written as shell words. A profile's theme and font size show
on its open tabs while you edit; the other settings apply to new tabs. Renaming
a profile keeps its open tabs linked to it and updates `default_profile`.

In both editors, tab and shift-tab move through the fields, then the buttons and
the dialog's close button, then the section list, and wrap around without
leaving the dialog (up and
down don't move between fields). Enter saves from a text field or a checkbox;
on a focused button, enter or space presses that button alone. A choice field (a profile's theme, tmux mode, remote kind or container
engine) works like a select: enter, space, up or down opens its menu, where
up/down move, enter picks and escape closes the menu; left and right do nothing.
Otherwise escape discards the editor's changes and goes back to the list. A
click outside the dialog doesn't close it.

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
### Find

ctrl-shift-f (or "Find" in the command palette) opens a find bar over the
focused terminal. Matches are highlighted as you type, and the current one is
scrolled into view.

| Keys in the find bar | Action |
|---|---|
| enter / shift-enter | next match up (older output) / down |
| alt-c | match case (off by default) |
| alt-r | regular expression (off by default: the text matches literally) |
| escape | close, and put the view back where it was |

The bar shows the current match's number, counted from the newest output, and
the total. Clicking the terminal leaves the bar open, so you can type into the
program while the matches stay highlighted. In tmux control mode each pane has
its own bar and searches only its own scrollback. `find_next`, `find_previous`
and `find_close` can be bound to keys of your own. Themes can set the highlight
colors with `search_match_background`, `search_match_foreground`,
`search_focused_match_background` and `search_focused_match_foreground`;
otherwise they come from the theme's yellow.

**Breaking change:** ctrl-shift-p used to open the profile list and now opens the
command palette. The profile list is on ctrl-shift-space (the strip's `⌄` button
opens the profile menu instead); to get the old key back, bind
`"ctrl-shift-p" = "open_profile_picker"`.

### Session restore

Every launch reopens the tabs you had when Brindle last quit (the old
`restore_session` setting is ignored; restoring is always on), then opens
what it asked for (`-p`, `-d`, `-e` or `--attach`) as the active tab:

- the window's size, its tabs in order, and its active tab;
- each tab's profile and zone split tree, with each zone in its shell's last directory and the active zone focused;
- classic tmux tabs, which reattach to their session;
- control-mode sessions, attached once each. Their windows come back as tabs
  where they were, and tmux's current window and panes are left as they are.

On Linux, a local tab or zone gets its terminal server session back: the same
program, still running, with its screen and scrollback. A zone whose session
has ended since (after a reboot or `brindle-remote stop`, say) isn't restored:
the other side of its split takes its place, and a tab with nothing left is
dropped. When nothing at all is left, Brindle opens one tab with the default
profile. On Windows, and for remote profiles, shells start fresh in the
recorded directory. A state file written by more than one window (before
Brindle had one) comes back as one window, with the first window's size.
`brindle --no-restore` skips restoring once, and leaves the detached
sessions alone.

The state is written to `~/.local/state/brindle/session.toml`
(`$XDG_STATE_HOME/brindle/`, or `$BRINDLE_STATE_DIR` when it is set) a second
or two after anything changes, and again on quit, so a crash or logout loses
at most the last couple of seconds. If a profile or directory has gone
since then, the default profile or the home directory is used. A tmux session
that no longer exists is created again. Closing the last tab forgets the
tabs, and quitting keeps everything that is open.

### Shell integration

Shells can mark their prompts and command output with OSC 133 sequences. Brindle
ships scripts that do this for bash, zsh, fish and PowerShell. Load one at the
**end** of your shell's startup file, after your prompt theme and anything that
sets `PROMPT_COMMAND` or defines `prompt`:

```sh
eval "$(brindle --shell-integration bash)"    # ~/.bashrc
eval "$(brindle --shell-integration zsh)"     # ~/.zshrc
brindle --shell-integration fish | source     # ~/.config/fish/config.fish
```

```powershell
brindle --shell-integration pwsh | Out-String | Invoke-Expression   # $PROFILE
```

The PowerShell script works in PowerShell 7 and Windows PowerShell 5.1, and also
reports the current directory (OSC 7). On Windows, where Brindle can't read
another program's directory, that report is what a new tab and a restored tab
start in; without it they start in your home directory. Tools that report the
directory themselves with OSC 7 or ConEmu's OSC 9;9 (oh-my-posh, starship and
the like, when configured to) work the same way.

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
osc7501 = true       # program status (OSC 7501), see below
when = "unfocused"   # or "always"
```

In tmux control mode, bells and notifications from any pane, in any tmux window,
work the same way. In classic tmux, tmux itself swallows OSC 9 and 777 unless the
program wraps them in tmux's passthrough (`set -g allow-passthrough on`); bells
get through.

#### Program status

Programs that speak the [Program Status Protocol][osc7501] (OSC 7501: Claude
Code, Codex, Terraform, Homebrew, …) tell Brindle what they are doing, and each
tab shows a dot for it: yellow while the program is **blocked** on you (a
permission, a question, a login), red when it **failed**, green when it is
**done** with something you haven't looked at, and grey while it is **working**.
A `blocked`, `done` or `error` report also raises a desktop notification, with
the program's message when it sent one. `working` and `blocked` end at the
program's next shell prompt or when it exits; `done` and `error` stay until you
type into that tab. Brindle answers the protocol's `OSC 7501 ; ?` query, so
programs that check first know to report. Any shell can report:

```sh
status() { printf '\033]7501;state=%s:msg=%s\033\\' "$1" "$(printf %s "$2" | base64 | tr -d '\n')"; }
status working "Syncing photos"
rsync -a ~/Photos backup:/photos && status done "Photos synced" || status error "rsync failed"
```

`notifications.osc7501 = false` keeps the dots and drops the notifications. In
tmux control mode, reports from any pane reach the window's tab (tmux passes the
sequence through); classic tmux swallows it like OSC 9.

[osc7501]: https://gist.github.com/mitchellh/7acae3abd8355c1c00287d67e96c913a

On Windows, notifications are toasts from "Brindle". Brindle registers that name
for your user account the first time it shows one (under
`HKCU\Software\Classes\AppUserModelId\Brindle.Terminal`), so toasts work without
a Start Menu shortcut too; `brindle --uninstall-desktop` removes the entry.
Clicking a toast brings its window to the front with its tab active. If Windows
won't hand Brindle the foreground, the taskbar button flashes instead. Turn
Brindle's toasts off or on under Settings > System > Notifications; while they're
off, Brindle logs one warning and carries on.

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

When a session goes away, by detaching, closing its last tab, or its last shell
exiting, its tabs close. If they were all the window had, `tmux_session_end`
decides what happens next:

- `"close_window"` (the default): the window closes, as when a local tab's shell
  exits, which quits Brindle.
- `"new_tab"`: a tab with your default profile opens in their place, or with the
  first non-tmux profile if the default is a control-mode one.
- `"ask"`: a palette offers "Close window", or a new tab with any profile.
  Escape closes the window; a click outside leaves the question open.

It can also be changed in the settings dialog's General section.

On Windows, control mode runs tmux inside WSL. Install tmux in your
distribution, then add a profile whose command is `wsl`:

```toml
[[profiles]]
name = "tmux (WSL)"
tmux = "control"
command = "wsl"
tmux_args = ["tmux"]
```

`tmux_args` are WSL's options, then `tmux`, then tmux's own options, for example
`["-d", "Ubuntu", "tmux", "-L", "work"]`. Brindle runs
`wsl -d Ubuntu --exec tmux -u -L work -C new-session -A -s main`. New sessions
start in tmux's default directory, because a Windows tab's directory means
nothing inside WSL.

Each session is shown at most once. Picking a control-mode profile whose
session the window shows switches to its tab, and the new-tab palette marks
such a profile with "open in this window"; ctrl-alt-N and ctrl-shift-t do the
same. Brindle doesn't attach a session that another Brindle (a debug run, or
one with another `BRINDLE_STATE_DIR`) shows: a palette asks what to do
instead, with these rows:

- "New tmux session…": create a new session instead (see below);
- "New tab: <profile>" for your other profiles.

Escape opens nothing, and a click outside leaves the question open. Profiles with the same `command`,
`tmux_args` and `tmux_session` count as the same session.

**tmux: New Session** (command palette, from any tab while you have a
control-mode profile) asks for a name and creates that session, showing it in
the current window next to your other tabs. Leave the name empty to let tmux
pick one. It starts in the current directory, on the server of the active
tab's session, or else of your default (or first) control-mode profile. If
tmux refuses the name, e.g. `duplicate session: work`, the prompt stays open
with that message. It has no default key; bind `tmux_new_session` if you want
one.

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
