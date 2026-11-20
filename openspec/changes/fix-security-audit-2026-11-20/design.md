# Design

## Context

The findings, their paths and the audit's suggested fixes are in `docs/security/audits/2026-11-20.md`. Behaviour is in this change's specs. The five fixes are independent of each other: they share no types and no wire messages, and none needs a protocol, config or state-file change. They are one change because they come from one audit and land together.

## Goals / Non-Goals

**Goals:**
- Each fix closes its finding where the untrusted value enters (`fcis` R1, `types` R12: parse at the boundary), not at one of its sinks.
- Each refusal is a value, shown on the tab or printed by the launch it belongs to (`railway` R1, R2).

**Non-Goals:**
- **The audit's low findings.** These are the clipboard writes, the Linux client's peer check (except where the instance socket check below covers it), the config file mode, and the rest.
- **Changing unelevated behaviour on Windows**, beyond the bare-name lookup and the instance pipe's check.

## Decisions

### 1. Reported directories: drive letters only, checked off the UI thread

- **The parser change.** `brindle-term-core`'s `reported_dir` (pure, `shell_integration.rs`) treats a Windows-style path as absolute only when it is `X:\` or `X:/`. The `|| path.starts_with("\\\\")` arm goes. The OSC 7 branch returns `None` unless the decoded path is `/X:/…`, so `file://x//host/share` is refused there, before any backslash conversion.
  - **One rule, one function.** The rule is `shell_integration::is_absolute_path(path, style)`. In the Windows style a path passes only when it is on a drive letter and what follows the drive has no `:`, no `"` and no control character. That also rules out a stream of a file and anything a later reader would take for more than a name. The parser, the restore check and `links::plan` all call it, so they can't drift.
  - Because the parser is in `term-core`, the core's emulator applies the same rule. A UNC directory then never reaches the core's replay either, which keeps client and core tracking identical.
  - Alternative: filter at `Terminal::working_directory`. Rejected, because every other reader of `Marks::reported_dir` (the record, `new_tab_cwd`, splits, the core) would still need the same check.
- **The off-thread check.** The existence check moves into a GPUI-free model in `src/terminal/` (`DirCheck`, pure, with unit tests). It records the reported path last checked, and the outcome as a closed enum `Checked::{Exists(PathBuf), Missing, Pending}`. When the reported directory changes, the model says to check it. The `Terminal` entity runs `is_dir` on the background executor and gives the result back to the model.
  - `working_directory()` only reads the model: an existing directory, or `None` while pending or missing. So `record_session` and `new_tab_cwd` do no I/O on the UI thread (`process` R13).
  - The model starts at most one check per terminal at a time. A report that arrives during a check is checked after it, and only the latest is kept. A directory reported again is not checked again.
  - Where the directory comes from is the seam's to say: `process::LOCAL_DIRECTORY` is `Reported` on Windows and `Process` elsewhere, so the terminal's code has no `cfg` for it and builds on every platform.
  - Alternative: a timeout around `is_dir` on the UI thread. Rejected, because any wait there is a freeze.
- **The check when a tab or split starts.** `Terminal::spawn` probed the directory a new tab or split starts in on the UI thread, through `spawn_request`'s `is_dir`. A directory that passed its check and whose drive then stopped answering would still freeze the window there. The probe moves to the thread that already starts the session:
  - `spawn_request` (pure) names the directory without checking it.
  - `Session::start` takes the request as a closure and calls it on its own thread, once connected. For a session on this machine the closure probes the directory and passes the result to `with_checked_dir` (pure), which leaves out a directory that isn't one. The outcome is the same as before: the program starts in the server's default directory.
  - A remote host's directory is a path on that host and is never probed here, as before.
  - Nothing changes on the wire or in the order of messages.
- **Recorded directories.** A state file written before this change can still hold a UNC path that a program reported. `session::resolve_tab` therefore keeps a directory recorded for this machine only when it is absolute in this machine's path style, before it probes it. The rule is the parser's own, `shell_integration::is_absolute_path`.

### 2. Elevated files on Windows: protected directories, held open

- **The directories.** An elevated process uses three directories, beside the unelevated ones:
  - `run-elevated` for the server directory, as today;
  - `server-elevated` for its helper copies;
  - `conpty-elevated\<version>` for the ConPTY files.

  The names come from pure functions in `remote-protocol/src/server_dir.rs` and `remote-agent/src/conpty/choice.rs`, which take `elevated: bool` as `choose_windows` already does.
- **Making and checking a directory.** A new seam function, `win::protected_dir(path, Contents)`, makes or checks each directory and returns a held `ProtectedDir` handle.
  - It applies the security descriptor `O:BAD:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)S:(ML;OICI;NW;;;HI)` (from a pure builder in `access.rs`, beside `dir_sddl`). Owner Administrators, no ACE for the user's SID, and a High no-write-up label, so a Medium process of the same user can neither write nor read in it.
  - It opens the directory with `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT` and no `FILE_SHARE_DELETE`, and refuses a reparse point.
  - It compares the directory's owner, DACL and label with the descriptor, through a pure `access::matches_elevated`.
- **A directory that doesn't match.** What happens depends on its `Contents`:
  - `Contents::Executables` (the copies and the ConPTY files): the directory is renamed aside (`<name>.untrusted-<pid>-<n>`) and made fresh, since nothing in it can be trusted.
  - `Contents::State` (`run-elevated`: locks and logs, nothing that is run): it is re-protected in place, and any reparse point inside it is deleted. Renaming it would orphan the sessions of an elevated core started by an older build.
- **Holding the directory.** The handle is held while the process uses the directory: for a role, its whole life; for Brindle's client, while it ensures and launches a copy. Windows refuses to rename or delete a directory while a handle to it, or to anything under it, is open without delete sharing. So once the check passes, neither the directory nor its parents can be swapped before use, which closes the check-then-run gap at `win.rs:475/482`, `daemon.rs:260` and `conpty/api.rs:117`.
- **The prune.** `prune_helper_copies` walks the held directory's children and skips any reparse point, never descending into one.
- **Unelevated processes** keep today's directories and DACLs. The same user owns them, so there is no boundary to protect.
- **Alternatives considered:**
  - `%ProgramData%`. Rejected: users can create subdirectories there, so it has the same squatting problem plus a machine-wide name.
  - Verifying the copy with SHA-256 before each launch. Rejected as the only defence: it still leaves a gap between check and launch unless the file is also held, and holding the directory covers every file at once.

### 3. Bare program names: an explicit PATH search

`CreateProcessW` with a bare name searches the calling image's directory first, which is the helper copy's directory. The holder now resolves a bare name itself:
- **The pure part.** `conpty::cmdline::resolve_program(name, path_var, pathext, exists: impl Fn(&str) -> bool) -> Resolved` returns an enum, `Resolved::{Found(String), NotFound}`. It walks `PATH` (and `PATHEXT` when the name has no extension). It is tested on every platform, with `str` paths as the Windows pitfall note asks.
- **The seam.** `pty.rs` passes its absolute result as the application name.
- **When nothing is found,** the session fails with `SPAWN_FAILED`, naming the program, as an unknown program does today.

### 4. Links: one decision function

`links::plan(uri, this_host: &str, windows: bool) -> Plan` is pure, with a unit test per scenario. It returns a closed enum:
- `Plan::Open(String)`: the link with `"` and whitespace percent-encoded;
- `Plan::Reveal(String)`: a Windows `file:` path on a drive letter, for Explorer's `/select,` unless the shell finds it is a directory, in which case it opens it;
- `Plan::Refuse(Refusal)`, where `Refusal` is `NoScheme`, `Scheme(String)`, `OtherHost(String)` or `NoDrive`.

How it fits into the rest of the code:
- **One scheme list.** The allowed schemes are the same list `find_url` detects (one `const`, so detection and opening can't drift).
- **The host check.** `this_host` comes from the shell (`gethostname`, or `COMPUTERNAME`), read once.
- **Only a plain drive-letter path reaches Explorer.** A `file:` link can carry anything where its path goes: `file:ms-settings:privacy`, `file:shell:AppsFolder\X!App`, a path with no drive. Explorer opens links and `shell:` items it is given, so handing it such a "path" could bring back a handler the scheme list refuses. And the path goes inside one pair of quotes on Explorer's command line, which a double quote in it would end.
  - **The check.** On Windows `plan` takes the path as Windows will read it: without query or fragment, percent-decoded, and with every `/` a `\`. That form must pass `shell_integration::is_absolute_path`, the rule a reported directory follows: `X:\`, then no `:`, no `"` and no control character. Nothing is checked in an earlier form, so an encoded backslash or quote is seen as what it becomes.
  - **A network path** (`\\host\…`, however its separators were written) is `OtherHost`, naming the host.
  - **Any other** is `NoDrive`: a device or extended path (`\\?\…`, `\\.\…`), a path from the root of the object namespace (`\??\…`, `\Device\…`, `\GLOBALROOT\…`), a path with no drive or none from its root (`C:x`), a stream of a file (`C:\a.txt:s`), and a path with a double quote or a control character.
- **Refusals in the tab.** `links::failure_text` words each refusal for the tab's dim note, as `NoScheme` is worded today.
- **What `open` does.** It applies the plan. Only `Reveal` is new: it runs `explorer.exe /select,<path>` on the opener thread, whatever `BRINDLE_LINK_OPENER` says. A directory is opened with the shell's `explore` verb, which takes nothing but a folder.
- **Platform code.** The `cfg(windows)` stays in `links.rs`'s existing opener functions (`ports` R4).

### 5. Session sizes: one pair of limits

- **The limits.** `brindle-term-core::emulator` gains `MAX_COLS` (2048) and `MAX_ROWS` (1024) beside `MIN_COLS`.
- **Where they apply:**
  - `emulator::size`, `GridSize::clamped` and the client's resize and replay resize clamp to both ends.
  - `ListedSession::of` (`src/remote/identity.rs`) turns a size outside the limits into `None`, the "no size" an old core lists. It doesn't clamp, since a clamped huge size would still misplace the replay. That treats the listing as parsed at the boundary (`types` R12).
  - The core's `Emulator::new` clamps through `emulator::size`, so a `SPAWN` or `RESIZE` past the limits can't make the core allocate either.
- **Why these numbers.** At 2048×1024 a grid is at most 2 M cells per screen, well under 200 MB with both screens. 2048 columns is wider than a 7680-pixel display at a 4-pixel cell.

### 6. Instance forwarding: identify, then check

- **On Windows,** `instance::forward` opens the pipe with `security_qos_flags(SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION)`. Before `exchange`, it calls `brindle_remote_protocol::win::check_server`, which requires the same SID and the same integrity. A refusal is returned as the launch's error string, which `main` prints before exiting 1.
- **The pipe name stays the same,** `<name>-<USERNAME>`. Changing it would stop a launch of the new build from finding an older running instance. Squatting the name is now a denial of service only.
- **On Linux,** `forward` reads `SO_PEERCRED` on the connected socket and refuses a different uid. That covers the `/tmp/brindle/run` fallback's spoofed socket too.
- **Platform code.** Both checks are in `instance.rs`'s existing `cfg` modules, which are already on the `ports` R4 list.

## Risks / Trade-offs

- **Held directory handles might not lock renames everywhere.** The "can't rename a directory under an open handle" rule is NTFS behaviour; other file systems may not enforce it.
  - Mitigation: after opening, `protected_dir` also compares `GetFinalPathNameByHandleW` with the expected path. A Windows real-process test in `remote-agent/tests/launch.rs` tries to rename the held directory and its parent from an unelevated token, and expects a failure. Where elevation isn't available, the test skips through `skip(Capability, …)`.
- **Directories renamed aside pile up** in `%LOCALAPPDATA%\brindle` until removed.
  - Mitigation: the core's start removes `*.untrusted-*` siblings it can delete, skipping reparse points, as `prune_copies` does.
- **Users lose links.** Ctrl-click no longer opens a scheme outside the detected list, such as `vscode://`, and Windows `file:` links reveal instead of run.
  - Mitigation: the tab says why, naming the scheme. A config key for extra schemes can follow if people ask.
- **Elevated beside unelevated.** An elevated launch beside an unelevated instance now fails instead of opening an unelevated tab.
  - Mitigation: the message names the cause. The old behaviour silently ran the request without elevation, which wasn't what the user asked for.
- **Very wide grids are capped.** A legitimate grid wider than 2048 columns or taller than 1024 rows (a tiny font on a huge display) is capped.
  - Mitigation: the program sees the capped size through `TIOCSWINSZ` or ConPTY, as it would with `MIN_COLS`.
- **The restore probe stays on the UI thread.** `Workspace::restore` still asks `is_dir` there, once for each recorded directory, through `session::resolve_tab`. With the recorded-directory rule it only sees drive-letter paths on Windows, so no host is contacted, but a mapped drive that has stopped answering can delay one launch.
  - Mitigation: a follow-up change moves this probe off the UI thread.

## Migration Plan

Nothing to migrate in config or state.
- **Elevated sessions from an older build** keep running: `run-elevated` is re-protected in place, and its pipe names are unchanged. Their copies directory isn't touched, since the new build uses `server-elevated`.
- **Rollback** is a downgrade. An older build ignores the `*-elevated` copies and ConPTY directories, and may not be able to write into a re-protected `run-elevated` from an unelevated process, which is the point.
