# Testing architecture

How Brindle's tests are organised so they prove behaviour, run in parallel, and fail only for real reasons. It governs the unit tests in every crate, the real-process tests in `remote-agent/tests/`, the protocol's transcript and fuzz tests, `tests/architecture.rs`, and the two opt-in suites, `tests/e2e/` and `tests/perf/`. The stance: a test checks a rule of the system, every run is hermetic, and a flaky test is a bug with a cause, never something to retry. Code review enforces the **Rules** section; reviewers cite rules by number (`testing R8`).

## Why

Brindle's GUI can only be checked through a window, and a window on the user's desktop steals focus and takes whatever they are typing. Its server tests fork processes from multi-threaded test binaries, where a file descriptor open in one thread leaks into another thread's child. Both made tests that passed alone and failed in CI. The fixes that held were always structural: a hook instead of injected input, a private directory instead of a shared one, a cause found and measured instead of a retry.

## The idea

Each kind of test answers one question and has one home:

| Layer | Answers | Where |
| --- | --- | --- |
| unit tests on pure code | does this function or model decide correctly? | `#[cfg(test)] mod tests` in the same file, every crate |
| transcript tests | is this message's encoding still byte for byte? | `remote-protocol/` |
| fuzz tests | does any input make a decoder or parser panic? | `remote-protocol/src/fuzz.rs`, `term-core/src/fuzz.rs` |
| architecture tests | do the crates still point the right way? do platform `cfg`s stay in their seams? | `tests/architecture.rs` |
| real-process tests | do the real holder, core and relay cooperate? | `remote-agent/tests/` |
| end-to-end cases | does the shipped app behave, seen from outside? | `tests/e2e/` (opt-in) |
| performance cases | how fast, compared with last time? | `tests/perf/` (opt-in, numbers only) |
| gallery checks | does each `brindle-ui` component meet its scenarios? | `cargo run -p brindle-ui --example gallery -- --check` |
| manual checks | what needs real keys or a pointer | the list in `CLAUDE.md`, "Manual checks" |

## Idioms

**Driving the app through its hooks:**

```sh
brindle --send 'echo hi\r' --wait-for 'hi' --action split_zone_right --key ctrl-shift-t --dump-screen-after 5
```

`--key` goes through the keymap and focus as a real key does; `--action` bypasses the keymap. The dump is line-oriented (`--- grid`, `--- zones`, `--- focus`, `--- wait-for timeout "TEXT"`), so a failed run says which wait failed.

**A hermetic case:** its own config, `BRINDLE_STATE_DIR` and `BRINDLE_SERVER_DIR` (under `/tmp`, because socket paths must fit 108 bytes), `BRINDLE_NO_SCOPE=1`, artifacts kept in `target/tmp/e2e/<case>/`, the server stopped afterwards, and a role still running fails the case.

**Time passed in, so nothing sleeps:**

```rust
#[test]
fn bursts_are_paced_after_four_quick_updates() {
    let t0 = Instant::now();
    let (quick, wait) = step(Some(t0), BURST, t0 + Duration::from_millis(1));
    assert_eq!(quick, BURST + 1);
    assert!(wait.is_some());
}
```

**Asserting the rule, not a count:** the `drain` test records when each callback started and ended and checks that drain takes another item only while the budget hasn't run out. That holds on any machine; "exactly two items" did not.

**Platform pairs:** a server test that needs a Unix shell, exact PTY bytes, signals or `/proc` is `#[cfg(unix)]`, with a Windows counterpart beside it where the property matters on both. Test code is outside `ports R4`, so it may pair platforms this way: `#[cfg(test)]` items and the modules declared under one, `tests/**` and `remote-agent/tests/**`.

## How it applies here

| A change to | Ships with |
| --- | --- |
| a pure function or model | unit tests in its file |
| a pure helper in a harness (`tests/e2e/`, `tests/perf/`) | its `--self-test` |
| a wire message or field | a transcript test; older fixtures unchanged; the decoder in `fuzz.rs` |
| holder, core or relay behaviour | a real-process test in `remote-agent/tests/` |
| anything visible in the window | a named e2e case in `tests/e2e/cases/` |
| a `brindle-ui` component's behaviour | a gallery check, and a `ui/README.md` entry if it differs from shadcn |
| a crate boundary or dependency | `tests/architecture.rs` |
| something only real keys or a pointer can show | a line in `CLAUDE.md`'s manual checks, and in the PR body |

## Rules

- **R1.** A test MUST sit in the layer that answers its question, and pure code MUST be tested directly: values in, assertions on the value out, no window, server, fake or sleep. If a rule can only be reached through the e2e suite, it lives in the wrong place (`fcis R1`).
- **R2.** The app MUST be driven through its own debug hooks (`--send`, `--action`, `--key`, `--modality`, `--wait-for`, `--dump-screen-after`), never through synthetic OS input (XTest, xdotool, `SendInput`): injected input goes to whatever window has focus. A check that needs a new observation adds a line to the dump.
- **R3.** An expensive suite MUST be an opt-in `harness = false` target with its own runner: an environment variable to enable it, name filters, `--list`, a `--self-test` that tests the harness's own parsers, and `--shard K/N` taken round-robin after filters. Not libtest `#[ignore]`. The `--self-test` SHOULD also test the harness's other pure helpers, since `fcis R8` binds production code and doesn't reach harness code.
- **R4.** Every run MUST be hermetic: each case has its own config, state directory and server directory, touches nothing of the user's (their config, saved session, running `brindle-remote`) or another case's, keeps its artifacts in its own directory, and cleans up the processes it started. A process still running at the end fails the case.
- **R5.** Windows MUST NOT open on the user's desktop without asking first. Headless runs go through `scripts/with-xvfb` (a private Xvfb, openbox, lavapipe, `C.UTF-8`, a private session bus). Every delay scales with one factor (`BRINDLE_E2E_SLOW`), the app's own step interval included. An offset measured against one of the app's own unscaled limits is not a test delay: it is written unscaled, through a named helper, with the limit named beside it.
- **R6.** Expected values MUST come from fixtures the test controls: colours from the `e2e` theme in the generated config, never a built-in theme; content the case typed itself.
- **R7.** Unit and real-process tests MUST pass in parallel: no fixed sleeps, no shared temporary names, no process-wide environment changes (`ports R10`), no dependence on test order. Each test has its own temporary directory with a cleanup guard. To wait, poll for the condition within a limit scaled by `BRINDLE_TEST_SLOW` (e2e: `BRINDLE_E2E_SLOW`), through `--wait-for` or the suite's wait helper, not padding.
- **R8.** A test MUST assert the rule, not a number that depends on timing or machine speed.
- **R9.** A flake MUST be fixed at its cause. No whole-case retry, no skip, no longer wait on passing runs. A bounded retry is allowed only for a proven OS-level transient, with the cause written beside it (a forked child holds a lock's open file description until its `exec`). A flake not yet diagnosed is filed as an issue, and rerunning it in CI is triage, not a fix.
- **R10.** A flake fix MUST be shown by numbers: reproduce it in a loop or a scratch miniature, and report failures over N runs before and after ("0 in 3,000, against 99 before"), looping whole test binaries in parallel as CI runs them.
- **R11.** Confidence MUST be labelled honestly. When a cause is inferred and not observed, say so, ship the mitigation with diagnosis that makes the next failure explain itself, and don't claim a cure. Results a subagent or another machine reported are labelled as theirs, and checks that couldn't run stay open.
- **R12.** Tests SHOULD use a stand-in at a seam (`BRINDLE_NOTIFY_LOG`, a private server directory, a Docker container for a remote host) rather than a mocking layer. Brindle has no mocks.
- **R13.** A claim about behaviour MUST be a runnable test before it is relied on, and a GUI check MUST be a named case in `tests/e2e/`, never a one-off script. A disputed edge case is settled by a small executable spike.
- **R14.** The performance suite MUST report numbers against a baseline and never pass or fail. Each run writes a timestamped `summary.toml`; `BRINDLE_PERF_BASELINE` prints the change; the benchmark tool is pinned to a commit. A change made for speed is measured against `main` before it is accepted.
- **R15.** Tests MUST be named for the behaviour they check, as a sentence (`drain_stops_at_the_budget`, `a_foreign_uid_is_closed_without_being_read`), and e2e cases by feature with their area's prefix (`local-server-*`, `remote-*`, `windows-*`).
- **R16.** What automation can't reach (IME, wheel scrolling, drags, a real pointer press) MUST stay on the manual list in `CLAUDE.md` and be listed in the PR body. It is handed to the user with `scripts/run-throwaway`, not approximated with injected input.
- **R17.** A test that can only compile or run on one platform SHOULD have its counterpart on the other where the property matters there, and its first run on that platform is treated as its first run: don't report it as verified from the other.

## Anti-patterns

- **`sleep(2)` before an assertion, or extra `--step-interval` padding.** Instead: `--wait-for`, or poll the condition (R7).
- **Raising a timeout that passing runs also wait for.** Instead: a wait that ends when the condition holds (R9).
- **"Rerun until green" as the fix.** Instead: an issue, a reproduction, a cause, before and after numbers (R9, R10).
- **A shell script that launches Brindle and greps its output.** Instead: an e2e case (R13).
- **Expected colours copied from a built-in theme.** Instead: the `e2e` theme (R6).
- **A unit test that sets `XDG_RUNTIME_DIR` for the whole process.** Instead: a constructor argument (R7, issue #43).
- **Running the e2e suite against the user's server directory or state.** Instead: per-case directories (R4).
- **Several harness processes in one CI job.** They compete for the software GPU and bring the timing flakes back. Instead: shards across jobs (R3).
- **Captures timed from process spawn.** Instead: count from the window's first appearance, on the app's own clock.

## Enforcement

- `tests/architecture.rs`: `the_opt_in_suites_are_harness_false_targets` and `each_opt_in_runner_has_its_variable_list_and_self_test` (R3: each runner names its `BRINDLE_<SUITE>` variable, `--list`, `--self-test` and `--shard` in code, `--shard` through the shared `Selection::parse`, whose own `--shard` the same test checks; `a_runner_without_a_self_test_is_found` and `a_runner_without_shard_is_found` test the search), and `platform_cfgs_stay_in_their_seams`, which holds `ports R4` in production code and leaves test code's platform pairs alone.
- Both harnesses hold R4 through `tests/common/server.rs`, which gives each case private directories and a hermetic environment, refuses a missing or stale `brindle-remote`, and fails a case that leaves a `serve` or `hold` running or ran one in a systemd scope.
- CI runs `cargo test --workspace` and both harness self-tests on every push for Linux and Windows, `cargo clippy --workspace --all-targets -- -D warnings` on both, and the e2e suite in three advisory shards that upload `target/tmp/e2e/` on failure.
- `tests/architecture.rs`: `test_waits_go_through_the_wait_helper` (R7: no `std::thread::sleep` in `remote-agent/tests/**` outside `support/wait.rs`, the one module that sleeps, scaled, and none in `tests/e2e/cases/**`, whose waits go through `Scale`; `every_allowed_sleeper_exists` and `a_sleep_in_a_test_is_found` test the search, which follows imports and renames), and `tests_set_no_process_environment` (R7, `ports R10`: no `std::env::set_var` or `remove_var` in `tests/**`, `remote-agent/tests/**` or the test code of any crate's `src/`, with `a_set_var_in_a_test_is_found`).
- R1, R2, R5–R17 otherwise rely on review. No lint keeps `std::thread::sleep` or `Instant::now` out of the `#[cfg(test)]` modules in `src/`.

## PR checklist

- [ ] Each new test is in the right layer; pure code is tested without a window, server or sleep.
- [ ] GUI checks are named e2e cases driven by debug hooks, hermetic, with artifacts kept.
- [ ] No fixed sleep, shared temporary name, process-wide environment change or order dependence.
- [ ] Expected values come from fixtures the test controls.
- [ ] Assertions check the rule, not a timing-dependent count.
- [ ] A flake fix names its cause, shows before and after numbers, and adds no retry or longer wait.
- [ ] Results are labelled by who ran them; what couldn't run is listed as open, and manual checks are in the PR body.

## References

- The architecture plugin's `testing-architecture` skill (its Rust examples are this repo's).
- `CLAUDE.md`, "Verifying GUI behaviour", "End-to-end suite", "Manual checks", "Performance suite".
- `openspec/specs/` for the capabilities the e2e cases cover; GitHub issues #32, #36 and #43 for the flakes and hazards behind R7 and R9.
