# Design

## Context

See proposal.md, "Why". The audit findings this change closes (`docs/architecture/audits/2026-11-16.md`, "Findings" 2 and 11; instance lists in `docs/architecture/audits/2026-11-16/`):
- **F2, `testing R4`:**
  - b36 #1, `tests/perf/cases.rs:27-42`;
  - b34 #1, `tests/e2e/cases/restore.rs:126-152`;
  - b34 #2, `tests/e2e/cases/remote.rs:1941-2012`;
  - the b33 smell on `tests/e2e/cases/instance.rs:37-61` (`Launcher::launch`);
  - the b34 smell naming their common cause: each hand-built spawn repeats the isolation settings, and misses some.
- **F11, the part in this change:**
  - `ports R10` and `testing R7`: b24 #4 (`src/remote/local.rs:820-891`) and b23's three sites (`src/remote/client.rs:1254, 1533, 1798`);
  - `testing R7` and `R8`: b27 #23 (`src/session.rs:310-333` and its tests);
  - `testing R4`: the b16 smell (`remote-agent/tests/core.rs:1734-1758`).

How things stand:
- **The e2e harness already isolates.**
  - `Run::run` (`tests/e2e/brindle.rs:454-575`) sets:
    - `BRINDLE_STATE_DIR`, `BRINDLE_CONFIG`, `BRINDLE_SERVER_DIR`;
    - `BRINDLE_NO_SCOPE=1`;
    - `DISPLAY`, with `WAYLAND_DISPLAY` removed;
    - then the case's own environment.
  - After every case, `tests/e2e/main.rs` cleans up:
    - `stop_server` (`:273-308`) runs `brindle-remote stop`, waits for the roles to go (`roles_left`), kills and reports any left (`end_roles`), and copies the core's log (`save_logs`);
    - `server_processes` (`:352-391`, one per platform, Windows through `winproc`) finds roles by their `BRINDLE_SERVER_DIR`;
    - `server_dir` (`:222-228`) puts Linux server directories under the temp directory, because of `sun_path`;
    - `check_helper` (`:232-257`) refuses a missing or stale helper.
- **The perf suite has none of this.** `tests/perf/` shares only `tests/common/mod.rs` (`Result`, `ensure!`, `trim_screen`), included with `#[path]`. Perf runs only on Linux (`main.rs:133`), but CI compiles it and runs its `--self-test` on Windows too (`build-windows.yml:66`).
- **A core started from a given `Endpoint` doesn't use that endpoint's directory.** `Endpoint::at(helper, dir)` (`src/remote/local.rs:455`, `#[cfg(test)]`) holds the directory, and on Linux it starts the core without a scope (`scope: false`). But `sys::start` (`src/remote/local/unix.rs:38`, `windows.rs:61`) passes the core neither `BRINDLE_SERVER_DIR` nor `BRINDLE_NO_SCOPE`. A real core, and the holders it starts, read both from the environment they inherit.
  - The tests that use `at` today run stand-in scripts, which read neither variable.
  - The one test with a real core, `a_real_core_starts_from_a_copy_and_answers`, sets both process-wide (b24 #4). That is why it is `#[ignore]` and "runs alone".
- **The relay's helper comes from `BRINDLE_REMOTE_AGENT`, read at each launch.** The chain is `client::launch` (`client.rs:358`) → `Helper::find` → `agent_path` → `remote::find_helper`, reached from `relay::Deployed::launch` (`local/relay.rs:120-128`), which `local::shared` makes for a remote `Host` (`local.rs:141-152`). The Docker tests (`client.rs`, `mod docker`) go through it, so they set the variable process-wide, taking turns under `DOCKER_TEST_LOCK`.
- **Write ordering is process-wide.** `WRITE_GENERATION` and `WRITE_LOCK` (`session.rs:310-311`) order the writes of every directory. A write whose ticket is lower than the last landed one is skipped silently (`:323-325`). `record_session` (`src/main.rs:663-715`) is the only production caller, with one state directory per process.

## Goals / Non-Goals

**Goals:**
- **One place for the isolation settings.** Every Brindle a GUI suite launches gets its hermetic environment from that one place, and every case of either suite ends with the same stop-and-check (`testing R4`).
- **No test changes process-wide state that another test reads:** no environment variable and no shared static (`testing R7`, `ports R10`). Tests substitute at a seam through a constructor argument instead.
- **The scopes test touches no user's systemd manager** unless asked to.
- **No change to what Brindle, its server or the relay do.**

**Non-Goals:**
- **An allow-listed environment for e2e launches.** Clearing the environment and passing back only a list would stop `HOME`, `XDG_*`, `BRINDLE_NOTIFY_LOG` and `BRINDLE_LINK_OPENER` leaking in (the design-smells entry on `tests/e2e/brindle.rs:476-496`). Which variables each case relies on is not known yet, and finding out is a change of its own.
- **Fixed sleeps, unscaled deadlines, timing asserts, silent skips and the perf `--shard`** (b34 #3–#4, b36 #2, the rest of F11). Change 11 (`deterministic-tests`) owns them. The shared stop step keeps its bounded 100 ms polls as they are, and `Run` keeps its unscaled `+ 30.0` deadline.
- **`src/main.rs:305`** `set_var("BRINDLE_CONFIG", …)`. m2 counts it among its 8 `set_var` sites, but it is production code in `parse_args_from` (b01 #11, `fcis R9`, SHOULD), not a test steering code. So this change has 7 sites, not 8.
- **A `tests/architecture.rs` search for `env::set_var`/`remove_var`.** It would keep the 7 sites from coming back, but it belongs with the test-source searches change 11 adds. Those are written on the needles change 7 reworks (grouped imports, full paths), so the search is written once there. The `testing` and `ports` Enforcement sections keep saying R7 and R10 rely on review until then.
- **b27 #25** (pid-named temporary directories without a guard): change 11's.

## Decisions

### 1. `tests/common/server.rs`: isolation and the stop step, shared by e2e and perf

What moves out of `tests/e2e/main.rs` into a new submodule of `tests/common/` (`pub mod server;` in `common/mod.rs`, resolved as `tests/common/server.rs` because the `#[path]` file is a `mod.rs`):
- `helper_name`, `HELPER_VARIABLE` and `helper(bin)`;
- `check_helper(helper, default)`, with `files`. It now takes the helper that sits beside the suite's default binary, since perf's binary is `target/release/brindle`, not `CARGO_BIN_EXE_brindle`.
- `stop(helper, server_dir, case_dir)`, which is today's `stop_server(ctx)` with its paths passed in, together with `save_logs`, `roles_left`, `end_roles`, both `server_processes` and the Windows `under`.

New in it:
- **`enum Suite { E2e, Perf }`** and `private_dir(suite, case_dir, n) -> PathBuf`. On Linux that is `temp_dir()/brindle-{e2e|perf}-<pid>-<n>`, and on Windows `case_dir/server`. The enum is closed, so a third suite has to name its own prefix.
- **`struct Isolation<'a> { config, state_dir, server_dir, display }`** and `isolate(command: &mut Command, run: &Isolation)`. It sets `BRINDLE_CONFIG`, `BRINDLE_STATE_DIR`, `BRINDLE_SERVER_DIR`, `BRINDLE_NO_SCOPE=1` and `DISPLAY`, and removes `WAYLAND_DISPLAY`.
  - Callers apply it first and the case's own removals and additions after. A case can still override it, as `local-server-elevated`'s `.env("BRINDLE_SERVER_DIR", "")` does through `Run::env`.
  - No case removes a variable `isolate` sets. The removals today are `BRINDLE_REMOTE_AGENT`, `BRINDLE_SERVER_HELPER` and `TMUX`.
- **`scoped_unit(cgroup: &str) -> Option<&str>`**, pure: the unit of a `/proc/<pid>/cgroup` line when it is a `brindle-serve-*.scope` or `brindle-hold-*.scope`. The Linux stop step reads each role still running before it stops the server, and records a problem for any role in such a unit ("ran in a systemd scope: serve 123 in brindle-serve-….scope"). So a launch that misses `isolate` fails its case on any machine with a user manager, the developer's included, instead of passing quietly.
- **`self_test() -> Result`** for `private_dir` (distinct per suite and per `n`, under the temp directory on Linux, under the case directory on Windows) and `scoped_unit` (a Brindle scope, a session scope, an app scope, an empty file). Both runners' `--self-test` list it. These are harness helpers, held by `--self-test` (decision D3).

`tests/e2e/winproc.rs` moves to `tests/common/winproc.rs` (`#[cfg(windows)] pub mod winproc;`), since the Windows `server_processes` and `end_roles` need it.
- It already names only `crate::Result` and `crate::ensure`, which both runners provide (`pub use common::Result`, and `ensure!` is `#[macro_export]`).
- e2e keeps its paths with `#[cfg(windows)] use common::winproc;` and `use common::server::{server_processes, …}` in `main.rs`, so `crate::winproc::…`, `crate::server_processes` and `crate::under` in the cases don't change.
- `common/mod.rs`'s `#![allow(dead_code)]` covers what perf doesn't use.

*Rejected:*
- **A copy of the e2e code in perf:** repeating the settings by hand is what caused b34 #1–#2.
- **Perf including `tests/e2e/main.rs`'s functions by `#[path]`:** they read e2e's `Ctx`, and a test target can't import another.
- **A Linux-only shared module:** e2e needs the Windows arm, and perf compiles on Windows for its self-test.

The `cfg(unix)`/`cfg(windows)` items stay in test code under `tests/**`, which decision D1 puts outside `ports R4` (change 5).

### 2. Every launch goes through `isolate`; the two hand-built e2e spawns go through `Run`

- **`Run::run`** calls `isolate` in place of its own five lines.
- **`restore.rs` `run_until_window_closes`** becomes a `Run`: `.restore()`, the steps through `.send` and `.action`, `.dump_at(20.0)` (reached only if the window doesn't close) and `.expect_exit()`. `Run` already fails such a run if it prints a dump or exits unsuccessfully.
  - `restore-close-window`'s run adds `.keep_detached()`. Its next run, `reopen`, prints `pwd` in the session the quit detached; without it, `Run`'s hang-up would end that session.
  - `restore-forget-on-exit`'s tab exits, so `Run`'s hang-up finds nothing.
  - The label parameter becomes `&'static str`, as `Run::new` takes.
- **`remote.rs` `run_brindle`** goes, and its callers `container_exec` and `ssh_container` build a `Run` as `Needs::run` does:
  - `.profile(P)` and `.default_profile(P)`, where `P` is `CONTAINER_PROFILE` or `SSH_PROFILE`;
  - `.config(…)` with the profile tables (the container case's `[notifications]` table included);
  - `.env("BRINDLE_REMOTE_AGENT", …)` and `.env("BRINDLE_NOTIFY_LOG", <label>.notify)`;
  - the steps through `.wait_for`, `.send` and `.action`, then `.dump_at(…)`.
  - `ChildGuard` goes with `run_brindle`, its only user; `Run` has `Reaped`.
  - `Run` adds `--no-restore` after `-p`. That is the same here, since the case's state directory is new.
  - `Run`'s normal path checks for a screen in the dump, not the exit status that `run_brindle` also checked. A Brindle that failed before its dump has no screen, and every other `Run` in the suite relies on the same check.
- **`instance.rs` `Launcher::launch`** calls `isolate` and gains a `display` field.
  - It can't be a `Run`. `Run` always passes debug options (`--dump-screen-after`, `--no-restore`, the step interval), and a launch with any debug option never hands itself to the running Brindle, which is what the case checks.
  - `DISPLAY` and the `WAYLAND_DISPLAY` removal now come with `isolate` too. If a hand-off fails, the stray Brindle opens on the case's display, not on the user's Wayland desktop (`testing R5`).
- **`win.rs` `run_with`** (Windows only) calls `isolate`. It already sets the three directories, and scopes don't exist there. It is the last hand-built launch, and using the shared helper means the next isolation setting reaches it too.

*Rejected:* adding `.env("BRINDLE_NO_SCOPE", "1")` to each spawn (b34's smallest fix). The cause would stay, and the next isolation setting would be missed in the same places.

### 3. Perf: a private server per case, stopped and checked

- `Ctx` (`tests/perf/main.rs:52-60`) gains `helper` and `server_dir`.
- `main` takes the helper with `common::server::helper(&bin)`. Before running a case it calls `check_helper(&helper, &target_dir.join("release/brindle-remote"))`: a missing helper, or the release one older than its sources, ends the run with exit 1 and a message saying `cargo build --release` builds it.
- Each case gets `private_dir(Suite::Perf, &dir, n)`.
- `cases.rs` `spawn` calls `isolate`.
- After each case `main` calls `stop`. As in e2e, a problem fails a case that measured. Its numbers then stay out of `summary.toml`, because a run that left a role behind or ran in the user's manager isn't one to compare (`testing R4`).
- The run notes gain `server = "private per case"`. A baseline taken before this change ran in the user's server, and the notes now tell the two apart (`testing R14`).

### 4. An `Endpoint` given its values tells the roles it starts

`Endpoint` gains `role_env: Vec<(&'static str, OsString)>`, the variables set on the command of every role it starts, on top of this process's environment.
- **`from_env`** leaves it empty. The roles inherit this process's environment, which is where the endpoint came from, so production starts exactly as before.
- **`at(helper, dir)`** sets `BRINDLE_SERVER_DIR=<dir>` and `BRINDLE_NO_SCOPE=1`. A real core started from it then serves `dir` and starts its holders without scopes, as the process-wide variables made it do.
- **Both `sys::start` arms apply it** (`ports R4` asks for an item in both sets of a per-platform module set):
  - `local/unix.rs:38` sets it with `Command::envs` before `spawn`.
  - `local/windows.rs:61` starts the trampoline with `CreateProcessW` itself (`create`, `:185-230`) and passes no environment, so the trampoline inherits Brindle's. With `role_env` empty it still passes none, so production is unchanged. Otherwise it passes an environment block with `CREATE_UNICODE_ENVIRONMENT`: this process's UTF-8 variables merged with `role_env` by `terminal::cmdline::merge_environment` (names compared case-insensitively, the endpoint's winning, sorted as `CreateProcessW` requires), then encoded by `environment_block`. The trampoline passes its environment on to the role.
- **The encoder moves to a shared crate.** `environment_block` is the agent's (`remote-agent/src/conpty/cmdline.rs:47`), where the holder uses it for ConPTY programs. Both binaries now need it, and they never reference each other, so it moves with its tests to a new pure module `remote-protocol/src/environment.rs` (`ports R1`, `R3`: no new dependency). `conpty::cmdline` re-exports it, so the agent's callers don't change.
- **Windows copies:** given `BRINDLE_SERVER_DIR`, the core puts its per-build copies in `<dir>\bin`, which is where `at` already puts them (`helper_copies(Some(&dir), …)`).

`running_sessions(limit)` becomes `running_sessions_at(&Endpoint::from_env()?, limit)`, and the test calls `running_sessions_at(&endpoint, …)`. The ignored test then builds both its endpoints with `at`, the newer Windows build's included. It drops `assert_eq!(endpoint.dir, dir)`, since `server_dir::resolve`'s own tests hold that choice. It keeps `#[ignore]` only because it needs a built helper.

*Rejected:*
- **Passing `BRINDLE_SERVER_DIR` to every core:** without a named directory a Windows core keeps its copies in `%LOCALAPPDATA%\brindle\server`, but with one in `<dir>\bin`, so production would change. Production would also switch from an inherited environment to a rebuilt block on Windows.
- **A second encoder in the app:** a copy of a function the other binary has is what the dependency rule's "what both need goes in a shared crate" forbids.
- **Making the test `#[cfg(unix)]`:** its Windows checks (the core runs from a per-build copy, and a newer build replaces it) would be lost (`testing R17`).
- **A closed enum for the endpoint's origin:** its `at` variant would be built only under `cfg(test)`, which is a dead-code warning in `cargo build`. Brindle builds with none.

### 5. The relay's helper is a constructor argument

- **The type:** `pub(crate) type HelperLookup = Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>` in `client.rs`.
- **Taking it:**
  - `client::launch(target, helper: &HelperLookup, start)` calls it at each launch, as `Helper::find` read the variable at each launch.
  - `Helper::find()` becomes `Helper::of(path)`, which hashes the file it is given.
  - `relay::Deployed` holds a lookup, and `relay::dial_with(name, target, helper)` makes one with it.
  - `relay::dial(name, target)` passes `client::agent_path` (`BRINDLE_REMOTE_AGENT`, else beside the executable, unchanged).
  - `local::shared(host)` becomes `shared_with(host, &default)`, where only a remote host's arm reads the lookup.
- **What the Docker tests pass:**
  - `fn static_helper() -> PathBuf` names the musl helper, the constant path `opted_in` builds today.
  - `connect` and `counted_launch` use it.
  - `Tab::start` calls `Session::start_with(move || local::shared_with(&host, &lookup), …)`, so tabs still share one connection per endpoint through the process's map. The test containers' names are unique, so their hosts don't clash with any other test's.
  - `scenario_bad_helper` passes a lookup naming its bad file, and the `set_var` that restored the variable afterwards (`:1798`) goes.
- **The lock:** `DOCKER_TEST_LOCK` existed for the variable. It goes unless a scenario shares a container, image or cache name with another test's; then it stays, with that reason in its comment.

*Rejected:*
- **`enum HelperSource { Found, At(PathBuf) }`:** `At` is built only by tests, the same dead-code warning as in decision 4.
- **A process-wide override slot** (`OnceLock<PathBuf>`): still process-wide state (`testing R7`).

### 6. Write ordering is per state directory

`session::StateFile`, a GPUI-free value, replaces the two statics, `next_generation` and the free `write_atomic`:

```rust
/// One state directory's `session.toml`, written in ticket order: a write
/// is skipped when a newer ticket has landed, so a slow background write
/// can't replace the final one on quit.
pub struct StateFile {
    dir: PathBuf,
    issued: AtomicU64,
    /// The ticket of the last write that landed; `None` before the first.
    landed: Mutex<Option<u64>>,
}
impl StateFile {
    pub fn new(dir: PathBuf) -> Self;
    pub fn ticket(&self) -> u64;
    pub fn write(&self, text: &str, ticket: u64) -> std::io::Result<()>;
    pub fn path(&self) -> PathBuf;
}
```

- `record_session` makes one `Arc<StateFile>` from `session::state_dir()`. It moves a clone into the tick's background writes and one into the quit handler, and its `write` helper takes the `StateFile`.
- Each test makes its own, so tests no longer share an ordering.
- **`landed: Option<u64>`:** today `0` stands for "nothing written yet" only because tickets start at 1 (`types R7`).
- **Production is unchanged:** one process has one state directory, so one `StateFile` orders exactly what the statics did.
- **Restarts and crashes:** the ordering is in memory, as before, so a Brindle crash or restart starts it afresh, as now. No server state is involved.

*Rejected:* a ticket field on `SessionStore`. The store is a GPUI global read on the UI thread, while the landed ticket must be shared with the background writes. A separate `Arc` keeps the store as it is, which matters because change 1 edits `pending_write`.

### 7. The scopes test is opt-in

- `holders_and_restarted_cores_run_in_scopes_of_their_own` runs only with `BRINDLE_SCOPE_TEST=1`, named as `BRINDLE_REMOTE_DOCKER_TEST` is.
- Without it, it prints `skipped: set BRINDLE_SCOPE_TEST=1 …` and returns, before the existing `scope::current()` check. Change 11 turns such skips into reported skips across the suite.
- CI's `test` job sets the variable. The runner is disposable, and where it has no user manager the test skips as it does today, so CI keeps whatever coverage it has.

*Rejected:* a private user manager. systemd creates scopes only under a cgroup subtree the system manager delegated, which is what `user@.service` gets. A `systemd --user` started inside the test would have no delegation, so it couldn't create the scopes the test checks. That is the documented model, not something tried here (`testing R11`).

### Layers, crates and rules

- **Test harnesses (`tests/common`, `tests/e2e`, `tests/perf`):** shell code, with the pure helpers (`private_dir`, `scoped_unit`) held by `--self-test` (D3).
- **App (`brindle`):** `StateFile` is a small adapter around one file, with its ordering kept beside it. `HelperLookup` and `role_env` are constructor arguments at existing seams (`ports R10`, `R12`'s stand-in style). The Windows block is built in the seam module from two pure helpers (`merge_environment`, `environment_block`), so the seam keeps only the call (`fcis R1`, `R7`). No decision moves into a view or a thread, and no crate reference changes (`ports R1`–`R3`).
- **`brindle-remote-protocol`:** gains `environment`, a pure module with no IO (`fcis R4`), with its tests run on every platform.
- **Server behaviour and protocols:** untouched. The agent only re-exports the moved encoder. No wire message, config key or state-file field changes, so `process R4`–`R6` and `R15` don't apply. A core restart, a Brindle crash and an upgrade behave as before. The only new input a role gets is the two variables, and only from a test's `Endpoint::at`.
- **Rules satisfied:** `testing R4`, `R7`, `R12`; `ports R10`; `types R7` (`landed`). Flake evidence follows `testing R9`–`R11`: see Risks.

## Risks / Trade-offs

- **[Moved code first runs on Windows in CI]** The Windows arm of the moved `stop` step, and `winproc` under its new path, are first checked by the `windows` job's self-tests and the advisory `windows-e2e` job. → Treat their first Windows run as their first run, not as verified from Linux (`testing R17`).
- **[The Windows environment block runs only in an ignored test]** Only `Endpoint::at` builds one, and the one test with a real core is `#[ignore]`. → Run that test by hand on a Windows machine before merging, and report it as that machine's result (`testing R11`, `R17`). The encoder itself is unit-tested on every platform.
- **[The b27 #23 flake is rare]** It needs two tests' writes to interleave, and a loop may show 0 failures before the fix too. → The new unit test that writes to two directories with interleaved tickets fails deterministically on the old statics. Write it first and see it fail, then report loop counts before and after (`testing R10`), saying plainly if the loop never reproduced it (`testing R11`).
- **[A slower failure in the restore cases]** `restore`'s exit runs now wait `20 + 30` s, not 12 s, before failing when the window doesn't close. Passing runs aren't slowed. → Acceptable; change 11 scales the `+ 30`.
- **[Parallel Docker tests]** If `DOCKER_TEST_LOCK` goes and two scenarios turn out to share something, the opt-in run could flake. → Keep the lock when in doubt (decision 5), with its new reason written down.
- **[The scope guard sees only roles still running]** A role that ran in a scope and exited before the stop step isn't caught. → `isolate` is the cause-level fix; the guard is a backstop.
- **[Merge friction with change 1]** Both edit `record_session` within a few lines. → Whichever lands second rebases; neither changes the other's functions.

## Migration Plan

None. No config, state or protocol changes. Rollback is a revert.
