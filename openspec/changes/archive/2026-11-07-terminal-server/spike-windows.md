# Windows spike findings (tasks group 0)

Code: `spike/` on branch `spike/windows-conpty` (throwaway, own workspace, not part of the product). Run on Windows 11 Pro 10.0.26300, shipped ConPTY 1.25.260930003, `x86_64-pc-windows-msvc`. Subcommands: `exports`, `handover`, `reparent`, `survive <mode>`, `pipe`.

## 0.1 Reparenting

**Ran:** `spike reparent`. Two top-level windows W1 and W2 stand in for Brindle windows. A pseudoconsole is created with the shipped `conpty.dll`, running a child (`spike dialog`) that calls `GetConsoleWindow()` and opens a `MessageBoxW` owned by it. `ConptyReparentPseudoConsole` is called on the live session and the dialog's owner chain is read (dialog → console window → owner) after an 800 ms settle.

**Found:**
- Reparent works: dialog → console window (in OpenConsole.exe) → W1, and then W2 after reparenting. The console window's `GWLP_HWNDPARENT` equals the target.
- It is asynchronous. Read immediately, the chain lagged a step behind (first run, no delay); after 800 ms it was exact. Callers must not assume it has applied on return.
- Reparenting to a destroyed HWND returns `S_OK` and changes nothing (owner stayed W2). Reparenting back to a live window works, and a null HWND clears the owner (`0x0`).
- Same-session reparent between two windows works with no restart.

**Not covered:** (a) destroying the *current* owner while the session lives (I destroyed W1 only after moving the session to W2); (b) the helper and the HWND owner in different processes. OpenConsole already runs in its own process and sets the owner itself, so I expect this to work, but I did not test it. Both are cheap follow-ups.

## 0.2 Handover of a live pseudoconsole

**Ran:** `spike exports` and `spike handover`.

**Found:**
- Shipped exports: Create, CreateAsUser, Resize, Clear, ShowHide, Reparent, Release, Close, **Pack**. Inbox kernel32 has Create, Resize, Close and Release only; **no Reparent, no Pack, no Clear**. No export exists that extracts handles from an HPCON (probed three plausible names).
- Second process takes over via `DuplicateHandle` (PROCESS_DUP_HANDLE on the first): the input pipe, output pipe and child process handle all moved. After the first process stopped reading, the second read the program's output and `GetExitCodeProcess` returned the real code (7).
- The HPCON itself could not be rebuilt using exports alone. `ConptyPackPseudoConsole(hServerProcess, hRef, hSignal, &hpc)` builds an HPCON from three handles, but `ConptyCreatePseudoConsole` never returns them and nothing exports them. Per the decision on this spike, I did not reconstruct the internal struct layout, so **resize, reparent and close after a handover were not tested and are unavailable by supported means**.
- The inbox ConPTY cannot be handed over at all (no Pack).

**Consequence:** I/O and exit status can move between processes, the control surface (resize, reparent, close) cannot. A process that owns an HPCON has to keep owning it.

## 0.3 Outliving the client

**Ran:** `spike survive <mode>`: the parent starts the helper with `DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB`, which runs a program on a pseudoconsole, then the parent exits. Helper and program pids are checked 2 s later.

| mode | result |
|---|---|
| no job | spawn OK; helper and program alive after parent exit |
| job with `BREAKAWAY_OK` | spawn OK; both alive |
| job without `BREAKAWAY_OK` | `CreateProcess` fails, **Access is denied (0x80070005)** |
| job `KILL_ON_JOB_CLOSE`, spawned without breakaway | spawn OK; both **killed** when the job closed |

**Not covered:** a real Start Menu launch and a real kill of a Brindle process (the parent here exits normally; a kill is equivalent for job semantics, but I did not run it from the Start Menu or under Windows Terminal/CI). The breakaway flag is only honored when the job allows it, so a no-breakaway job needs a fallback: retry without `CREATE_BREAKAWAY_FROM_JOB` (survives unless the job kills on close) or refuse to start a persistent server and say why.

## 0.4 Transport and loop

**Ran:** `spike pipe`.

**Found:**
- A control pipe created with SDDL `D:P(A;;GA;;;<user SID>)(A;;GA;;;SY)` and `PIPE_REJECT_REMOTE_CLIENTS` accepts a same-user client. `GetNamedPipeClientProcessId` plus a token-SID `EqualSid` accepts it and refuses a wrong SID.
- A second *user* was not available on this machine, so denial by another account was not exercised; the DACL and SID compare were each exercised same-user only.
- One IOCP served two pseudoconsoles at once. The output pipes have to be overlapped named pipes that Brindle (or the server) creates and gives to ConPTY, instead of today's anonymous `CreatePipe`; both outputs arrived through one `GetQueuedCompletionStatus` loop.
- Quiescing works: cancel outstanding reads with `CancelIoEx` and drain completions, which leaves a clean stop with no thread holding session state. (In this run no reads were in flight at that point because both sessions had finished; the cancel-and-drain path itself ran, but with nothing to cancel.)

## 0.5 Decisions in design.md

- **Handover by re-exec.** Linux keeps it. The Windows mechanism cannot be shared: an HPCON cannot move, so a Windows "new process takes the handles" design loses resize, reparent and close. Do not move Linux to `SCM_RIGHTS`.
- **Handover format.** Needs platform-specific state, and on Windows effectively cannot carry sessions at all: only I/O pipes, process handles and replay data can move.
- **The single loop.** Holds in shape (one loop, quiescent point), but on Windows it is IOCP over overlapped pipes, not `poll`. Make the loop a platform abstraction (readiness on Linux, completions on Windows), as the design anticipated.
- **Protocol 1 message set.** Add `REPARENT { session, hwnd }` from the start; reparent works and is asynchronous, and the server owns the HPCON so only it can call it. Specify that it is advisory and may take a moment, and that a destroyed HWND is silently ignored, so the client must re-send on window changes.
- **Windows upgrade policy.** Re-exec is unavailable, so Windows needs the kill-with-warning or side-by-side option. Side-by-side (new server for new sessions, old server kept until its sessions end) is the one that preserves work; it breaks one-server-per-host, so decide that explicitly for Windows.
- **Persistence.** Spawn the server with breakaway and fall back without it; a no-breakaway job refuses breakaway, and a kill-on-close job ends everything.
