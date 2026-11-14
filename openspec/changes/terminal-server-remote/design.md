# Design

## Context

See proposal.md for why. The current state this design starts from:

- **Local tabs** (Linux, Windows) are sessions of the terminal server, reached through `remote::local`: one process-wide `Connection` (`shared()`) on one `Endpoint` (helper plus server directory), with a reader thread that demultiplexes into `SessionSink`s, a writer thread that holds messages while disconnected, and reconnects that give up after `RECONNECT_LIMIT` (15 s). Titles and directories come from the local `/proc` (or Toolhelp and OSC on Windows) by the session's pid.
- **Remote tabs** use `Backend::Agent`. Each tab runs `remote::client::connect` on its own thread:
  - deploy: an SSH check, then an upload if needed; a container gets a fresh copy, removed afterwards;
  - start `brindle-remote --server` over `ssh` or `docker exec`, and speak the stdio protocol to it;
  - a cancel watcher, a polling writer with heartbeats, and `try_wait` loops (follow-up 2);
  - a `QUERY_PROCESS` round trip per tab every second (follow-up 1b).
- **The core** hangs up a client's `kill` sessions as soon as its connection ends. The reconnect grace covers only sessions adopted by a new core. A detached session keeps its `owner`, so a client reconnecting with the same token gets its `detach` sessions back (`reattach`).
- **The holder's spawn** (`pty/unix.rs`, shared with the stdio agent) already expands `~` and falls back to `$HOME` for a missing directory, and runs the login shell when `SPAWN` has no command. With `exact_env` the program gets exactly `SPAWN`'s environment.
- **Restore** drops a recorded session for remote profiles (`session.rs`, `resolve_tab`), and the launch lists local sessions synchronously before the window opens.

## Goals / Non-Goals

**Goals:**
- Remote tabs on the same server backend as local ones, with one connection per endpoint.
- A link that dies silently is noticed on both ends, and the client gets its sessions back.
- Nothing remote ever delays the window or the UI thread.
- Remove the stdio agent and its per-tab lifecycle code.

**Non-Goals:**
- Windows SSH hosts: remote endpoints stay Linux. Users chose to drop this rather than defer it.
- Starting or creating containers.
- `--attach ID` for remote sessions: remote sessions are attached from the sessions palette or at launch.
- Storing layouts in the server (`session-layout-in-server`).
- Showing offline hosts anywhere but the sessions palette.
- Making a replay cheaper over slow links.
- Remote profiles on clients other than Linux and Windows: `Backend::Server` and the client are built for those two only. `port-to-macos` brings remote profiles to macOS with its server port.

## Decisions

### 1. `brindle-remote relay` and its link

The relay is a client of its host's core that pipes the core's protocol to its stdio. The link between Brindle and the relay carries the same frame header (`type: u8, len: u32 BE`) as protocol 1, and adds relay-only types that protocol 1 never uses:
- `RELAY_HOST` (JSON): version, build id, its own environment, the login shell (from `SHELL`, else passwd), home, the server directory and the core's pid;
- `RELAY_HEARTBEAT` (empty);
- `RELAY_ERROR` (JSON message), when the relay can't reach or start the core.

They live in `remote-protocol/src/relay.rs`, and transcript tests pin them.

The sequence is:
1. Write the frame marker. `skip_preamble` stays, since an SSH login can print before `exec`.
2. Resolve the server directory with `server_dir::resolve`, then connect to the core, starting it when none listens (decision 3).
3. Offer an upgrade (decision 4).
4. Send `RELAY_HOST`, or `RELAY_ERROR` with the core log's path if any of this failed.
5. Pipe frames both ways.

The relay reads frame headers in both directions, so it can drop heartbeats and insert its own at frame boundaries. A writer mutex is shared by the core→stdout copier and a heartbeat timer, which sends `RELAY_HEARTBEAT` after 10 s without a write. The stdin reader thread stamps a `Liveness` on every frame it reads, heartbeats included. The timer thread exits the relay after 60 s without one. Following CLAUDE.md's `Liveness` rule, silence is never measured where a blocked write could stall it.

Alternative considered: a `PING`/`PONG` pair in client protocol 2, answered by the core. Rejected, because it fails against a protocol-1 core that a refused upgrade leaves running, and liveness belongs to the link, not the core.

### 2. Client side of the link

`remote::local` becomes per-endpoint:
- `Host` is `Local` or `Remote(RemoteTarget)`, compared by target identity. It is follow-up 3's `Endpoint` type and replaces the five `profile.remote.is_some()` checks.
- `shared(host)` keeps a map of `Arc<Connection>`.
- Each remote host's connection has a `client_token` of its own (`host_token`: the process's token plus a hash of the host), because two endpoints that reach one core (`ssh localhost`, or two names for one machine) would otherwise take each other over (decision 6).
- `Connection` takes a `Transport`:
  - `Local(Stream)`: as today;
  - `Relay`: the `ssh`/`docker` child plus its pipes.

The relay transport's reader drops `RELAY_HEARTBEAT` and stamps the time of each frame. Its writer sends a heartbeat after 10 s idle. The link is dead after 30 s with nothing read: the client kills the child and reconnects. A dedicated thread waits on the child with a blocking `wait()`. There are no `try_wait` loops and no cancel watcher (follow-up 2).

Reconnect policy is per transport:
- **Local:** unchanged (`START_AFTER`, `RECONNECT_LIMIT`).
- **Relay:** back off 1 s, doubling to 30 s (60 s for an eager endpoint with no terminals), for as long as the connection is wanted. Sessions are never declared lost by time, only by the reconnected core's `LIST`.
- Each loss writes a dim "connection to A8 lost; reconnecting…" line into the endpoint's terminals. The reconnect's replay starts with `ESC c`, so the line never corrupts a full-screen program.

Requests (`SPAWN`, `ATTACH`, `LIST`) on a relay connection wait while it is connecting, with no `REQUEST_LIMIT`. When a first connection fails to deploy or start:
- a new tab's `SPAWN` fails with the bootstrap error, as a tab does today;
- restored zones' and the launch's attaches, and an eager endpoint, keep waiting through the retries, and the error is written once into the waiting zones.

Later failures only retry.

`HELLO` from a relay transport sets `linger_secs: 300` and `process_info: true` (decision 5).

### 3. Where a relay starts the core

On a desktop, roles go into `systemd-run --user --scope`. On an SSH host without user lingering, the user manager stops shortly after the last login ends and takes its scopes with it. So the relay decides:
- **Linger enabled** (`/var/lib/systemd/linger/<user>` exists), and a user manager is reachable: scope, as locally.
- **Otherwise:** start plain (double fork, `setsid`), with `BRINDLE_NO_SCOPE=1` in the core's environment so its holders start plain too. Log one line saying so. The processes stay in the login's session scope, which outlives the login unless the host sets `KillUserProcesses=yes` (see Risks).
- **Containers** have no user manager, so they start plain already.

The core and holders run as `/proc/<own pid>/exe`, as today.

### 4. Upgrades through the relay

The relay makes the upgrade decision itself, on the host, where it can read `/proc/<core pid>/exe`:
- `Build`, `is_newer` and `semver` move from `src/remote/local/upgrade.rs` to `remote-protocol/src/upgrade.rs`, shared by Brindle's local offer and the relay.
- The relay connects with its own token (`<token>-upgrade`) and reads `WELCOME`.
- If it is newer, it runs its own holder-protocol check in-process (its H1 range against `holder_protocols`), sends `STEP_DOWN` and starts `serve` from its own image. Then it connects again for the client.
- A refused or failed check relays to the running core.

Brindle's client runs no upgrade offer for a relay.

For equal versions with different builds, the newer file time wins. Uploads keep their upload time, so two machines with different builds of one version settle on the later upload and don't flip-flop.

### 5. Client protocol 2 and `HELLO` fields

- **`HELLO` gains two additive JSON fields:**
  - `linger_secs` (default 0, capped at 600);
  - `process_info` (default false).

  Older cores ignore them, and nothing else changes.
- **Protocol 2 adds `PROCESS { session, name, cwd }`** (core→client). `PROTOCOLS` becomes 1–2, and `PROCESS` is sent only on connections that negotiated 2 and asked for it.
- **Sampling:** each session has an activity flag, set on output and input. A 1 s timer in the core looks at flagged sessions, plus once after a spawn or attach. It reads the foreground process from `/proc/<pid>/stat` (`tpgid`), then `comm` and `cwd`, reusing `pty/unix.rs`'s `/proc` helpers. It sends `PROCESS` only when the name or directory changed.
- **Windows:** a core sends none.
- **Local clients** keep reading `/proc` themselves and don't ask for process info.

Alternative considered: a `QUERY_PROCESS` request, as the stdio protocol had. Rejected: one round trip per tab per second over SSH is what follow-up 1b wanted gone.

### 6. Takeover and linger in the core

- **Takeover:** in `welcome`, before `reattach`, every connected client with the same token is dropped without `client_gone`. Its sessions' `attached` is cleared, their owner kept, and its waiting requests fail. `reattach` then replays them to the new client. This also covers local clients.
- **Linger:** `client_gone` on a client with `linger_secs > 0` leaves its `kill` sessions running with their owner and a `linger_until`. The core's timer hangs them up when it passes. `reattach` clears it.
  - Linger isn't stored in holder meta, so a core restart falls back to `RECONNECT_GRACE` for adopted sessions. A client with a live link reconnects within that anyway.

### 7. The program's environment

The core's rule, that a program gets exactly `SPAWN`'s environment, stays. Brindle builds that environment for a remote spawn from the connection's `RELAY_HOST` environment (a fresh login's), with `child_env(profile)` on top. `HOST` is refreshed on each reconnect.

The other fields:
- **`TERM`:** `child_env` already sets `TERM`, `COLORTERM` and `TERM_PROGRAM*`, which a tty-less `ssh` or `exec` environment lacks.
- **Command:** `SPAWN` carries none when the profile sets none, so the holder runs the login shell from `SHELL`, else passwd.
- **Directory:** never checked locally. The holder already expands `~` and falls back to `$HOME`.
- **Parser:** remote terminals keep `Parser::posix()`, which the agent path uses for OSC 7 paths.

### 8. Deployment

- **SSH:** one connection on the warm path. The launch script is roughly:

  ```sh
  set -- "$HOME/.cache/brindle/V-D/brindle-remote"
  if test -x "$1" && test "$("$1" --version)" = V; then exec "$1" relay; fi
  exit 97
  ```

  The path is a positional parameter, not a variable. The relay's environment becomes its sessions' (decision 7), and assigning a variable that the user's environment already has would change it there.

  On 97, Brindle uploads as today (a second connection), then launches again. That removes follow-up 4's extra connection from every connect. A relay never exits 97, and ssh's own failure (255) is a connection error, not a missing helper.
- **Containers:** the cache is `/tmp/brindle-helper-<version>-<digest>/brindle-remote`.
  - The launch exec runs as the profile's user. It checks that the directory and file are root-owned and not symbolic links, then `exec`s `relay`; otherwise it exits 97.
  - On 97, a root exec creates the directory with mode 755, copies the helper in and makes it root-owned (the current `chown`/`chmod`/`stat` check), and removes other builds' root-owned `brindle-helper-*` directories.
  - The copy goes in under a temporary name and is renamed into place once it is root's. Connections share the path now, so it must never hold a partial file, and a helper that is running must never be written to.
  - An existing directory that isn't root-owned is an error naming it. Brindle never deletes or reuses it.
  - Nothing is removed at session end. A running core or holder keeps working if its file is pruned, because Linux runs deleted images.

### 9. Terminals and tabs

- **Backend:** remote terminals use `Backend::Server` with a `Session` on their `Host`'s connection.
  - `Session` stores the last `PROCESS` for remote sessions, and `foreground_process_name` and `working_directory` read it (OSC 7 still wins for the directory).
  - `Closing` applies as locally: profile tabs spawn `detach`, `-e` spawns `kill`.
- **Pinning:** a `Tab` records its `Host` when created. Splits use it, whatever the profile now targets (follow-up 1a, pinning kept). `apply_config` already keeps the tab's remote.
- **Closing while disconnected:** `CLOSE` waits in the writer's queue and goes out on reconnect.
- **Connection lifetime:**
  - A non-eager connection is reference-counted by its `Session`s and by any queued `CLOSE` not yet written. It closes 5 s after the count reaches zero, absorbing a close-then-reopen, so closing an offline endpoint's last tab keeps reconnecting until its `CLOSE` goes out.
  - An eager one lives for the process.
- **This-machine assumptions:** every `Backend::Server` arm that assumes the session runs on this machine is gated on `Host::Local`:
  - `parent_console_to` and the `REPARENT` re-sent after each `REPLAY_END` (a Linux core refuses it, and Windows clients reach Linux hosts);
  - the pid-based `/proc` and Toolhelp lookups;
  - local directory checks.

  Task 6.1 audits each arm.
- **Removed:**
  - `Backend::Agent`;
  - `remote::client`'s per-tab connection (only deploy and the launch command stay);
  - `remote-agent/src/agent.rs`, `--server`, `idle_limit_secs`;
  - the stdio session messages in `remote-protocol` (frames and the marker stay).

### 10. Restore and launch

- **Recording:** `resolve_tab` keeps a remote zone's `session`, and the recorded target identity names the `Host` to attach on. A record without a session follows the existing endpoint-identity rule.
- **Restoring:** remote zones restore immediately as `Terminal::attach` on their `Host`. The attach waits in the connection while it connects. A refused attach, or one for a session that is gone, makes the sink show the reason and marks the terminal as a message, which isn't recorded.
  - Until its attach is answered, a `Session` started by an attach reports the requested id and key (`id_and_key` returns `None` only for a pending spawn). A quit while the host is offline therefore records the zone again, instead of dropping it from `session.toml`.
  - Local `drop_lost_zones` is unchanged. Remote zones can't use it without blocking on the network.
- **Launch connections:** after the window opens, the launch connects, on background threads, to the `Host` of every restored remote zone and every `connect_at_launch` profile.
- **Reattaching:** on each such host's first `WELCOME` of the launch, its connection posts its `LIST` to the UI. The workspace attaches the detached sessions that decision 13 allows and that weren't restored into a zone, as tabs at the end in start order, with the profile rule from the spec. The active tab stays.
- **Local reattach** picks the default profile when the recorded profile is gone or is a remote profile, and follows the same sweep rule.

### 13. The launch sweep leaves a dropped owner's sessions alone

The core can't tell an owner whose link dropped from one that crashed: both leave the session detached with its `owner` set. An explicit detach (quit, `DETACH`) clears the owner. Without a rule, another machine's launch sweep, which attaches everything detached, would take a session whose owner is about to reconnect, and that owner's tab would then say "session lost".

So:
- **The session list reports owner state:** `SessionInfo` gains `owned` (an owner is set) and `detached_secs` (seconds since a client was last attached), as additive JSON fields.
- **The sweep** attaches a detached session when it has no owner, or when it has been detached for more than 5 minutes. Recorded zones attach their own sessions whatever the owner state.
- **Everything else** stays in the sessions palette, where the user can take it explicitly.

This narrows the user's "everything detached" choice only for sessions whose owner may still be coming back. A Brindle relaunched within 5 minutes of a crash still restores its recorded zones, but its unrecorded sessions wait in the palette until then.

### 11. Eager endpoints and the sessions palette

- **State:** a GPUI global `Hosts` holds each wanted remote endpoint's state (connecting, connected, offline with the last error), updated from the connections' threads through the event channel.
- **Palette:** the sessions palette sends `LIST` on the local connection and on every connected remote one, concurrently. Rows are grouped local first, then endpoints in profile order. Offline and connecting endpoints get a row of their own that does nothing on enter.
- **Killing:** `picker_kill` sends `KILL` on the row's connection.
- **Settings:** the dialog's remote section gets a `Switch` for `connect_at_launch`, written only when on.

### 12. Hardening folded in from `harden-terminal-server`

- **Signals (#44):** in the holder's `pre_exec`, every signal from 1 to `NSIG` except `SIGKILL`/`SIGSTOP` is set to `SIG_DFL` (async-signal-safe), and the mask is emptied with `sigprocmask`. This is the "every signal" answer to that proposal's open question, as `POSIX_SPAWN_SETSIGDEF` would do. It is tested with a holder started with SIGHUP ignored.
- **Draining a dead core:** on a failed write to its core, the holder reads until `WouldBlock`, bounded at 16 MiB, before `lose_core`. It is tested with a `SET_META` queued behind a full socket buffer.
- **Ready pipe:** the starter reads the ready pipe to EOF, and counts the role as ready only when it read exactly the one ready byte. An exec error writes 8 bytes, so it can't pass. Roles close the ready fd right after writing. It is tested by forcing a failed exec with `--ready-fd 3`.

## Risks / Trade-offs

- [Hosts with `KillUserProcesses=yes` and no lingering kill the server when the last login ends.] → The relay logs it on the host. README says to run `loginctl enable-linger`. Restored zones then say their sessions ended.
- [Reattaching everything detached on a shared host moves sessions between machines at launch.] → The user chose this. Attached sessions are refused, and decision 13 keeps the sweep off sessions whose owner dropped in the last 5 minutes. A Brindle disconnected for longer can still lose a session to another machine's launch, and then says so in the tab.
- [Queued input runs minutes later after a long disconnect.] → It mirrors the local reconnect. The "connection lost" line tells the user that typing is queued.
- [A replay sends the whole scrollback on every reconnect, which is slow over thin links.] → Accepted for now. A scrollback-delta replay would be its own change.
- [A container without a home or `XDG_RUNTIME_DIR` can't resolve a server directory.] → The relay reports `RELAY_ERROR` naming the problem, and the tab shows it.
- [Root `exec` may be refused in hardened containers.] → Same failure as today, with the same message.
- [`/tmp` cleaners can remove a cached container helper.] → The next connect re-deploys. Running roles keep running.
- [An older Brindle can't see sessions a newer one left on a host.] → `brindle-remote list`/`kill` on the host still manage them. The older Brindle's stdio agent doesn't touch the server.

## Migration Plan

- State files: remote zones gain session records (additive). Older files' remote zones without records respawn as before. An older Brindle reading a newer file drops remote session records and respawns.
- Hosts: the first connect of this version deploys its helper and starts (or upgrades to) its server. A host where only older Brindles ran has no server, and the old cached helpers stay unused in `~/.cache/brindle/`.
- Rollback: an older Brindle ignores the host's server. Its sessions stay detached there until `brindle-remote kill --all` or `stop` on the host.
