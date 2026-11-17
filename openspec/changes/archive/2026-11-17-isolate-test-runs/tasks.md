# Tasks

## 1. Shared crate: the Windows environment block (`remote-protocol`, `remote-agent`)

- [x] 1.1 Move `environment_block` from `remote-agent/src/conpty/cmdline.rs:47` into a new pure module `remote-protocol/src/environment.rs` (`pub mod environment;` in `remote-protocol/src/lib.rs`, no `cfg`), with its three tests: `environment_is_sorted_case_insensitively`, `environment_values_keep_non_ascii_text` and `an_empty_environment_is_two_nuls`. Then:
  - in `conpty/cmdline.rs`, replace it with `pub use brindle_remote_protocol::environment::environment_block;`, so `conpty/console.rs:24` doesn't change;
  - give the module a doc line saying both binaries use it (the holder for ConPTY programs, Brindle for a core started from a given endpoint).
  - Verify: `cargo test -p brindle-remote-protocol environment` runs the three tests on Linux; `cargo test -p brindle-remote-agent` passes; `cargo test -p brindle --test architecture` passes, since no dependency was added (`ports R3`).

## 2. Session writes ordered per state directory (`src/session.rs`, `src/main.rs`)

- [ ] 2.1 Before changing anything, take the flake's numbers (`testing R10`):
  - build the tests with `cargo test -p brindle --locked --no-run`;
  - run the `brindle` unit-test binary that build names 200 times, 8 at a time, as CI runs whole binaries (`seq 200 | xargs -P8 -I{} <binary> -q`);
  - count failed runs, and which tests failed (`atomic_write_and_load`, `old_tmux_entries_are_skipped`, …).
  - Record the count for the PR, 0 included.
- [x] 2.2 Reproduce b27 #23 deterministically on the old code: add `a_newer_write_elsewhere_does_not_skip_this_directorys_write` to `src/session.rs`'s `mod tests`. It works in two directories from `tmp_dir`:
  - take a ticket for `a`;
  - write `b` with a later `next_generation()`;
  - write `a` with the first ticket;
  - assert that `load(&a)` returns `a`'s state.
  - Verify: `cargo test -p brindle session::tests::a_newer_write` fails on the statics. Keep its output for the PR as the reproduction.
- [x] 2.3 Add `session::StateFile` (design.md decision 6) and remove `WRITE_GENERATION`, `WRITE_LOCK`, `next_generation` and the free `write_atomic` (`:310-333`):
  - `StateFile::new(dir)`, `ticket()`, `write(text, ticket)` (skips a ticket below the landed one; `landed: Mutex<Option<u64>>`, `None` before the first write, `types R7`) and `path()`;
  - port 2.2's test to two `StateFile`s, plus `atomic_write_and_load` (its old-ticket check on one `StateFile`) and `old_tmux_entries_are_skipped`, each test making its own.
  - Verify: `cargo test -p brindle session::` passes, 2.2's test included.
- [x] 2.4 In `record_session` (`src/main.rs:663-715`):
  - make one `Arc<StateFile>` from `session::state_dir()`;
  - change the `write` helper to `write(file: &StateFile, text, ticket)`, logging `file.path()` on an error as now;
  - give the tick's background task and the `on_app_quit` handler their own clones, with tickets from `file.ticket()`.
  - Verify: `cargo build --locked` has no warnings, and `rg -n 'next_generation|write_atomic' src` finds only `desktop.rs`'s own `write_atomic`.
- [ ] 2.5 Run the 2.1 loop again on the new code and record "N failed runs in 200, against M before" for the PR. If M was 0, say so: the evidence is then 2.2's reproduction (`testing R11`).

## 3. A given endpoint tells the cores it starts (`src/remote/local.rs`, `src/remote/local/{unix,windows}.rs`)

- [x] 3.1 Test first, `#[cfg(unix)]`, in `src/remote/local.rs`'s `mod tests`: `a_given_endpoint_tells_the_core_its_directory_and_no_scope`.
  - Use a `TempDir::script` helper that runs `printf '%s %s' "$BRINDLE_SERVER_DIR" "$BRINDLE_NO_SCOPE" > <file>; exit 1`.
  - `connect(&Endpoint::at(helper, dir), "t")` fails; assert the file holds `"<dir> 1"`.
  - Verify: `cargo test -p brindle a_given_endpoint` fails until 3.2.
- [x] 3.2 Add `role_env: Vec<(&'static str, OsString)>` to `Endpoint` (`local.rs:405-418`), documented as the variables set on every role it starts, beside this process's environment:
  - `from_env` leaves it empty;
  - `at` sets `BRINDLE_SERVER_DIR=<dir>` and `BRINDLE_NO_SCOPE=1`, and its doc says so;
  - `local/unix.rs` `start` (`:38`) applies it with `Command::envs` before `spawn`;
  - `local/windows.rs` `start` (`:61`) and `create` (`:185`) pass `None` as today when it is empty. Otherwise they pass the block that `environment_block` (task 1.1) makes from `terminal::cmdline::merge_environment(<this process's UTF-8 variables>, <role_env>)`, with `CREATE_UNICODE_ENVIRONMENT`.
  - Verify: 3.1 passes; `cargo test -p brindle remote::local` passes; the Windows arm compiles in CI's `windows` job.
- [x] 3.3 Split `running_sessions(limit)` (`local.rs:379-401`) into `running_sessions_at(endpoint: &Endpoint, limit)`, with `running_sessions` calling it with `Endpoint::from_env()?`. Its callers (`src/main.rs:584`) don't change, and its doc keeps "it never starts one". Verify: `cargo build --locked` has no warnings.
- [x] 3.4 Rewrite `a_real_core_starts_from_a_copy_and_answers` (`local.rs:813-891`):
  - remove both `unsafe { std::env::set_var(…) }` blocks (`:825-829`, `:865`);
  - build `Endpoint::at(helper.clone(), dir.clone())`, and for the newer Windows build `Endpoint::at(newer.clone(), dir.clone())`;
  - call `running_sessions_at(&endpoint, Duration::from_secs(5))`;
  - drop `assert_eq!(endpoint.dir, dir)`;
  - reword the doc comment so it no longer says the test sets the environment or runs alone, keeping `#[ignore = "needs BRINDLE_TEST_SERVER_HELPER, a built brindle-remote"]`.
  - Verify on Linux, after `cargo build --locked`:
    - `brindle-remote status --json` against the user's own server, before and after, shows the same core pid (or `{"running":false}` both times);
    - `BRINDLE_TEST_SERVER_HELPER=$PWD/target/debug/brindle-remote cargo test -p brindle --locked -- --include-ignored` passes with every other test running beside it.
- [ ] 3.5 Hand 3.4's command to a Windows machine (`target\debug\brindle-remote.exe`), the first run of the environment-block path (`testing R17`), and record its result in the PR as that machine's (`testing R11`).

## 4. The relay's helper as a constructor argument (`src/remote/client.rs`, `src/remote/local/relay.rs`, `src/remote/local.rs`)

- [x] 4.1 Test first, in `client.rs`'s outer `mod tests` (not `mod docker`): `a_relay_launch_deploys_the_helper_its_lookup_names`.
  - Write a file with known bytes to a temporary directory.
  - Call `launch(&container_of("docker", None), &lookup, |command| …)`, where the lookup returns that file. The `start` closure records `line(&command)` and returns `Err(Unstarted { code: Some(1), … })`.
  - Assert the recorded launch command names `/tmp/brindle-helper-<version>-<sha256 of the file>`.
  - Verify: it fails to compile until 4.2.
- [x] 4.2 Implement design.md decision 5:
  - `pub(crate) type HelperLookup = Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>`, and `pub(crate) fn default_helper() -> HelperLookup` wrapping `agent_path`;
  - `Helper::find()` becomes `Helper::of(path: PathBuf)`;
  - `launch(target, helper: &HelperLookup, start)` calls the lookup at each launch;
  - `relay::Deployed` gains a `helper` field, `relay::dial(name, target)` passes `default_helper()`, and a new `relay::dial_with(name, target, helper)` takes one;
  - `local::shared(host)` becomes `shared_with(host, &default_helper())`, a `pub(crate)` sibling whose `Host::Remote` arm passes the lookup to `relay::dial_with`;
  - re-export `dial_with` beside `dial_relay` (`local.rs:53`).
  - Verify: 4.1 passes; `cargo test -p brindle remote::` passes.
- [ ] 4.3 Update `client.rs`'s `mod docker` (`:1214-…`):
  - add `fn static_helper() -> PathBuf`, the musl helper path `opted_in` builds;
  - `opted_in` no longer sets `BRINDLE_REMOTE_AGENT` (`:1254`);
  - `connect` dials with `relay::dial_with(…, lookup of static_helper())`, and a `connect_with(target, path)` serves `scenario_bad_helper`, whose `set_var` (`:1533`) goes;
  - `docker_transport_scenarios` loses its restoring `set_var` (`:1798`);
  - `Tab::start` calls `Session::start_with(move || local::shared_with(&host, &lookup), request, sink)`;
  - the four `Helper::find()` calls (`:1517`, `:1877`, `:1903`, `:1953`) become `Helper::of(static_helper())`;
  - doc comments that say "the helper `BRINDLE_REMOTE_AGENT` names" say "the static helper".
  - Remove `DOCKER_TEST_LOCK` and its doc, unless a scenario shares a container, image or cache name with another test's; then keep it with that reason in its comment.
  - Verify: `rg -n 'set_var|remove_var' src` lists only `src/main.rs:305`; `cargo test -p brindle --locked` passes (the Docker tests return early).
  - With Docker and the musl helper built as README.md says, verify that `BRINDLE_REMOTE_DOCKER_TEST=1 cargo test -p brindle --locked remote::client` passes, and so does the same run without the `remote::client` filter.
- [x] 4.4 In CLAUDE.md's `BRINDLE_REMOTE_DOCKER_TEST` paragraph, add that the tests pass their helper to the relay's dial (`relay::dial_with`, `local::shared_with`), never through the environment. Verify by reading it against 4.2–4.3.

## 5. Shared harness module and the e2e suite (`tests/common`, `tests/e2e`)

- [x] 5.1 Create `tests/common/server.rs` (`pub mod server;` in `tests/common/mod.rs`). Start with the pure parts and their self-test (design.md decision 1):
  - `enum Suite { E2e, Perf }` and `private_dir(suite, case_dir, n)`;
  - `scoped_unit(cgroup: &str) -> Option<&str>`;
  - `pub fn self_test() -> Result`, covering:
    - `private_dir` distinct per suite and `n`, under `std::env::temp_dir()` on Linux and under `case_dir` on Windows;
    - `scoped_unit` on `0::/user.slice/user-1000.slice/user@1000.service/app.slice/brindle-serve-1a2b.scope` (that unit), a `brindle-hold-…` scope (that unit), a `session-3.scope` line (none), an `app-…` scope (none) and `""` (none).
  - Add `("server helpers", common::server::self_test)` to `tests/e2e/main.rs`'s `self_test`.
  - Verify: `BRINDLE_E2E=1 cargo test --locked -p brindle --test e2e -- --self-test` passes.
- [x] 5.2 Move the server code from `tests/e2e/main.rs` into `tests/common/server.rs`:
  - `HELPER_VARIABLE`, `helper_name`, `helper`, `files`;
  - `check_helper(helper, default)`, which takes the helper beside the suite's default binary;
  - `stop(helper, server_dir, case_dir)`, from `stop_server(ctx)`, with `save_logs`, `roles_left`, `end_roles`, both `server_processes` and the Windows `under` and its self-test.
  - `git mv tests/e2e/winproc.rs tests/common/winproc.rs` and declare it `#[cfg(windows)] pub mod winproc;` in `common/mod.rs`.
  - In `tests/e2e/main.rs`:
    - `#[cfg(windows)] use common::winproc;` and `use common::server::{server_processes, …}` (`under` on Windows), so the cases' `crate::winproc`, `crate::server_processes` and `crate::under` resolve unchanged;
    - `server_dir` becomes `private_dir(Suite::E2e, …)`;
    - the runner calls `common::server::stop(&ctx.helper, &ctx.server_dir, &ctx.dir)`.
  - Verify: `BRINDLE_E2E=1 cargo test --locked -p brindle --test e2e -- --self-test` and `-- --list` match before; CI's `windows` job builds and runs the e2e self-test, the first Windows run of the moved code (`testing R17`).
- [x] 5.3 Add `struct Isolation<'a> { config, state_dir, server_dir, display }` and `isolate(command, run)` to `tests/common/server.rs`. Make `Run::run` (`tests/e2e/brindle.rs:454-497`; the settings at `:466-491`) call it first, then the case's `env_remove` and `env`, in place of its own five settings. Verify: `cargo test --locked -p brindle --test e2e --no-run` builds with no warnings.
- [x] 5.4 In `stop`, on Linux, read `/proc/<pid>/cgroup` for each role `server_processes` finds before `brindle-remote stop`. Record a problem for any role whose `scoped_unit` is `Some`: "ran in a systemd scope: <role> <pid> in <unit>". Verify: 5.1's self-test covers `scoped_unit`, and 5.9 the wiring.
- [x] 5.5 Replace `restore.rs` `run_until_window_closes` (`:124-152`) with a `Run`: `.restore()`, the steps through `.send`/`.action`, `.dump_at(20.0)` and `.expect_exit()`, and its label `&'static str`. `close_window`'s run adds `.keep_detached()`. Verify in 5.10.
- [x] 5.6 Remove `remote.rs` `run_brindle` (`:1941-2012`) and `ChildGuard` (`:2015-2026`). Build `container_exec`'s and `ssh_container`'s runs with `Run` (design.md decision 2): profile, default profile, `.config(…)` tables, `BRINDLE_REMOTE_AGENT`, `BRINDLE_NOTIFY_LOG` at `<label>.notify`, steps, `dump_at`. Verify in 5.10.
- [x] 5.7 Make `instance.rs` `Launcher` (`:13-61`) gain `display` and call `isolate` in `launch`, keeping `current_dir`, stdio and the bounded wait. Verify in 5.10.
- [x] 5.8 Make `win.rs` `run_with` (`:314-…`, Windows only) call `isolate`, keeping `RUST_LOG` and the case's `env` after it. Verify: CI's advisory `windows-e2e` job; its first Windows run (`testing R17`).
- [ ] 5.9 Positive-direction check of 5.4, only with the user's go-ahead, since it starts one scope in their own user manager, which ends with the case:
  - comment out `BRINDLE_NO_SCOPE` in `isolate`;
  - run `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --locked -p brindle --test e2e -- restore-forget-on-exit`;
  - confirm the case fails with "ran in a systemd scope";
  - restore the line.
  - Without a go-ahead, record it as an open check in the PR (`testing R11`).
- [ ] 5.10 After `cargo build --locked`, which builds the helper, run `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --locked -p brindle --test e2e -- restore remote instance local-server`. With Docker available and the musl helper built, add `BRINDLE_REMOTE_E2E_REQUIRED=1`, so `remote-container-exec` and `remote-ssh-container` run instead of skipping. Verify that every selected case passes and that no `/tmp/brindle-e2e-*` directory is left.
- [x] 5.11 Update CLAUDE.md's "End-to-end suite" paragraph:
  - the isolation, server directories and stop step live in `tests/common/server.rs`, shared with perf;
  - a role still running in a systemd scope fails the case, as a role left running does;
  - `winproc` now lives in `tests/common/`.
  - Verify by reading it against 5.2–5.4.

## 6. The perf suite (`tests/perf`)

- [x] 6.1 Add `helper` and `server_dir` to perf's `Ctx` (`tests/perf/main.rs:52-60`). In `main`:
  - after the Linux check, take `common::server::helper(&bin)` and call `check_helper(&helper, &target_dir.join("release").join(<helper name>))`, exiting 1 with its message;
  - give each case `private_dir(Suite::Perf, &dir, n)`;
  - after each case call `common::server::stop`, turning a problem into the case's `FAILED` outcome as e2e does, so its numbers stay out of `summary.toml`;
  - add `server = "private per case"` to the run notes (`environment` or `summary.meta`);
  - add `("server helpers", common::server::self_test)` to perf's `self_test`.
  - Verify: `BRINDLE_PERF=1 cargo test --locked -p brindle --test perf -- --self-test` passes, on Linux here and in CI's `windows` job.
- [x] 6.2 Make `cases.rs` `spawn` (`:27-42`) call `isolate` (config, `ctx.dir.join("state")`, `ctx.server_dir`, `ctx.display`) in place of its own settings. Verify: `cargo test --locked -p brindle --test perf --no-run` has no warnings.
- [ ] 6.3 Run a hermetic perf case headless (no window on the desktop):
  - record `brindle-remote status --json` and `brindle-remote list` for the user's own server;
  - `cargo build --release --locked`;
  - `scripts/with-xvfb env BRINDLE_PERF=1 BRINDLE_PERF_SAMPLES=20 cargo test --locked -p brindle --test perf -- latency`.
  - Verify:
    - the case measures, or, if lavapipe can't, fails for that reason and not for the server;
    - `target/tmp/perf/latest/latency-shell/server-core.log` exists;
    - no `/tmp/brindle-perf-*` directory is left;
    - the user's server shows the same core and sessions as before.
- [x] 6.4 Docs:
  - CLAUDE.md's "Performance suite": each case gets `/tmp/brindle-perf-<pid>-<n>` with `BRINDLE_NO_SCOPE=1`, and is stopped and checked as e2e cases are; the suite needs `brindle-remote` beside the release binary (`cargo build --release` builds it); the run notes say `server = "private per case"`.
  - `docs/architecture/testing-architecture.md`'s Enforcement bullet "The e2e harness itself holds R4 …" becomes: both harnesses hold R4 through `tests/common/server.rs`, which gives each case private directories and a hermetic environment, refuses a missing or stale helper, and fails a case that leaves a `serve` or `hold` running or ran one in a systemd scope.
  - No rule's text changes, so `tests/architecture.rs` doesn't either.
  - Verify by reading both against 5.1–6.2.

## 7. The scopes test is opt-in (`remote-agent/tests/core.rs`, CI)

- [x] 7.1 In `holders_and_restarted_cores_run_in_scopes_of_their_own` (`remote-agent/tests/core.rs:1734-1758`):
  - return early, unless `BRINDLE_SCOPE_TEST` is `1`, with `eprintln!("skipped: set BRINDLE_SCOPE_TEST=1 to start scopes in this user's systemd manager")`, before the `scope::current()` check;
  - reword its opening comment.
  - Verify: `cargo test -p brindle-remote-agent --locked --test core holders_and_restarted -- --nocapture` prints the skip line and passes.
- [x] 7.2 Set `BRINDLE_SCOPE_TEST: "1"` in the `env` of the `cargo test --locked --workspace` step of `.github/workflows/ci.yml`'s `test` job (`:99`). Verify: the job is green on the PR.
- [x] 7.3 In CLAUDE.md's terminal-server "Tests" bullet, add that the scopes test runs only with `BRINDLE_SCOPE_TEST=1` (CI sets it), since it starts scopes in the user's own systemd manager. Verify by reading it.

## 8. Integration checks

- [ ] 8.1 Run these and check that they pass with no new warnings:
  - `cargo test --workspace --locked` with the default test threads;
  - `cargo build --release --locked`;
  - `BRINDLE_E2E=1 cargo test --locked -p brindle --test e2e -- --self-test`;
  - `BRINDLE_PERF=1 cargo test --locked -p brindle --test perf -- --self-test`.
  - Also verify that `rg -n 'set_var|remove_var' src ui term-core remote-protocol remote-agent tests` lists only `src/main.rs:305`.
- [ ] 8.2 On the PR, check these CI jobs:
  - `commits`, `test` and the title check are green;
  - the `windows` job is green, which is the first Windows run of `tests/common/{server,winproc}.rs`, the moved encoder's tests and the Windows `start` path's build;
  - the three `e2e` shards and `windows-e2e` are advisory. Read any failure, and fix it if it comes from this change.
- [ ] 8.3 Manual checks: none. Nothing here needs real keys or a pointer. Write "Manual checks: none" in the PR body, with 3.5's Windows run and 5.9's check stated as run or open (`testing R16`, `R11`).

## Workflow follow-up

- Land as one PR from a branch off `main`, titled `test: make perf, e2e and unit test runs hermetic` (at most 93 characters, lowercase subject). Put the archive commit (`docs(openspec): archive isolate-test-runs`) on the same PR. There is no spec delta to sync (`skip_specs: true`).
- The PR body carries:
  - 2.1/2.5's numbers and 2.2's reproduction;
  - 3.5's Windows result, labelled as that machine's;
  - 5.9's outcome;
  - that b27 #23 is closed here, so change 11 keeps only b27 #25.
