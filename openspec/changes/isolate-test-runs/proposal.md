# Proposal

## Why

Some test runs reach outside their own directories (audit 2026-11-16, finding F2 and part of F11; `testing R4`, `R7`, `ports R10`):
- **Perf uses the user's terminal server** (b36 #1). `tests/perf/cases.rs:27-42` `spawn` sets no `BRINDLE_SERVER_DIR` and no `BRINDLE_NO_SCOPE`, and nothing stops a server afterwards. So both perf cases open sessions in the user's real terminal server and can leave them there.
- **Three e2e launches skip some of `Run`'s isolation** (b34 #1, #2, the b33 `Launcher` smell):
  - `restore.rs:130` `run_until_window_closes` and `remote.rs:1957` `run_brindle` build their `Command` by hand, without `BRINDLE_NO_SCOPE=1`. Their cores and holders start in `systemd-run --user --scope` units in the developer's or runner's own user manager.
  - `instance.rs:37` `Launcher::launch` does the same, though only on its failure path.
- **Unit tests change process-wide state that parallel tests read:**
  - Seven `std::env::set_var` calls steer code under test (`ports R10`, `testing R7`): `src/remote/local.rs:826-828, 865` (b24 #4) and `src/remote/client.rs:1254, 1533, 1798` (b23).
  - `src/session.rs`'s write-generation statics are shared by every directory (b27 #23). One test's newer ticket can make another test's write be skipped without an error, which is a parallel-order flake.
  - The scopes test `remote-agent/tests/core.rs:1746` sets `BRINDLE_NO_SCOPE=0`, which starts real scopes in the user's own systemd manager (b16 smell).

These are the defects in the remediation plan's first group (change 4). Change 11 (`deterministic-tests`) builds on the shared harness code this change adds.

## What Changes

- **One shared isolation and stop step for both GUI suites.** `tests/common/server.rs` gets three pieces, which the e2e and perf runners both use:
  - the case's private server directory;
  - the hermetic environment of a launch: config, state and server directories, `BRINDLE_NO_SCOPE=1`, `DISPLAY`, and no `WAYLAND_DISPLAY`;
  - the after-case stop-and-check now in `tests/e2e/main.rs`.
- The Windows process helper `tests/e2e/winproc.rs` moves to `tests/common/` with it.
- **Perf runs are hermetic:**
  - each case gets its own `/tmp/brindle-perf-<pid>-<n>` server directory and `BRINDLE_NO_SCOPE=1`;
  - the suite refuses a missing or stale helper;
  - after each case the server is stopped and the core's log kept, and a role still running fails the case.
- **Every e2e launch is hermetic:**
  - `restore-forget-on-exit`, `restore-close-window`, `remote-container-exec` and `remote-ssh-container` go through `Run`;
  - `Launcher::launch` and the Windows `run_with` take the shared environment;
  - the stop step also fails a case whose core or holder is running in a systemd scope (Linux).
- **No unit test sets the process environment:**
  - The ignored real-core test in `src/remote/local.rs` builds its `Endpoint` from values and lists sessions through that endpoint.
  - An `Endpoint` built from values tells the cores it starts their directory and `BRINDLE_NO_SCOPE=1`. Today the started core reads both from the environment it inherits, which is why the test set them process-wide. On Windows that means passing an environment block, so the agent's `environment_block` encoder moves to `brindle-remote-protocol` for both binaries.
  - The relay's helper lookup becomes a constructor argument, so the Docker transport tests pass their helper instead of setting `BRINDLE_REMOTE_AGENT`.
- **Session writes are ordered per state directory.** A `session::StateWrites` value replaces the two statics behind `next_generation` and `write_atomic`, and `record_session` holds one.
- **The scopes test is opt-in** (`BRINDLE_SCOPE_TEST=1`). CI's `test` job sets it, since its runner is disposable.
- **Docs:** `testing`'s Enforcement section says both harnesses hold R4. CLAUDE.md's perf, e2e and server-test paragraphs match.
- No change to the app's or the server's behaviour, wire protocols, config keys or state file. Production still reads `BRINDLE_REMOTE_AGENT` and `BRINDLE_SERVER_DIR` exactly as before.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The change touches test harnesses, test code and two internal seams (where the relay's helper comes from, and per-directory write ordering), with no user-visible behaviour. `.openspec.yaml` sets `skip_specs: true`. The `command-line` spec's "Terminal server directory for tests" requirement already covers the variable the harnesses set.

## Impact

- **Test harnesses:**
  - `tests/common/mod.rs`, new `tests/common/server.rs`, and `tests/common/winproc.rs` (moved from `tests/e2e/winproc.rs`);
  - `tests/e2e/main.rs` and `tests/e2e/brindle.rs` (`Run::run`);
  - the cases `tests/e2e/cases/{restore,remote,instance,win}.rs`;
  - `tests/perf/{main,cases}.rs`.
- **Shared crate:** a new pure `remote-protocol/src/environment.rs` (`environment_block`, moved from `remote-agent/src/conpty/cmdline.rs`, which re-exports it).
- **App code** (internal signatures only):
  - `src/remote/local.rs`: `Endpoint` gains the environment its roles get; `running_sessions` splits off a function that takes the endpoint; `shared` gets a sibling that takes the helper lookup.
  - `src/remote/local/{unix,windows}.rs`: `start` applies the endpoint's role environment.
  - `src/remote/local/relay.rs`: `Deployed` holds the lookup.
  - `src/remote/client.rs`: `launch` takes the lookup, and `Helper` is built from a path.
  - `src/session.rs`: `StateWrites` replaces `WRITE_GENERATION`, `WRITE_LOCK`, `next_generation` and the free `write_atomic`.
  - `src/main.rs` `record_session`.
- **Server tests and CI:** `remote-agent/tests/core.rs` (the scopes test) and `.github/workflows/ci.yml` (the `test` job's environment).
- **Docs:** `docs/architecture/testing-architecture.md` (Enforcement) and `CLAUDE.md`.
- **Overlap with the remediation plan:**
  - Change 1 (`keep-session-file-on-serialize-error`) edits `SessionStore::pending_write` and `record_session`'s `pending` helper. This change edits `record_session`'s `write` helper and its two `next_generation()` calls, a few lines apart. Whichever lands second rebases; neither changes the other's functions.
  - Change 8 (`list-sessions-off-ui-thread`) moves the caller of `running_sessions` (`src/main.rs:863`) off the UI thread. The endpoint-taking function split off here is what it can call.
  - Change 5 (`enforce-platform-seams`) exempts test code from `ports R4` (decision D1). The `cfg(windows)`/`cfg(unix)` items moving into `tests/common/` stay under `tests/**`, inside that exemption.
  - Change 11 (`deterministic-tests`) depends on this one. It can route the shared stop step's polls through its `eventually` helper and scale the `+ 30.0` deadline `Run` keeps (b34 #3). The plan lists b27 #23 under change 11 too. This change closes it, so change 11 keeps only b27 #25.
  - Change 12 (`surface-swallowed-failures`) reports `shared`'s connect failures (b24 #11). This change adds the helper-lookup sibling next to `shared` and changes neither failure path.
- **Overlap with changes in flight:**
  - `add-blocks` task 7.2 runs `tests/perf` against a baseline. After this change perf needs `brindle-remote` beside the release binary (`cargo build --release` builds it), and each case starts a fresh core. The measured path, keystroke to paint through a session in a core, is the same. A baseline from before this change ran in the user's server, possibly a busier one, so its run notes should say so.
  - `port-to-macos`, `add-pocket-client` and `update-dependencies` don't touch these files.
