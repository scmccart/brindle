# Design

## Context

- **The helper today.** `brindle-remote` (`remote-agent/`, protocol in `remote-protocol/`) is one stdio child per tab, started over `ssh` or `docker`/`podman exec` with `--server`. It owns one PTY, on threads, with only `libc` as a dependency. It ends the session on stdin EOF, or when it has read no frame for `idle_limit_secs` (60). It is cached by Brindle version and digest, so a remote client always talks to a helper of its own build. Frames are `type: u8, len: u32 BE, payload`, and carry no session id.
- **Local terminals** use `Backend::Pty`. alacritty's event loop runs over `MarkedPty`, whose `ParsingReader` parses output on the I/O thread under the term lock (parser → term → marks). The child inherits Brindle's environment plus `child_env(profile)`. Titles and split directories come from `process::PtyProcess`, which reads `/proc` synchronously.
- **`Backend::Agent`** (remote tabs) is fed through `Terminal::feed()` on the GPUI thread, and caches process info from `ProcessInfo` events.
- **Shared parsing state.** The OSC 133 marks (`shell_integration.rs`) and the OSC 7501 records (`status.rs`) live in the `brindle` crate. The helper can't use them yet.
- **Finding the helper.** `remote::client` looks for it in `BRINDLE_REMOTE_AGENT`, then beside the executable (`brindle-remote-linux-x86_64`, `brindle-remote`). `scripts/install.sh` and the Linux archive already ship it.
- **The Windows spike** (`spike-windows.md`) found four things:
  - a pseudoconsole can be reparented to another process's window, asynchronously;
  - a live HPCON can't be rebuilt in another process. The pipes, process handle and exit code can move, but resize, reparent and close can't, and the inbox ConPTY can't move at all;
  - a detached helper outlives its parent unless a job forbids breakaway or kills on close;
  - one IOCP loop can serve a named pipe and several pseudoconsoles over overlapped pipes.
- **herdr-core** (Apache-2.0, used as a reference for approach only) avoids the HPCON problem by never handing a PTY over. A small holder process owns every PTY and is never replaced while it has terminals. A larger core runs the emulators and restarts freely.

## Goals / Non-Goals

**Goals:**
- The server is complete enough for phases 2–4 to change only policy and transport: sessions, both close policies, attach with replay, reconnect, versioned protocols, upgrades and management.
- One upgrade and crash model for Linux and Windows, built on the holder/core split.
- Local Linux tabs behave as they do today, latency and throughput included, within the measured cost of the extra hops.

**Non-Goals:**
- Brindle using the `detach` policy, reattaching through `session-restore`, and the "sessions on host" palette (phase 2).
- Windows: the IOCP loops, named pipes and ConPTY holder (phase 3). Windows keeps `Backend::Pty` until then.
- Remote profiles and the stdio `--server` mode, which stay as they are (phase 4).
- Several clients attached to one session at once.
- Replacing a holder that still has sessions. Holders drain instead.

## Decisions

### Core and holders (from the spike)
The server is two roles of one binary.
- **`brindle-remote hold`** is a holder: it owns many sessions' PTYs, is the programs' parent, pumps their I/O, resizes, signals and reaps them, and parses nothing. It keeps two things per session, as bytes it doesn't interpret: the core's latest checkpoint and the raw output since then. It is kept small and changes rarely.
- **`brindle-remote serve`** is the core: the client socket, the emulators and replay, the close policies, the management commands, and the holders' supervision. It can restart at any time without ending a session.
- *Why:* the spike showed a Windows HPCON can't move to another process, so whatever owns a PTY must outlive every upgrade of the code above it. Splitting also means a core crash, the likelier kind since the core holds the parser and the protocol, no longer ends every terminal.
- *Alternatives rejected:*
  - **re-executing one server in place** (the earlier plan): there is no `execve` on Windows, and a crash still kills everything;
  - **side-by-side whole servers on Windows:** two platform models, and two servers to manage;
  - **killing sessions on upgrade.**

### Directory, single instance and access
- **Server directory:** `$BRINDLE_SERVER_DIR`, else `$XDG_RUNTIME_DIR/brindle`, else `~/.cache/brindle/run`, created with mode 0700. It holds:
  - `core.lock`, `core.sock` and `core.log`;
  - per holder, `holder-<build>-<n>.sock` and `holder-<build>-<n>.log`.
- **Single core:** the core takes `flock(LOCK_EX | LOCK_NB)` on `core.lock` for its whole life, and only then unlinks a stale `core.sock` and binds. A core that can't take the lock exits 0.
- **Holders:** a holder binds its own socket and also flocks it, so a core can tell a live holder from a stale socket.
- **Access:** both kinds of socket check every peer with `SO_PEERCRED` against their own uid before reading anything.
- **Start:** a process runs `brindle-remote serve --ready-fd N` (or `hold --ready-fd N`), which double-forks, `setsid`s, redirects stdio to its log and writes one byte once it is listening. The starter waits up to 2 s. In a race, the loser exits on the lock, and its starter connects to the winner.
- **Exit:**
  - a holder exits when it has no sessions;
  - the core exits after 5 s with no sessions and no clients.

### Event loops
The core and the holders are each one thread running an event loop over non-blocking fds, with outgoing queues per peer.
- On Linux the loop is `poll(2)` with a `SIGCHLD` self-pipe in the holder. The loop sits behind a small platform trait (register, wait, wake), so phase 3 can provide IOCP with overlapped pipes, as the spike ran it.
- **Backpressure:**
  - a client whose queue passes 1 MiB makes the core stop reading that client's sessions from the holder;
  - a holder whose queue to the core passes 8 MiB stops reading those PTYs;
  - with no core connected, a holder keeps reading until a session's tail is full, then stops reading it, so a program blocks rather than lose output while the core is away.
- If the emulators slow the core's loop, they move to worker threads, fed in order per session. That changes no protocol.

### Shared terminal-state crate
A new workspace crate, `brindle-term-core` (`term-core/`), takes the GPUI-free parts that both sides need: Brindle's `Parser` (vte plus the OSC 133 scanner and `Marks`) and the OSC 7501 `Records`, with their unit tests. It depends on `alacritty_terminal`, which is already in `Cargo.lock`. `brindle` and the core use it; the holder doesn't.
- Its version matches the other crates'. The CI check covers every workspace crate, and release-please has to bump it with the others (task 1.1).
- *Why:* the core must track marks and records exactly as the client does, or replay drifts.

### Per-session emulator, replay and checkpoints
In the core, each session feeds output into a `Term` through the shared `Parser`. Its listener drops every event, `PtyWrite` included, so the server never answers queries. The `Term` uses the session's scrollback limit.

The replay renderer turns a `Term` into a stream:
- `ESC c`;
- the primary screen: scrollback, then the visible rows, with SGR runs. Rows joined by `WRAPLINE` have no line break between them;
- OSC 133 marks at their lines;
- if the alternate screen is active, `CSI ? 1049 h` and the alternate grid;
- the modes (DECCKM, keypad, mouse 1000/1002/1003/1005/1006, focus 1004, bracketed paste, 1007, cursor visibility, DECSTBM, origin, autowrap);
- the cursor position, its shape and blink, and the title;
- the latest OSC 7501 report for each status record.

The same stream does two jobs:
- **Attach:** the stream is sent between `REPLAY_START` and `REPLAY_END`.
- **Checkpoints:** the core sends a session's stream to its holder as a `CHECKPOINT`, and the holder then drops that session's tail:
  - when the tail since the last checkpoint passes 512 KiB;
  - for every session before a planned step-down;
  - when a session is detached.

A new core rebuilds each session by feeding a fresh `Term` the checkpoint, then the tail. If a tail ever overflowed its 1 MiB limit, the holder marks it truncated, and the rebuild is the last 1 MiB only, which may be approximate.
- The renderer is pure (`&Term → Vec<u8>`) and tested by round trips over fixtures.
- **Settled in task 5.2:**
  - The signature is `replay::render(&mut Term, &Marks, &Records, title) -> Vec<u8>`. It takes `&mut` because alacritty hides the primary grid under the alternate screen, the scroll region, the tab stops and the charset: the renderer probes for them and restores the term afterwards.
  - `Term` keeps its title private, so the core's listener keeps the title from `Title`/`ResetTitle` events and drops every other event.
  - Records replay least recently updated first.
  - Each render costs about O(screen rows) of clones, so it runs on attach and at checkpoints only, never per output chunk.
  - The known fidelity limits are listed in `term-core/src/replay.rs`, for example the title stack and marks scrolled above the history.
- *Alternatives rejected:* a raw ring buffer alone, which breaks full-screen apps and rotated scrollback, and hidden tmux.

### Core ↔ holder protocol
It uses the same frame header, with its own message numbers and its own version, H1. It is kept minimal so holders rarely need to change.
- **Core to holder:**
  - `HELLO { min, max, core_build, instance }`;
  - `SPAWN { session, command, args, cwd, env, cols, rows, cell_w, cell_h, meta }`;
  - `INPUT`, `RESIZE`, `HANGUP { session }`, `CHECKPOINT { session, bytes }`;
  - `DRAIN`, `GOODBYE { handover }`;
  - `REPARENT { session, hwnd }`, which a Linux holder answers with `ERROR unsupported`.
- **Holder to core:**
  - `WELCOME { protocol, build, pid, previous: none | handover | crash, sessions: [{ session, pid, cols, rows, exit, meta, truncated }] }`, followed by each session's `CHECKPOINT` and `TAIL`;
  - `OUTPUT`, `SPAWNED { session, pid }`, `SPAWN_FAILED { session, message }`, `EXITED { session, code }`.
- **`meta` is an opaque JSON blob the core owns:** the policy, the owning client's token, the scrollback limit, and the command and directory for `list`. The holder only stores it.
- **Session ids:** the core allocates them, and a new core continues from the largest id its holders report.
- **One core per holder:** a newer `HELLO` takes a holder over from an older connection.
- **Settled in task group 3:**
  - **Locking:** a holder flocks a sibling `holder-<build>-<n>.lock`, which also picks `n`, and checks the inode after locking.
  - **Empty start:** a holder that never had a session exits 10 s after starting with no core.
  - **Exits while no core is connected** are reported in the next `WELCOME`, and the session is dropped after that.
  - **Takeover:** a takeover reports `previous: crash` unless the old core sent `GOODBYE { handover: true }`.
  - **Input:** more than 4 MiB of queued input for one session is dropped, with an `ERROR`.
  - **Supervision:** the holder starts `/proc/self/exe serve --ready-fd 3` 3 s after its core leaves, retrying at 3, 6, 12 s and so on, capped at 60 s. Tests override the arguments with `BRINDLE_TEST_HOLDER_CORE_ARGS`.
  - **Code layout:** the PTY code shared with the stdio helper lives in `remote-agent/src/pty.rs`, and the server modules in `remote-agent/src/server/`.
- **Settled in task group 4:**
  - **Per-session flow control:** H1 gained `PAUSE { session }` and `RESUME { session }` (types 12 and 13). One holder connection carries every client's sessions, so the core can't stop reading from it for one client. Instead it pauses a client's sessions once more than 1 MiB is queued to that client, and resumes them below 256 KiB. H1 hasn't shipped, so this edits H1 rather than bumping its version.
  - **A `serve` that loses the start race** waits up to 2 s for the winner to accept, writes its ready byte and exits 0, so both starters succeed.
  - **The core accepts clients only after adopting the startup holders** (at most 5 s), so session ids continue correctly and returning owners find their sessions.
  - **A returning `client_token`** gets back every session of its own that has no client, `detach` ones included. A session that exits while its owner is away keeps its `EXIT` for that owner.
  - **Hangup:** the holder sends the final SIGKILL to the session's process group while the group still has members (`kill(-pgid, 0)` succeeds), even once the leader is reaped. A pgid can't be reused while members hold it, so this reaches children that ignore SIGHUP without a pid-reuse risk.
  - **Code layout:** the core lives in `remote-agent/src/server/core.rs`, with `core/{clients,holders,sessions}.rs`.
- **Settled in task group 5:**
  - **`CHECKPOINT_END { session, covers: u64 }`:** a checkpoint covers only the tail bytes the core had fed when it rendered, and the holder drops just that many from the front of its tail. Dropping the whole tail lost output that was still on its way to the core. This is an H1 edit made before release.
  - **`Emulator`:** the client's OSC scanner (`notify/osc.rs`) moved into `brindle-term-core` (Brindle re-exports it), and a new `Emulator` type there bundles the `Term`, `Parser`, `Marks`, `Records` and the kept title. term-core re-exports `alacritty_terminal`, so the agent crate needs no new dependency.
  - **Feeding details:**
    - Scrollback 0 in `SPAWN` means 10 000 lines, Brindle's default.
    - `ESC c` clears the kept title.
    - A synchronized update still in progress is applied before rendering, so its output is in the replay and the checkpoint.
    - Client `DATA` drops `done` and `error` records, as user input does in Brindle. The core can't tell typing from replies to queries, so its records can drift slightly from the client's.
  - **Ownership:** an attaching client becomes a session's owner only inside the running core, because H1 can't update the owner the holder keeps. After a core restart, the original owner gets the session back. Phase 1's Brindle never attaches, so this only matters from phase 2.
  - **A `kill` session explicitly `DETACH`ed** has no client and no policy that ends it. Phase 1's Brindle never detaches. Phase 2 must settle this with `on_close`.
  - **Rebuilding:** a replay asked for while an adopted session is still being rebuilt waits for the holder's tail.
  - **Fault injection for tests:** `BRINDLE_TEST_FEED_PANIC`, compiled in only with `debug_assertions`.
- **Settled in task group 6:**
  - **`STEP_DOWN`** flushes the checkpoints and `GOODBYE` to every holder (up to 10 s each) before `OK`. A `SPAWN` during a step-down is refused with "try again". `STEP_DOWN` is refused, with a log entry, only during a `STOP` or another step-down. A session still starting isn't checkpointed, and the next core adopts it from its holder.
  - **Logging refusals:** `check` runs in the client, so it appends its refusal reason to `core.log` when the server directory exists, as well as printing it on stderr.
  - **Draining:** every core sends `DRAIN` to any adopted holder of another build, not only after an upgrade. The core tracks the draining flag itself.
  - **The commands:**
    - With no server, `status --json` prints `{"running":false}`. With one, it prints `SERVER_STATUS` plus `"running": true`.
    - `list` prints one tab-separated line per session (id, pid, attached or detached, command or "(login shell)", directory or "-"), with no header. With no server, `list --json` prints nothing.
    - `kill` and `stop` with no server say so on stderr and exit 0.
    - `status`'s client count includes the command's own connection.
  - **Code layout:** the commands live in `remote-agent/src/server/cli.rs`. `brindle-remote-protocol` re-exports `serde_json`.
- **Settled in task group 7:**
  - **Shared code:** `server_dir::resolve` and `build_id::of_binary` live in `remote-protocol`, so Brindle and the helper resolve the same directory and build id.
  - **`EXIT { session, code, lost? }`:** the optional `lost` reason (omitted when unset, so the existing encodings don't change) tells a holder's death apart from a program killed by a signal. Brindle keeps the tab open, with the "session lost" line, when `lost` is set.
  - **After a reconnect** Brindle sends `LIST` to learn which sessions the core no longer has.
  - **Upgrade decision:** the running core's binary mtime comes from `/proc/<core pid>/exe`, so `WELCOME` is unchanged.
  - **Environment:** variables that aren't UTF-8 are dropped, because the protocol carries JSON strings. `WINDOWID` and `ALACRITTY_WINDOW_ID`, which alacritty's PTY code used to set to `0`, are no longer set.
  - **The holder sets `IUTF8`** on every PTY, as alacritty did, so line editing handles multibyte characters.
  - **Spawning never blocks the UI thread:** `Terminal::spawn` returns at once, and the connection, a reconnect or an upgrade's step-down finishes in the background, with input queued, as `Backend::Agent` does.
  - **e2e server directories** are under `/tmp`, not `target/tmp/e2e/<case>/`, because holder socket paths there exceed the 108-byte `sun_path` limit. The harness copies `core.log` into the case directory.

### Client protocol
It uses the same frame header with its own numbers, and this change is protocol 1. The stdio protocol is untouched.
- **Handshake:** `HELLO { min, max, client_version, client_token }` → `WELCOME { protocol, core_version, core_build, pid, holder_protocols }` or `ERROR` naming both ranges. `client_token` is random per Brindle process, and it is how a reconnecting client gets its sessions back.
- **Requests carry `req` ids.** Session-scoped messages carry the session id: `DATA`'s payload starts with it, and the JSON messages have a `session` field.
- **Messages:**
  - sessions: `SPAWN { req, command, args, cwd, env, cols, rows, cell_w, cell_h, scrollback, policy }` → `SPAWNED { req, session, pid }` or `ERROR { req, message }`;
  - I/O: `DATA`, `RESIZE`, `EXIT { session, code }`, `CLOSE { session }`;
  - attach: `ATTACH { req, session, cols, rows }` → `REPLAY_START`, `DATA`…, `REPLAY_END`; `DETACH { session }`;
  - reconnect: after `WELCOME`, the core sends a `REPLAY_START` … `REPLAY_END` for each session it still holds for that `client_token`;
  - management: `LIST`, `KILL { session | all }`, `STATUS`, `STOP`;
  - upgrade: `STEP_DOWN { req }`;
  - Windows: `REPARENT { session, hwnd }`, reserved in protocol 1 and refused on Linux.
- **Compatibility:**
  - JSON fields are additive, with defaults;
  - new message types need a new version;
  - the core answers each connection in its negotiated version, and drives each holder in its own negotiated version;
  - transcript tests pin protocol 1 and H1.
- **No heartbeat locally:** a local socket's EOF is reliable.
- **Details settled while implementing (task group 2):**
  - **Replies:** `SESSIONS` answers `LIST`, `SERVER_STATUS` answers `STATUS`, and `OK { req }` answers `KILL`, `STOP` and `STEP_DOWN`.
  - **`ERROR`** has optional `req` and `session` fields, so a refused message with no `req` (`REPARENT`, `RESIZE`) can be matched. H1 has its own holder-to-core `ERROR { session?, message }`.
  - **`REPLAY_START`** carries `req?` (empty on reconnect) and the session's `pid`.
  - **Session entries in H1's `WELCOME`** have `exit: Option<{ code: Option<i32> }>`, to tell a running session from one killed by a signal.
  - **`KILL`** is `{"target":{"session":N}}` or `{"target":"all"}`.
  - **Optional fields:** `command` is optional (none means the login shell), `cell_w`, `cell_h` and `scrollback` default to 0, and `instance` is a `u64`.
  - **Checkpoints and tails** are chunked as `*_BEGIN { session, total: u64 }`, `*_CHUNK`s of 64 KiB and `*_END`. A holder replaces a checkpoint only on `CHECKPOINT_END`. The limits are 64 MiB for a checkpoint and 1 MiB for a tail.
  - **`meta`** is a `serde_json::Value`, and the core writes it with sorted keys. A workspace build turns on serde_json's `preserve_order`, so key order would otherwise differ between builds.

### Close policies across core restarts
A `kill` session ends on `CLOSE`, `KILL` or `STOP`, and when its owner is gone. While the core runs, gone means its client's connection ended. After a core starts with adopted sessions, owners get a reconnect grace period of 10 s; when it passes, the `kill` sessions of owners who didn't return are hung up. Hangup is the holder's job: it closes the master, sends `SIGHUP` to the process group, and sends `SIGKILL` after a grace period, with agent.rs's timings. A `detach` session ends only on `KILL`, `STOP` or its program's exit.

### Upgrades
1. A client connects, and `WELCOME` reports the core's build and the holder protocols in use. The client decides whether its own helper is newer: a higher semver, or an equal version with a different build id and a later mtime (dogfooding builds keep the version).
2. If it is newer, the client runs `<helper> check --holder-protocols <list>`. If that fails, it uses the running core when they share a protocol version, and otherwise shows the error in its tabs.
3. The client sends `STEP_DOWN`. The core checkpoints every session, sends `GOODBYE { handover }` to each holder, answers `STEP_DOWN`, closes its clients and exits.
4. The upgrading client starts its own `serve`. The new core finds the holders by their sockets, adopts them, and rebuilds the screens. It sends `DRAIN` to every holder of another build, and starts a holder of its own build when it next needs one.
5. Other clients see EOF and reconnect. They wait up to 5 s for a core to appear before starting one, so the upgrading client's core wins the race.

If no core connects to a holder within 3 s, the holder starts one from its own binary through `/proc/self/exe`, which works even after an install has replaced the file. It retries with backoff capped at 60 s. That covers both a crashed core and a new core that won't start.
- *Why the client decides:* the core only needs to vet and step down. That also lets tests "upgrade" to the same binary.

### Brindle's client
- **One connection per Brindle process** (`remote::local`), with a reader thread that demultiplexes frames and a writer thread with a queue.
- **Output is parsed on the reader thread,** in the usual lock order, which then sends `Wakeup` and `Osc` events on the terminal's channel, as `ParsingReader` does.
- **During a replay** the sink applies everything to the terminal's state, but drops bells, OSC 9/777 notices and status *notices* (`TerminalEvent::Status(Some(_))` becomes `Status(None)`), so a reconnect raises no attention.
- **A new `Backend::Server { session, conn, pid }`.** Input, resize and close go to the connection, and `EXIT` marks the terminal exited.
- **Reconnect:** when the connection ends, the client reconnects with the same `client_token`, starting a core if none appears within 5 s, and gets every session back with a replay. Input typed meanwhile is queued per session.
- **A session that doesn't come back,** or no core within 15 s, gets the red "session lost" line, and the terminal is marked exited without emitting `Exited`, so the tab stays open.
- **Process info stays local and synchronous:** `/proc/<pid>/stat`'s `tpgid`, then that process's `comm` and `cwd`.
- **The environment in `SPAWN` is complete:** Brindle's own environment with `child_env(profile)` applied.
- **Always the server on Linux.** There is no in-process fallback, and the Unix side of `Backend::Pty` is removed. A failure to find the helper, connect or negotiate becomes that terminal's error message (`Terminal::message`) and isn't cached, so the next spawn tries again.
  - *Alternative rejected:* a fallback or an off switch, which keeps a second backend alive and hides server problems.

### Failure isolation
- Each `Term` feed in the core runs under `catch_unwind`. A panic marks that session's emulator broken; it keeps forwarding output, and its checkpoints and replays are just `ESC c`.
- A core crash ends no session.
- A holder crash ends only its own sessions, so holders stay small.

### Builds and tests
- **Builds:** the root `Cargo.toml` gets `default-members` listing every crate, so plain `cargo build` produces `brindle-remote`.
- **Server tests** (agent crate, no GUI) run a real core and holders in a temp `BRINDLE_SERVER_DIR`, with a test client. They cover:
  - the roles, the start race, peer checks and exit when unused;
  - spawn, environment, I/O, exit codes and a stalled client;
  - both policies, client SIGKILL, attach and replay fidelity, and refusing to attach a taken session;
  - status and marks, and that the server never answers queries;
  - reconnect within and after the grace period, and a core SIGKILL with respawn;
  - checkpoints and tail truncation;
  - an upgrade to the same binary, drain, a refused check, and a new core that won't start;
  - the management commands.
- **e2e:** every case gets its own `BRINDLE_SERVER_DIR` under `/tmp` (see "Settled in task group 7") and a `brindle-remote stop` afterwards. The new `local-server-*` cases are listed in tasks 8.2.
- **perf:** `tests/perf` on `main` before and on the branch after, with `BRINDLE_PERF_BASELINE`.

### Windows (phase 3, shaped by the spike)
- The holder owns the HPCONs, and the ConPTY code moves into it.
- It is started with `DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB`. When a job refuses breakaway, it retries without the flag and logs that sessions won't outlive the job.
- The output pipes are overlapped named pipes it creates. Its loop and the core's are IOCP.
- The sockets are named pipes with a user-and-SYSTEM DACL, `PIPE_REJECT_REMOTE_CLIENTS` and a SID check.
- `REPARENT` is asynchronous and advisory, and the client sends it again whenever its window changes.
- The inbox ConPTY works the same way, since nothing ever moves an HPCON.

## Risks / Trade-offs

- [Two hops each way, client ↔ core ↔ holder] → Measure `latency-shell` and vtebench against `main`. If they regress noticeably, add a data fast path in which the core passes the client a direct holder channel for one session's I/O. That would be a later protocol version.
- [The output is parsed twice, by the core and by the client] → Worker threads for emulators, or batched feeding, if the numbers need it.
- [A holder crash ends its sessions] → Holders have no parser and little code, a fuzzed H1 decoder, and are tested heavily.
- [Drained holders keep old behaviour until their sessions end] → Accepted. `status` shows draining holders, and H1 stays small, so there's little behaviour to differ.
- [Checkpoints cost CPU on busy sessions] → A 512 KiB trigger bounds it to one render per 512 KiB of output. Measure in perf.
- [A race between a reconnecting old client and the upgrading client] → Reconnecting clients wait 5 s before starting a core. A wrong winner converges, because the next newer client steps it down again.
- [Both protocols must stay compatible for good] → Minimal v1 and H1, transcript tests, per-connection versions.
- [The helper grows] → Check the musl build and binary size early (task 1.3).
- [Phase 2: the desktop's app scope may kill holders when Brindle exits] → Doesn't matter with `kill`. Before `detach`, check GNOME and KDE scopes and use `systemd-run --user` if needed.

## Migration Plan

- No configuration changes. On first launch after the update, Brindle starts a core and a holder, and local tabs run in them.
- The helper is required on Linux. `scripts/install.sh` and the Linux archive already ship it.
- Rollback is to the previous release.
- Remote profiles and tmux control mode are unaffected.

## Open Questions

- Grace periods and thresholds (unused exit, reconnect grace, holder respawn, checkpoint trigger, queue marks), to be tuned in tests.
- Whether emulators need worker threads, and whether the data fast path is needed, to be decided by the perf numbers.

## Later phases (separate changes)

2. **`on_close`:** a per-profile `kill | detach | ask`, applied to closing a zone, tab or window, quitting, crashes, dropped connections and the shell exiting. Restore reattaches by session id and falls back to respawning in the recorded directory. Adds the "sessions on host" palette.
3. **Windows server:** local Windows tabs always run in the server too, with no fallback, and `Backend::Pty` is removed entirely. This means the holder and core on IOCP and named pipes, and the HWND reparent, as in "Windows (phase 3, shaped by the spike)".
4. **Remote and eager:** `brindle-remote --attach` relays ssh and exec stdio to each host's core. Eager profiles connect at launch, with an offline state, may start their containers, and Windows SSH hosts get a PowerShell deploy. This absorbs follow-ups 2–4 of `remote-connections-followups` and replaces the idle-limit requirement.
