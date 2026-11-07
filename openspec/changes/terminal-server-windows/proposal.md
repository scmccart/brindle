# Proposal

**Status: proposal stage.** This is phase 3 of the terminal-server series. It needs phase 1 (`terminal-server`, PR #22) merged first. It has no specs or tasks yet.

## Why

Phase 1 moved local Linux tabs into the terminal server, but Windows tabs still run on Brindle's in-process ConPTY. So Windows gets neither the persistence from phase 2 nor the core-restart and upgrade survival. The series' goal is the same model on every platform.

## What Changes

- **Local Windows tabs always run in the server,** with no fallback. `Backend::Pty` is removed entirely, and the ConPTY code (`conpty.rs`, `cmdline.rs`, the packaged `conpty.dll`) moves into the holder.
- **The holder owns the pseudoconsoles.** It never hands them over, which is why the core/holder split exists (phase 1's Windows spike).
- **Named pipes replace Unix sockets:**
  - a user-and-SYSTEM DACL, `PIPE_REJECT_REMOTE_CLIENTS`, and a SID check on the client pid;
  - the single-instance lock is `LockFileEx`.
- **Both loops run on IOCP,** behind phase 1's event-loop trait, over overlapped named pipes that the holder creates for ConPTY's output.
- **Started detached:** the server starts with `DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB`. When a job refuses breakaway, it retries without the flag and logs that sessions won't outlive that job.
- **`REPARENT { session, hwnd }`** (reserved in protocol 1) reparents a session's console window to Brindle's window. It is asynchronous and advisory, and the client sends it again whenever its window changes.
- **Upgrades and core crashes** work as on Linux, since nothing ever moves an HPCON. The built-in ConPTY works the same way.

## Capabilities

### Modified Capabilities
- `terminal-server`: Windows transport, access and lifetime.
- `terminal-emulation`: the Windows requirements (shipped host, passthrough, glyph widths, closing never blocks) now hold through the server.
- `command-line`: `BRINDLE_SERVER_DIR` on Windows, or the pipe-name equivalent.

## Impact

- `remote-agent/`: Windows `hold`/`serve`, IOCP loop, named pipes, ConPTY. The crate becomes cross-platform.
- `src/terminal/`: `conpty.rs` and the Windows `Backend::Pty` move out. `parent_console_to` becomes `REPARENT`.
- Packaging: `brindle-remote.exe` in the release zip, the WinGet manifest, and the ConPTY files beside it.
- Tests: the Windows e2e cases run through the server, plus new Windows `local-server-*` cases.
