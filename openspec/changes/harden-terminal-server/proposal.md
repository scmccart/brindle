## Why

Root-causing the flaky tests (#32, #36, `fix-flaky-tests`) turned up defects that weren't flakes. `terminal-server-remote` fixed three of them (default signal handling for session programs, draining a dead core, the ready pipe mistaken for an exec error). Two remain:
- A unit test sets `XDG_RUNTIME_DIR` for the whole test process, so whether the `remote::local` tests start real systemd scopes on a desktop depends on test order (#43).
- A hang-up closes the PTY before it signals, so a program blocked reading its terminal sometimes ends by "read failed, exit 1" instead of SIGHUP (#46, item 1). The spec already says a hang-up ends such a program with SIGHUP; today that holds about 3,999 times in 4,000.

Both are cheap to fix while the analysis is fresh.

## What Changes

- **A hang-up signals before it closes the terminal.** The holder sends SIGHUP to the session's process group first, then closes the PTY, then keeps the SIGKILL after the grace period. Signalling first gave a SIGHUP death in 24,000 of 24,000 tries. The session's exit is then reported the same way every time (killed by SIGHUP, no exit code), which `EXIT` and the notifications built on it rely on. On Windows the order of closing the pseudoconsole and the holder's pipe ends changes the same way, with no visible effect: the close already runs on a thread of its own.
- **Tests that need hang-ups to end by SIGHUP say so.** `TestCore::hang_up_and_wait` and `TestClient::hung_up` stop accepting "either ending" on Linux and check for SIGHUP; a new holder test hangs up a blocked `cat` many times and expects SIGHUP each time.
- **Unit tests don't change the process environment.**
  - The instance socket's path can be given to `listen` and `forward` rather than read from `XDG_RUNTIME_DIR`, so `instance`'s test needs no `set_var`.
  - Starting a core takes the scope decision from the `Endpoint` instead of the environment. `Endpoint::at` (the tests' constructor) never uses a scope, so the `remote::local` tests that start stand-in cores can't create `brindle-serve-*.scope` units, whatever ran before them.
  - The remaining `set_var` calls in tests stay where they are, because they are in opt-in tests that run alone (`a_real_core_starts_from_a_copy_and_answers`, ignored) or under the Docker test's own lock (`BRINDLE_REMOTE_DOCKER_TEST`). Each gets a comment saying why that is safe.

The GPUI window fixes found in the same analysis (#45) need a gpui upgrade and belong to `update-dependencies`.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `terminal-server`: "Programs start with default signal handling" gains the hang-up order: a program blocked reading its terminal ends by SIGHUP, not by a failed read.

The environment fix is test-only and changes no requirement.

## Impact

- `remote-agent/src/server/holder.rs` (`Holder::hang_up`): signal, then drop the PTY.
- `remote-agent/tests/support/mod.rs` and the `hung_up` helpers in `remote-agent/tests/{holder,core,manage,upgrade}.rs`: stricter on Linux; `holder.rs` gains a repeated blocked-`cat` test.
- `src/instance.rs` (Unix): `listen` and `forward` resolve the socket path and call path-taking forms, which the test calls with its own path.
- `src/remote/local.rs`, `src/remote/local/unix.rs`: `Endpoint` carries whether to use a scope; `from_env` asks `scope::current()`, `at` says no.
- `src/remote/client.rs` tests: comments only.
- Issues: closes #43 and #46 item 1.
