# Tasks

## 1. Protocol

- [x] 1.1 Add `remote-protocol/src/relay.rs`: `RELAY_HOST` (version, build, env, shell, home, server dir, core pid), `RELAY_HEARTBEAT` and `RELAY_ERROR`, on message numbers protocol 1 never uses, with encode/decode and transcript tests; verify with `cargo test -p brindle-remote-protocol relay`
- [x] 1.2 Add `linger_secs` (default 0, capped at 600 by the core) and `process_info` (default false) to `Hello` as additive JSON fields. Verify that old `HELLO` frames still decode, in the protocol-1 transcript tests (`cargo test -p brindle-remote-protocol client`)
- [x] 1.3 Raise `PROTOCOLS` to 1–2 and add `ToClient::Process { session, name, cwd }` (protocol 2 only), with a transcript test and an `answer_hello` test showing that a 1–2 client and a 1-only core agree on 1; verify with `cargo test -p brindle-remote-protocol`
- [x] 1.4 Move `Build`, `is_newer` and `semver` from `src/remote/local/upgrade.rs` to `remote-protocol/src/upgrade.rs`, with their unit tests, and use them from `upgrade.rs`; verify with `cargo test -p brindle-remote-protocol upgrade` and `cargo test -p brindle remote::local`
- [x] 1.5 Add `owned` and `detached_secs` to `SessionInfo` as additive JSON fields, and verify that old `SESSIONS` frames still decode (`cargo test -p brindle-remote-protocol client`)
- [x] 1.6 Extend `remote-protocol/src/fuzz.rs` to the relay frames, `PROCESS`, and the new `HELLO` and `SessionInfo` fields; verify it passes

## 2. Holder hardening (from `harden-terminal-server`)

- [x] 2.1 In the PTY child's `pre_exec` (`remote-agent/src/pty/unix.rs`), set every signal except `SIGKILL`/`SIGSTOP` to `SIG_DFL` and empty the mask. Verify with a new `remote-agent/tests/holder.rs` case: a holder started with SIGHUP ignored runs `cat`, and a `CLOSE` ends `cat` by SIGHUP before the SIGKILL grace
- [x] 2.2 On a failed write to its core, make the holder read until `WouldBlock` (bounded at 16 MiB) before dropping the core. Verify with a holder test where a `SET_META` queued behind a full socket buffer is stored
- [x] 2.3 Make starters count a role as ready only when the ready pipe holds exactly the ready byte at EOF, and make roles close the ready fd right after writing it (`platform/unix/daemon.rs`, `src/remote/local/unix.rs`, `tests/support`). Verify with a test that forces a failed exec with `--ready-fd 3` and gets "didn't start", not "ready"

## 3. Core: takeover, linger, process info

- [ ] 3.1 Takeover: in `welcome`, drop every connected client with the same token before `reattach`, without `client_gone`, failing its waiting requests. Verify in `remote-agent/tests/core.rs`: a second connection with the same token gets every session's replay with the same pids, and the first connection is closed
- [ ] 3.2 Linger: `client_gone` keeps a lingering client's `kill` sessions with their owner and a `linger_until`. The core's timer hangs them up when it passes, and `reattach` clears it. Verify in `core.rs` (with a test-short linger) that a session survives a reconnect within the linger, ends without one, and that a client asking for no linger keeps today's immediate hang-up
- [ ] 3.3 Process info: track per-session activity (output, input), and on a 1 s timer sample the flagged sessions' foreground process (`tpgid`, `comm`, `cwd`, reusing the `/proc` helpers in `pty/unix.rs`), also once after a spawn or attach. Send `PROCESS` on change to protocol-2 clients that asked. Verify in `core.rs` that `sleep` started in a shell is reported within about a second, an idle session reports nothing for 3 s, and a protocol-1 client gets no `PROCESS`
- [ ] 3.4 Report `owned` and `detached_secs` in `SESSIONS` (an owner is set and no client is attached; seconds since the last client left), and show them in `brindle-remote list`. Verify in `core.rs` and `manage.rs` that a dropped connection's session lists as owned, and an explicitly detached one as not owned
- [ ] 3.5 Update the core's module docs and CLAUDE.md's "Core" and "Protocols" bullets for takeover, linger and protocol 2; verify `cargo doc -p brindle-remote-agent --no-deps` builds without warnings

## 4. `brindle-remote relay`

- [ ] 4.1 Add the `relay` subcommand (`remote-agent/src/server/relay.rs`, with `main.rs` and `cli.rs` usage):
  - resolve the server directory;
  - connect, or start the core from its own image, choosing a scope only when the user lingers (`/var/lib/systemd/linger/<user>`), otherwise plain with `BRINDLE_NO_SCOPE=1` and one log line;
  - write the marker, then `RELAY_HOST` or `RELAY_ERROR`.

  Verify with `remote-agent/tests/relay.rs`: a relay against a temp `BRINDLE_SERVER_DIR` starts a core, reports `HOST` with its environment, and a test client spawns a session through it
- [ ] 4.2 Relay framing and liveness:
  - copy frames both ways;
  - drop the client's heartbeats;
  - send a heartbeat after 10 s without a write, under one writer lock;
  - stamp a `Liveness` on the stdin reader thread and exit after 60 s of silence, with timings overridable for tests.

  Verify in `relay.rs` that a silent client ends the relay, an idle client that sends heartbeats doesn't, a client that stops reading during a flood doesn't, and a protocol-1 core never receives a heartbeat
- [ ] 4.3 Relay upgrade offer: connect with `<token>-upgrade`, compare with `upgrade::is_newer` against `/proc/<core pid>/exe`'s mtime, check the holder protocols in-process, `STEP_DOWN`, start `serve` from its own image, then connect for the client. Verify in `remote-agent/tests/upgrade.rs` that a newer relay upgrades a running core with sessions kept (same pids), and an older relay leaves it alone
- [ ] 4.4 Document `relay` in CLAUDE.md's terminal server section (link, liveness, scope decision, upgrade) and in `brindle-remote`'s usage text; verify `brindle-remote --help` lists it

## 5. Client: one connection per endpoint

- [ ] 5.1 Introduce `Host` (`Local`, `Remote(RemoteTarget)`, compared by target identity) and make `shared(host)` keep one `Connection` per host; local behaviour unchanged. Verify with the existing `remote::local` tests and a unit test that two profiles with the same target share a `Host`
- [ ] 5.2 Give `Connection` a `Transport`:
  - local streams as today;
  - relay children with heartbeat send and drop, a 30 s dead-link timeout that kills the child, and a blocking `wait()` thread;
  - per-transport reconnect: relays back off 1 s to 30 s with no time limit, and their requests wait while connecting;
  - after a failed first connect, a new tab's `SPAWN` fails, while attaches and eager connections keep waiting through the retries.

  Verify with unit tests on an in-memory transport for the dead-link timeout, backoff, queued requests and that first-connect split
- [ ] 5.3 Deployment:
  - the SSH launch script that execs a cached `relay` or exits 97, then upload and relaunch;
  - the container cache in `/tmp/brindle-helper-<version>-<digest>`, with the ownership check in the launch exec, a root deploy on 97, refusal of a non-root-owned directory, and pruning of other builds.

  Verify with unit tests of the generated commands, and `BRINDLE_REMOTE_DOCKER_TEST=1 cargo test -p brindle --locked remote::client` updated to:
  - the warm SSH connect making one `ssh` process;
  - the container helper reused across two connections;
  - a squatted directory refused.
- [ ] 5.4 Build remote `SPAWN`s from the connection's `RELAY_HOST` environment plus `child_env(profile)`, with no command when the profile sets none and no local directory check; set `linger_secs: 300` and `process_info: true` in relay `HELLO`s. Verify with unit tests on the request builder
- [ ] 5.5 Write the "connection to <endpoint> lost; reconnecting…" line into the endpoint's terminals on a loss, and show a session as lost only from the reconnected core's `LIST`. Verify with a `connection.rs` unit test on an in-memory transport that drops and returns

## 6. Remote terminals on the server backend

- [ ] 6.1 Route remote profiles through `Terminal::spawn`/`attach` on their `Host` with `Backend::Server`, using `Parser::posix()`. Keep the last `PROCESS` in `Session` and read it in `foreground_process_name` and `working_directory`, with OSC 7 winning for the directory. Gate every arm that assumes this machine on `Host::Local`: `parent_console_to`, `REPARENT` after `REPLAY_END`, the pid lookups and local directory checks. Verify with unit tests on `Session`'s process state, a `connection.rs` test that a remote session never sends `REPARENT` after a replay, and `cargo test -p brindle terminal`
- [ ] 6.2 Record a `Host` on each `Tab` and use it for splits and the "same endpoint" directory rule, whatever the profile now targets (`workspace.rs`, `zone_view.rs`). Verify with a workspace unit test where a profile's host changes and a split keeps the old host
- [ ] 6.3 Reference-count non-eager relay connections by their sessions and by queued `CLOSE`s not yet written, and close them 5 s after the count reaches zero. Verify with `remote::local` unit tests: the connection closes after its last session goes, and a `CLOSE` queued while disconnected keeps it reconnecting until the `CLOSE` is written
- [ ] 6.4 Remove `Backend::Agent`, `remote::client`'s per-tab connection (keeping deployment), `remote-agent/src/agent.rs`, `--server`, `idle_limit_secs` and the stdio session messages. Verify `cargo build --workspace` has no warnings, `cargo test --workspace` passes, and `rg -n 'Backend::Agent|--server|idle_limit' src remote-agent remote-protocol` finds nothing
- [ ] 6.5 Update CLAUDE.md's `terminal/` and `remote/` bullets (backends, `Host`, the relay transport, deployment) to match; verify no remaining mention of the stdio agent with `rg -n 'stdio .*agent|--server' CLAUDE.md`

## 7. Restore and launch

- [ ] 7.1 Keep remote zones' session records in `resolve_tab`, attached on the recorded target's `Host`. Restore them at once as attaches that wait for their connection. Make a pending attach's `Session` report the requested id and key, so a quit while offline records the zone again, and make a refused or missing session show its reason as a message terminal that isn't recorded. Verify with `session.rs` unit tests (remote record kept; profile moved to another host still attaches on the recorded one), a `session.rs` (local) test that a pending attach reports its id and key, and a workspace test of the message terminal not being recorded
- [ ] 7.2 After the window opens, connect in the background to the `Host` of every restored remote zone and every `connect_at_launch` profile. On each host's first `WELCOME` of the launch, attach every detached session not restored into a zone that has no owner or has been detached for more than 5 minutes (design decision 13), as tabs at the end in start order, with the spec's profile rule and the active tab kept. Verify with `reattach.rs` unit tests for ordering, exclusion of restored sessions, the owner rule and the profile choice
- [ ] 7.3 Make local reattach use the default profile when a session's recorded profile is remote or gone, and apply the same owner rule; verify with `reattach.rs` unit tests

## 8. Eager endpoints, the sessions palette and settings

- [ ] 8.1 Add `connect_at_launch` to both remote target kinds in `config.rs` and `config_edit.rs` (written only when true). Verify with config round-trip tests
- [ ] 8.2 Add the `Hosts` global (connecting, connected, offline with its error), fed from the connections. Keep eager connections for the process's life, retrying up to 60 s apart. Verify with a unit test of the state transitions on an in-memory transport
- [ ] 8.3 List every connected endpoint's detached sessions in the sessions palette, grouped local first then by profile order, with inert rows for connecting and offline endpoints. Make `picker_kill` and enter act on the row's connection. Verify with `picker.rs` unit tests of the grouping and the inert rows
- [ ] 8.4 Add the "Connect at launch" `Switch` to the settings dialog's remote section, for both kinds; verify with the settings dialog's unit tests and a `--dump-screen-after` e2e check of the field
- [ ] 8.5 Document remote persistence, `connect_at_launch`, `loginctl enable-linger` and the sessions palette's hosts in README.md; verify the README's example config parses with a config unit test

## 9. End-to-end checks

- [ ] 9.1 Extend the Docker `remote-*` e2e cases (`tests/e2e/cases/remote.rs`) to:
  - quit with an SSH tab and relaunch, getting the same pid;
  - split sharing one connection;
  - cut the network (`docker network disconnect`) and restore it, getting the session back;
  - `connect_at_launch` attaching a detached session;
  - an offline row for a stopped container.

  Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- remote` on Linux
- [ ] 9.2 Run `cargo test --workspace --locked` and `cargo build --release --locked` on Windows and confirm both pass. Ask the Linux machine's session to run the same commands plus `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e` and `BRINDLE_REMOTE_DOCKER_TEST=1 cargo test -p brindle --locked remote::client`, and record the results

## Workflow follow-up

- Run `openspec validate terminal-server-remote --strict` before opening the PR.
- Archive the change after review (`openspec archive terminal-server-remote`), and delete `remote-connections-followups` in the same PR (its proposal says it is superseded).
