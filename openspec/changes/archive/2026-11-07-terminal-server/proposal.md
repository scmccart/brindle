# Proposal

This change is phase 1 of the terminal-server series (see "Phasing" in `design.md`). It was split out of `remote-connections-followups` on 2026-11-04 as an exploration. On 2026-11-06 the user settled its two blocking decisions:
- the work is a series of changes, and this one keeps phase 1;
- upgrades must not lose sessions.

The group 0 Windows spike (`spike-windows.md`) then showed that a live Windows pseudoconsole can't move to another process. So, after looking at how herdr-core does it, the user chose to split the server into a core and holders (2026-11-06). That replaced the earlier plan of re-executing one server in place.

## Why

Today `brindle-remote` is one stdio child per tab, and it deliberately ends the session when the tab closes or the client goes quiet (`remote-connections`: "A vanished client ends the remote session"). Local terminals are PTYs owned by the Brindle process, so they end with it. Only tmux control-mode profiles persist, and only where tmux is available, which excludes native Windows.

The goal of the series is a UX like herdr, where a server owns the terminals: they keep running when Brindle closes, and they are reattached when it opens again. Phase 1 builds that server and moves local Linux terminals onto it. It keeps today's close behaviour, so users see no change yet. The later phases only change policy and transport.

## What Changes

- **`brindle-remote` gains a terminal server in two roles.** Holders own the PTYs, are the programs' parent and parse nothing; they change rarely. One core per user per host runs an emulator per session, the client protocol and the management commands, and drives the holders. Both use user-only Unix sockets, start on demand and exit when unused.
- **Local Linux terminals run in the server.** Brindle connects to the local server, starting it if needed, and every local Linux tab and zone gets its PTY from there. Each session gets the environment of the Brindle that opened it, not the server's. Titles, directories, shell integration, status and notifications behave as they do today.
- **Close behaviour stays `kill`.** A session ends when its tab or zone closes, when Brindle quits or crashes, and when its connection drops. The server protocol also has a `detach` policy, which the server's own tests use; Brindle doesn't use it until phase 2.
- **Attach replays the screen.** A client that attaches to a detached session first receives a repaint of the session's screen, scrollback, modes, title, program status and prompt marks, then live output.
- **The protocols are versioned.** A core speaks every client and holder protocol version since this one, so an older Brindle, or an older holder, keeps working with a newer core.
- **Upgrades and crashes keep every session.** The core checkpoints each session's screen into its holder.
  - On an upgrade, the old core steps down and the newer one adopts the holders and rebuilds the screens.
  - Holders of an older build drain: they take no new sessions and exit when empty.
  - A crashed core is restarted by its holders.
  - Brindle reconnects and gets its tabs back with their screens, and the programs keep their pids.
- **Brindle reconnects.** When the core restarts, Brindle reconnects and replays each tab without bells or notifications. A tab whose session is lost for good keeps its screen and says so.
- **Management commands:** `brindle-remote list`, `kill <id>`, `kill --all`, `status` and `stop` manage the local server from a shell.
- **Always the server, no fallback.** On Linux, Brindle no longer runs local programs on PTYs of its own. When the server can't be used (helper missing, server unreachable, no common protocol version), the tab shows the error, as it does for a program that can't start.
- **Isolated servers for tests:** `BRINDLE_SERVER_DIR` picks the server's directory, so each test gets its own server.

Not in this phase:
- the per-profile `on_close` option, reattaching on launch through `session-restore`, and the "sessions on host" palette (phase 2);
- Windows (phase 3), where local tabs keep Brindle's in-process ConPTY;
- remote profiles (phase 4), which keep the per-tab stdio helper, its heartbeat and its idle limit.

tmux control-mode profiles (`tmux -C` over pipes) are unchanged. Classic tmux profiles run on a PTY, so they go through the server like any other local tab.

### Relation to `remote-connections-followups`

Phase 4 absorbs or supersedes three of that change's follow-ups:
- follow-up 2 (lifecycle polling): the server replaces the per-tab cancel watcher, writer and `try_wait` loops;
- follow-up 3 (an `Endpoint` type): a transport-agnostic server is the `Endpoint` type;
- follow-up 4 (connection cost): one connection per host instead of per tab, and a long-lived helper instead of per-tab container copies.

Phase 1 leaves them, and the idle-limit requirement in `remote-connections`, untouched.

## Capabilities

### New Capabilities
- `terminal-server`: the per-user server. It covers the core and holder roles, their lifetime, sockets and access, sessions and their close policies, reconnect, attach with replay, protocol versions, upgrades with draining holders, isolation of failures, and the management commands.

### Modified Capabilities
- `terminal-emulation`: local Linux terminals run in the terminal server, with the launching Brindle's environment. Brindle reconnects quietly when the core restarts. A lost session, or a server that can't be used, is reported in the tab.
- `command-line`: the `BRINDLE_SERVER_DIR` environment variable.

## Impact

- `remote-agent/`: two new roles, both single-threaded event loops behind a platform trait (`poll` on Linux, IOCP in phase 3):
  - `hold`: PTYs, checkpoints and tails, drain, and core supervision;
  - `serve`: the core, with a per-session `alacritty_terminal::Term` (already in `Cargo.lock`), the replay renderer, checkpoints, reconnect, upgrades and the management subcommands.

  The stdio `--server` mode is kept for remote profiles.
- `remote-protocol/`: two versioned, multiplexed protocols beside the existing stdio frames: the client protocol (protocol 1) and the core–holder protocol (H1).
- `src/terminal/`: a server backend for local Linux tabs. Output is parsed on the connection's reader thread, as `pty.rs` does today, and process info is read from `/proc` with the pid the server reports. `Backend::Pty` remains only on Windows; its Unix side (alacritty's `tty::Pty` and `MarkedPty` in `pty.rs`) goes away.
- `src/remote/`: a per-process connection to the local server that multiplexes every local terminal, found and started with the same helper lookup as remote deployment (`BRINDLE_REMOTE_AGENT`, then beside the executable).
- Packaging: `scripts/install.sh` and the Linux archive already ship the helper. Dev and test runs need `cargo build -p brindle-remote-agent` too.
- Tests: the e2e harness gives each case its own `BRINDLE_SERVER_DIR` and stops that server afterwards. The agent crate gets server tests (roles, attach, replay, checkpoints, policies, reconnect, upgrades and drain, core respawn, peer checks, management). `tests/perf` baselines are taken before and after the extra hops.
