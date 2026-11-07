# Tasks

## 0. Windows pseudoconsole spike (gate)

Throwaway code on a spike branch, run on the Windows machine. GUI runs there steal focus, so ask first. Findings go in `openspec/changes/terminal-server/spike-windows.md`. Groups 1–9 wait until the user has reviewed them, and any decision they change has gone back into the specs, design and tasks (`/opsx:update`).

- [x] 0.1 Cross-process reparent: a helper process creates the pseudoconsole with the shipped `conpty.dll` and starts a program. Brindle sends it its HWND, and the helper calls `ConptyReparentPseudoConsole`. Verify that a console program's dialog (one that calls `GetConsoleWindow` and opens a message box or credential prompt owned by it) is owned by Brindle's window. Record what happens when that HWND is destroyed while the session lives, and when the helper reparents to a second Brindle window or process, or to none (phase 2's detach and reattach). *(Works, asynchronously. Destroying the current owner, and an owner window in another process, are left for phase 3.)*
- [x] 0.2 Pseudoconsole handover: a second helper process takes over a live session from the first. It duplicates the pipe, process and internal pseudoconsole handles (the shipped ConPTY's layout is known, since Brindle builds and ships it), rebuilds a usable HPCON, and the first helper exits without closing it. Verify that after the handover, output, input, resize, reparent and close still work, and that the exit code is readable with `GetExitCodeProcess`. Record whether the inbox ConPTY can be handed over at all. *(Failed: I/O and the exit code move, but the HPCON can't be rebuilt, and the inbox ConPTY can't move at all. This led to the holder/core split.)*
- [x] 0.3 Outliving the client: start the helper from Brindle with `DETACHED_PROCESS`, `CREATE_NO_WINDOW` and `CREATE_BREAKAWAY_FROM_JOB`. Then kill Brindle, both from the Start Menu and from inside a job that forbids breakaway (as Windows Terminal or CI may set up). Verify whether the helper and its programs survive in each case. *(They survive with no job or a breakaway-OK job. A no-breakaway job refuses the spawn, and a kill-on-close job kills them. A Start Menu launch and a real kill are left for phase 3.)*
- [x] 0.4 Transport and loop: a per-user named pipe with a current-user-only DACL, a peer check (`GetNamedPipeClientProcessId` plus the token's user SID), and one IOCP loop serving the pipe clients and several pseudoconsoles. Verify that a second user is refused, and record whether one IOCP loop can be quiesced for handover the way design.md's poll loop is. *(Works. The output pipes must be overlapped named pipes. A real second account wasn't available.)*
- [x] 0.5 Write up `spike-windows.md`: each finding, and for each design decision it touches (handover by re-exec versus a new process taking handles, the handover format, the single loop, protocol 1's message set, for example a `REPARENT { session, hwnd }` message, and the Windows upgrade policy if an HPCON can't move), whether it holds or what should change. Verify the user has reviewed it and the change's artifacts are updated to match. *(The user chose the holder/core split on 2026-11-06.)*

## 1. Shared crate and builds

- [x] 1.1 Add the `term-core/` crate (`brindle-term-core`, same version as the others) to the workspace and to `default-members`, with every member in `default-members`. Verify that plain `cargo build` produces `target/debug/brindle-remote`, that the CI version check passes, and, from release-please's docs or a dry run, that a release bumps the new crate with the others; adjust `release-please-config.json` if it wouldn't.
- [x] 1.2 Move `Parser` and `Marks` (`src/terminal/shell_integration.rs`) and `Records` (`src/terminal/status.rs`) into `brindle-term-core`, keeping anything GPUI-specific in `brindle`. Verify that `cargo test --workspace --locked` passes with the moved unit tests, and that `Cargo.lock` gains only the new crate's own entry.
- [ ] 1.3 Make `brindle-remote-agent` depend on `brindle-term-core` and build the static musl helper (README's Docker command). Record the binary size before and after in the PR, and verify that `--server` still passes `BRINDLE_REMOTE_DOCKER_TEST=1 cargo test -p brindle --locked remote::client`.

## 2. Protocols

- [ ] 2.1 The client protocol (protocol 1) in `remote-protocol`, as a module beside the stdio frames, with everything design.md lists:
  - `HELLO`/`WELCOME` with version negotiation and `client_token`;
  - `req` ids and session-prefixed `DATA`;
  - the session, attach, reconnect replay, management and `STEP_DOWN` messages;
  - a reserved `REPARENT`;
  - unknown JSON fields ignored.

  Verify with unit tests for round trips, negotiation (overlap, no overlap, ranges) and the session prefix.
- [ ] 2.2 The core–holder protocol (H1) as a separate module, with everything design.md lists:
  - `HELLO`/`WELCOME`, with `previous`, the session list and `truncated`;
  - `SPAWN`, `INPUT`, `RESIZE`, `HANGUP`, `CHECKPOINT`, `DRAIN`, `GOODBYE` and `REPARENT`;
  - `TAIL`, `OUTPUT`, `SPAWNED`, `SPAWN_FAILED` and `EXITED`;
  - opaque `meta`.

  Verify with round-trip unit tests and a fuzz-style test that feeds the decoder random and truncated bytes and expects errors, never a panic.
- [ ] 2.3 Transcript fixtures for protocol 1 and H1: encoded exchanges for spawn, I/O, attach, reconnect, checkpoint and adopt, and management. Add a test that decodes them. Verify it passes, and that changing an encoding fails it.

## 3. Holder

- [ ] 3.1 The shared server pieces: directory resolution (`BRINDLE_SERVER_DIR`, `$XDG_RUNTIME_DIR/brindle`, `~/.cache/brindle/run`, mode 0700), the `--ready-fd` start with double fork and `setsid`, log files, the `SO_PEERCRED` check, and the event-loop trait with its `poll(2)` implementation. Verify with unit tests for directory resolution and modes, the peer check against a faked uid, and the loop's register/wait/wake.
- [ ] 3.2 `brindle-remote hold`:
  - the socket and flock;
  - one core connection, with a newer `HELLO` taking over;
  - `SPAWN` with the request's full environment (reusing agent.rs's PTY code), and output to the core;
  - `INPUT`, `RESIZE`, `EXITED` with status via the `SIGCHLD` self-pipe;
  - `HANGUP` (SIGHUP, then SIGKILL after the grace period);
  - exiting when empty.

  Verify with agent-crate tests driving a holder with a test core: the environment comes from the request, a missing command gives `SPAWN_FAILED` naming it, `exit 3` reports 3, hangup kills the process group, and the holder exits once empty.
- [ ] 3.3 The holder's checkpoints and tails:
  - store `CHECKPOINT` and drop the tail;
  - the tail since the last checkpoint is limited to 1 MiB, with `truncated`;
  - on a new `HELLO`, report the sessions, then each `CHECKPOINT` and `TAIL`.

  Add the backpressure: stop reading PTYs past an 8 MiB queue to the core, and stop reading a session whose tail is full while no core is connected. Verify with tests for a reconnect after output with and without a checkpoint, truncation, and a program blocked (not losing output) while no core is connected.
- [ ] 3.4 `DRAIN` (no new sessions, exit when the last ends) and core supervision: with no core for 3 s, start `serve` from `/proc/self/exe`, retrying with backoff capped at 60 s. Verify with tests: a drained holder refuses `SPAWN` and exits after its last session, and a holder whose core was SIGKILLed starts a new core within a few seconds, even after its binary file has been replaced.

## 4. Core

- [ ] 4.1 `brindle-remote serve`: `core.lock`, `core.sock`, accept, the client handshake, per-client queues with 1 MiB backpressure, finding and adopting holders by their sockets (skipping stale ones whose flock is free), starting a holder of its own build when needed, session id allocation continuing from adopted holders, and exiting after the unused grace period. Verify with tests: two racing starts leave one core, the parents aren't the test process, three sessions share one holder, and an unused core and holder exit and remove their sockets.
- [ ] 4.2 Client sessions through the holder: `SPAWN`, `DATA`, `RESIZE`, `EXIT`, `CLOSE`. Verify with tests: the environment comes from the request, not the core's own, a missing command gives an `ERROR` naming it, `exit 3` reports 3, and a stalled client doesn't delay another client's session.
- [ ] 4.3 Close policies with `client_token`: `kill` sessions end on `CLOSE`, `KILL`, `STOP` and when their owner is gone (connection ended, or not back within the 10 s reconnect grace period after a core start), while `detach` sessions survive. Verify with tests: a SIGKILLed client's `kill` session and its children are gone within a few seconds, a `detach` session is listed as detached, a client reconnecting within the grace period after a core restart keeps its `kill` sessions, and one that doesn't loses them.
- [ ] 4.4 Peer checks on both sockets. Verify with a test where one can `setuid`, otherwise with a unit test of the check under a faked peer uid, that a foreign uid is closed without being read.

## 5. Emulator, replay and checkpoints

- [ ] 5.1 A per-session `Term` in the core, fed through `brindle-term-core`'s `Parser`, with a listener that drops every event, the scrollback limit from `SPAWN`, and `catch_unwind` around each feed. Verify with tests: `ESC [ 6 n` gets exactly one reply (from the test client), and an injected panic leaves the session forwarding output and other sessions untouched.
- [ ] 5.2 The pure replay renderer (`&Term → Vec<u8>`), with everything in design.md. Verify with round-trip tests over fixtures:
  - shell output with wrapped lines;
  - an `htop`-like alternate-screen app with mouse modes;
  - 500 lines into a 1000-line scrollback;
  - marks for three commands;
  - a `working` status record.

  Each must reproduce grid, attributes, modes, cursor, marks and records in a fresh `Term`.
- [ ] 5.3 `ATTACH` and `DETACH`: `REPLAY_START`, replay, `REPLAY_END`, then live output, with the client's size applied after the replay, and a refusal when the session is attached. Verify with server tests for the full-screen, scrollback, status and marks cases and the refusal.
- [ ] 5.4 Checkpoints: send `CHECKPOINT` to the holder past 512 KiB of tail, on detach and before stepping down, and rebuild adopted sessions from the checkpoint plus the tail. Verify with tests:
  - after a core SIGKILL and respawn, a fresh attach to a full-screen session matches its screen;
  - a session with more than 512 KiB of output since its start rebuilds exactly;
  - a truncated tail still gives a working session.
- [ ] 5.5 Reconnect: after `WELCOME`, send each held session's replay to a returning `client_token`. Verify with a test that restarts the core under an attached client running a full-screen fixture: the client gets the replay, then live output, and the pid is unchanged.

## 6. Upgrades and management

- [ ] 6.1 Upgrades:
  - `WELCOME` reports the core's build and its holders' protocols;
  - `brindle-remote check --holder-protocols <list>`;
  - `STEP_DOWN`: checkpoint everything, `GOODBYE { handover }`, close the clients, exit;
  - a new core sends `DRAIN` to holders of other builds;
  - `core.log` entries for refused and completed upgrades.

  Verify with server tests:
  - an "upgrade" to the same binary with two protocol 1 clients and attached and detached sessions keeps every program, pid, screen and later exit status;
  - when an old-build holder drains, the next session goes to a new holder;
  - a failing check leaves the core running, with the reason in the log;
  - a new core that exits at startup is replaced by the holders' own core within a few seconds, with every session listed.
- [ ] 6.2 `brindle-remote list`, `kill <id>`, `kill --all`, `status` and `stop`, with `--json` for `list` and `status`. Verify with tests for each spec scenario:
  - two sessions listed;
  - killing an attached session delivers `EXIT` to its client;
  - an unknown id fails naming it;
  - `status --json` has the core and per-holder fields;
  - `stop` ends everything;
  - with no server running, there is no output and exit 0.

  Update the usage text and README's helper section.

## 7. Brindle client (Linux)

- [ ] 7.1 `remote::local`: helper lookup shared with `remote::client`, starting the core with `--ready-fd`, connecting and negotiating, with each failure becoming an error naming its reason, not cached. Add `BRINDLE_SERVER_DIR`, and compile it only on Linux. Verify with unit tests for the error messages (helper missing, core unreachable, no common version) and with `cargo build` for `x86_64-pc-windows-msvc` in CI.
- [ ] 7.2 The connection's reader thread (demux, parsing under parser → term → marks, `Wakeup`/`Osc` on the terminal's channel) and its writer thread. Add `Backend::Server` with input, resize, close and `EXIT`, route `Terminal::spawn_pty` on Linux to it, and remove the Unix side of `Backend::Pty` (`tty::Pty`, the Unix `MarkedPty`, `tcgetpgrp`). Verify that `cargo test --workspace` passes and that `--dump-screen-after` on a local tab (under `scripts/with-xvfb`) shows the prompt.
- [ ] 7.3 Send the full environment (Brindle's own with `child_env(profile)` applied) in `SPAWN`. Verify with the `local-server-env` e2e case from 8.2.
- [ ] 7.4 Process info from `/proc/<pid>/stat`'s `tpgid` in `process.rs`, used by `Backend::Server` for titles and `working_directory`. Verify with a unit test against a spawned child, and that the existing `titles` and `zones` e2e cases pass.
- [ ] 7.5 Reconnect: on EOF, reconnect with the same `client_token`, start a core if none appears within 5 s, queue input per session meanwhile, and apply each replay with bells, OSC 9/777 notices and status notices suppressed between `REPLAY_START` and `REPLAY_END`. If a session is missing, or no core appears within 15 s, show the "session lost" line and mark the terminal exited without `Exited`. Verify with unit tests for the replay suppression and with the `local-server-core-restart`, `local-server-replay-quiet` and `local-server-holder-lost` e2e cases from 8.2.
- [ ] 7.6 The upgrade offer: compare the found helper's version and build id with `WELCOME`, apply the equal-version mtime rule, run the check, then send `STEP_DOWN` and start the new core. Verify with unit tests for the newer/older/equal decisions and with the `local-server-upgrade` e2e case from 8.2.

## 8. Test harness and e2e cases

- [ ] 8.1 Give every e2e case `BRINDLE_SERVER_DIR=target/tmp/e2e/<case>/server`, run `brindle-remote stop` after it, and have the harness fail with a clear message if the helper isn't built. Update CLAUDE.md's e2e section and the CI e2e and Windows jobs as needed. Verify that `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e` passes on the existing cases, and that no `brindle-remote serve` or `hold` is left running afterwards.
- [ ] 8.2 Add `tests/e2e/cases/local_server.rs` with these cases:
  - `local-server-session`: listed as attached, and the shell's parent is a holder;
  - `local-server-close`, `local-server-quit` and `local-server-crash`: the programs are gone;
  - `local-server-env`: a second Brindle with `FOO=2`;
  - `local-server-core-restart`: the core SIGKILLed while a tab runs a full-screen program, and the tab comes back with the same pid;
  - `local-server-replay-quiet`: a bell and an OSC 9 since the last checkpoint raise nothing on reconnect (`BRINDLE_NOTIFY_LOG` stays empty);
  - `local-server-holder-lost`: the holder SIGKILLed, and the tab shows "session lost";
  - `local-server-no-helper`: the tab shows the missing-helper error, and a tab opened after `BRINDLE_REMOTE_AGENT` names the helper runs in the server;
  - `local-server-upgrade`: a second Brindle with a helper copy whose mtime is newer keeps the first Brindle's tab and pid, and `status` shows the old holder draining;
  - `local-server-manage`: `brindle-remote list` run in a tab lists its own session.

  Verify they pass under `scripts/with-xvfb`, with `BRINDLE_E2E_SLOW=2` too.

## 9. Integration and measurements

- [ ] 9.1 Run `tests/perf` on a release build of `main` before the change, then on the branch with `BRINDLE_PERF_BASELINE` set to the first run, and put both summaries in the PR. If latency or throughput regress noticeably, bring the numbers to the user with the options in design.md (emulator worker threads, the data fast path) before changing anything.
- [ ] 9.2 Run the full Linux checks (`cargo test --locked`, `cargo build --release --locked`, `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e`) and the Windows CI jobs, and confirm the Windows build and its e2e cases are unchanged.
- [ ] 9.3 Update README.md (the terminal server, its core and holders, the `brindle-remote` commands, and the helper being required on Linux) and CLAUDE.md's architecture section (`remote::local`, `Backend::Server`, `brindle-term-core`, the core and holder loops, protocol 1 and H1, checkpoints and upgrades). Verify both against the implementation.
- [ ] 9.4 Hand the user the manual checks: dogfood via `scripts/install.sh` with a Brindle open, and confirm the reinstall upgrades the core without losing the open tabs (`brindle-remote status` shows the new core and the old holder draining, and the programs keep their pids).

## Workflow follow-up

- Land through a squash-merged PR (title e.g. `feat(terminal-server): run local Linux terminals in a persistent server`), then archive the change with its delta specs synced.
- Propose phase 2 (`on_close`, reattach on restore, the sessions palette).
