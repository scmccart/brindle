# Tasks

## 0. Close the spike's gaps (before anything builds on them)

- [x] 0.1 On the `spike/windows-conpty` branch, extend `spike reparent` to cover two cases: the current owner HWND destroyed while the session lives (then a dialog opened, then reparenting to a new window), and the owner window in a third process, not the one that created the pseudoconsole. Record in design.md ("REPARENT") whether the console window and dialogs survive, and whether `hwnd: 0` on quit is enough. Verify: the findings are written with the commands that produced them.
- [x] 0.2 Run `spike survive` from a Start Menu shortcut, from Explorer, inside Windows Terminal (closing its tab), and on a GitHub `windows-latest` runner, and record which allow breakaway in design.md ("Starting a role detached"). Verify: a table of launch method → survived or not.
- [x] 0.3 In the spike, check the access model: a medium-integrity client can't open a pipe labelled `S:(ML;;NW;;;HI)`; `GetNamedPipeClientProcessId` plus `OpenProcessToken` reads `TokenUser` and `TokenIntegrityLevel` of an unelevated client, and fails or reads high integrity for an elevated one; `ImpersonateNamedPipeClient` before any read fails with `ERROR_CANNOT_IMPERSONATE`; and `FILE_FLAG_FIRST_PIPE_INSTANCE` fails when the name exists. Verify: the outcomes are recorded in design.md ("Access").
- [x] 0.4 If 0.1–0.3 contradict a spec scenario ("Owner after reattach", "Started from the Start Menu", the access requirements) or design.md, revise the specs and design with `/opsx:update` before group 1 starts. Verify: `openspec validate terminal-server-windows --strict` passes after any revision, and design.md has no "check early" items left.

## 1. Platform seams in the server (Linux behaviour unchanged)

- [x] 1.1 Change `EventLoop` to take a platform `Source` instead of `RawFd`, with `PollLoop` wrapping fds. Verify: `cargo test -p brindle-remote-agent` passes on Linux, including `event_loop`'s tests.
- [x] 1.2 Put `conn` (framed connection over a stream, with `input_waiting`), `listen` (accept plus peer check), `lock` (single-instance locks), `daemon` (detached start with a ready signal) and `pty` (spawn, resize, hang up, child exit) behind per-platform modules. The core's and the holder's logic must not name a Unix type. Verify: `grep -n 'std::os::unix\|libc::' remote-agent/src/server/core*.rs remote-agent/src/server/core/` finds nothing, and the Linux agent tests pass.
- [x] 1.3 Compile `server` on `any(target_os = "linux", windows)` in `main.rs`, with Windows stubs that return "not implemented yet". Verify: `cargo build --workspace --locked` succeeds on Windows and Linux, with no warnings from Brindle's crates.

## 2. Directory and names

- [x] 2.1 Extend `remote-protocol`'s `server_dir::resolve` for Windows: `BRINDLE_SERVER_DIR`, else `%LOCALAPPDATA%\brindle\run`, with `run-elevated` for an elevated token. Add pure helpers for the core and holder pipe names (FNV-1a of the canonical directory plus the user SID) and the per-build copy path. Verify: unit tests for each, runnable on Linux with injected inputs, as `cmdline::program_stem` is.
- [x] 2.2 Create the directory with a protected DACL for the user and SYSTEM on Windows. Verify: a test reads it back with `GetSecurityInfo` and finds exactly those two entries.

## 3. Windows I/O: IOCP loop, pipes, access

- [x] 3.1 Implement the IOCP `EventLoop` with the readiness facade from design.md: owned read and write buffers per pipe source, a pending `ConnectNamedPipe` for listeners, a `PostQueuedCompletionStatus` waker, process-exit waits that post packets, and cancel-and-drain on deregister and drop. Add the `windows` 0.61 dependency to the agent for Windows only. Verify: `PollLoop`'s tests (tokens, pausing, writable, waker from a thread) pass against it, plus tests for EOF, closing with a read in flight, and a process exit; `git diff Cargo.lock` shows only the agent's own dependency list changing.
- [x] 3.2 Implement the Windows `conn` and `listen`: overlapped named pipe server instances with the user-and-SYSTEM DACL, the integrity mandatory label, `PIPE_REJECT_REMOTE_CLIENTS` and `FILE_FLAG_FIRST_PIPE_INSTANCE`; peer check by `GetNamedPipeClientProcessId` and the client's token (user and integrity, refusing a token that can't be opened); `input_waiting` through the buffer plus `PeekNamedPipe`; the graceful close (send the queue, then wait for the peer, up to 10 s). Verify: unit tests for a same-user client accepted, a mismatched SID and a lower integrity refused (the comparison as a pure function), a name taken by another instance failing the create, and a flood read in order.
- [x] 3.3 Implement the client-side server check: `GetNamedPipeServerProcessId`, then the server's token user and integrity, refusing a mismatch before writing. Verify: a unit test of the comparison, and a test that a pipe created without the first-instance owner check is refused.

## 4. Locks, starting roles, the per-build copy

- [x] 4.1 Implement the Windows `lock` with `LockFileEx` on `core.lock` and `holder-<build>-<n>.lock` (which also picks `n`). Verify: a test takes a lock in a child process, fails to take it in the test, and takes it once the child is killed.
- [x] 4.2 Implement the Windows `daemon` start: the `launch` trampoline that starts the role and exits, `--ready-handle` passed through it, the detached flags with `CREATE_BREAKAWAY_FROM_JOB`, the handle list, the retry without breakaway on `ERROR_ACCESS_DENIED` with one log line, and a 2 s wait for the ready byte. Verify: tests start a role inside a job without breakaway (it starts, and logs), inside a job with breakaway (it survives the job's `KILL_ON_JOB_CLOSE`), and check that `taskkill /T /F` on the starter leaves the role running.
- [x] 4.3 Implement the per-build copy: copy to a temporary name, rename into `<copies>\<build>\brindle-remote.exe` (`%LOCALAPPDATA%\brindle\server`, or `bin` in `BRINDLE_SERVER_DIR` when set), accept a racing winner whose build id matches, and prune other build directories that can be deleted. Roles restart roles from `GetModuleFileNameW`. Verify: tests for two racing copies, the kept last-write time, and pruning skipping a directory whose exe is running.

## 5. The holder on Windows

- [x] 5.1 Move the pseudoconsole code into the agent: `conpty.rs`'s `Api` resolution, spawn and `Pseudoconsole`, `cmdline`'s `command_line`, `command_line_raw` and `launch_line` with their tests, and the ConPTY embedding from Brindle's `build.rs` to `remote-agent/build.rs`. Verify: the moved unit tests pass in the agent; `brindle-remote.exe` embeds the pinned `conpty.dll` and `OpenConsole.exe` after `scripts/fetch-conpty.ps1`.
- [x] 5.2 Implement the Windows `pty` for the holder: an overlapped named pipe pair per session, spawn with the session's complete environment, `PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH` and the host chosen from `BRINDLE_CONPTY` in that environment, resize, the exit wait, then release and close on a short-lived thread while output drains to EOF, and `EXITED` after the last output. Verify: holder tests for spawn and I/O, resize, the exit code, and a flood followed by exit losing no output.
- [x] 5.3 Implement hangup: close the pseudoconsole, then `TerminateProcess` the root after `agent.rs`'s grace. Verify: a holder test where `HANGUP` ends `cmd /c ping -t localhost` and its console children within the grace, and one where the holder's loop keeps serving another session while a close blocks.
- [x] 5.4 Apply `REPARENT` in the Windows holder with `ConptyReparentPseudoConsole`, logging and ignoring it on the inbox host. Verify: a holder test that reparents a session's console window to a test window and reads the console window's owner after the settle time.
- [x] 5.5 Port `remote-agent/tests/holder.rs` to Windows: kill with `TerminateProcess`, check liveness with `GetExitCodeProcess`, and check parents with Toolhelp. Verify: `cargo test -p brindle-remote-agent --test holder` passes on Windows and on Linux.

## 6. The core and the management commands on Windows

- [x] 6.1 Run the core on Windows: the core pipe listener, holder discovery through lock files, the holder connections, supervision restarting a core from the holder's own image, and the unused exit. Forward `REPARENT` to the holder on Windows; Linux keeps refusing it. Verify: `cargo test -p brindle-remote-agent --test core` passes on Windows (roles, start race, peer check, spawn, policies, reconnect, replay, checkpoints, core killed and restarted by its holder) and on Linux.
- [x] 6.2 Make the upgrade path work on Windows: `check`, `STEP_DOWN`, adoption and drain, with the core's exe time read through `QueryFullProcessImageNameW`. Verify: `cargo test -p brindle-remote-agent --test upgrade` passes on Windows, including the upgrade to the same binary and a new core that won't start.
- [x] 6.3 Run `list`, `kill`, `status` and `stop` on Windows. Verify: `cargo test -p brindle-remote-agent --test manage` passes on Windows, and `brindle-remote status --json` with no server prints `{"running":false}`.
- [x] 6.4 Update `remote-agent`'s docs and the server parts of CLAUDE.md's architecture section to cover Windows: the directory, pipe names, access, detached start, the per-build copy and ConPTY in the holder. Verify: the text names each Windows mechanism in design.md.

## 7. Brindle's client on Windows

- [x] 7.1 Add the Windows connection: open the core's pipe with `FILE_FLAG_OVERLAPPED` as a `Read + Write` type, with the server check from 3.3, so `connection.rs`'s reader and writer threads run unchanged. Verify: a unit test where a reader blocked on an empty pipe doesn't delay a write from another thread.
- [x] 7.2 Add `BRINDLE_SERVER_HELPER` to the helper lookup (on Windows it is first, then `brindle-remote.exe` beside `brindle.exe`; on Linux it is first, then `BRINDLE_REMOTE_AGENT`, then beside), update `HELPER_MISSING`, and start the core from the per-build copy. Verify: unit tests of the lookup order on both platforms, and `remote::find_helper` still resolving the Linux helper from `BRINDLE_REMOTE_AGENT` for remote profiles on Windows.
- [x] 7.3 Build `Backend::Server` and `server::Sink` on Windows. Always resolve the program on Windows (`windows_default_program`) and send the merged environment. Take process info from the session's pid through the Toolhelp snapshot. Verify: `cargo test -p brindle` passes on Windows, and a debug run with `--dump-screen-after` shows a PowerShell prompt in a tab whose program's parent is a holder.
- [x] 7.4 Send `REPARENT` where `parent_console_to` ran, after spawn, after attach, and after each reconnect's replay. Verify: the e2e case in 8.3 checks a message box's owner.
- [x] 7.5 Remove `Backend::Pty`, `pty::MarkedPty`, the ConPTY reader in `terminal/pty.rs`, `terminal/scan.rs`'s Windows path, `conpty.rs` and the ConPTY embedding in Brindle's `build.rs`. Verify: `cargo build --release --locked` on Windows succeeds with no warnings, and `grep -rn 'Backend::Pty\|conpty::' src/` finds nothing.
- [x] 7.6 Make the e2e harness give each Windows case a `BRINDLE_SERVER_DIR` under `target/tmp/e2e/<case>/server`, require the helper beside the binary (and not older than its sources), run `brindle-remote stop` afterwards, copy `core.log` and fail the case if a role is still running. Verify: `BRINDLE_E2E=1 cargo test --test e2e -- --self-test` passes, and every existing Windows case passes through the server, with the host case now checking that `OpenConsole.exe`'s parent is a holder.

## 8. Phase 2 on Windows

- [x] 8.1 Drop the Linux-only gates on spawn policies (`Closing::Keeps` → `detach`, `Ends` → `kill`), zone session records, launch reattach (`reattach.rs`), `--attach` and the "Attach Session…" palette. Verify: their unit tests run on Windows, and `--list-actions` lists `attach_session` on Windows.
- [x] 8.2 Update README (Windows tabs persist, quitting keeps programs, `BRINDLE_SERVER_HELPER`, `brindle-remote` commands on Windows) and `scripts/run-throwaway.ps1` (a private `BRINDLE_SERVER_DIR`, and `brindle-remote stop` on exit unless `-Keep`). Verify: README's Windows section and the script's header match the behaviour; the script stops its server when the window closes.
- [x] 8.3 Add the Windows `local-server-*` e2e cases: quit keeps and relaunch restores (same pid, scrollback), `taskkill /F` keeps, `-e` ends, the core killed and reconnect, missing helper and recovery, a message box owned by Brindle's window, the inbox host for one session beside a shipped-host session, and the elevated server separate from the unelevated one (skipped when the runner can't run both). Verify: `BRINDLE_E2E=1 cargo test --test e2e -- local-server` passes on Windows, and `--list` shows the new cases.

## 9. Packaging, CI and checks

- [x] 9.1 Ship `brindle-remote.exe` in the Windows zip: `scripts/package-windows.ps1` copies it and checks that it, not `brindle.exe`, embeds the pinned ConPTY; `build-windows.yml` checks the helper's version resource and that it runs `--version`; the WinGet manifest installs it beside `brindle.exe`. Verify: the `windows` CI job builds the zip, `winget validate` passes, and the unpacked zip opens a tab in the server.
- [x] 9.2 Check an upgrade while sessions run: install the zip build with a running detached session, replace it with a newer local build (or run WinGet's upgrade from a local manifest), and relaunch. Verify: the replacement succeeds, the session reattaches through the new core, and the old holder drains (`brindle-remote status`).
- [x] 9.3 Compare `--latency-bench` and a flood's responsiveness on Windows between `main` and the branch. Verify: the numbers are recorded in the PR description; a noticeable regression gets a follow-up issue before merge.
- [x] 9.4 Run the Linux checks on the Linux machine through Remote Control: `cargo test --locked`, `cargo build --release --locked`, and `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e`. Verify: all pass, with the seams from group 1 changing no Linux behaviour.
- [ ] 9.5 Hand the user the manual checks from CLAUDE.md that touch Windows tabs (wheel scrolling, right-click menus, IME in a terminal), plus a Start Menu launch, a quit and a relaunch, and a dialog from a program in a tab, through `scripts/run-throwaway.ps1`. Verify: the user confirms each one.

## Workflow follow-up

- Open the PR as `feat!: run Windows tabs in the terminal server`, since quitting now keeps Windows programs running.
- Archive the change with `openspec archive terminal-server-windows` after merge, and check that `terminal-emulation`'s main spec no longer says quitting ends every program.
