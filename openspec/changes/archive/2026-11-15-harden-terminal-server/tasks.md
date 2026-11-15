# Tasks

## 1. Signal before closing the terminal

- [x] 1.1 In `remote-agent/src/server/holder.rs`, have `Holder::hang_up` call `group.hang_up()` and queue the grace kill before it takes the PTY and deregisters its source, and update its doc comment; verify `cargo test -p brindle-remote-agent --test holder` passes on Linux
- [x] 1.2 Make the Linux hung-up checks strict: `support/programs.rs`'s `hung_up`, `holder.rs`'s `hung_up`, `TestClient::hung_up` and `TestCore::hang_up_and_wait` assert `code == None` on Unix (Windows keeps `is_some()`), drop the duplicate `hung_up` in `tests/core.rs` for `programs::hung_up`, and rewrite the doc comments that explain the EIO race; verify `cargo test -p brindle-remote-agent` passes
- [x] 1.3 Add a Linux holder test that hangs up a blocked `cat` 200 times in one holder and asserts each `EXITED` has no code, arrives within 500 ms and is logged as killed by SIGHUP; verify it passes with 1.1 (it does not reproduce the race on an idle machine: see design decision 2)
- [x] 1.4 Update CLAUDE.md's holder paragraph ("its `Hangup` sends SIGHUP then SIGKILL…") to say SIGHUP goes before the PTY is closed; verify the sentence matches `Holder::hang_up`

## 2. Keep tests out of the process environment

- [x] 2.1 In `src/instance.rs` (Unix), split `listen` and `forward` into wrappers over `listen_at(path, …)` and `forward_to(path, …)`, and have `a_forwarded_request_reaches_the_handler` use a socket path in its temp directory with no `set_var`; verify `cargo test -p brindle instance` passes and `grep -n set_var src/instance.rs` finds nothing
- [x] 2.2 Give `Endpoint` a Unix-only `scope: bool` (true from `from_env`, false from `at`) and have `remote::local::unix::start` use `scope::Decision::Plain("the endpoint uses no scope")` when it is false; verify `cargo test -p brindle remote::local` passes and, on a desktop session, that `systemctl --user list-units 'brindle-serve-*'` shows no unit after `cargo test -p brindle remote::local -- --test-threads=1`
- [x] 2.3 Review the remaining `set_var` calls in tests (`a_real_core_starts_from_a_copy_and_answers`, the `remote::client` Docker tests) and make their `SAFETY` comments say why each is safe, as in design decision 5; verify `grep -rn "set_var" src --include=*.rs` lists only those, `main.rs` and the ignored real-core test in `remote::local`

## 3. Integration

- [x] 3.1 Run `cargo test --workspace --locked` and `cargo build --release --locked` on Linux; verify both pass with no new warnings
- [x] 3.2 Run the Windows checks (`cargo test --workspace` on Windows, or the `windows` CI job) for the changed hang-up order; verify the holder and core tests pass (this PR's `windows` CI job; the merge waits for it to pass)
- [x] 3.3 Run `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- local-server` (after `cargo build`); verify the `local-server-*` cases pass

## Workflow follow-up

- Land through a squash-merged PR (branch, implementation commit, archive commit); the PR closes #43 and #46 item 1.
- Archive the change with spec sync after review.
