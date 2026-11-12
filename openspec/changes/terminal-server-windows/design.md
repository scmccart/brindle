# Design

Phase 3 of the terminal-server series. Phase 1's design is in `openspec/changes/archive/2026-11-07-terminal-server/design.md`, its Windows spike in `spike-windows.md` beside it, and phase 2's design in `openspec/changes/archive/2026-11-07-terminal-server-on-close/design.md`. See `proposal.md` for why.

## Context

- **The server is Linux-only.** `remote-agent/src/main.rs` compiles `server` and `pty` under `cfg(target_os = "linux")`, and `serve`, `hold` and the management commands fail elsewhere. Unix types run through the server modules: `RawFd`/`OwnedFd` in the `EventLoop` trait, `UnixListener`/`UnixStream` in `conn.rs`, `peer.rs`, `core.rs` and `holder.rs`, `flock` in `dir.rs`, double fork and `--ready-fd` in `daemon.rs`, `FIONREAD` in `Conn::input_waiting`, signals and process groups in `pty.rs` and `holder.rs`. The core's session logic (`core/{clients,sessions,manage,screen}.rs`) and both protocols are already platform-free.
- **The event loop is readiness-based and level-triggered:** handlers read and write the fd themselves, and a source that is still readable is reported again.
- **Windows tabs** use `Backend::Pty`: alacritty's `EventLoop` over `pty::MarkedPty`, which wraps `conpty::spawn`'s pseudoconsole. `PipeReader`/`PipeWriter` do blocking I/O on threads and post IOCP packets through the `polling` crate, `ChildWatcher` uses `RegisterWaitForSingleObject`, and dropping the last `Arc<Pseudoconsole>` closes it on a short-lived thread while the reader drains. Brindle's `build.rs` embeds `conpty.dll` and `OpenConsole.exe`, extracted to `%LOCALAPPDATA%\brindle\conpty\<version>` when they aren't beside the exe. `parent_console_to` reparents a tab's console window to Brindle's HWND.
- **Phase 2's rules are Linux-only in code:** spawn policies (`Closing::Keeps` → `detach`, `Ends` → `kill`), zone session records, launch reattach (`reattach.rs`), `--attach` and the sessions palette.
- **Brindle's client** (`remote::local`) has a reader thread and a writer thread on one `UnixStream`. The upgrade check reads the running core's binary mtime through `/proc/<pid>/exe`; the holder restarts a core from `/proc/<own pid>/exe`.
- **Helper lookup** (`remote::find_helper`): `BRINDLE_REMOTE_AGENT`, else beside the exe. On Windows that variable already names the *Linux* helper, for the `remote-*` e2e cases.
- **`build_id::of_binary`** reads the ELF build-id note and otherwise hashes the bytes, so it already works for a PE file.
- **The instance pipe** (`instance.rs`) already uses `CreateNamedPipeW` with `PIPE_REJECT_REMOTE_CLIENTS`, a per-user name, and `GetNamedPipeServerProcessId` on the client side.
- **The spike** proved reparenting across processes (asynchronous; a dead HWND is ignored), that an HPCON can't move between processes, detached survival with breakaway, the user-only pipe DACL with a SID check, and one IOCP loop over overlapped pipes for two pseudoconsoles.
- **Its follow-up (tasks 0.1–0.3, on `spike/windows-conpty`: `spike reparent-ext`, `spike survive none --log FILE [--stay N]` launched through a `.lnk`, `wt.exe -w new nt` and Git Bash, and `spike access run|first …`)** found:
  - **Losing the owner:** destroying the current owner window, in the same process or another, or killing that process, leaves the console window alive with no owner. Open dialogs survive, new ones open, and reparenting to a new window (in any process) works. Reparenting to null first changes nothing.
  - **Launch contexts:** an Explorer-launched process (a `.lnk` started through `explorer.exe`, standing in for the Start Menu) is in a job that allows breakaway. Windows Terminal tabs and Git Bash are in no job. Sessions survived closing the tab, killing its shell and closing the WT window. Only `taskkill /T` on a still-running ancestor killed them, because it walks parent pids. Not tested: a GitHub runner, a real Start Menu click, a second user account.
  - **Access:** a process can label a pipe only at or below its own integrity (an `HI` label from a Medium process fails with 1314). A lower-integrity client can't open a labelled pipe for writing (5), but it *can* open it read-only and connect. `GetNamedPipeClientProcessId` → `OpenProcessToken` reads the true user and integrity before any data, for higher and lower clients alike. `ImpersonateNamedPipeClient` fails with 1368 until a byte has been read. `FILE_FLAG_FIRST_PIPE_INSTANCE` on a taken name fails with 5 (231 when the holder allows one instance). Only Medium/Low was tested; the elevated pair wasn't available.

## Goals / Non-Goals

**Goals:**
- Every requirement of `terminal-server` holds on Windows, with the same core and holder code above a platform layer.
- Windows tabs keep today's behaviour (shipped host, passthrough, glyph widths, titles, closing never blocks) and gain phase 2's persistence.
- WinGet and zip upgrades work while sessions run.

**Non-Goals:**
- Remote Windows hosts over SSH (phase 4).
- macOS (`port-to-macos`).
- Handing an HPCON to another process. Holders drain instead, as on Linux.
- Sessions surviving logoff.
- Several Windows user accounts sharing a server.

## Decisions

### Same roles, a platform layer underneath
The core and holder stay the same two roles of one binary, and their logic stays shared. Platform code goes behind a few seams:
- `event_loop` (sources, wait, waker);
- `conn` (a framed connection over a stream);
- `listen` (accept with a peer check);
- `lock` (the single-instance and holder locks);
- `daemon` (a detached start with a ready signal);
- `pty` (spawn, resize, hang up, child exit).

Linux keeps its implementations; Windows adds its own. `main.rs` compiles `server` on `any(linux, windows)`.
- *Alternative rejected:* a separate Windows server. That means two copies of the policy, replay and upgrade logic, which is exactly what the holder/core split was meant to avoid.

### The event loop on Windows: IOCP with a readiness facade
- **The seam changes from `RawFd` to a platform `Source`.** On Linux it wraps the fd. On Windows it wraps a handle plus the loop-owned I/O state.
- **The Windows loop is one IOCP, run with the `windows` crate** (0.61, already in `Cargo.lock` through Brindle). It works like this:
  - each pipe source keeps one overlapped read in flight into a buffer the source owns, and is *readable* while that buffer holds data or has hit EOF;
  - writes copy into an owned buffer and are issued overlapped, and the source is *writable* while that buffer is below its mark;
  - a listening pipe keeps one overlapped `ConnectNamedPipe` pending, and is readable when a client has connected;
  - the waker is `PostQueuedCompletionStatus`;
  - a child's exit is a `RegisterWaitForSingleObject` callback that posts a packet.
- **The handlers' contract doesn't change:** they read what's there, stop after a fair share, and are told again. `Conn::input_waiting`'s `FIONREAD` becomes the buffered count plus `PeekNamedPipe`.
- **Stopping cancels outstanding I/O with `CancelIoEx` and drains the completions** before buffers are freed, as the spike did.
- *Alternatives rejected:*
  - **Blocking reader and writer threads per pipe posting packets through `polling`** (today's `conpty.rs`). It is proven code, but needs two threads per session and per client, has no listener story, and loses the single-thread loop the server was designed around.
  - **mio's `NamedPipe`.** It does the same buffering, but brings another style of I/O into a crate that otherwise uses `windows` directly, and mio's feature set is chosen by tokio's lock entry.

### Directory, pipe names and locks
- **The server directory** is `$BRINDLE_SERVER_DIR`, else `%LOCALAPPDATA%\brindle\run`, or `run-elevated` for an elevated process (see "Elevation"). It is created with a protected DACL for the user and SYSTEM. It holds `core.lock`, `core.log`, and per holder `holder-<build>-<n>.lock` and `.log`, as on Linux.
- **Pipes don't live in the directory.** Each is named from the directory: `\\.\pipe\brindle-server-<h>-core` and `\\.\pipe\brindle-server-<h>-holder-<build>-<n>`. `<h>` is the FNV-1a hash of the directory's canonical path plus the user's SID string, as the instance pipe does. So `BRINDLE_SERVER_DIR` keeps its meaning, with no length limit.
- **The core finds holders by their lock files,** as Linux does through its sockets, and derives each pipe name.
- **Single instance:** `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)` on `core.lock` and each holder lock, held for the process's life. Windows releases it when the process dies.

### Access
- **Pipes are created with these protections:**
  - the SDDL `D:P(A;;GA;;;<user SID>)(A;;GA;;;SY)` plus a mandatory label at the creator's own integrity level (`S:(ML;;NW;;;HI)` when elevated, `ME` otherwise; a process can't label above its own level), so a lower-integrity process can't open a pipe for writing. It can still open one read-only and connect, so the label alone isn't enough and the integrity check below is what refuses it;
  - `PIPE_REJECT_REMOTE_CLIENTS`;
  - `FILE_FLAG_FIRST_PIPE_INSTANCE` on a name's first instance, so a pipe someone else already created under that name (squatting) fails the create, with error 5 or 231, both read as "name taken", and the core or holder exits with a log line.
- **Server side:** before reading anything, the server identifies each client as the spike did. It calls `GetNamedPipeClientProcessId`, then `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` and `OpenProcessToken(TOKEN_QUERY)`, and reads `TokenUser` and `TokenIntegrityLevel`. It refuses a different SID, a lower integrity than its own, or a token it can't open; an elevated client of an unelevated server is refused that way, which the elevation rule wants anyway.
  - `ImpersonateNamedPipeClient` can't be used: it fails with `ERROR_CANNOT_IMPERSONATE` until data has been read (task 0.3), so the check couldn't run before reading.
  - The elevated/unelevated pair is checked by the e2e case in task 8.3 where a runner allows it (CI's runner is elevated).
- **Client side:** Brindle, the CLI commands and a holder dialling its core each check the server with `GetNamedPipeServerProcessId`, then that process's token user and integrity. A different SID or integrity is refused, so a squatter is never sent a request.
- **The integrity check runs both ways.** A process never uses a server running at another integrity level, which also keeps an elevated server from being driven to start elevated programs.

### Elevation
An elevated Brindle (Run as administrator, or started from an elevated console as on CI) runs its tabs elevated today. With a shared server, its tabs would either run unelevated, or let a medium-integrity process spawn elevated programs. So the directory has an elevation suffix (`run-elevated`), which separates the pipe names too, and the integrity checks above make it binding. `BRINDLE_SERVER_DIR`, when set, is used as given; the checks still refuse a mismatched peer.

### Starting a role detached
- **`daemon.rs` gains a Windows start:** `CreateProcessW` of `<helper> <role> --ready-handle <h>`. The flags are `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB | CREATE_UNICODE_ENVIRONMENT`, and a `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` lets the child inherit only the ready pipe's write end.
- **Through a trampoline:** the starter runs `<helper> launch <role> …`, which starts the role with these flags and exits at once, as Linux's double fork does. So no role's parent pid is a live process: `taskkill /T` or Task Manager's "End process tree" on Brindle, which walk parent pids, don't reach the server (task 0.2 saw them do so without it).
- **The role writes one byte once it is listening;** the starter waits up to 2 s, as with `--ready-fd`. The ready pipe's write end passes through the trampoline to the role.
- **Where breakaway is needed:** Explorer and Start Menu launches run in a job that allows it, and Windows Terminal and consoles run in none (task 0.2). The GitHub runner is unknown until the `windows-e2e` job runs the new cases.
- **A job that refuses breakaway** fails the create with `ERROR_ACCESS_DENIED`. The starter then retries without `CREATE_BREAKAWAY_FROM_JOB` and logs once that sessions won't outlive the starter's job. A `KILL_ON_JOB_CLOSE` job then still ends them; that is accepted and logged.
- **No console window:** `brindle-remote.exe` stays a console-subsystem program for its CLI, and the detached flags keep server roles from opening a console.
- **The same start serves all three cases:** Brindle starting a core, a core starting a holder, and a holder restarting its core.

### Running from a per-build copy
- **Brindle never starts a server role from the installed helper.** It copies `brindle-remote.exe` to `<copies>\<build id>\brindle-remote.exe`, where `<copies>` is `%LOCALAPPDATA%\brindle\server`, or `bin` inside `BRINDLE_SERVER_DIR` when that is set, so test runs never write to or prune the user's copies: it copies to a temporary name, then renames, and if another process won the rename, uses that file once its build id checks out. Then it starts the role from the copy.
  - `CopyFileW` keeps the last-write time, so the upgrade check's "later mtime" rule still compares build times.
  - Cores and holders start their own roles from their own image path (`GetModuleFileNameW`), which is the copy.
- **Pruning:** at each core start, Brindle deletes the other build directories it can. A running image can't be deleted, so a directory still in use survives.
- *Why:*
  - Windows locks a running image, and with persistent sessions a holder runs for days, so the installed `brindle-remote.exe` would block every WinGet upgrade and every `cargo build` of the helper.
  - Old holders keep running from their own copy while the new build drains them.
- **The upgrade check reads the core's exe path with `QueryFullProcessImageNameW`,** where Linux reads `/proc/<pid>/exe`.

### Helper lookup
`BRINDLE_SERVER_HELPER` names the local server's helper on every platform. Without it:
- Linux falls back to `BRINDLE_REMOTE_AGENT`, then beside the exe, as today;
- Windows looks only for `brindle-remote.exe` beside `brindle.exe`. There `BRINDLE_REMOTE_AGENT` keeps naming the Linux helper for remote profiles and the `remote-*` cases.

`HELPER_MISSING` names the new variable.

### ConPTY in the holder
- **What moves into the agent:** `conpty.rs`'s `Api` resolution, spawn, `Pseudoconsole` and `cmdline`'s launch-line quoting (`command_line`, `command_line_raw`, `launch_line`).
  - Brindle keeps `program_stem`, `is_console_title`, `windows_editor_command`, `windows_default_program` and `merge_environment`.
  - The ConPTY embedding moves from Brindle's `build.rs` to `remote-agent/build.rs`, with the same extraction rules (beside the exe, else `%LOCALAPPDATA%\brindle\conpty\<version>` when the bytes match).
  - `scripts/package-windows.ps1` checks the embedding in `brindle-remote.exe` instead.
- **`SPAWN` carries the command and arguments.** On Windows, Brindle always resolves the program (`windows_default_program` for the default profile), so `command` is never empty, and the holder builds the line with `launch_line`. The environment is complete and already merged case-insensitively by Brindle, as Linux sends it. The holder passes it as given.
- **`BRINDLE_CONPTY` is read from the `SPAWN` environment, not the holder's.** The holder resolves the shipped and the inbox `Api` lazily, and caches each. So the fallback e2e case works when a holder is already running.
- **Output pipes:** the holder creates two overlapped named pipes per session, input then output (`\\.\pipe\brindle-pty-<holder pid>-<n>` with consecutive `n`, first instance, the user DACL). It hands ConPTY the synchronous ends, and keeps the overlapped ends in its loop. The flags (`PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH`) are unchanged.
- **Exit:** the child's exit wait posts to the loop.
  - The holder then calls `ReleasePseudoConsole` when the export exists, and `ClosePseudoConsole` on a short-lived thread, because it can block until output drains.
  - The loop keeps reading the output pipe to EOF.
  - `EXITED` follows the last output, with `GetExitCodeProcess`'s code.
- **Hangup (`CLOSE`, `KILL`, `STOP`, a `kill` session's owner gone):**
  1. The holder closes the pseudoconsole in the same way, which ends the console's attached processes as closing a tab does today.
  2. After `agent.rs`'s grace, it calls `TerminateProcess` on the session's root process if it is still alive.
  - Programs that left the console (a GUI app started from the shell) keep running, as they do today. This is the Windows meaning of "the processes under it".
- **No per-session job object.** A kill-on-close job would also end programs the user started to outlive the tab, and nested jobs interact with the starter's job.

### REPARENT
- **No protocol change.** Protocol 1 (message 13) and H1 (message 11) have carried `REPARENT { session, hwnd }` since phase 1, and a Linux core and holder refuse it. A Windows core forwards a client's `REPARENT` to the session's holder, which applies it with `ConptyReparentPseudoConsole`. With the inbox ConPTY, which has no such export, the holder logs and ignores it.
- **Brindle sends it** where `parent_console_to` runs today: when a terminal's view is first in a window, and after spawn, attach and each reconnect's replay.
- **Nothing is sent on quit.** Task 0.1 found that a destroyed owner (a quit, a crash, the process killed) leaves the console window alive and ownerless, and the next `REPARENT` re-owns it; clearing the owner first made no difference.
- **It is advisory:** there is no reply, the call is asynchronous, and the holder ignores errors.
- **The core doesn't remember the HWND across restarts;** the client re-sends after reconnecting.

### Brindle's client on Windows
- **`Backend::Server` and `server::Sink` build on Windows.** `Backend::Pty`, `pty::MarkedPty`, `ParsingReader` over ConPTY, `terminal/scan.rs`'s Windows path and `conpty.rs` leave Brindle. alacritty's `tty` and `EventLoop` are then unused on every platform.
- **The connection opens the core's pipe with `FILE_FLAG_OVERLAPPED`.** A synchronous pipe handle serializes all I/O on it, so a blocked `ReadFile` on the reader thread would stall the writer thread's `WriteFile`. Each thread then waits on its own event. The pipe client is a `Read + Write` type, so `connection.rs` stays shared.
- **Process info:** the pid from `SPAWNED`/`REPLAY_START` drives the Toolhelp snapshot, as today's `PtyProcess` does. The directory comes from OSC 7 and 9;9.
- **Phase 2 on Windows:** spawn policies, zone session records, launch reattach, `--attach` and the sessions palette drop their `cfg(target_os = "linux")`.

### Tests
- **`remote-agent/tests/{holder,core,upgrade,manage}.rs` run on Windows.** Where a test relies on signals, process groups or `/proc`, the Windows form does this:
  - kills with `TerminateProcess`;
  - checks liveness with `OpenProcess` and `GetExitCodeProcess`;
  - checks a program's parent with Toolhelp.
- **New unit tests:** the IOCP loop has the same tests as `PollLoop` (tokens, pausing, writable, waker), plus cancel-and-drain. The access checks get their own tests: SID and integrity comparison as pure functions, and the pipe's security descriptor read back with `GetSecurityInfo`.
- **e2e on Windows:**
  - Each case gets a `BRINDLE_SERVER_DIR` under `target/tmp/e2e/<case>/server`, and the harness runs `brindle-remote stop` and copies `core.log` afterwards, as on Linux.
  - The suite requires the helper beside the binary, and checks that it isn't older than its sources.
  - New `local-server-*` Windows cases: quit keeps, `taskkill /F` keeps, reattach with replay, core killed and reconnect, no helper, `REPARENT` (a dialog in a session is owned by Brindle's window), and an elevated-or-not mismatch refused.
  - The existing Windows cases run through the server unchanged. The host case now checks that `OpenConsole.exe` is the holder's child.
- **Perf:** `tests/perf` runs only its self-test on Windows. Compare `--latency-bench` on `main` and the branch by hand, and run a flood (the responsiveness case) for throughput.

## Risks / Trade-offs

- [Jobs that forbid breakaway (possibly CI runners and some launchers) end sessions with the starter] → It is logged once, and sessions still survive the core's restarts. Explorer, Windows Terminal and consoles are fine (task 0.2); the CI runner shows up in `windows-e2e`.
- [A parent-pid tree kill reaching the server] → The trampoline leaves no live parent. A pid reused by an unrelated process could still be walked into, which is accepted as rare.
- [Two hops each way on Windows] → Measure latency against `main` (task 9.3). The data fast path from phase 1's risks stays available.
- [The per-build copy uses disk and could leak] → It is one helper per build, pruned when not running. The copies live in `%LOCALAPPDATA%\brindle`, which uninstall keeps, as it keeps state.
- [Antivirus scanning or blocking a copied exe under `%LOCALAPPDATA%`] → The helper is signed as Brindle is (or not, as today), and the copy is byte-identical to the installed one. The tab's error names the start failure.
- [Elevated and unelevated Brindles show different sessions] → Intended, and `brindle-remote list` from each shows its own.
- [IOCP buffer lifetimes are unsafe code] → Buffers are owned by sources freed only after their operations complete or are cancelled and drained; loop unit tests cover cancel-and-drain and closing with I/O in flight.
- [Quitting no longer ends Windows programs] → Breaking for Windows users, marked `feat!`; closing a tab still ends its program.

## Migration Plan

- No configuration changes. The first launch after the update starts a server, and Windows tabs run in it. Tabs from an earlier version's state file have no session records, so they restore as fresh shells in their recorded directories.
- The helper is required on Windows. The zip and the WinGet package ship it beside `brindle.exe`; a `cargo build` puts it in `target/<profile>/`.
- Rollback is to the previous release. Sessions a newer server kept keep running until they end or `brindle-remote stop`.
