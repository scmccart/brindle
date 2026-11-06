# Proposal

This change is phase 1 of the terminal-server series (see "Phasing" in `design.md`). It was split out of `remote-connections-followups` on 2026-11-04 as an exploration. On 2026-11-06 the user settled its two blocking decisions:
- the work is a series of changes, and this one keeps phase 1;
- a running server is upgraded by handing its sessions over to the new version, so no session is lost.

## Why

Today `brindle-remote` is one stdio child per tab, and it deliberately ends the session when the tab closes or the client goes quiet (`remote-connections`: "A vanished client ends the remote session"). Local terminals are PTYs owned by the Brindle process, so they end with it. Only tmux control-mode profiles persist, and only where tmux is available, which excludes native Windows.

The goal of the series is a UX like herdr, where a server owns the terminals: they keep running when Brindle closes, and they are reattached when it opens again. Phase 1 builds that server and moves local Linux terminals onto it. It keeps today's close behaviour, so users see no change yet. The later phases only change policy and transport.

## What Changes

- **`brindle-remote` gains a terminal-server mode.** One server runs per user per host, on a user-only Unix socket. It starts on demand and exits when it has neither sessions nor clients. It owns each session's PTY and runs an emulator per session.
- **Local Linux terminals run in the server.** Brindle connects to the local server, starting it if needed, and every local Linux tab and zone gets its PTY from there. Each session gets the environment of the Brindle that opened it, not the server's. Titles, directories, shell integration, status and notifications behave as they do today.
- **Close behaviour stays `kill`.** A session ends when its tab or zone closes, when Brindle quits or crashes, and when its connection drops. The server protocol also has a `detach` policy, which the server's own tests use; Brindle doesn't use it until phase 2.
- **Attach replays the screen.** A client that attaches to a detached session first receives a repaint of the session's screen, scrollback, modes, title, program status and prompt marks, then live output.
- **The protocol is versioned.** A server speaks every protocol version since this one, so an older Brindle keeps working with a newer server.
- **Upgrades hand the sessions over.** When a Brindle whose server binary is newer connects to an older running server, the server re-executes itself as the new binary in place. Sessions, their screens and all client connections carry over: programs keep running, with the same pids. If the handover can't happen, the old server keeps running.
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
- `terminal-server`: the per-user server. It covers its lifetime, socket and access, sessions and their close policies, attach with replay, protocol versions, the in-place upgrade, isolation of failures, and the management commands.

### Modified Capabilities
- `terminal-emulation`: local Linux terminals run in the terminal server, with the launching Brindle's environment. Losing the server, or not being able to use it, is reported in the tab.
- `command-line`: the `BRINDLE_SERVER_DIR` environment variable.

## Impact

- `remote-agent/`: a server mode with a poll loop over the socket, its clients and the PTYs. It also gets a per-session `alacritty_terminal::Term` (already in `Cargo.lock`) with a replay renderer, the handover (state serialization and re-exec), and the management subcommands. The stdio `--server` mode is kept for remote profiles.
- `remote-protocol/`: a versioned, multiplexed socket protocol beside the existing stdio frames, with session ids, a hello/upgrade handshake, close policies, attach and replay markers.
- `src/terminal/`: a server backend for local Linux tabs. Output is parsed on the connection's reader thread, as `pty.rs` does today, and process info is read from `/proc` with the pid the server reports. `Backend::Pty` remains only on Windows; its Unix side (alacritty's `tty::Pty` and `MarkedPty` in `pty.rs`) goes away.
- `src/remote/`: a per-process connection to the local server that multiplexes every local terminal, found and started with the same helper lookup as remote deployment (`BRINDLE_REMOTE_AGENT`, then beside the executable).
- Packaging: `scripts/install.sh` and the Linux archive already ship the helper. Dev and test runs need `cargo build -p brindle-remote-agent` too.
- Tests: the e2e harness gives each case its own `BRINDLE_SERVER_DIR` and stops that server afterwards. The agent crate gets server tests (attach, replay, policies, handover, peer checks, management). `tests/perf` baselines are taken before and after the extra hop.
