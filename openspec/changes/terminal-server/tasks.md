# Tasks

## 0. Windows pseudoconsole spike (gate)

Throwaway code on a spike branch, run on the Windows machine. GUI runs there steal focus, so ask first. Findings go in `openspec/changes/terminal-server/spike-windows.md`. Groups 1–9 wait until the user has reviewed them, and any decision they change has gone back into the specs, design and tasks (`/opsx:update`).

- [ ] 0.1 Cross-process reparent: a helper process creates the pseudoconsole with the shipped `conpty.dll` and starts a program. Brindle sends it its HWND, and the helper calls `ConptyReparentPseudoConsole`. Verify that a console program's dialog (one that calls `GetConsoleWindow` and opens a message box or credential prompt owned by it) is owned by Brindle's window. Record what happens when that HWND is destroyed while the session lives, and when the helper reparents to a second Brindle window or process, or to none (phase 2's detach and reattach).
- [ ] 0.2 Pseudoconsole handover: a second helper process takes over a live session from the first. It duplicates the pipe, process and internal pseudoconsole handles (the shipped ConPTY's layout is known, since Brindle builds and ships it), rebuilds a usable HPCON, and the first helper exits without closing it. Verify that after the handover, output, input, resize, reparent and close still work, and that the exit code is readable with `GetExitCodeProcess`. Record whether the inbox ConPTY can be handed over at all.
- [ ] 0.3 Outliving the client: start the helper from Brindle with `DETACHED_PROCESS`, `CREATE_NO_WINDOW` and `CREATE_BREAKAWAY_FROM_JOB`. Then kill Brindle, both from the Start Menu and from inside a job that forbids breakaway (as Windows Terminal or CI may set up). Verify whether the helper and its programs survive in each case.
- [ ] 0.4 Transport and loop: a per-user named pipe with a current-user-only DACL, a peer check (`GetNamedPipeClientProcessId` plus the token's user SID), and one IOCP loop serving the pipe clients and several pseudoconsoles. Verify that a second user is refused, and record whether one IOCP loop can be quiesced for handover the way design.md's poll loop is.
- [ ] 0.5 Write up `spike-windows.md`: each finding, and for each design decision it touches (handover by re-exec versus a new process taking handles, the handover format, the single loop, protocol 1's message set, for example a `REPARENT { session, hwnd }` message, and the Windows upgrade policy if an HPCON can't move), whether it holds or what should change. Verify the user has reviewed it and the change's artifacts are updated to match.

## 1. Shared crate and builds

- [ ] 1.1 Add the `term-core/` crate (`brindle-term-core`, same version as the others) to the workspace and to `default-members`, with every member in `default-members`. Verify that plain `cargo build` produces `target/debug/brindle-remote`, that the CI version check passes, and, from release-please's docs or a dry run, that a release bumps the new crate with the others; adjust `release-please-config.json` if it wouldn't.
- [ ] 1.2 Move `Parser` and `Marks` (`src/terminal/shell_integration.rs`) and `Records` (`src/terminal/status.rs`) into `brindle-term-core`, keeping anything GPUI-specific in `brindle`. Verify that `cargo test --workspace --locked` passes with the moved unit tests, and that `Cargo.lock` gains only the new crate's own entry.
- [ ] 1.3 Make `brindle-remote-agent` depend on `brindle-term-core` and build the static musl helper (README's Docker command). Record the binary size before and after in the PR, and verify that `--server` still passes `BRINDLE_REMOTE_DOCKER_TEST=1 cargo test -p brindle --locked remote::client`.

## 2. Socket protocol

- [ ] 2.1 Add the socket protocol to `remote-protocol` as a module beside the stdio frames: message numbers, `HELLO`/`WELCOME` with version negotiation, `req` ids, session-prefixed `DATA`, and the JSON messages from design.md, with unknown fields ignored. Verify with unit tests for round trips, negotiation (overlap, no overlap, ranges), and the session id prefix.
- [ ] 2.2 Add protocol 1 transcript fixtures (encoded byte streams for a spawn, I/O, attach and management exchange), with a test that decodes them. Verify the test passes and that changing a message's encoding fails it.

## 3. Server core

- [ ] 3.1 Server directory resolution (`BRINDLE_SERVER_DIR`, `$XDG_RUNTIME_DIR/brindle`, `~/.cache/brindle/run`, mode 0700), the `server.lock` flock, stale socket cleanup, bind, `server.log`, and `serve --ready-fd` with double fork and `setsid`. Verify with agent-crate tests: two racing starts leave one server, a second server exits 0, the server's parent isn't the test process, and the directory is 0700.
- [ ] 3.2 The single-threaded `poll` loop: accept, the `SO_PEERCRED` check, the handshake, per-client outgoing queues, the `SIGCHLD` self-pipe, and exit after the unused grace period. Verify with tests: a connection under another uid is refused (run where the test can `setuid`, otherwise a unit test of the check with a faked peer uid), and the server exits and removes its socket once unused.
- [ ] 3.3 `SPAWN` with the full environment from the request, PTY creation (reusing agent.rs's PTY and `/proc` code), `DATA`, `RESIZE`, `EXIT` with status, and the per-client high-water backpressure. Verify with tests: the environment comes from the request and not the server, a missing command gives an `ERROR` naming it, `exit 3` reports 3, and a stalled client doesn't delay another client's session.
- [ ] 3.4 Close policies and hangup: `CLOSE`, client EOF and error, `KILL`, `STOP`, SIGHUP then SIGKILL after the grace period, `kill` versus `detach`. Verify with tests: a SIGKILLed client's `kill` session and its children are gone within a few seconds, and a `detach` session survives its client and is listed as detached.

## 4. Emulator and replay

- [ ] 4.1 A per-session `Term` fed through `brindle-term-core`'s `Parser`, with a listener that drops every event, the scrollback limit from `SPAWN`, and `catch_unwind` around each feed. Verify with tests: `ESC [ 6 n` gets exactly one reply (from the test client), and an injected panic leaves the session forwarding output and other sessions untouched.
- [ ] 4.2 The pure replay renderer (`&Term → Vec<u8>`), with everything listed in design.md: reset, scrollback, `WRAPLINE` joins, SGR, OSC 133 marks at their lines, the alternate screen, the modes, cursor, title and OSC 7501 records. Verify with round-trip tests over fixtures: shell output with wrapped lines, an `htop`-like alternate-screen app with mouse modes, 500 lines into a 1000-line scrollback, marks for three commands, and a `working` status record. Each must reproduce grid, attributes, modes, cursor, marks and records in a fresh `Term`.
- [ ] 4.3 `ATTACH` and `DETACH`: `REPLAY_START`, replay `DATA`, `REPLAY_END`, then live output, with the attaching client's size applied after the replay, and a refusal when the session is attached. Verify with server tests for a detached session running a full-screen fixture, the scrollback case, the status and marks cases, and the refusal.

## 5. Handover

- [ ] 5.1 The handover state (sessions, clients with their partial reads and queues, the listener and lock fds, the ids and start time) as a versioned format in a `memfd`, plus `handover-check --format F`. Verify with unit tests for serialize/deserialize round trips and an unknown format being rejected.
- [ ] 5.2 `UPGRADE`: binary vetting (regular file, owner, mode), the check, quiescing, clearing `FD_CLOEXEC`, `execve` into `serve --resume`, restoring on `execve` failure, rebuilding the `Term`s from the replays, then `UPGRADED`. Verify with server tests that upgrade to the same binary while two clients (protocol 1) have attached and detached sessions running. Afterwards the server pid, the program pids, the screens (via a fresh attach), live I/O and a later exit status must all be intact, and a group-writable or check-failing binary must leave the server running and answering.
- [ ] 5.3 `server.log` entries for refused and completed upgrades. Verify the refusal test from 5.2 finds the reason in the log.

## 6. Management commands

- [ ] 6.1 `brindle-remote list`, `kill <id>`, `kill --all`, `status` and `stop`, with `--json` for `list` and `status`, as clients of the socket protocol. Verify with tests for each spec scenario: two sessions listed, killing an attached session delivers `EXIT` to its client, an unknown id fails naming it, `status --json` fields, `stop` ends everything, and no server running gives no output with exit 0. Update the usage text and README's helper section.

## 7. Brindle client (Linux)

- [ ] 7.1 `remote::local`: helper lookup shared with `remote::client`, starting the server with `--ready-fd`, connecting and negotiating, with each failure turned into an error naming its reason and no cached failure. Add `BRINDLE_SERVER_DIR`, and compile it only on Linux. Verify with unit tests for the error messages (helper missing, server unreachable, no common version) and with `cargo build` for `x86_64-pc-windows-msvc` in CI.
- [ ] 7.2 The connection's reader thread (demux, parsing under parser → term → marks, `Wakeup`/`Osc` on the terminal's channel) and its writer thread. Add `Backend::Server` with input, resize, close and `EXIT` handling, and route `Terminal::spawn_pty` on Linux to it. Remove the Unix side of `Backend::Pty` (`tty::Pty`, the Unix `MarkedPty`, `tcgetpgrp`). Verify that `cargo test --workspace` passes and that `--dump-screen-after` on a local tab (under `scripts/with-xvfb`) shows the prompt.
- [ ] 7.3 Send the full environment (Brindle's own with `child_env(profile)` applied) in `SPAWN`. Verify with the `local-server-env` e2e case from 8.2.
- [ ] 7.4 Process info from `/proc/<pid>/stat`'s `tpgid` in `process.rs`, used by `Backend::Server` for titles and `working_directory`. Verify with a unit test against a spawned child, and that the existing `titles` and `zones` e2e cases pass.
- [ ] 7.5 Connection loss: each server terminal gets the "terminal server went away" line and is marked exited without `Exited`. Verify with the `local-server-lost` e2e case from 8.2.
- [ ] 7.6 The upgrade offer: compare the found helper's version and digest with `WELCOME`, apply the equal-version mtime rule, and send `UPGRADE`. Verify with unit tests for the newer/older/equal decisions and the `local-server-upgrade` e2e case from 8.2.

## 8. Test harness and e2e cases

- [ ] 8.1 Give every e2e case `BRINDLE_SERVER_DIR=target/tmp/e2e/<case>/server`, run `brindle-remote stop` after it, and have the harness fail with a clear message if the helper isn't built. Update CLAUDE.md's e2e section and the CI e2e and Windows jobs as needed. Verify that `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e` passes on the existing cases, and that no `brindle-remote serve` is left running afterwards.
- [ ] 8.2 Add `tests/e2e/cases/local_server.rs` with these cases:
  - `local-server-session`: listed as attached, and the shell's parent is the server;
  - `local-server-close`, `local-server-quit` and `local-server-crash`: the programs are gone;
  - `local-server-env`: a second Brindle with `FOO=2`;
  - `local-server-lost`: the server SIGKILLed while a tab is open;
  - `local-server-no-helper`: the tab shows the missing-helper error, and a tab opened after `BRINDLE_REMOTE_AGENT` names the helper runs in the server;
  - `local-server-upgrade`: a second Brindle with a helper copy whose mtime is newer keeps the first Brindle's tab and pid;
  - `local-server-manage`: `brindle-remote list` run in a tab lists its own session.

  Verify they pass under `scripts/with-xvfb`, with `BRINDLE_E2E_SLOW=2` too.

## 9. Integration and measurements

- [ ] 9.1 Run `tests/perf` on a release build of `main` before the change, then on the branch with `BRINDLE_PERF_BASELINE` set to the first run, and put both summaries in the PR. If throughput drops noticeably, move emulators to worker threads (design.md, "One poll loop") and measure again.
- [ ] 9.2 Run the full Linux checks (`cargo test --locked`, `cargo build --release --locked`, `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e`) and the Windows CI jobs, and confirm the Windows build and its e2e cases are unchanged.
- [ ] 9.3 Update README.md (the terminal server, `brindle-remote` commands, the helper being required on Linux) and CLAUDE.md's architecture section (`remote::local`, `Backend::Server`, `brindle-term-core`, the server loop and handover). Verify both against the implementation.
- [ ] 9.4 Hand the user the manual checks: dogfood via `scripts/install.sh` with a Brindle open, and confirm the reinstall upgrades the server without losing the open tabs (`brindle-remote status` shows the new version, with the same pid).

## Workflow follow-up

- Land through a squash-merged PR (title e.g. `feat(terminal-server): run local Linux terminals in a persistent server`), then archive the change with its delta specs synced.
- Propose phase 2 (`on_close`, reattach on restore, the sessions palette).
