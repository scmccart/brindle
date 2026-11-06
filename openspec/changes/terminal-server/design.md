# Design

## Context

- **The helper today.** `brindle-remote` (`remote-agent/`, protocol in `remote-protocol/`) is one stdio child per tab, started over `ssh` or `docker`/`podman exec` with `--server`. It owns one PTY, on threads, with only `libc` as a dependency. It ends the session on stdin EOF, or when it has read no frame for `idle_limit_secs` (60). It is cached by Brindle version and digest, so a remote client always talks to a helper of its own build. Frames are `type: u8, len: u32 BE, payload`, and carry no session id.
- **Local terminals** use `Backend::Pty`. alacritty's event loop runs over `MarkedPty`, whose `ParsingReader` parses output on the I/O thread under the term lock (parser → term → marks). The child inherits Brindle's environment plus `child_env(profile)`. Titles and split directories come from `process::PtyProcess`, which reads `/proc` synchronously.
- **`Backend::Agent`** (remote tabs) is fed through `Terminal::feed()` on the GPUI thread, and caches process info from `ProcessInfo` events.
- **Shared parsing state.** The OSC 133 marks (`shell_integration.rs`) and the OSC 7501 records (`status.rs`) live in the `brindle` crate. The helper can't use them yet.
- **Finding the helper.** `remote::client` looks for it in `BRINDLE_REMOTE_AGENT`, then beside the executable (`brindle-remote-linux-x86_64`, `brindle-remote`). `scripts/install.sh` and the Linux archive already ship it.

## Goals / Non-Goals

**Goals:**
- The server is complete enough for phases 2–4 to change only policy and transport: sessions, both close policies, attach with replay, versioned protocol, handover and management.
- Local Linux tabs behave exactly as they do today, latency and throughput included, within the measured cost of one more hop.

**Non-Goals:**
- Brindle using the `detach` policy, reattaching through `session-restore`, and the "sessions on host" palette (phase 2).
- Windows: the server, named pipes and pseudoconsole handover (phase 3). Windows keeps `Backend::Pty` unchanged.
- Remote profiles and the stdio `--server` mode, which stay as they are (phase 4).
- Several clients attached to one session at once.

## Decisions

### A Windows spike comes first
Windows is phase 3, but several phase 1 decisions have to hold there too, and the riskiest Windows unknowns could overturn them. So the work starts with a spike (tasks group 0), and the rest waits for its findings. The spike covers:
- reparenting a pseudoconsole that another process owns to Brindle's window;
- handing a live pseudoconsole to a new process;
- keeping the server alive after Brindle dies, including inside jobs;
- a named-pipe transport with a peer check, served by one IOCP loop.

Decisions it may change:
- **Handover by re-exec.** Windows has no `execve`. If a new process can take the handles over, Linux might use the same model (`SCM_RIGHTS`) so both platforms share one design. That would cost Linux the parent link, and so exit statuses after a handover.
- **The handover format:** whether it needs platform-specific session state.
- **The single loop:** whether it becomes a platform-neutral abstraction over `poll` and IOCP.
- **Protocol 1's message set:** whether it needs `REPARENT { session, hwnd }` from the start.
- **The upgrade policy on Windows:** if an HPCON can't move, Windows may need the kill-with-warning or side-by-side option that Linux doesn't.

### Server mode in the existing binary
`brindle-remote serve` runs the server. It is started by a client, never by hand. `--server` (stdio, remote profiles) and `--version` keep their meaning. The management commands are subcommands of the same binary.
- *Why:* a single artifact to ship and locate, and phase 4 runs the same server on remote hosts.

### Directory, single instance and access
- **Server directory:** `$BRINDLE_SERVER_DIR`, else `$XDG_RUNTIME_DIR/brindle`, else `~/.cache/brindle/run`, created with mode 0700. It holds `server.lock`, `server.sock` and `server.log`.
- **Single instance:** the server takes `flock(LOCK_EX | LOCK_NB)` on `server.lock` and holds it for its whole life. Only after that does it unlink any stale socket and bind. A server that can't take the lock exits 0. On exit it unlinks the socket before the lock drops.
- **Access:** every accepted connection is checked with `SO_PEERCRED` against the server's uid before anything is read. The 0700 directory is the first barrier, and the check is the second.
- **Start:** the client runs `brindle-remote serve --ready-fd N`, with the write end of a pipe as fd N. The server double-forks and `setsid`s, redirects stdio to `server.log`, and writes one byte to the pipe once it is listening. The client waits up to 2 s for that byte, then connects. If two clients race, the loser's server exits on the lock, its pipe closes without the byte, and its client connects to the winner.
- **Exit:** the server exits after 5 s with no sessions and no clients.

### One poll loop, no threads
The server is a single thread running `poll(2)` over the listener, the client sockets, the PTY masters and a self-pipe for `SIGCHLD`. Every fd is non-blocking. Each client has an outgoing queue. When a client's queue passes a high-water mark (1 MiB to start with), the loop stops polling that client's sessions' PTYs for reading, so the kernel PTY buffer pushes back on that program alone.
- *Why:* the handover needs a quiescent point with no thread holding state, and one loop is that point. It also keeps one session's slow client from blocking another.
- *Alternative rejected:* agent.rs's thread-per-direction model, which is fine for one session per process but makes a consistent snapshot for handover hard.
- If measurements show the emulators slowing the loop, they move to worker threads, fed in order per session, with the loop still owning every fd. That changes neither the protocol nor the specs.

### Shared terminal-state crate
A new workspace crate, `brindle-term-core` (`term-core/`), takes the GPUI-free parts that both sides need: Brindle's `Parser` (vte plus the OSC 133 scanner and `Marks`) and the OSC 7501 `Records`, with their unit tests. It depends on `alacritty_terminal`, which is already in `Cargo.lock`. Both `brindle` and `brindle-remote-agent` use it.
- Its version matches the other crates'. The CI check already covers every workspace crate. Release-please has to bump it with the others, which task 1.1 confirms.
- *Why:* the server must track marks and records exactly as the client does, or replay drifts from what the client would have built.
- *Alternative rejected:* copying the scanners into the agent, which gives two implementations to keep in step.

### Per-session emulator and replay
Each session feeds its PTY output into a `Term` through the shared `Parser`. Its event listener drops every event, `PtyWrite` included, so the server never answers queries. The `Term` uses the session's scrollback limit from the spawn request.

On attach, the server renders a replay stream:
- `ESC c`;
- the primary screen: scrollback lines, then the visible rows, with SGR runs. Rows joined by `WRAPLINE` are written without a line break, so the client rewraps them;
- OSC 133 marks, written at their lines, so the client's own parser rebuilds its `Marks`;
- if the alternate screen is active: `CSI ? 1049 h` and the alternate grid;
- the modes (DECCKM, keypad, mouse 1000/1002/1003/1005/1006, focus 1004, bracketed paste, alternate scroll 1007, cursor visibility, DECSTBM scroll region, origin mode, autowrap);
- the cursor position, its shape and blink (DECSCUSR), and the title (OSC 2);
- the latest OSC 7501 report for each status record.

The replay is framed by `REPLAY_START` and `REPLAY_END` messages. Phase 2's client suppresses bells and notifications between them.
- The renderer is pure (`&Term → Vec<u8>`) and tested by round trips: feed a fixture into a `Term`, replay it into a fresh `Term`, and compare grids, cells, modes, cursor and marks.
- *Alternatives rejected:* a raw output ring buffer, which breaks on full-screen apps and once scrollback has rotated out, and hidden tmux, which brings back the dependency.

### Socket protocol
The socket protocol reuses the stdio frame header, with its own message numbers. The stdio protocol is untouched.
- **Handshake:** the client sends `HELLO { min, max, client_version }`. The server replies `WELCOME { protocol, server_version, build_id, pid }`, choosing the highest version both speak, or `ERROR` naming both ranges. This change is protocol 1.
- **Requests carry a `req` id** and replies echo it. One connection multiplexes sessions, so session-scoped messages carry the session id (`u32`): `DATA`'s payload starts with it, and the JSON messages have a `session` field.
- **Messages:**
  - sessions: `SPAWN { req, command, args, cwd, env, cols, rows, cell_w, cell_h, scrollback, policy }` → `SPAWNED { req, session, pid }` or `ERROR { req, message }`;
  - I/O: `DATA`, `RESIZE`, `EXIT { session, code }`, `CLOSE { session }`;
  - attach: `ATTACH { req, session, cols, rows }` → `REPLAY_START`, `DATA`…, `REPLAY_END`, then live `DATA`; `DETACH { session }`;
  - management: `LIST`, `KILL { session | all }`, `STATUS`, `STOP`;
  - upgrade: `UPGRADE { req, path }` → `UPGRADED { req, server_version }` or `ERROR`.
- **Compatibility rules:** JSON payloads ignore unknown fields, and new fields have defaults. A new message type or a changed meaning needs a new protocol version. The server keeps the protocol version each connection negotiated and answers it in that version. Tests pin protocol 1 transcripts, so later servers are checked against them.
- **No heartbeat or idle limit locally.** A local socket's EOF is reliable, a crashed client included, because the kernel closes its fds.

### Brindle's client
- **One connection per Brindle process** (`remote::local`), opened on the first local spawn. It has a reader thread that demultiplexes frames and a writer thread with a queue.
- **Output is parsed on the reader thread.** Each session's sink takes the terminal's `Parser`, `Term` and `Marks` in the usual lock order, then sends `Wakeup` and `Osc` events on the terminal's `ChannelEvent` channel, exactly as `ParsingReader` does. Local output so never queues on the GPUI thread, unlike `Backend::Agent`.
- **A new `Backend::Server { session, conn, pid }`.** Input, resize and close go to the connection. `EXIT` marks the terminal exited as a PTY exit does.
- **Losing the connection** feeds each server terminal a red "the terminal server went away" line and marks it exited without emitting `Exited`, so the tab stays open, as `Failed` does for remote tabs.
- **Process info stays local and synchronous.** On the same host and as the same user, `process.rs` reads `/proc/<pid>/stat` for the session's `tpgid`, then that process's `comm` and `cwd`. This replaces `tcgetpgrp` on the master fd. Titles and split directories are as fresh as today.
- **The environment** in `SPAWN` is complete: Brindle's own environment with `child_env(profile)` applied over it, so the server's environment never leaks in.
- **Always the server on Linux.** Local spawns never fall back to in-process PTYs, so there is one local code path, and its Unix `Backend::Pty` side is removed. The first local spawn finds the helper, connects (starting the server if needed) and negotiates. A failure becomes that terminal's error message (`Terminal::message`, as for a program that can't start), and the connection isn't cached, so the next spawn tries again. On Windows the code isn't compiled in.
  - *Alternative rejected:* an automatic fallback, or a `BRINDLE_TERMINAL_SERVER=off` switch. Either would keep a second backend alive and hide server problems; the user chose the server always.
- **Upgrade offer.** The client knows the version of the helper it found. When that is newer than `server_version` in `WELCOME`, it sends `UPGRADE` with the helper's path. Newer means a higher semver, or an equal version whose binary digest differs and whose mtime is later than the server's (dogfooding builds keep the version). The client then spawns as usual, upgraded or not.

### Handover by re-executing in place
On `UPGRADE`, the server:
1. Checks that `path` is a regular file owned by the user or root, and not group- or world-writable.
2. Runs `path handover-check --format F`, which exits 0 only if that binary reads handover format `F`.
3. Stops reading clients and PTYs. Each client's partially read frame and unsent queue go into the state.
4. Serializes the state into a `memfd`:
   - sessions: id, pid, master fd number, policy, size, scrollback limit, attached client, and the replay stream, which carries the marks and records;
   - clients: fd, negotiated protocol, read buffer and outgoing queue;
   - the listener and lock fds, the next ids and the start time.
5. Clears `FD_CLOEXEC` on exactly those fds and calls `execve(path, ["brindle-remote", "serve", "--resume", "<memfd>"])`.

The new image reads the state and rebuilds each `Term` by feeding it the replay. It sets `CLOEXEC` again, resumes the loop, and sends `UPGRADED` to the requesting client. If `execve` itself fails, the old image restores `CLOEXEC` and resumes, and the client gets an `ERROR`.
- *Why exec rather than passing fds over `SCM_RIGHTS` to a new process:*
  - the programs stay the server's children, so it keeps reaping them and reporting exit status;
  - the pid doesn't change;
  - the listening socket and the client connections carry over, with no reconnect and no gap in accepting.
- The handover format is versioned apart from the protocol, and a binary reads every older format. The server doesn't compare versions itself: the client decides what is newer, and the server only vets the binary. That also lets tests upgrade a server to the same binary.
- *Alternatives rejected:*
  - killing sessions with a warning (the user chose handover);
  - side-by-side servers, which break one server per host.

### Kill and hangup
Ending a session closes the master, sends `SIGHUP` to the session's process group, and sends `SIGKILL` after a grace period, reusing agent.rs's shutdown timings. A `kill` session ends on `CLOSE`, on its client's EOF or error, on `KILL` and on `STOP`. A `detach` session ends only on `KILL`, `STOP` or its program's exit.

### Failure isolation
Each `Term` feed runs under `catch_unwind`. A panic marks that session's emulator broken and logs it, and the session keeps forwarding output. Its replay is then just `ESC c`, and its handover state carries no replay.

### Builds and tests
- **Builds:** the root `Cargo.toml` gets `default-members` listing every crate, so plain `cargo build` and `cargo build --release` produce `brindle-remote` beside `brindle`.
- **Server tests** run in the agent crate with no GUI. Each spawns a real server in a temp `BRINDLE_SERVER_DIR` and drives it with a test client. They cover:
  - the start race, the peer check and exit when unused;
  - spawn with an explicit environment, I/O, exit codes and a stalled client;
  - both policies, client SIGKILL, attach and replay fidelity, and refusing to attach a taken session;
  - status and marks after attach, and that the server never answers queries;
  - version negotiation, handover to the same binary (with `htop`-like fixtures, pids and exit codes kept), a refused handover and the management commands.
- **e2e:** the harness sets `BRINDLE_SERVER_DIR=target/tmp/e2e/<case>/server` for every case and runs `brindle-remote stop` after it. New `local-server-*` cases cover:
  - the session listed, the shell's parent, close, quit and crash;
  - the environment of a second Brindle;
  - server loss shown in the tab;
  - a missing helper shown as the tab's error, and the next tab recovering.
- **perf:** `tests/perf` runs on `main` before the change (`BRINDLE_PERF_BASELINE`) and on the branch after, and the numbers go into the PR.

## Risks / Trade-offs

- [Every local terminal dies if the server crashes] → One small loop, emulator panics caught per session, heavy tests, and a clear in-tab message when it happens.
- [The output is parsed twice, by the server's emulator and the client's] → Measure throughput with vtebench. If needed, move emulators to worker threads (see "One poll loop") or feed the server's `Term` lazily, in batches.
- [Each keystroke takes one more hop] → `latency-shell` before and after. The hop is a local socket write and a non-blocking PTY write in one loop iteration.
- [A bug in the handover loses every session] → The format pre-check, round-trip tests of the state and the replay, a handover test in CI, and `server.log` for diagnosis. Upgrades happen only when a newer client connects.
- [The protocol must stay compatible for good] → Version 1 is kept minimal, transcript tests pin it, and the server answers each connection in its negotiated version.
- [Ping-pong upgrades between two dev builds of one version] → Only a later mtime counts as newer at equal versions, and a refused check leaves the server as it is.
- [The helper grows: `alacritty_terminal`, the shared crate, the renderer] → Check the musl build and the binary size early (task 1). The remote deploy copies it once per version.
- [Phase 2: the desktop's app scope may kill the server when Brindle exits] → Doesn't matter with `kill`. Before `detach` ships, check GNOME and KDE app scopes, and start the server through `systemd-run --user` if needed.

## Migration Plan

- No configuration changes. On first launch after the update, Brindle starts a server and local tabs move to it.
- The helper is required on Linux. `scripts/install.sh` and the Linux archive already ship it, and a missing one shows in each local tab with how to fix it.
- Rollback is to the previous release; there is no in-process switch.
- Existing remote profiles and tmux control mode are unaffected.

## Open Questions

- The exact grace periods (unused-server exit, SIGHUP to SIGKILL) and the queue high-water mark, to be tuned in tests.
- Whether emulators need worker threads, to be decided by the perf numbers.

## Later phases (separate changes)

2. **`on_close`:** a per-profile `kill | detach | ask`, applied to closing a zone, tab or window, quitting, crashes, dropped connections and the shell exiting. Restore reattaches by session id and falls back to respawning in the recorded directory. Adds the "sessions on host" palette and the replay-marker attention suppression in the client.
3. **Windows server:** local Windows tabs always run in the server too, with no fallback, and `Backend::Pty` is removed entirely. ConPTY moves into the helper, with a named-pipe transport, `CREATE_BREAKAWAY_FROM_JOB` and `DETACHED_PROCESS`, the HWND reparent sent to the server, and the pseudoconsole handover, all built on what the group 0 spike found.
4. **Remote and eager:** `brindle-remote --attach` relays ssh and exec stdio to each host's server. Eager profiles connect at launch, with an offline state, may start their containers, and Windows SSH hosts get a PowerShell deploy. This absorbs follow-ups 2–4 of `remote-connections-followups` and replaces the idle-limit requirement.
