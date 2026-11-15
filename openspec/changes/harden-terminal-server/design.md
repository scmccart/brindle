## Context

`fix-flaky-tests` (#47) found five hazards in the terminal server; `terminal-server-remote` (its decision 12) fixed three. This change takes the last two, both on Linux:

- **Hang-up order (#46 item 1).** `Holder::hang_up` (`remote-agent/src/server/holder.rs`) takes the session's PTY (closing the master), then calls `Group::hang_up` (SIGHUP to the process group), then queues `Group::kill_remaining` for `GRACEFUL_SHUTDOWN` (1 s) later. Closing the master first makes the kernel fail a pending read on the slave with EIO and then raise SIGHUP for the session. A program blocked in `read`, such as `cat`, occasionally runs its "read failed, exit 1" path before the signal is delivered: about 1 in 4,000 hang-ups in the flaky-test analysis. Signalling first gave SIGHUP 24,000 times out of 24,000. Every test helper that waits for a hung-up session (`hung_up` in `tests/{holder,core}.rs`, `support/programs.rs`, `TestClient::hung_up`, `TestCore::hang_up_and_wait`) accepts any status on Linux because of this.
- **Leaked test environment (#43).** `instance::tests::a_forwarded_request_reaches_the_handler` sets `XDG_RUNTIME_DIR` for the whole test process. `remote::local::unix::start` reads it through `scope::current()`. The `remote::local` tests that start stand-in cores (`a_core_that_never_listens_is_unreachable`, `a_core_that_fails_to_start_shows_why`, `a_failed_execs_error_in_the_ready_pipe_is_a_core_that_didnt_start`) therefore run their script in a real `brindle-serve-*.scope` on a desktop when they run before the instance test, and directly when they run after it.

## Goals / Non-Goals

**Goals:**
- A hang-up ends a program that keeps default signal handling with SIGHUP every time, and the tests check that.
- No unit test outside the opt-in ones changes the process environment, and starting a core from a test never makes a systemd scope.

**Non-Goals:**
- Changing the Windows hang-up (`CTRL_CLOSE_EVENT`, `TerminateProcess`); its order changes only as a side effect of the shared holder code.
- Reporting which signal ended a program. `EXIT`'s `code` stays `None` for any signal; the SIGHUP check is "no code, and well before the SIGKILL".
- The GPUI fixes from #45 (`update-dependencies`).

## Decisions

### 1. Signal, then close
`Holder::hang_up` calls `group.hang_up()` (and queues the kill) before it takes the PTY and deregisters its source. The group exists until reaped, so signalling first is safe; reaped sessions still skip both. A program that ignores SIGHUP now meets an open terminal for an instant longer, then the EIO, then the SIGKILL as before.

On Windows `Group::hang_up` takes the pseudoconsole, which a thread of its own closes, so taking it before the holder drops its pipe ends only starts that thread a moment earlier. The holder already stops reading output at the hang-up either way. `hang_up_a_ping` and the Windows holder tests cover it.

*Alternative:* leave the order and keep the tolerant helpers. Rejected: the spec scenario "`cat` is ended by SIGHUP" is then true only statistically, and a status that varies run to run leaks into `EXIT`.

### 2. Strict helpers, in one place
The client-side `hung_up` in `tests/core.rs` duplicates `support/programs.rs`'s; `core.rs` uses the shared one. On Linux the helpers (`programs::hung_up`, `holder.rs`'s `hung_up`, `TestClient::hung_up`, `TestCore::hang_up_and_wait`) assert `code == None`. Windows keeps `code.is_some()`. A call site whose program legitimately exits with a code on a hang-up (a shell trapping HUP) switches to `exit_code` with its expected value; none is expected.

A new Linux holder test hangs up a blocked `cat` 200 times in one holder (a fresh session each round) and asserts `None`, an exit within 500 ms and a "killed by signal 1" log line each time. It checks the outcome; it can't show the race. Run against the old order on an idle machine, 4 runs of 2,000 rounds (43 s each) all passed: the window is inside the kernel's hang-up, between waking the reader and signalling the session, and the 1-in-4,000 rate needed the load of a full parallel test run. The evidence for the order stays the flaky-test analysis (24,000 of 24,000 with the signal first).

### 3. Inject the instance socket path
On Unix, `instance::listen` and `forward` become thin wrappers that compute `socket_path(state_dir)` and call `listen_at(path, …)` / `forward_to(path, …)`. The test calls those with a path in its temp directory. No public signature changes, so `main.rs` and Windows are untouched.

*Alternative:* pass the runtime directory through every caller. Rejected as churn for a test-only need.

### 4. The scope decision belongs to the `Endpoint`
`Endpoint` gains a Unix-only `scope: bool` (`cfg(not(windows))`, beside the Windows-only `copies`). `Endpoint::from_env` sets it true; `Endpoint::at`, used by tests and by nothing in production, sets it false. `unix::start` asks `scope::current()` only when `endpoint.scope` is true and otherwise uses `scope::Decision::Plain` with the reason "the endpoint uses no scope", which reaches the log as today's plain starts do.

*Alternative:* have the tests set `BRINDLE_NO_SCOPE=1`. Rejected: that is another process-wide `set_var`, the thing being removed.

### 5. The other `set_var` calls stay
- `remote::local::tests::a_real_core_starts_from_a_copy_and_answers` is `#[ignore]` and documented to run alone.
- `remote::client` Docker tests set `BRINDLE_REMOTE_AGENT` under a static mutex, only with `BRINDLE_REMOTE_DOCKER_TEST=1`. Another test in the same process that reads `BRINDLE_REMOTE_AGENT` (`find_server_helper`) could still see it; their `SAFETY` comments say so, and that the opt-in run is filtered to `remote::client`.
- `main.rs`'s `set_var("BRINDLE_CONFIG")` is not a test.

## Risks / Trade-offs

- [A program that handles SIGHUP by reading its terminal one last time now reads before EIO.] → It sees the open terminal, not a closed one, for the moment until the close; it is ended by the grace SIGKILL as before if it lingers. Shells re-raise SIGHUP after cleanup.
- [The strict helpers could turn a rare legitimate non-signal exit into a failing test.] → The repeated test shows the rate; the e2e and CI runs before landing would show any site that needs `exit_code`.
- [Windows order change.] → Covered by the Windows holder tests on CI's `windows` job; the close stays on its own thread.
