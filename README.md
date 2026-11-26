# Brindle

[![CI](https://github.com/scmccart/brindle/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/scmccart/brindle/actions/workflows/ci.yml)

A GPU-accelerated terminal emulator for Linux and Windows 11, written in Rust
on [GPUI](https://crates.io/crates/gpui), the toolkit Zed is built with.
Terminal emulation comes from `alacritty_terminal`; rendering goes through
GPUI's Vulkan renderer on Linux and its Direct3D 11 renderer on Windows.

- **Tabs** in a client-side-decorated title bar: drag to reorder, middle-click to close.
  A new tab starts in the active tab's directory: on Linux the foreground
  program's, on Windows the one its shell last reported (see
  [Shell integration](#shell-integration)).
- **Zones**: split any local tab horizontally (ctrl-shift-e) or vertically (ctrl-shift-o)
  into a binary tree of zones, each a card in its terminal's colours with a header bar
  and controls once the tab has two or more. Move between them with alt-arrows (the
  active zone gets a focus ring when you move by keyboard), and close a zone with
  ctrl-shift-x.
- **Profiles**: command, args, working directory, environment, theme and font size per
  profile, editable in the settings dialog.
- **Agent tabs**: a profile with `type = "agent"` runs an ACP (Agent Client
  Protocol) agent in the terminal server and shows its conversation in a zone:
  prompts, a queued prompt, tool calls, permission requests and thoughts, kept
  across quits like a terminal's session. See [Agent tabs](#agent-tabs).
- **Shell integration** (OSC 133, and VS Code's OSC 633): jump between prompts, select or
  copy the last command's output or command line, and see failed commands marked in the
  gutter.
- **Desktop notifications** from OSC 9, OSC 777, kitty's OSC 99 and bells in tabs
  you can't see; clicking one takes you to its tab. OSC 99 notifications can be
  updated, closed, given buttons and silenced, and report clicks back.
- **Progress bars** from ConEmu's OSC 9;4 (cargo, winget): a thin bar along the
  bottom of the tab, and of each zone's header in a split tab.
- **Context badges** from systemd's OSC 3008 (`run0`, containers, VMs, remote
  logins): a yellow warning icon on a tab while it runs as root. On Windows the
  same icon marks a tab whose program runs elevated (labelled Administrator).
- **Pointer shapes** from OSC 22, with kitty's stack and queries: a resize
  pointer over an editor's split border, a hand over a button.
- **Themes**: five built in, plus your own. The settings dialog (ctrl-shift-,)
  previews themes live on your open tabs and has an editor for custom themes.
- **Fonts**: the terminal font's family, size and line height, and the family of
  the tab strip, dialogs and menus, picked from your installed fonts in the
  settings dialog with a live preview.

## Remote hosts and containers

Profiles can run on a Linux host over SSH or inside an existing Docker/Podman
container. Brindle keeps drawing the terminal locally, and the programs run in
the endpoint's own terminal server (see [Terminal server](#terminal-server)),
which Brindle reaches through `brindle-remote relay`, one connection per
endpoint for all its tabs. The `brindle-remote` helper is copied to a
versioned cache on first connection (user-only on an SSH host) and reused
while its build matches. Output remains the terminal's original byte stream,
so OSC 7 directories, OSC 9/777/99 notifications, OSC 7501 program status, OSC 9;4
progress, OSC 3008 contexts, OSC 22 pointer shapes, clipboard/color replies and
shell-integration marks work as in a local tab.
Zone splits stay on the same endpoint and inherit its remote directory.

SSH uses your normal OpenSSH configuration, agent and host aliases:

```toml
[[profiles]]
name = "A8"
remote = { type = "ssh", host = "A8" }

[[profiles]]
name = "A8 dev"
remote = { type = "ssh", host = "A8", user = "dev", port = 22 }
```

`identity_file` and `ssh_args` are optional SSH fields. Brindle never stores
credentials and leaves host-key verification to OpenSSH.

For an already-running container:

```toml
[[profiles]]
name = "dev container"
remote = { type = "container", container = "my-devbox" }

[[profiles]]
name = "dev container (podman)"
remote = { type = "container", engine = "podman", container = "my-devbox", user = "1000" }
```

The engine defaults to `docker`. Brindle keeps the helper in a root-owned
directory, `/tmp/brindle-helper-<version>-<digest>`, which every user of the
container runs it from and only root can write, and starts it with `exec -i` as
the profile user. The helper stays there for later connections of the same
build, and other builds' directories are removed. If something that isn't
root's already has that name, Brindle runs nothing from it, leaves it alone and
says so in the tab. It does not create or start containers.

Official Windows archives include the static x86-64 Linux helper beside
`brindle.exe`, and so does `scripts/install.ps1` (see "On Windows" below).
For a Windows source build, `scripts/run-throwaway.ps1` builds that helper in
Docker (through `scripts/linux-helper.ps1`, which `install.ps1` shares) when
it is missing, older than its sources or not the binary's release, and points
Brindle at it for the run. To build it by hand instead, build it in Docker
and point Brindle at it:

```powershell
docker run --rm -v "${PWD}:/src" -w /src rust:1.99.1 sh -c `
  "apt-get update -qq && apt-get install -y -qq musl-tools && rustup target add x86_64-unknown-linux-musl && cargo build --locked --release -p brindle-remote-agent --target x86_64-unknown-linux-musl"
$env:BRINDLE_REMOTE_AGENT = "$PWD/target/x86_64-unknown-linux-musl/release/brindle-remote"
```

### Remote tabs persist

Remote tabs follow the same rules as local ones. Closing a zone or a tab ends
its program on the host. Quitting Brindle, a crash or a dropped network leaves
it running in the host's terminal server, detached, and the next launch
attaches it again with its screen: a restored remote zone takes back its own
session, once Brindle has connected to its host. A tab opened with
`brindle terminal -e` ends at quit, as a local one does. `brindle-remote list` on the host shows
the sessions.

Brindle notices a connection that has gone silent within about half a minute.
The host's tabs then say it was lost, and Brindle reconnects, pausing up to
30 s between tries for as long as a tab uses the host. Each tab gets its screen
back with its program still running (the same pid), and what you typed
meanwhile is sent once it is back. A session the host no longer has (ended
there with `brindle-remote kill`, say) shows the red "session lost" line, and
its tab stays open.

A host is normally connected when its first tab opens, and the connection
ends a few seconds after its last tab closes. To connect at every launch
instead, set `connect_at_launch = true` in the profile's `remote` table (it
is the "Connect at launch" switch in the settings dialog's profile editor, for
SSH and container targets alike):

```toml
[[profiles]]
name = "build host"
remote = { type = "ssh", host = "builder", connect_at_launch = true }

[[profiles]]
name = "dev container (eager)"
remote = { type = "container", container = "my-devbox", connect_at_launch = true }
```

Brindle then connects in the background and keeps the connection while it
runs, so the host's detached sessions open as tabs soon after the window
does, without opening a tab of the profile first. The window doesn't wait for
it. A host that can't be reached counts as offline and is retried, at most 60 s
apart. A stopped container is never started: it is offline too.

On a host that runs systemd, run `loginctl enable-linger` once, as the user
Brindle logs in as. Without lingering, the server runs inside your SSH login's
session. If the host's systemd ends a user's processes at logout
(`KillUserProcesses=yes` in `logind.conf`), closing your last login there ends
the server and every remote tab's session with it. A container's server ends
with the container.

"Attach Session…" in the command palette lists the detached sessions of every
host Brindle is connected to, under a heading for each: this machine first,
then the remote hosts in profile order, newest first within each. A host that
is still connecting, or offline, has a row that says so and does nothing when
chosen. Enter attaches a remote session in a new tab with the profile it was
started with (else the first profile of that host), and ctrl-shift-delete ends
it on its host.

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
`%LOCALAPPDATA%\brindle\run` (`run-elevated` for an elevated Brindle), or
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
launch attaches them again. A command run with `brindle terminal -e` ends at quit instead. A session
also ends when its program exits, and if Brindle's connection drops (say the
core crashes) the tabs reconnect and get their screens back; a session the
server no longer has shows a red "session lost" line and the tab stays open.

Every launch except `--no-restore` attaches the detached sessions, quietly (a
replayed bell or notification doesn't fire again): restored zones take back
their own sessions ([session restore](#session-restore)), and any others
(sessions a `--no-restore` launch left, say) open as tabs after them, oldest
first. To pick one by hand, "Attach Session…" in the command palette lists
the detached sessions, newest first, with their directory and age: enter
attaches one into a new tab, ctrl-shift-delete ends the selected one. It lists
remote hosts' sessions too, in groups ([Remote tabs persist](#remote-tabs-persist)).
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
brindle-remote list [--json]     # id, pid, attached/detached, owned/unowned, time detached, policy, age, title, command, directory
brindle-remote kill ID           # hang one session up (kill --all for every one)
brindle-remote status [--json]   # the core's version, pid and uptime, and its holders
brindle-remote stop              # hang every session up; the core and holders exit
brindle-remote check --holder-protocols 1,2   # whether this binary can take those holders over
```

With no server running, `list` prints nothing and `status` says so. An
agent session is listed with `agent: <activity>` (`idle`, `working`,
`blocked`, `done` or `error`) in the title's place, and `--json` gives it an
`agent` object; `kill` and `stop` end it as they end a terminal, without
asking.

## Building

On Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev \
  libxcb1-dev libfreetype-dev
cargo build --release
./target/release/brindle
```

Wayland, fontconfig and Vulkan are loaded at runtime (dlopen), so only their
runtime libraries are needed, and every desktop install has them.

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
BRINDLE_E2E=1 cargo test --test e2e -- --shard 2/3  # every 3rd case, from the 2nd
```

The end-to-end suite in `tests/e2e/` runs Brindle and checks what it shows:
screen text, window pixels and replies to color queries. It is opt-in because
it opens Brindle windows (under Xwayland, display `:0` unless
`BRINDLE_E2E_DISPLAY` says otherwise) and needs `bash` and `less`. Without `BRINDLE_E2E=1`, `cargo test` skips it.
Each case keeps its logs, screen dumps and window captures in
`target/tmp/e2e/<case>/`.

Cases that need a debug-only fault switch are skipped, with the reason, when the binary under test has none (`cargo test --release`, or `BRINDLE_E2E_BIN` naming a release build); `BRINDLE_E2E_SWITCHES_REQUIRED=1`, as CI sets, makes that a failure.

The Linux clipboard cases read and set the clipboard with `xclip`, as another program would, and skip with the reason when it is missing; `BRINDLE_E2E_CLIPBOARD_REQUIRED=1`, as CI sets, makes that a failure.

To run the suite headless, the way CI does, wrap it in `scripts/with-xvfb`
(after `sudo apt install xvfb x11-utils xclip openbox mesa-vulkan-drivers dbus`):

```sh
BRINDLE_E2E=1 scripts/with-xvfb cargo test --test e2e
```

It starts a private X server with a window manager, renders with Mesa's
software Vulkan driver, sets a UTF-8 locale and gives the run its own session
bus, so nothing opens on the desktop.

CI (`.github/workflows/ci.yml`) checks the new commits' messages, and runs the
unit tests, a release build, and the end-to-end suite that way on Ubuntu
26.04, for pushes to `main` and every pull request. One job builds the
suite and what it runs, and `scripts/e2e-bundle` packs that build as an
artifact. The suite is split into 3 shards that run as separate jobs, each
from that one build. A failed shard uploads its `target/tmp/e2e/` as an
artifact, and `cargo test --test e2e -- --shard K/3` reruns it locally.
A Windows job runs the
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

The performance suite in `tests/perf/` measures two things in a local shell tab,
and one in an agent zone:

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
- **Markdown streaming** (`agent-markdown-stream`): the time an agent zone
  takes to show a 64 KiB Markdown reply, from when a test agent starts
  answering until Brindle sees the reply's last line and quits, streamed in 32-byte chunks and again in one, and the
  difference. It needs the test agent `brindle-fake-acp-agent`, which
  `cargo build --release` builds beside Brindle.
- **Long conversation attach** (`agent-attach-long`): the time from a launch
  to its restored agent tab showing the end of a conversation of about
  15 MiB, which it attaches again and pages, until Brindle quits on it. It
  needs the same test agent.
- **Tool output streaming** (`agent-tool-output`): the time an agent zone
  takes to show the end of a command's 1 MiB output in its tool call's open
  row, streamed in 4 KiB pieces and again in one, and the difference. It
  needs the same test agent.

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
breaking change, as in `feat(settings)!: rename a config key`. Merges keep their
`Merge …` subjects.

[commitlint](https://commitlint.js.org/) checks them, with the rules in
`commitlint.config.mjs`. It needs Node 22.12 or later:

```sh
git config core.hooksPath scripts/hooks   # once per clone: check each new commit
scripts/lint-commits                      # check the commits not on origin/main
```

## Installing

On Linux x86-64, one command installs the latest release as you: `brindle`
and its remote helper in `~/.local/bin`, with the launcher entry and icon. It
works with `sh` or `bash`, checks the download against the release's
`SHA256SUMS` before installing anything, says so when `brindle` can't run on
the system (it needs the glibc and libraries named below), and uses `sudo`
only to write a `PREFIX` you can't:

```sh
curl -fsSL https://github.com/scmccart/brindle/releases/latest/download/install.sh | sh
```

Run it again to update; a Brindle that is already running keeps its old build
until it quits. `BRINDLE_VERSION` picks another release and `PREFIX` another
directory, and `--uninstall` removes it all again, keeping your config in
`~/.config/brindle`:

```sh
curl -fsSL https://github.com/scmccart/brindle/releases/latest/download/install.sh | env BRINDLE_VERSION=0.4.0 sh
curl -fsSL https://github.com/scmccart/brindle/releases/latest/download/install.sh | env PREFIX=/usr/local sh
curl -fsSL https://github.com/scmccart/brindle/releases/latest/download/install.sh | sh -s -- --uninstall
```

The one-command installers (`install.sh` here, `install.ps1` on Windows) come
with the releases after 0.3.0. Each release's copy installs that release
unless told another, and its `SHA256SUMS` lists them. That checksum catches a
corrupt download, not a compromised release; for provenance, check the
attestation as below. `BRINDLE_DOWNLOAD_BASE` points either script at a
directory of built assets instead (`file:///DIR`), which is how CI tests them.
The scripts are `scripts/get.sh` and `scripts/get.ps1` in a checkout.

To install by hand instead: each
[GitHub release](https://github.com/scmccart/brindle/releases) has a Linux
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

Each release's archives, and the binaries in them (`brindle`,
`brindle-remote-linux-x86_64`, `brindle.exe`, `brindle-remote.exe`), carry a
[build provenance attestation](https://docs.github.com/en/actions/security-guides/using-artifact-attestations-to-establish-provenance-for-builds)
from the release workflow. With the GitHub CLI, check a download, or a binary
you unpacked from one, before running it:

```sh
gh attestation verify brindle-<version>-x86_64-unknown-linux-gnu.tar.gz -R scmccart/brindle
gh attestation verify ~/.local/bin/brindle -R scmccart/brindle
```

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

To uninstall, run `scripts/uninstall.sh` (with the same `PREFIX`), which also
removes a one-command install, or `brindle --uninstall-desktop` followed by
`cargo uninstall brindle`. Your config in `~/.config/brindle` is kept either
way.

### On Windows

In PowerShell 5.1 or 7, one command installs the latest release for you,
without elevation: `brindle.exe`, the terminal server's helper
`brindle-remote.exe` and the Linux remote helper in
`%LOCALAPPDATA%\Programs\brindle`, that directory on your user `PATH`, and a
Start Menu shortcut. It checks the zip against the release's `SHA256SUMS`
before installing anything, and reports a failure as an error, leaving your
PowerShell open:

```powershell
irm https://github.com/scmccart/brindle/releases/latest/download/install.ps1 | iex
```

Run it again to update, even while Brindle runs: the running Brindle keeps its
old build until it quits, and the next launch reattaches its sessions through
the new version's terminal server. `iex` can't pass parameters, so the script
reads `$env:BRINDLE_VERSION` (another release), `$env:BRINDLE_PREFIX` (another
directory) and `$env:BRINDLE_UNINSTALL` (remove it all again, keeping your
config and session state). The variables stay set in that PowerShell, so the
script block form, which takes `-Version`, `-Prefix` and `-Uninstall`, suits a
one-off:

```powershell
$env:BRINDLE_UNINSTALL = 1; irm https://github.com/scmccart/brindle/releases/latest/download/install.ps1 | iex
& ([scriptblock]::Create((irm https://github.com/scmccart/brindle/releases/latest/download/install.ps1))) -Uninstall
```

To install by hand instead: the terminal server's helper,
`brindle-remote.exe`, carries the packaged ConPTY and unpacks it to
`%LOCALAPPDATA%\brindle\conpty\<version>` on first run; it must sit beside
`brindle.exe`. Each [GitHub release](https://github.com/scmccart/brindle/releases) has a
zip, `brindle-<version>-x86_64-pc-windows-msvc.zip`, with `brindle.exe`,
`brindle-remote.exe`, the Linux remote helper and the licences (`SHA256SUMS`
beside it lists its hash, and `gh attestation verify` checks it and both exes
as above); unpack it anywhere, keeping the two exes together,
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

From a checkout, `scripts/install.ps1` does it all. It fetches the packaged
ConPTY, builds `brindle.exe` and `brindle-remote.exe`, builds the matching
Linux remote helper in Docker (Linux containers; `-RemoteAgent PATH` uses a
helper you already have, which must be this version), and installs the three
together in `%LOCALAPPDATA%\Programs\brindle` (`-Prefix DIR` picks another
directory). It then adds that directory to your user `PATH` and creates the
Start Menu shortcut. Re-run it to update. `scripts/uninstall.ps1` (with the
same `-Prefix`) removes it all, as it does a one-command install. Prefer it
to `cargo install`, which leaves out the Linux helper, so remote profiles
would upload a stale one or none.

Releases aren't code-signed yet, so SmartScreen may stop the first start of a
downloaded `brindle.exe` ("Windows protected your PC"): choose More info, then
Run anyway. Uninstalling any of these ways keeps your config in
`%APPDATA%\brindle` and session state in `%LOCALAPPDATA%\brindle`. `scripts/package-windows.ps1`
builds the release zip from a release build, and `scripts/winget-manifest.ps1`
writes its WinGet manifest.

## Usage

```
brindle [--no-restore]              # the window: the restored tabs, else the default profile
brindle terminal [PROFILE] [-d DIR] [-e COMMAND ARGS...]   # a terminal tab
brindle agent [PROFILE] [-d DIR] [-p PROMPT]               # an agent tab
brindle --attach ID                 # a detached terminal server session
brindle --list-actions | --list-themes | --shell-integration bash|zsh|fish|pwsh
brindle --export-ui-tokens          # the built-in themes' UI tokens as CSS
brindle --install-desktop | --uninstall-desktop [--prefix DIR]
```

`brindle terminal` opens a terminal tab of PROFILE (matched without regard to
case; the default profile when it names none), in DIR, running COMMAND instead
of the profile's program. PROFILE and `-d` may come in either order, and `-e`
comes last: it takes every argument after it. `--config FILE`, `--no-restore`
and the debug options go before or after the subcommand. A PROFILE no profile
has is refused (status 2, listing the profiles) rather than opening another.

Brindle has one window. Launching it again while it runs (from the desktop or
a shell) hands the launch to the running Brindle and exits: the window comes
to the front, and `brindle terminal` or `--attach` open their tab there as the
active one, with `-e` starting in the directory it was typed in.
`brindle terminal` alone opens a new default-profile tab (a running Brindle
from before the subcommand only comes to the front). The running Brindle
checks the profile against its own config, so a profile added to the config
is found once it has reloaded the file, and an unknown one is refused with
the profiles it has. A launch with the debug options (`--send`, `--action`,
`--dump-screen-after`, …) runs on its own instead, restoring nothing and
recording nothing. Brindles with different `BRINDLE_STATE_DIR`s are separate.

### Upgrading from the flat options

The flat launch options are gone, with no deprecation period: each is now a
usage error that says what to type instead. Desktop launchers, shortcuts and
scripts that pass them need changing:

| Before | Now |
| --- | --- |
| `brindle -p NAME` | `brindle terminal NAME` |
| `brindle -d DIR` | `brindle terminal -d DIR` |
| `brindle -e COMMAND …` | `brindle terminal -e COMMAND …` |

A profile name that no profile has used to open the default profile with a
dim line saying so; it is now refused with status 2, before any window opens.
`brindle agent` opens an agent tab (see [Agent tabs](#agent-tabs)).

On first launch Brindle writes a commented config to
`~/.config/brindle/config.toml`. The file is reloaded automatically when it changes.

Fonts are two tables. `[font]` is the terminal's: `family` (a list, the first
installed one is used), `size` (4 to 96) and `line_height` (0.8 to 3.0).
`[ui_font]` is the chrome's (the tab strip, zone headers, dialogs, menus and
tooltips) and takes only `family`, a list too; without one installed, Brindle
uses the first installed of Inter, Cantarell, Ubuntu Sans, Ubuntu, Noto Sans,
DejaVu Sans, Liberation Sans, Segoe UI Variable Text and Segoe UI, else the
system's UI font:

```toml
[ui_font]
family = ["Inter", "Segoe UI"]
```

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
| ctrl-shift-c / ctrl-shift-v | copy / paste (middle click pastes the primary selection; with `right_click = "paste"`, right-click pastes the clipboard over a selection too, clearing it, and a program that reports the mouse gets the click instead; shift-right-click pastes in Brindle there. See [Clipboard](#clipboard)) |
| shift-pageup/down, shift-home/end | scrollback |
| ctrl-shift-up / ctrl-shift-down | previous / next prompt ([shell integration](#shell-integration)); one line without it |
| ctrl-shift-f | find in the scrollback (see below) |
| ctrl-= / ctrl-- / ctrl-0 | font size |
| ctrl-shift-, | settings (profiles, theme picker with live preview, custom theme editor, fonts, clipboard; also the `⌄` menu's Settings) |
| ctrl-, | edit config |

The `⌄` button at the end of the tab strip opens the profile menu: every profile
in config order, each with what it runs and its ctrl-alt-N key. Up/down move,
enter opens the profile in a new tab, typing jumps to a profile whose name
starts with the text, and escape, a click outside or `⌄` again closes the menu.
It ends with Settings, which opens the settings dialog.
The ctrl-shift-space list is the one to use to filter by name.

Right-clicking a tab makes it the active tab and opens its menu at the pointer:
Rename Tab, Move Tab Left, Move Tab Right (each disabled at its end of the
strip) and Close Tab. Right-clicking a zone header focuses that zone and
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

The settings dialog has four sections: Profiles, Theme, Fonts and Clipboard. The section
list is the dialog's first Tab stop (shift-tab from where the dialog opens):
there up/down change the section at once (wrapping) and home/end go to the
first or last, then tab goes into the section. ctrl-tab and ctrl-pagedown
(next) and ctrl-shift-tab and ctrl-pageup (previous) change it from anywhere in
the dialog. An editor with unsaved changes keeps its section until you save or
discard. It opens on Profiles.

In the Theme section, the theme list has focus: up/down (and home/end) preview a
theme and enter applies it as the default; escape puts the previous theme back.
Tab moves from the list to its buttons and back, as in the profile list.
Built-in themes carry a lock and can't be edited or deleted: `n` makes an editable copy of the selected theme, `e` edits a custom theme and `d` deletes one. In the editor, every valid
color shows on the tabs as you type. Saving
updates `config.toml` in place: your comments and the rest of the file are kept.

The Profiles section lists your profiles with their ctrl-alt-N numbers. Enter or `e` edits one, `n` makes a new one, `c` duplicates
the selected one, `d` deletes it, and alt-up/alt-down reorder the list. The
editor covers every profile setting, including the remote target (`none`, `ssh` or
`container`, with the fields of the chosen kind); arguments and environment (`KEY=value`) are
written as shell words. A profile's theme and font size show
on its open tabs while you edit; the other settings apply to new tabs. Renaming
a profile keeps its open tabs linked to it and updates `default_profile`.

In both editors, tab and shift-tab move through the fields, then the buttons and
the dialog's close button, then the section list, and wrap around without
leaving the dialog (up and
down don't move between fields). Enter saves from a text field or a checkbox;
on a focused button, enter or space presses that button alone. A choice field (a profile's theme, remote kind or container
engine) works like a select: enter, space, up or down opens its menu, where
up/down move, enter picks and escape closes the menu; left and right do nothing.
Otherwise escape discards the editor's changes and goes back to the list.
The dialog's close button (×) closes the whole dialog from anywhere, dropping unsaved changes, as ctrl-shift-, does. A
click outside the dialog doesn't close it.

The Fonts section has four fields, always editable: Terminal font, Size, Line
height and UI font. A font field shows the first installed family of its
config list, or "Default" for Brindle's own list; it opens like a select, onto a
searchable list of your installed families (the terminal font's lists the
monospace ones first, once Brindle has measured them): type to filter, up/down
move, enter picks and escape closes the list. Every valid change shows in the
window at once, a profile's own font size still winning on its tabs. Apply (or
enter in Size or Line height) writes the changed values and closes the dialog:
a chosen family goes to the front of that table's `family` list, keeping the
others after it as fallbacks, and "Default" removes the list. Cancel, escape
or another section drop the changes.

The Clipboard section has two switches, Copy on select (`copy_on_select`) and Right-click pastes (`right_click = "paste"`, which pastes over a selection too, clearing it; see [Clipboard](#clipboard)). Apply (or enter) writes the changed ones and closes the dialog, and open tabs follow at once; Cancel, escape or another section drop the changes.

Plain `ctrl-<letter>` always goes to the program, so a program's own keys (tmux's prefix, say) work as usual.

### Agent tabs

A profile with `type = "agent"` runs its command as an
[ACP](https://agentclientprotocol.com) agent (Agent Client Protocol, version
1) instead of a terminal. The agent runs in the terminal server, like a
terminal tab's program, so it keeps working while Brindle is closed and its
tab comes back at the next launch. Its arguments, directory, environment,
theme, font size and remote target mean what they do for a terminal profile;
the command is required.

```toml
[[profiles]]
name = "Agent"
type = "agent"
command = "my-acp-agent"
args = ["--acp"]
cwd = "/home/me/src/project"
env = { MY_AGENT_LOG = "warn" }
font_size = 14
```

`brindle agent [PROFILE] [-d DIR] [-p PROMPT]` opens an agent tab of PROFILE
(the config's `default_profile` when it is an agent profile, else the first
agent profile), in DIR (else the profile's `cwd`, else the directory it was
typed in). `-p PROMPT` is a first prompt, sent once the agent is ready, and
`-p -` reads it from standard input (a file or a pipe, at most 256 KiB of
UTF-8; one trailing line break is dropped). A terminal profile named after
`agent`, an agent profile named after `terminal`, and `agent` in a config
with no agent profile are refused with status 2. As with `brindle terminal`,
a launch beside a running Brindle is handed to it. From `brindle --help`:

```
    brindle agent [PROFILE] [-d DIR] [-p PROMPT] [OPTIONS]
                                 Open an agent tab
```

An agent zone shows the conversation, newest at the bottom, with the prompt
input under it. Tool calls show as rows with their status, a permission
request as buttons for its options, and the agent's thoughts as closed
blocks that Toggle Agent Thoughts opens. A tool call's row opens to show
what the agent sent with it: the diffs of the files it edits (added lines
green, removed red), a command's output as it runs and how it ended, its
text, and the files it works on. A running call's row is open and a
finished one's closed; a click on the row, or Toggle Tool Details for every
row, opens or closes it, and a row you opened or closed stays as you left
it. A permission request shows its call's diff above its buttons. Each diff
and output has a copy button (a diff copies as a unified diff). A long item
is cut (a diff keeps its start, an output its end), and the row says what
was left out. The zone's status dot follows the agent: working, blocked
(waiting on a permission), done or error, as a program's status does; a done
or error dot goes once you type in the prompt input. An agent's tab title is
the name it gives, else the profile's.

A long conversation opens at its newest messages: an attach (a relaunch, Attach
Session…) shows the newest 256 KiB of it, and an "Earlier messages" row above
the first loads the rest a page at a time as you scroll up to it (with the
wheel or shift-pageup), keeping what you are reading where it is. While a page
loads the row turns a spinner; one that couldn't be loaded says why, and loads
again when you scroll back to it or press it.

The agent's messages and thoughts are drawn as Markdown (CommonMark, with
GitHub's strikethrough, task lists, tables and autolinks): headings,
emphasis, inline code, code blocks in the terminal's font with a label and a
copy button, lists, quotes, rules, tables as a grid, and links. A line break
inside a paragraph reads as a space, as CommonMark says. A message is drawn
again from its whole text as it streams, so a code fence not yet closed shows
as code to the end. Raw HTML shows as written, images are never fetched (they
show as a muted `[image: ALT]` and their target, as a link), and footnotes,
math and smart punctuation stay as written. Your own messages, tool and
permission titles and notices stay plain text. Copying a message, or Copy
Last Agent Message, copies its Markdown source; Copy Last Code Block
(`copy_last_code_block`, no default key) copies the last code block of the
latest message that has one.

Links in a message are drawn underlined in the link colour. Ctrl-click opens
one, as in a terminal, and a plain click does nothing; the pointer is a hand
over a link while ctrl is held, and a link whose text differs from its
target shows the target when the pointer rests on it. `http` and `https`
links open at once. A link of another scheme Brindle opens (`mailto:`,
`file:`, `ssh:`, …) asks first, in a dialog showing the whole link, with
Cancel focused (enter and escape cancel, as does a click outside it) and
Open. Links Brindle wouldn't open in a terminal are refused the same way,
the line under the tab strip saying why.

| Keys | Action |
|---|---|
| enter / shift-enter | send the prompt / add a line |
| escape | stop the agent's turn |
| tab / shift-tab | move between the prompt input, the queued prompt's buttons, a permission request's buttons and the stop button; enter or space presses the button |
| shift-pageup/down | scroll the conversation |

A prompt sent while the agent works waits in a row above the input, and is
sent when the turn ends normally; another one stays in the input. A turn that
is stopped, fails or ends for a limit holds the waiting prompt instead (the
row says why), which you can send again or discard. Commands in the palette
while an agent zone has focus: Send Prompt, Stop Agent Turn, Remove Queued
Prompt, Send Held Prompt Again, Copy Queued Prompt, Copy Last Agent Message,
Copy Last Code Block, Toggle Agent Thoughts, Toggle Tool Details and Scroll
Agent to Bottom (the actions `agent_submit`, `agent_cancel`, `agent_unqueue`,
`agent_resend`, `copy_queued_prompt`, `copy_last_agent_message`,
`copy_last_code_block`, `toggle_agent_thoughts`, `toggle_agent_tool_details`,
`agent_scroll_to_bottom`),
with the zone and tab commands; the terminal's commands aren't listed there.

Splitting an agent zone opens another agent zone, with a session of its own,
on the same profile. Closing a zone or tab whose agent is working or waiting
on a permission asks first, naming the agents, with Cancel focused (enter and
escape cancel; tab then enter closes); closing an idle one, and quitting,
don't ask. Detached agent sessions are listed in the sessions palette ("Attach
Session…") as `agent · <activity>`, and `brindle-remote list` lists them too.

An agent that needs a login says so, with its own message: log in through the
agent's own command line, then open a new tab. An agent that exits, or speaks
another ACP version, leaves its conversation readable.

Not yet: syntax highlighting, logging in from the zone (`agents-login`), going on after an agent exits (`agents-session-recovery`),
and ACP version 2
(`agents-acp-v2`), each a change of its own; nor the agent's file and
terminal access, modes, slash commands, mixing agent and terminal zones in a
tab, finding or selecting text in a conversation, or editing agent profiles
in the settings dialog.

**Release note:** a Brindle from before agent tabs rejects a config with an
agent profile (`type` is an unknown key to it): it shows the error on its
config line and runs on the default settings. Keep agent profiles out of a
config that an older Brindle also reads.

### Clipboard

Ctrl-shift-c copies the selection and ctrl-shift-v pastes the clipboard. With
`copy_on_select = true`, finishing a drag-selection copies it too. Middle click
pastes Brindle's own primary selection. With `right_click = "paste"`, a
right-click pastes the clipboard, over a selection too, which clears the
selection; with an empty clipboard nothing is pasted and the selection stays.

A program that reports the mouse (htop, or vim with the mouse on) gets the
clicks, right-clicks included, and copies and pastes in its own way. Shift
gives the mouse back to Brindle, so shift-drag selects and shift-right-click
pastes there.

A copy or paste made with the mouse shows a small label beside the pointer:
"Copied" with a copy icon, "Pasted" with a paste icon, or "Nothing to paste"
with a ban icon when a right-click found no text on the clipboard. It fades after about a second, and it takes
no clicks, so a click on it reaches the terminal.

On Windows, copied text has CRLF line ends, as Windows Terminal's copies do:
some applications, Excel cells among them, read a bare line feed as nothing. Text a program sets with OSC 52 is copied as
the program sent it. Paste sends each line end as a carriage return, which is
what terminal programs expect.

### Links

Hold ctrl over a URL (`http`, `https`, `file`, `mailto`, `ftp`, `ssh`, `git`, …)
or over text a program marked as an OSC 8 hyperlink: the link is underlined
and the pointer becomes a hand. Ctrl-click opens it with `xdg-open`, even in
programs that use the mouse themselves (vim, htop, tmux with `mouse on`).
Ctrl-clicks elsewhere still reach those programs. Trailing punctuation and
enclosing brackets aren't part of a link, and a URL that wraps onto the next
row is still one link. Ctrl-drag no longer block-selects when it starts on a
link; alt-drag still does. Links in an agent's messages take the same
ctrl-click, but those an agent wrote with a scheme other than `http` or
`https` ask first (see "Agent tabs").

A link is whatever a program printed, so Brindle opens only what it would
detect itself:

- **Schemes:** `http`, `https`, `ftp`, `file`, `ssh`, `git`, `gemini`,
  `gopher`, `ipfs`, `ipns`, `mailto`, `news` and `magnet`, in any case. An OSC 8
  hyperlink with another scheme (`vscode:`, `ms-settings:`, a Windows path
  such as `C:\tool.exe`) is underlined but not opened.
- **`file:` links:** only when the host is empty, `localhost` or this
  machine's name. On Windows a file is shown selected in Explorer instead of
  opened, so a click never runs a program; a directory opens in Explorer.
  Only a path on a drive letter (`file:///C:/…`) is taken there.
- **Quotes and spaces** in a link are percent-encoded before it is opened.

When a link isn't opened, or the opener fails, the line under the tab strip
says why, naming the link, and the terminal is left alone. The notice goes
after 10 seconds, or when you press its × button or run "Dismiss Notices"
(`dismiss_notices`) from the command palette; config errors on that line
stay until the config is fixed.

To open links with something other than `xdg-open`, set
`BRINDLE_LINK_OPENER` to a program; it gets the link as its only argument.
"Copy Link Under Mouse" (`copy_link`) in the command palette copies the link
under the mouse; it has no default key, so bind one if you want it.

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
program while the matches stay highlighted. `find_next`, `find_previous`
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
what it asked for (`brindle terminal`'s tab or `--attach`'s session) as the active tab:

- the window's size, its tabs in order, and its active tab;
- each tab's profile and zone split tree, with each zone in its shell's last directory and the active zone focused;

On Linux, a local tab or zone gets its terminal server session back: the same
program, still running, with its screen and scrollback. A zone whose session
has ended since (after a reboot or `brindle-remote stop`, say) isn't restored:
the other side of its split takes its place, and a tab with nothing left is
dropped. When nothing at all is left, Brindle opens one tab with the default
profile. On Windows, shells start fresh in the recorded directory. A remote
zone gets its session back from its host once Brindle has connected to it, and
the host's other detached sessions open as tabs after the local ones. A state
file written by more than one window (before Brindle had one) comes back as one
window, with the first window's size.
`brindle --no-restore` skips restoring once, and leaves the detached
sessions alone.

The state is written to `~/.local/state/brindle/session.toml`
(`$XDG_STATE_HOME/brindle/`, or `$BRINDLE_STATE_DIR` when it is set) a second
or two after anything changes, and again on quit, so a crash or logout loses
at most the last couple of seconds. On Windows it is
`%LOCALAPPDATA%\brindle\session.toml`, and an elevated Brindle keeps its own in
`%LOCALAPPDATA%\brindle\state-elevated\`, so it runs beside an unelevated one as
a separate Brindle with its own session. If a profile or directory has gone
since then, the default profile or the home directory is used. Closing the last tab forgets the
tabs, and quitting keeps everything that is open. A state that can't be saved,
such as one with a tab in a directory whose name isn't valid UTF-8, leaves the
file as it was, with a line in the log, until it can be saved again.

### Shell integration

Shells can mark their prompts and command output with OSC 133 sequences, or with
VS Code's OSC 633 (`A` to `D` are the same marks, `E` reports the command line and
`P;Cwd=` the directory), so a shell set up for VS Code's terminal works too. Brindle
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
directory themselves with OSC 7, ConEmu's OSC 9;9 or OSC 633 `P;Cwd=` (oh-my-posh, starship and
the like, when configured to) work the same way. Only a directory on a drive
letter counts: a report of a network path (`\\server\share`) or a device path
is ignored, so map a drive to have new tabs follow you there.

The same scripts are in [`shell-integration/`](shell-integration/) if you'd rather
`source` a file. Brindle never edits your startup files or injects the scripts
itself. They only emit escape sequences, and terminals that don't understand
them ignore them. With a script loaded:

- **ctrl-shift-up / ctrl-shift-down** scroll so the previous or next prompt is at
  the top of the view. In a terminal without marks they still scroll one line.
  To keep line scrolling on those keys, bind `"ctrl-shift-up" = "scroll_line_up"`
  and `"ctrl-shift-down" = "scroll_line_down"`.
- **Select Last Command Output** and **Copy Last Command Output** are in the
  command palette, once the terminal has received a prompt mark, as Copy Last
  Command Line is (below); a terminal that has received none (a shell without
  the scripts, say) doesn't list them, and a key bound to them does nothing. They have no default key; bind
  `select_last_output` or `copy_last_output`. Copy leaves your selection alone.
- **Copy Last Command Line** (`copy_last_command`, no default key) copies the last
  command as you typed it. It is in the palette only once the terminal has received a
  prompt mark, so a shell with no script loaded doesn't list it, and a key bound to it
  does nothing there. The scripts report each line with OSC 633 `E` (fish 4
  reports it on its own output mark), so a right prompt or a wrapped line never gets
  in; without a report it copies the text between the command's `B` and `C` marks.
  bash reports a line only when it adds a history entry, so a command
  `HISTCONTROL` or `HISTIGNORE` leaves out is read from the screen instead.
- A thin red bar (the theme's red) marks prompts whose command exited non-zero.
  Turn it off with `mark_failed_commands = false`.

Marks follow their text as it scrolls, reflows on resize, and is cleared. bash needs 4.4 or newer
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
osc99 = true         # kitty's OSC 99, see below (off: no replies either)
osc7501 = true       # program status (OSC 7501), see below
agent = true         # agent tabs: a permission, a finished turn, a failure
when = "unfocused"   # or "always"
```

An agent tab notifies when its agent asks for a permission (with the request's
title), ends a turn normally (`Done`) or fails (with the failure's message),
titled with the agent's name and the tab's. The same focus rule and rate limits
apply, and a conversation replayed when Brindle attaches to it again notifies
nothing. `notifications.agent = false` keeps the tab's status dot and drops the
notifications.

Whether tool call rows open on their own is the `[agent]` table's:

```toml
[agent]
tool_details = "open"              # a running call's row: "open" or "closed"
tool_details_when_done = "closed"  # a finished call's row
```

A row opened or closed by hand stays as you left it, and a change applies
to the open zones when the config is saved.

#### kitty's notification protocol (OSC 99)

Programs that speak [kitty's desktop notification protocol][osc99] can do more
than raise a notification. Brindle supports:

- **Titles and bodies, in chunks** (`d=0` holds a notification for more), plain
  or base64 (`e=1`, cut anywhere between chunks), named by `i=`.
- **Updates and closes:** a notification with the id of one still open replaces
  it; `p=close` closes it, and `p=alive` lists the terminal's open ids.
- **Clicks:** `a=focus` (the default) raises the window and shows the tab, as for
  other notifications; `a=report` writes `ESC ] 99;i=<id>; ESC \` back to the
  program; `a=-focus` leaves the window where it is. `c=1` reports the close too.
- **Buttons** (`p=buttons`, labels separated by U+2028, up to 4): shown as
  freedesktop actions where the notification service shows them, and as toast
  buttons on Windows. With `a=report`, pressing button N writes
  `ESC ] 99;i=<id>;N ESC \`, without raising the window. Some services hide or
  fold actions, so a program asking a question should keep its tab as a fallback.
- **When and how:** `o=unfocused` and `o=invisible` can only make a notification
  quieter than `when`; `s=silent` (base64) shows it without a sound; on Linux,
  `u=` sets its urgency and `w=` its timeout.
- **The query** `p=?`, answered with what Brindle supports there.

Icons, named sounds, application names and types are ignored. Replies go only to
the program's own terminal.

```sh
printf '\033]99;i=build:d=0;Build\033\\\033]99;i=build:p=body;running\033\\'
sleep 5
printf '\033]99;i=build:d=0;Build\033\\\033]99;i=build:p=body;3 failed\033\\'  # updates it
printf '\033]99;i=build:p=close;\033\\'                  # and closes it
```

[osc99]: https://sw.kovidgoyal.net/kitty/desktop-notifications/

#### Program status

Programs that speak the [Program Status Protocol][osc7501] (OSC 7501: Claude
Code, Codex, Terraform, Homebrew, …) tell Brindle what they are doing, and each
tab shows a dot before its title for it: yellow while the program is **blocked** on you (a
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

`notifications.osc7501 = false` keeps the dots and drops the notifications.

[osc7501]: https://gist.github.com/mitchellh/7acae3abd8355c1c00287d67e96c913a

#### Progress

Programs that report progress with ConEmu's `OSC 9;4`, as Windows Terminal and
Ghostty show it, get a 2-pixel bar along the bottom of their tab, and of their
zone's header in a split tab. The tab shows its most pressing zone: an error,
then a pause, then the least far along, then one of unknown length.

```sh
printf '\033]9;4;1;30\033\\'   # normal, 30%: the muted foreground, a third of the way
printf '\033]9;4;2\033\\'      # error, at the last percent: the theme's red
printf '\033]9;4;4;50\033\\'   # paused, 50%: the theme's yellow
printf '\033]9;4;3\033\\'      # under way, length unknown: a faint bar across the tab
printf '\033]9;4;0\033\\'      # done: no bar
```

The bar also goes at the next shell prompt (with shell integration), when the
program exits and on a reset, and a detached tab keeps it in the terminal
server. It never notifies.

Recent cargo releases report build progress this way only in terminals they
recognise (Windows Terminal, ConEmu, Ghostty, WezTerm), not yet Brindle, so
turn it on for Brindle with `CARGO_TERM_PROGRESS_TERM_INTEGRATION=true` or in
`~/.cargo/config.toml`:

```toml
[term]
progress.term-integration = true
```

winget reports its downloads and installs the same way.

#### Context badges

systemd 257 and later announce where a terminal's output comes from with
OSC 3008, the UAPI group's [context signalling][uapi15]: `run0` (elevation),
`systemd-nspawn` and `machinectl` (containers and VMs), ssh logins with
systemd on the far side, and more. Brindle keeps each terminal's stack of
contexts and shows the innermost notable one as a badge after the tab's title
(the active zone's), and after each zone header's title in a split tab:

- an elevation or privilege change (`elevate`, `chpriv`), or a login session
  whose target user is `root` (what `run0` reports): a warning icon in
  the theme's yellow, with no name beside it; resting the pointer on it shows
  the target user ("Running as root"). Brindle leaves what the program shows
  itself alone: `run0` tints the background red and puts a glyph in the title;
- a remote host (`remote`), a container or a VM: `user@host`, the host, or the
  container's or VM's name, in the muted foreground.

On Windows the elevation badge comes from the system instead. A local tab whose
foreground program runs elevated shows the same warning icon, and resting the
pointer on it shows "Running as Administrator". Brindle reads the token of the
tab's newest child process, so a program started as administrator from the
tab's shell shows it, whether Brindle runs elevated or not. Windows `sudo` in
inline mode doesn't: its elevated shell is no longer a child of the tab's
process by the time its prompt shows, so the badge shows for about a second at
most. An elevated Brindle's tabs show it too, since their programs
are elevated. It takes precedence over any context a program announces, so
a program can't swap the warning for a muted label. It comes up within about
a second of the program's start and goes within about a second of its exit,
as the tab's title does. A program started elevated in its own window
(`Start-Process -Verb RunAs`) counts too, while it runs as the tab's newest
child. When Brindle can't read a program's token (a process it isn't allowed
to open), no badge shows, so an elevated program can go unmarked.

Shell and command contexts, and sessions for any other user, show no badge. Labels are cut to 24
characters, and a narrow tab cuts its title to 8 characters before it cuts a
label badge.

```sh
printf '\033]3008;start=a1;type=elevate;user=me;targetuser=root\033\\'  # a yellow warning icon
printf '\033]3008;start=s1;type=session;targetuser=root\033\\'          # what run0 sends: the same icon
printf '\033]3008;start=c1;type=container;container=web\033\\'          # "web", muted
printf '\033]3008;end=a1\033\\'   # ends a1 and everything started inside it: no badge
```

Contexts outlive resets, prompts and full-screen programs; they end when their
program sends the end, or when the terminal's program exits. A detached tab keeps
them in the terminal server. Up to 32 nest, and malformed sequences are ignored.
A context whose end never arrives (an ssh connection that dropped) keeps its
badge until the zone's program exits.

The badge is informational: any program can print these sequences, so a badge
can be spoofed, and Brindle grants nothing on the strength of one.

[uapi15]: https://uapi-group.org/specifications/specs/osc_context/

On Windows, notifications are toasts from "Brindle". Brindle registers that name
for your user account the first time it shows one (under
`HKCU\Software\Classes\AppUserModelId\Brindle.Terminal`), so toasts work without
a Start Menu shortcut too; `brindle --uninstall-desktop` removes the entry.
Clicking a toast brings its window to the front with its tab active. If Windows
won't hand Brindle the foreground, the taskbar button flashes instead. Turn
Brindle's toasts off or on under Settings > System > Notifications; while they're
off, Brindle logs one warning and carries on.

### Pointer shapes

Programs that use the mouse can set the pointer over their terminal with
OSC 22, xterm's sequence with [kitty's extensions][kitty-pointer]: a stack per
screen (primary and alternate) of up to 16 shapes, and queries.

```sh
printf '\033]22;ew-resize\033\\'          # set: replaces the top of the stack
printf '\033]22;>pointer\033\\'           # push
printf '\033]22;<\033\\'                  # pop
printf '\033]22;\033\\'                   # Brindle's own pointer again
printf '\033]22;?pointer,wait\033\\'      # query: the reply is ESC ] 22 ; 1,0 ESC \
printf '\033]22;?__current__\033\\'       # the shape set now, or 0
```

As in kitty, a set or push applies each listed name in turn (`>a,b` pushes
both, `=a,b` leaves `b`), and skips names Brindle can't show. It shows
`default`, `text`, `vertical-text`, `pointer`, `crosshair`, `grab`,
`grabbing`, `not-allowed`, `no-drop`, `alias`, `copy`, `context-menu`,
`ew-resize`, `ns-resize`, `e-resize`, `w-resize`, `n-resize`, `s-resize`,
`nwse-resize`, `nesw-resize`, `col-resize` and `row-resize`; there is no
`wait`, `progress`, `help`, `cell`, `move`, `zoom-in` or `zoom-out` pointer to
show, and queries answer `0` for them. `__default__` answers `text` and
`__grabbed__` `default`.

The shape applies over the terminal's grid only, never the tab strip or zone
headers. A ctrl-hovered link still shows the hand, and dragging a selection
the text beam. With no shape set, the pointer is Brindle's own: the arrow while
the program reports the mouse, the text beam otherwise. A reset and the
program's exit empty both stacks, and an attached or reconnected tab starts
with Brindle's own pointer (the terminal server doesn't keep the stacks).

[kitty-pointer]: https://sw.kovidgoyal.net/kitty/pointer-shapes/

### Using tmux

tmux is an ordinary program here. A profile whose command is `tmux` opens it in
a tab, and tmux's prefix and bindings work as they do in any terminal:

```toml
[[profiles]]
name = "tmux"
command = "tmux"
args = ["new-session", "-A", "-s", "main"]
```

`new-session -A` attaches to `main` if it exists and creates it otherwise.
Brindle's terminal server already keeps ordinary tabs alive across quit, so the
tmux running in such a tab keeps going when Brindle quits.

### Upgrading from a version with tmux support

Versions with tmux support had control mode and classic tmux profiles. Those
settings are gone: a config that still sets `tmux_session_end` at the top level,
or `tmux`, `tmux_session` or `tmux_args` in a profile, still loads, and Brindle
logs one warning for each such key and ignores it. Saved tmux tabs are skipped
on the first launch, and the other tabs restore as before.

A profile that used tmux opens its plain shell until you give it the command.
Replace its tmux keys with two lines:

```toml
command = "tmux"
args = ["new-session", "-A", "-s", "main"]
```

If the profile used `tmux_args` for its own server, put those options first in
`args`, for example `["-L", "work", "new-session", "-A", "-s", "work"]`.

## License

MIT or Apache-2.0, at your option.
