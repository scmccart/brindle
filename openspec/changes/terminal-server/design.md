# Design

**Exploration notes, not a settled design.** They were captured so the context survives a pause, and moved here with the change when it was split out of `remote-connections-followups`. The work will probably become a series of changes before specs and tasks; see "Phasing" below.

## Context

- **The helper today.** `brindle-remote` (`remote-agent/`, protocol in `remote-protocol/`) is one stdio child per tab, started over `ssh` or `docker`/`podman exec`. It owns one PTY. It ends the session on stdin EOF, or when it has read no frame for `idle_limit_secs` (60). It is cached under `~/.cache/brindle/<version>-<digest>/`, so a client always talks to a helper of its own version. Its only dependency is `libc`, and it is built as a static musl binary.
- **Local terminals** use `Backend::Pty`: alacritty's event loop over `MarkedPty`, which wraps alacritty's Unix PTY, or Brindle's own pseudoconsole on Windows (`terminal/conpty.rs`, `cmdline.rs`, the embedded `conpty.dll`). Process and CWD info comes from `terminal/process.rs`. Remote terminals use `Backend::Agent`, which receives bytes through `feed()`.
- **`session-restore`** already records tabs, zones, directories and remote target identity, and respawns them on launch.
- **tmux control mode** already gives persistence for profiles that opt in, but it depends on tmux on the host. tmux is not available natively on Windows.

## Goals / Non-Goals

**Goals:**
- Every terminal, local or remote, Linux or Windows, is owned by one `brindle-remote` server per user per host, and can outlive Brindle.
- Reattaching draws the session's current screen, scrollback and modes before live output resumes.
- Close behaviour is chosen per profile and applies uniformly.
- Remote profiles marked eager are connected at launch, and their sessions are listed. An unreachable host shows as offline and never blocks startup.

**Non-Goals:**
- Routing tmux control clients (`tmux -C`) through the server. They are pipe clients rather than terminals, and tmux persists by itself.
- Several GUI clients sharing one session at the same time (mirroring). One attached client per session is enough at first. Which size wins with two clients is deferred.
- Moving sessions between hosts.

## Architecture sketch

```
                          +-----------------------------------+
 Brindle (GUI, client) -->| brindle-remote (one per user/host)|
  every tab and zone      |   sessions: PTY or ConPTY + vt    |
                          |   buffer + status/marks           |
                          +-----------------------------------+
   local Linux   : unix socket                -> server on this machine
   local Windows : named pipe                 -> server on this machine (ConPTY)
   SSH Linux     : ssh relay -> unix socket   -> server on that host
   SSH Windows   : ssh relay -> named pipe    -> server on that host
   container     : exec relay -> unix socket  -> server in the container

   Brindle quits --> relays and clients go away; server and shells keep running
   Brindle opens --> clients reconnect; tabs come back with their screens
```

## Decisions

### All terminals go through the server
`Backend::Pty` goes away, and every terminal uses the `feed()`-driven backend that `Agent` uses today. This code moves out of the GUI into the helper:
- process spawning: `pty.rs`'s PTY side, `conpty.rs` and `cmdline.rs`, with argument quoting, environment merging, `PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH` and the packaged `conpty.dll`;
- process and CWD info: `process.rs`, which is `/proc` on Linux and the Toolhelp/PEB code on Windows. This is the seam `port-to-macos` plans to use.

The client keeps parsing (shell integration, OSC scanning, links, search). The server runs its own emulator only for the replay buffer.
- *Why:* one model for persistence everywhere, Windows parity, and one terminal backend in the GUI.
- *Alternative rejected:* a server only for remote profiles, with local profiles getting only `kill` or `ask`. That leaves local and Windows tabs without persistence.

### A virtual buffer in the server for the first draw
Each session feeds its PTY output into an `alacritty_terminal::Term`. That crate is already a workspace dependency and is pure Rust, so a musl build should work. On attach, the server renders the `Term` as an ANSI repaint stream:
- `ESC c` first;
- the modes: alternate screen, mouse, bracketed paste, DECCKM and the rest;
- the scrollback lines, then the visible grid with SGR;
- the cursor position and shape, and the title.

The client `feed()`s that stream like any other output, so it needs no new snapshot format. This is the same pattern as the tmux pane restore.
- **Replay markers.** Replay begins and ends with a marker, and the client suppresses attention events between them: OSC 9/777 notifications and the bell.
- **Program status (OSC 7501) and OSC 133 marks** are kept by the server and sent again after the replay. Today only the client tracks them, so without this they would be lost on reattach.
- **Resize.** The attaching client's size goes to the PTY immediately, so the app redraws on top of the replay.
- **Memory.** Scrollback held by the server has a limit, probably the profile's scrollback setting.
- *Alternatives rejected:*
  - a raw output ring buffer, which breaks on full-screen apps and once scrollback has rotated out;
  - hidden tmux underneath, which brings back the tmux dependency.

### One server per user per host
- **Linux:** a socket at `$XDG_RUNTIME_DIR/brindle/remote.sock`, falling back to `~/.cache/brindle/run/`, with `flock` guaranteeing a single instance.
- **Windows:** a named pipe per user, with an equivalent single-instance guard.
- **Remote hosts:** `brindle-remote --attach` is a thin relay between ssh or exec stdio and the local socket or pipe. It starts the server if none is running.
- **Local machine:** Brindle starts the server and connects to it directly.

### Management commands in `brindle-remote`
`brindle-remote list | kill <id> | kill --all | status | stop` work from a shell on the host, and Brindle's palette ("sessions on host X") uses the same commands over its connection. Because sessions are no longer cleaned up automatically, these are how orphans get cleaned up.

### Close behaviour per profile
A per-profile option along the lines of `on_close = "kill" | "detach" | "ask"`, applying to local and remote profiles:

| Event | kill | detach |
|---|---|---|
| close a zone, tab or window | ends the session | session keeps running, listed under its host |
| quit Brindle, or a crash | ends the session | tab reattaches on next launch |
| connection drop | ends after the idle limit | stays; Brindle reconnects |
| the shell exits | tab closes | tab closes; the server forgets the session |

The first phase defaults to `kill`, so behaviour doesn't change.

### Session restore reattaches
A recorded tab or zone stores its session id beside its target identity. Restore reattaches by id, and falls back to today's respawn in the recorded directory when the session is gone.

### Eager connection at launch
Profiles opt in, rather than every remote profile being dialled, because some hosts are slow or offline. For each one, Brindle connects in the background, ensures the server is running, and runs `list`. An unreachable host shows as offline. A container profile may start its container. A container's server only lives as long as the container.

## Risks / Trade-offs

- **An upgrade meets a running server that owns live sessions.** This becomes a daily event when dogfooding through `scripts/install.sh`, since every local terminal is on a long-lived local server. The protocol has to be versioned and backwards compatible from the first release. Today the helper is cached by Brindle version, so a version mismatch never happens. → Unresolved; see "Unresolved decisions".
- **If the server crashes, every terminal dies.** → Keep the server small and heavily tested, and catch emulator panics per session.
- **Each keystroke takes one more IPC hop.** → Take a `latency-shell` and `throughput-brindle` baseline before and after (`tests/perf`).
- **On Windows, the server must outlive Brindle.** → `CREATE_BREAKAWAY_FROM_JOB`, `DETACHED_PROCESS` and no console window. On Linux, `setsid` and a double fork.
- **Reparenting the pseudoconsole becomes a cross-process call.** `parent_console_to` reparents the pseudoconsole to Brindle's HWND, but the HPCON now lives in the server. → Spike: send the HWND to the server, which calls the reparent API.
- **Remote Windows hosts over OpenSSH.** Deployment is `sh` plus `~/.cache` today. → A PowerShell deploy path for `brindle-remote.exe`.
- **Tests need isolated servers.** Every e2e case needs its own server, through an env var for the socket or pipe path (like `BRINDLE_STATE_DIR`), killed afterwards like the throwaway tmux servers. Many cases check close behaviour, which `on_close` changes.
- **The helper gets bigger and gains a Windows build.** `alacritty_terminal` and the ConPTY code move in, so it is no longer a small static Linux-only binary. → Check its size and the musl build early.

## Phasing (likely separate changes)

1. **`terminal-server`:** the server, a versioned protocol, the vt buffer and replay, the management commands, and local Linux terminals going through it. `on_close` defaults to `kill`, so nothing users see changes.
2. **`on_close`:** `detach` and `ask`, reattach through `session-restore`, and the "sessions on host" palette.
3. **Windows server:** ConPTY moves into the helper, the named-pipe transport, the job breakaway, and the HWND reparent.
4. **Remote and eager:** SSH and container relays to each host's server, eager connect with an offline state, starting containers, and Windows SSH hosts. This absorbs follow-ups 2–4 of `remote-connections-followups`.

## Unresolved decisions

These block phase 1 and are for the user to decide. They are not deferrable open questions.

- **Upgrades with a running server.** Options:
  - (a) kill its sessions with a warning, keeping a protocol versioned from day one, and add a handoff later. This was the recommendation.
  - (b) let old and new servers run side by side until the old one's sessions end. That breaks "one per host" for a while.
  - (c) hand sessions over to the new server: PTY fds over `SCM_RIGHTS` on Linux, and on Windows `DuplicateHandle` for the pipes and process. Whether an HPCON can move to another process is unknown and needs a spike.
- **Splitting.** Whether this change becomes the series above, keeping phase 1, or stays one change.
