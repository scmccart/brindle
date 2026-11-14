# Proposal

Phase 3 of the terminal-server series. Phase 1 (`terminal-server`) and phase 2 (`terminal-server-on-close`) are archived; this builds on both.

## Why

Local Linux tabs run in the terminal server: they survive a Brindle quit or crash, a core crash and an upgrade, and come back on the next launch. Windows tabs still run on Brindle's in-process ConPTY, so on Windows a quit, crash or upgrade still ends every local program. The series' goal is one model on every platform, and phase 1's Windows spike showed it can be built: a holder that keeps every pseudoconsole for its whole life.

## What Changes

- **Local Windows tabs always run in the server,** with no fallback. `Backend::Pty` and alacritty's Windows event loop go away in Brindle; the ConPTY code (`conpty.rs`, the launch-line quoting from `cmdline.rs`, the embedded `conpty.dll` and `OpenConsole.exe`) moves into the holder.
- **BREAKING (Windows): quitting keeps programs running.** Phase 2's rule now holds on Windows too: closing a zone or tab ends its program, while quitting, closing the window or a crash leaves it running detached, and the next launch reattaches it. `-e` command tabs still end.
- **Restore, `--attach` and "Attach Session…" work on Windows,** as they do on Linux.
- **The holder owns the pseudoconsoles** and never hands them over. Upgrades and core crashes work as on Linux, with the shipped and the built-in ConPTY alike.
- **Named pipes replace Unix sockets:** a user-and-SYSTEM DACL with the server's integrity level as its mandatory label, `PIPE_REJECT_REMOTE_CLIENTS`, a first-instance check against squatting, and a user and integrity check on every peer, both ways. An elevated Brindle gets its own server.
- **Both loops run on IOCP** behind phase 1's event-loop seam, over overlapped named pipes, including the pipes the holder gives ConPTY.
- **Started detached and outside the starter's job:** server roles start with no console window, breaking away from the job when it allows that. When it doesn't, they start inside it and log that sessions won't outlive the job.
- **The server runs from a per-build copy** under `%LOCALAPPDATA%\brindle\server\`, so a running server never locks the installed `brindle-remote.exe` against a WinGet upgrade.
- **`REPARENT { session, hwnd }`** (reserved in protocol 1 and H1 since phase 1, refused on Linux) makes Brindle's window own each session's console window. It is asynchronous and advisory, and Brindle sends it again whenever a session's window changes.
- **`BRINDLE_SERVER_HELPER`** names the local server's helper on every platform. On Windows, `BRINDLE_REMOTE_AGENT` keeps naming the Linux helper that remote profiles deploy.
- **The Windows zip ships `brindle-remote.exe`** beside `brindle.exe`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `terminal-server`: Windows transport, directory, access (user and integrity level), detached start and job breakaway, the per-build copy, and `REPARENT`. The Linux-only scenarios gain Windows forms.
- `terminal-emulation`: local terminals run in the server on both platforms (and the stale "quitting ends every program" text from phase 1 is corrected to phase 2's rule). The Windows requirements (shipped host, passthrough, glyph widths, closing never blocks) now hold through the server.
- `tabs`: quitting keeps programs running on Windows too.
- `zones`: closing a Windows zone or tab ends its program in the server.
- `session-restore`: zone session records and reattach on Windows.
- `command-palette`: "Attach Session…" on Windows.
- `command-line`: `--attach` and `BRINDLE_SERVER_DIR` on Windows, and `BRINDLE_SERVER_HELPER`.
- `desktop-integration`: the Windows zip holds `brindle-remote.exe`, and a WinGet upgrade works while sessions run.

## Impact

- `remote-agent/`: becomes cross-platform. Windows `hold`/`serve`, the IOCP loop, named-pipe listeners and connections, the peer check, the detached start, ConPTY in the holder, and a `build.rs` that embeds the shipped ConPTY. It takes the `windows` crate (0.61, already in `Cargo.lock`).
- `remote-protocol/`: Windows paths and pipe names in `server_dir`. No protocol version changes: `REPARENT` is already in protocol 1 and H1.
- `src/terminal/`: `Backend::Server` on Windows. `conpty.rs` and `pty.rs`'s Windows reader leave Brindle; `parent_console_to` becomes `REPARENT`. Brindle's `build.rs` stops embedding ConPTY.
- `src/remote/local*`: the named-pipe client, helper lookup, the per-build copy, and the upgrade's exe-time lookup on Windows.
- `src/reattach.rs`, `session.rs`, the sessions palette: no longer Linux-only.
- Packaging: `scripts/package-windows.ps1`, `build-windows.yml`, the WinGet manifest, `scripts/run-throwaway.ps1`.
- Tests: `remote-agent/tests/*` on Windows, the Windows e2e cases through the server, and new Windows `local-server-*` cases.
