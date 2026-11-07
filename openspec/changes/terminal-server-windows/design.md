# Design

**Proposal-stage notes.** The evidence is phase 1's Windows spike: `spike-windows.md` in `openspec/changes/archive/2026-11-07-terminal-server/` once PR #22 merges. The spike code is on the branch `spike/windows-conpty`.

## What the spike established

- **Reparent works across processes:** the dialog owner follows the latest live HWND, and the call is asynchronous (it settles within about 800 ms). A destroyed HWND returns `S_OK` and is ignored, and a null HWND clears the owner.
- **An HPCON can't be rebuilt in another process.** Pipes, process handles and exit codes can move, but resize, reparent and close can't. The inbox ConPTY has no `Pack` or `Reparent` at all. So the holder keeps every HPCON for its whole life.
- **A detached helper outlives its parent** when there's no job, and under a job that allows breakaway. A job without breakaway makes `CreateProcess` fail with access denied, and a `KILL_ON_JOB_CLOSE` job kills it.
- **The pipes and the loop work:** a user-only DACL with a peer pid and SID check works, and one IOCP loop served two pseudoconsoles over overlapped named pipes. Cancel-and-drain quiesces it cleanly.

## Not yet covered (check early)

- Destroying the **current** owner window while the session lives.
- An owner HWND in a different process from the holder.
- A real Start Menu launch, and a real kill of Brindle, under Windows Terminal or CI jobs.
- Refusing a second user account.

## Phase 1 seams to use

- The **event-loop trait** in `remote-agent/src/server/event_loop.rs` takes Unix `RawFd`s. It needs a source type that IOCP can implement.
- **The holder's PTY code** (`remote-agent/src/pty.rs`) gains a ConPTY side. It reuses `conpty.rs`'s argument quoting, environment merging (case-insensitive) and `PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH`.
- **`build_id::of_binary`** reads the ELF build-id note on Linux. Windows needs a PE equivalent, such as the debug directory's GUID and age, or the hash fallback.
- **Process info:** Brindle reads `/proc` locally on Linux. On Windows it keeps the Toolhelp snapshot by pid, and the directory comes from OSC 7 and 9;9 reports, as today.

## Open questions

- Where the server's pipe names live, and whether `BRINDLE_SERVER_DIR` keeps its meaning (as a lock and log directory plus a pipe-name prefix).
- Whether the shipped `conpty.dll` and `OpenConsole.exe` stay beside `brindle.exe` or move beside `brindle-remote.exe`.
- Remote Windows hosts over SSH belong to phase 4.
