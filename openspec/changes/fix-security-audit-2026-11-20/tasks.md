# Tasks

## 1. Shared limits and pure decisions

- [x] 1.1 Add `MAX_COLS` (2048) and `MAX_ROWS` (1024) beside `MIN_COLS` in `term-core/src/emulator.rs`, and clamp both ends in `emulator::size`. Verify with unit tests that `Emulator::new(65535, 65535, …)` builds a 2048×1024 grid, and that sizes inside the limits are unchanged.
- [x] 1.2 Restrict `reported_dir` (`term-core/src/shell_integration.rs`) in the Windows style to drive-letter paths. The OSC 7 branch returns `None` unless the path is `/X:/…`. Verify with parser unit tests for `\\host\share`, `\\?\UNC\h\s`, `\\.\pipe\x`, `\\host@80\x`, `file://x//host/share` (each ignored, keeping the previous directory) and the existing drive-letter cases. Fuzz targets keep passing.
- [x] 1.3 Add `access::elevated_dir_sddl()` (owner BA; protected DACL of BA and SY full control; High no-write-up label) and the pure `access::matches_elevated` in `remote-protocol/src/access.rs`. Verify with unit tests that the exact descriptor matches, and that ones with a user-SID ACE, another owner, no label or a Medium label don't.
- [x] 1.4 Add the elevated names in `remote-protocol/src/server_dir.rs` (`helper_copies(…, elevated)` giving `server-elevated`) and `remote-agent/src/conpty/choice.rs` (`conpty-elevated\<version>`), plus the pure `untrusted_name(name, pid, n)`. Verify with unit tests for both elevations, with and without `BRINDLE_SERVER_DIR`.
- [x] 1.5 Add `conpty::cmdline::resolve_program(name, path_var, pathext, exists) -> Resolved` (`Found(String)`/`NotFound`). It skips the search for names with a directory, and walks `PATH` with `PATHEXT`, never the image directory. Verify with unit tests run on every platform, using `str` paths.
- [x] 1.6 Add `links::plan(uri, this_host, windows) -> Plan` (`Open`, `Reveal`, `Refuse(Refusal::{NoScheme, Scheme, OtherHost})`). Share one scheme `const` with `find_url`, and percent-encode `"` and whitespace. Verify with a unit test per links scenario: `ms-settings:`, `C:\x.exe`, `file://host/…`, `file:///C:/…` on Windows and Linux, quotes, case-insensitive schemes, `localhost`, and this machine's name.
- [x] 1.7 Add the GPUI-free `DirCheck` model (`src/terminal/dir_check.rs`): the reported path, the last result as `Checked::{Exists, Missing, Pending}`, one check in flight, and the latest report kept. Verify with unit tests for a change during a check, a repeat report, and a missing directory.

## 2. Shared crates and server roles

- [x] 2.1 Make the core's emulator sizes go through the clamped `emulator::size` for `SPAWN`, `RESIZE` and adoption. Verify with a `remote-agent/tests/core.rs` test that spawns at 65535×65535 and gets a session listed at 2048×1024.
- [ ] 2.2 Implement `win::protected_dir(path, Contents)` in `remote-protocol/src/win.rs`:
  - it opens with `FILE_FLAG_OPEN_REPARSE_POINT` and without `FILE_SHARE_DELETE`, and refuses reparse points;
  - it checks the result with `matches_elevated` and `GetFinalPathNameByHandleW`;
  - a mismatch under `Executables` renames the directory aside and remakes it, and a mismatch under `State` re-protects it and deletes reparse points inside;
  - it returns the held `ProtectedDir`.

  Verify with a Windows test in `remote-agent/tests/launch.rs` that a planted junction and a user-owned directory are each replaced, and that renaming the held directory or its parent from a Medium token fails. The test skips through `skip(Capability, …)` when not elevated.
- [ ] 2.3 Use `protected_dir` for the elevated server directory (`remote-agent/src/server/platform/windows/dir.rs`, `State`) and the elevated helper copies (`win::ensure_helper_copy`, `Executables`), holding the handle for the role's life. Verify with a Windows test that a planted `brindle-remote.exe` in the elevated copies location is never run.
- [ ] 2.4 Make `prune_helper_copies` and the start-up removal of `*.untrusted-*` directories skip reparse points without descending. Verify with a Windows test that a junction among the copies leaves its target untouched.
- [ ] 2.5 Extract and load the elevated ConPTY files from a held `conpty-elevated\<version>` (`remote-agent/src/conpty/{choice,api}.rs`). Verify with a Windows test that an elevated holder's `OpenConsole.exe` path is inside the protected directory.
- [ ] 2.6 Resolve bare program names in `remote-agent/src/server/platform/windows/pty.rs` with `resolve_program` and pass the absolute application name. Verify with a Windows `remote-agent/tests/holder.rs` test that a `cmd.exe` placed beside the helper copy isn't the one started, and that an unknown name gives `SPAWN_FAILED` naming it.

## 3. Brindle's client and entities

- [x] 3.1 Make `ListedSession::of` (`src/remote/identity.rs`) give `size: None` outside `MIN_COLS..=MAX_COLS` × `1..=MAX_ROWS`, and clamp `GridSize::clamped`, `Terminal::resize` and the replay resize to both ends. Verify with unit tests for `of`, and a `src/terminal` test that a listed 65535×65535 attaches at the default size.
- [x] 3.2 Keep every check of a reported directory off the UI thread:
  - Run the existence check through `DirCheck` on the background executor. Where a local session's directory is the reported one (`process::LOCAL_DIRECTORY`, Windows), `Terminal::working_directory` reads only the model.
  - Move the directory probe of a new tab or split from `Terminal::spawn` to the thread that starts the session: `spawn_request` names the directory, `Session::start` calls the request on its own thread, and `with_checked_dir` takes the probe's result there.
  - Keep a directory recorded for this machine only when it is absolute in the local path style (`shell_integration::is_absolute_dir`, the rule `reported_dir` uses), in `session::resolve_tab`, before it is probed.

  Verify with `cargo test -p brindle terminal`, unit tests in `session.rs` for the recorded-directory rule and in `remote/local/session.rs` for the thread the request is made on, and confirm by reading the code that `record_session`, `new_tab_cwd` and splits check no directory on the UI thread. The restore probe (`Workspace::restore` → `session::resolve_tab`) stays on the UI thread, limited to drive-letter paths: see design.md's Risks.
- [ ] 3.3 Use the elevated, held `server-elevated` copy in `src/remote/local/windows.rs` when Brindle is elevated. Verify with `cargo clippy` on Windows CI and the existing `windows-*` e2e cases.
- [x] 3.4 Apply `links::plan` in `links::open`: `Reveal` runs `explorer.exe /select,<path>` on the opener thread, and opens a directory instead. Word the new refusals in `links::failure_text`. Verify with unit tests for the wording, and update the e2e `links` case to ctrl-click an `ms-settings:` OSC 8 link and expect the dim refusal line.
- [ ] 3.5 Make the Windows `instance::forward` open with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`, call `win::check_server` before writing, and return a refusal string. Make the Linux `forward` check `SO_PEERCRED` against the effective uid. Verify with a pure test of the refusal wording, and a Linux test that a socket served by a peer of another uid is refused, skipping when it can't make one.

## 4. Integration

- [ ] 4.1 Run `cargo test --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings` and `cargo test -p brindle --test architecture`, and verify all pass.
- [ ] 4.2 Run `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e` on Linux, and the Windows e2e cases on Windows CI, and verify both pass.
- [ ] 4.3 Manual checks on Windows, handed to the user:
  - an elevated Brindle opens a local tab, and an unelevated `New-Item` into `server-elevated` is denied;
  - ctrl-clicking a `file:///C:/…/x.exe` OSC 8 link shows it selected in Explorer;
  - `brindle` launched elevated beside an unelevated Brindle prints why and exits 1.

## Workflow follow-up

- Archive the change in the implementation PR, syncing the delta specs.
- Mark findings 1–5 as fixed in a remediation note beside `docs/security/audits/2026-11-20.md`.
