# Tasks

## 1. Pure decisions, with their unit tests

- [ ] 1.1 In `src/reattach.rs`, add `pub enum Listing { Skipped, Listed(Vec<SessionInfo>), Failed(String) }` (derive `Clone`, `Debug`, `PartialEq`; a doc comment per variant, as in design.md decision 3).
  - Add `Listing::attach_target(&self, id: u32) -> Result<SessionInfo, String>`:
    - `Listed` calls the existing `attach_target`;
    - `Failed(why)` gives `"Brindle could not list the terminal server's sessions: {why}"`;
    - `Skipped` gives the same text without the cause.
  - Tests in its `mod tests`: `listed_sessions_give_the_attach_target` (a detached id, an attached id, a missing id) and `a_failed_listing_names_its_cause_for_attach`.
  - Verify: `cargo test -p brindle reattach::`.
- [ ] 1.2 In `src/main.rs`, beside `forwarded_launch`, add `struct Startup { detached: Vec<SessionInfo>, first: Launch, then: Option<Launch> }` and the pure `startup_launch(listing: &Listing, recorded: Option<WindowState>, restoring: bool, attach: Option<u32>, asked: Option<LaunchRequest>, profile: usize) -> Startup`.
  - It holds the rules of `main.rs:863-887` and `restorable_window`'s zone drop. Drop lost zones with `session::drop_lost_zones` over a local `reattach::Detached::new(detached.clone())`, never the global (design.md decision 3).
  - Tests in `mod tests`, built with `session::WindowState` values and `SessionInfo { session, key, attached, .. }`:
    - `a_listing_restores_live_zones_and_sweeps_the_rest`: a zone whose session is listed and detached stays; a zone whose key doesn't match is dropped; the other detached sessions are in `detached`; an attached session isn't;
    - `a_failed_listing_keeps_every_zone_and_sweeps_nothing`;
    - `a_launch_that_restores_nothing_opens_the_asked_tab_or_a_default_one`;
    - `attach_opens_after_the_restored_tabs_and_is_not_swept`;
    - `attach_without_a_listing_says_why`.
  - Verify: the five new tests pass under `cargo test -p brindle`, with `forwarded_requests_open_tabs` unchanged until 1.3.
- [ ] 1.3 Change `forwarded_launch` to take `&Listing` instead of `Option<Option<Vec<SessionInfo>>>`, using `Listing::attach_target`. Change `Forwarded.listing` to `Listing`.
  - In `forwarded_requests_open_tabs`, pass `&Listing::Skipped` where it passes `None`.
  - Add `forwarded_attach_says_why_listing_failed`.
  - Verify: `cargo test -p brindle forwarded`.
- [ ] 1.4 In `src/main.rs`, under `#[cfg(debug_assertions)]`, add `enum TestListing { Delay(Duration), Fail }` and the pure `test_listing(value: Option<&str>) -> Option<TestListing>`:
  - `"fail"` gives `Fail`;
  - a whole number of milliseconds gives `Delay`;
  - anything else, or unset, gives `None`.
  - Test: `the_launch_listing_switch_reads_fail_and_a_delay` (`#[cfg(all(test, debug_assertions))]`).
  - Verify: `cargo test -p brindle launch_listing_switch`.
- [ ] 1.5 In `src/remote/local.rs`, add the pure `list_token(client: &str, n: u64) -> String` (`"{client}-list-{n}"`) and the test `each_listing_has_a_token_of_its_own`: two `n` differ, and neither equals `client`. Verify: `cargo test -p brindle each_listing_has_a_token`.
- [ ] 1.6 In `src/main.rs`, add the pure `merge_renames(batch: Vec<Vec<(String, String)>>) -> Vec<(String, String)>`, which concatenates in request order. Test: `reload_renames_keep_their_order`, with `a→b` then `b→c` kept as two steps in that order. Verify: `cargo test -p brindle reload_renames`.
- [ ] 1.7 In `src/notify/policy.rs` (pure, compiled on every platform, `fcis R7`), add:
  - `pub enum AttachState { Unused, Pending, Ready, Failed }`;
  - `pub enum RaisePlan { WithToken, AfterAttach, Plain }`;
  - `pub fn raise_plan(state: AttachState, has_token: bool) -> RaisePlan`. `Ready` with a token gives `WithToken`, `Pending` with a token gives `AfterAttach`, and everything else gives `Plain`.
  - Test: `a_raise_waits_for_a_pending_attachment_only_with_a_token`, over every pair.
  - Verify: `cargo test -p brindle notify::policy`.
- [ ] 1.8 In `src/terminal/search.rs`, add:
  - `pub struct ViewKey { display_offset: usize, cols: usize, rows: usize }`;
  - `pub struct VisibleMatches { key: ViewKey, matches: Arc<[Match]> }`;
  - `pub enum Cached { Fresh(Arc<[Match]>), Stale, Empty }`;
  - the pure `cached(visible: Option<&VisibleMatches>, now: ViewKey) -> Cached`.
  - Test: `cached_matches_are_fresh_only_for_the_view_they_were_found_in`, covering another offset, another size, and no cache.
  - Verify: `cargo test -p brindle terminal::search`.

## 2. Launch: the listing and the state file leave the UI thread (`src/main.rs`, `src/remote/local.rs`)

- [ ] 2.1 `remote::local::running_sessions` says `HELLO` with `list_token(&client_token(), n)`, with `n` from a private `static LISTINGS: AtomicU64` (design.md decision 8). Its doc comment says that it blocks and that its callers run it off the UI thread: the launch's `launch-list` thread and the instance listener. If change 4 (`isolate-test-runs`) has already split it into `running_sessions_at`, make this edit in that function. `main.rs`'s `running_sessions()` returns `Listing`:
  - `Listed(sessions)` on success;
  - `Failed(format!("{why:#}"))` after its existing `not attaching detached sessions` log line;
  - `Listed(Vec::new())` under `cfg(not(any(target_os = "linux", windows)))`.
  
  Verify: `cargo build --locked` with no warnings.
- [ ] 2.2 In `src/main.rs`, add:
  - `struct LaunchReads { listing: Listing, recorded: Option<WindowState> }`;
  - `fn read_launch(restoring: bool, attach: bool, state_dir: &Path) -> LaunchReads`, the blocking work:
    - it lists when `restoring || attach`, else gives `Listing::Skipped`;
    - when restoring it runs `session::load`, logs an `Err` as `restorable_window` did, and calls `session::merge_windows`;
    - in debug builds it applies `test_listing(std::env::var("BRINDLE_TEST_LAUNCH_LISTING").ok().as_deref())` first: a `std::thread::sleep` for `Delay`, `Failed("BRINDLE_TEST_LAUNCH_LISTING=fail")` for `Fail`;
  - `fn start_launch_reads(restoring, attach, state_dir) -> futures::channel::oneshot::Receiver<LaunchReads>`. It runs `read_launch` on a thread named `launch-list`. When the spawn fails, it logs that and runs `read_launch` inline (before GPUI starts, design.md decision 1).
  
  In `main`, call it right after the instance socket is set up, with `restoring = !args.no_restore && lock.is_some()`. Also move `Config::load_or_default()` from the run closure into `main`, before `Application::new()`, and pass `(config, error)` into the closure. Verify: `cargo build --locked`.
- [ ] 2.3 Rework the `Application::run` closure:
  - Call `watch_hosts(cx)` before the wait.
  - Spawn one foreground task that awaits the receiver. If the sender is dropped (`Canceled`), the task logs an error, reads the recorded window on the background executor, and uses `Listing::Failed("the launch's listing thread ended without an answer")`.
  - Then, in one `cx.update`, the task:
    - calls `startup_launch`;
    - sets `reattach::Detached::new(startup.detached)`;
    - logs `restoring N tab(s) from …` for a `Launch::Restore`;
    - logs at debug level how long after `main` started the listing answered and the window opened (the `Instant` is taken in `main`);
    - runs what `main.rs:888-975` runs today, in the same order, from `open_window` on: `open_window` (with `cx.quit()` on failure), `take_rest` and `attach_detached`, the asked tab, `sweep_launch_hosts` when restoring, `watch_config`, `record_session`, `take_launches`, the debug steps and the dump timer.
  - Delete `restorable_window`. The `Forwarded` handler (`main.rs:765-775`) keeps listing on its own thread, now as `request.attach.map_or(Listing::Skipped, |_| running_sessions())`.
  - Verify: `cargo build --locked` with no warnings, `cargo test -p brindle`, and `rg -n 'running_sessions\(' src/main.rs` shows calls only inside `read_launch` and the instance handler.
- [ ] 2.4 Add the e2e case `local-server-reattach-slow-listing` in `tests/e2e/cases/local_server.rs` and register it in `tests/e2e/cases/mod.rs`.
  - It mirrors `reattach_no_restore`, and its third run sets `BRINDLE_TEST_LAUNCH_LISTING` to 3 s scaled by `BRINDLE_E2E_SLOW`, in milliseconds.
  - Assert the same as that case: two tabs, the recorded tab first and active before `next_tab`, and `left-2` in its scrollback.
  - If the switch had no effect (a release `BRINDLE_E2E_BIN`), the case can't tell, so its doc comment says it needs a debug build.
  - Verify: `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- local-server-reattach-slow-listing`.
- [ ] 2.5 Add the e2e case `local-server-reattach-failed-listing`, with four runs:
  - **First run:** `restore`, `keep_detached`, `cd /tmp; echo first=$$`. Quit, then `detached_pids` finds its pid `P`.
  - **Second run:** `restore`, `keep_detached`, `BRINDLE_TEST_LAUNCH_LISTING=fail`, `echo second=$$; pwd`. Assert:
    - one tab;
    - a `second=` pid other than `P`;
    - `/tmp` on screen;
    - after the run, `P` is still listed as detached.
    - If `P` came back instead, fail with "BRINDLE_TEST_LAUNCH_LISTING had no effect: is BRINDLE_E2E_BIN a release build?".
  - **Third run:** a plain `restore`. It shows two tabs, the second showing `first=P`.
  - **Fourth run:** `--attach 1` with `BRINDLE_TEST_LAUNCH_LISTING=fail`. Its active tab says `could not list the terminal server's sessions`.
  - Register it, and verify as in 2.4.
- [ ] 2.6 Run the cases that cover the launch path, unchanged, under `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- restore local-server-reattach local-server-attach-flag local-server-layout instance zones-restore remote-ssh-restored-closed`. Every one must pass. This is a long-output chore: route it per CLAUDE.md's Delegation section.
- [ ] 2.7 Docs:
  - **CLAUDE.md, the Reattach bullet:** rewrite its first sentence. The instance's launch (unless `--no-restore`) lists the core's sessions once, and reads the state file, on a `launch-list` thread started in `main` before GPUI (`running_sessions`, which never starts a core). A foreground task awaits that and opens the window once the listing has answered or failed, applying `startup_launch`'s result (the `Detached` global, the restored window without lost zones, the asked tab). A failed listing (`reattach::Listing::Failed`) keeps every recorded zone and sweeps nothing.
  - **The rest of that bullet:** say the window still opens only after the listing, and that `BRINDLE_TEST_LAUNCH_LISTING` (debug builds) makes it slow or fail for the e2e cases.
  - **`src/reattach.rs`'s module doc:** "listed once before any window opens, off the UI thread".
  - **`process-architecture.md`:** unchanged (D2).
  - Verify by reading CLAUDE.md against design.md decisions 1–3, and checking that `rg -n "before the window opens" CLAUDE.md` names no UI-thread wait.

## 3. Config reads leave the UI thread (`src/main.rs`, `src/workspace.rs`)

- [ ] 3.1 Split `reload_config_renaming` into the request and the apply (design.md decision 4):
  - **The apply:** `apply_config(config, error, renames, cx)`, today's body from `Settings::new` on.
  - **The queue:** a `ConfigReloads` GPUI global holding an unbounded sender of `Vec<(String, String)>`, set in the run closure, and `pub(crate) fn request_config_reload(renames: Vec<(String, String)>, cx: &mut App)`.
  - **The worker:** one foreground task takes a request, drains the queued ones with `try_next`, merges them with `merge_renames`, awaits `background_executor().spawn(Config::load_or_default)`, then calls `apply_config`.
  - **The callers:**
    - `reload_config` becomes `request_config_reload(Vec::new(), cx)`;
    - `workspace.rs:1645-1648`'s `ConfigWritten` handler calls `request_config_reload(renames, cx)`, and needs no `defer` any more because the apply comes from the worker task;
    - `watch_config` awaits `background_executor().spawn(async { config_mtime() })` each second, and requests a reload on a change.
  - Verify: `cargo build --locked`, `cargo test -p brindle`, and `rg -n 'load_or_default\(\)' src` shows the call only in `main` (startup) and in the worker.
- [ ] 3.2 Run the reload and settings cases, unchanged, under `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- reload colors-reload colors-palette-reload settings-apply settings-profile-save settings-profile-rename settings-fonts ui-font-config`. Every one must pass.
- [ ] 3.3 Docs: in CLAUDE.md's `settings.rs` bullet, "Live reload (`main.rs` polls the file mtime)" becomes: `main.rs` polls the file's mtime and reads it on the background executor, and one task applies reloads in request order (`ConfigReloads`). Verify by reading it against decision 4.

## 4. Notification log leaves the UI thread (`src/notify/mod.rs`)

- [ ] 4.1 Add the writer (design.md decision 5):
  - **The writer:** `LogWriter`, a `std::sync::mpsc::Sender<LogCommand>` with `enum LogCommand { Line(String), Flush(futures::channel::oneshot::Sender<()>) }`, served by a thread named `notify-log` that appends each `Line` exactly as `send_request` does today, warnings included.
  - **The sink:** `Sink::Log` holds `{ writer: LogWriter, next_id }`; `send_request` sends the line and keeps the id and `remember` as today.
  - **The flush:** `Notifier::init` registers `cx.on_app_quit` for the log sink. It sends `Flush` and returns a future awaiting the reply, ignoring a closed channel.
  - **No thread:** if the thread can't start, `init` logs a warning naming the file, and the sink drops lines.
  - **Comments:** the "Written right away" comment goes, and the module doc says the log has a writer thread.
  - Verify: `cargo build --locked`, `cargo test -p brindle notify::`.
- [ ] 4.2 Add the e2e case `notify-log-at-exit` in `tests/e2e/cases/notify.rs` and register it in `mod.rs`.
  - Run Brindle with `BRINDLE_NOTIFY_LOG`, `when = "always"` and `-e sh -c 'printf "\033]9;one\a\033]777;notify;t;two\a"'`, with no dump.
  - The program exits, its tab closes, and Brindle quits.
  - Assert that the log holds the `osc9 … one` line, then the `osc777 … two` line (the command-line delta's "Quit right after notifications").
  - Verify with `notify-osc-local` and `notify-background-tab`: `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- notify`.
- [ ] 4.3 Docs: in CLAUDE.md's `notify/` bullet, "(written synchronously, since `--dump-screen-after` quits right after)" becomes: appended by a `notify-log` writer thread, which quitting waits for. Verify by reading it against decision 5.

## 5. Wayland activation off the UI thread (`src/notify/activation.rs`, `src/notify/mod.rs`, `src/main.rs`)

- [ ] 5.1 Rework activation (design.md decision 6):
  - **The state:** replace the `thread_local!` `Option<Attached>` with a UI-thread `RefCell<Attachment>`, where `enum Attachment { Unused, Pending { waiting: Vec<(AnyWindowHandle, String)> }, Ready(Attached), Failed(String) }`. `Attached` keeps the display as a `usize` key, so it is `Send`.
  - **`pub fn prepare(window: &Window, cx: &mut App)`:** it reads `wayland_pointers`, does nothing off Wayland, and sets `Pending`. It spawns the wrap, `registry_queue_init` and `globals.bind` on the background executor. A foreground continuation stores `Ready` or `Failed` (logged once) and raises each waiting window through its handle.
  - **`raise(window, token, cx)`:** it maps `Attachment` to `policy::AttachState` and follows `policy::raise_plan`.
  - **The Windows module:** `notify::activation`'s `raise` in `mod.rs` takes and ignores `cx`, and gains a no-op `prepare`.
  - **The callers:**
    - `Notifier::show` (`mod.rs:324`) and `take_launches` (`main.rs:541`) pass `cx`;
    - the launch task calls `notify::activation::prepare` right after `open_window`.
  - Verify: `cargo build --locked` and `cargo test -p brindle` on Linux, and CI's `windows` job builds.
- [ ] 5.2 Docs: in CLAUDE.md's click bullet under `notify/`, add that the attachment to GPUI's display is made on the background executor once the window opens, and that a raise arriving before then waits for it. Update `activation.rs`'s module doc to match. Verify by reading them against decision 6.
- [ ] 5.3 Manual check, handed to the user, under GNOME on Wayland with `scripts/run-throwaway.sh` (it opens a window, so the run is theirs). Confirm each step:
  - With another window focused, `printf '\033]777;notify;t;hi\a'` in a background tab, then a click on the notification, raises Brindle with that tab active, and no "Brindle is ready" notice appears.
  - Running `brindle -e true` from a launcher while the throwaway Brindle is behind another window raises it.
  - Nothing hangs at launch.

## 6. Find highlights out of `prepaint` (`src/find_bar.rs`, `src/terminal_element.rs`)

- [ ] 6.1 Add the cache (design.md decision 7):
  - **The field:** `FindBar` gains `visible: Option<search::VisibleMatches>`.
  - **The refresh:** a private `refresh_visible(&mut FindBar, &Term<…>)` runs `search::visible_matches` and stores it with the term's `ViewKey`. Call it under the lock already held in `research`, `step_match` and `recount`.
  - **On output and scrolling:** `find_terminal_changed` takes the lock once and refreshes while the bar has a regex, and still schedules the count as today.
  - **The read:** `find_highlights` no longer locks or scans. It returns the cached `VisibleMatches` and the focused match.
  - **`TerminalElement::prepaint`:** it compares the cache with `ViewKey { display_offset, cols, rows }`, read under the lock it takes at `:210`, using `search::cached`. `Fresh` draws the matches. `Stale` draws none and calls a new `TerminalView::find_view_moved`, which refreshes the cache and notifies, deferred.
  - Verify: `cargo build --locked`, `cargo test -p brindle`, and `rg -n 'visible_matches' src` shows no call reachable from `prepaint`.
- [ ] 6.2 Run the search cases, unchanged, under `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- search`. Every one must pass.
- [ ] 6.3 Docs: in CLAUDE.md's `find_bar.rs` bullet, add that the matches in view are found when the query, the output or the view changes, and that `prepaint` reads that cache. Verify by reading it against decision 7.

## 7. Integration checks

- [ ] 7.1 `cargo test --workspace --locked` (it includes `tests/architecture.rs`) and `cargo build --release --locked` pass with no new warnings. If change 6 (`add-clippy-to-ci`) has landed, `cargo clippy --locked --workspace --all-targets -- -D warnings` passes too.
- [ ] 7.2 The whole e2e suite passes under `scripts/with-xvfb env BRINDLE_E2E=1 BRINDLE_E2E_SLOW=2 cargo test --test e2e`, as CI's `e2e` job runs it. This is a long-output chore: route it per CLAUDE.md's Delegation section.
- [ ] 7.3 On the PR, CI's `windows` job (build, `cargo test`, self-tests) and `windows-e2e` are green. The launch path, the config worker and the log writer run there with Windows' own server. Treat that as their first run on Windows, not as verified from Linux (`testing R17`).
- [ ] 7.4 Manual check, handed to the user with `scripts/run-throwaway.sh` (it opens a window). With `RUST_LOG=brindle=debug` and five detached sessions in the throwaway server, compare the new debug line's time to window open on this branch with the log timestamps of a `main` build on the same server. The branch is no slower. Put the numbers in the PR body (`testing R11`).

## Workflow follow-up

- Land as one PR, with title `fix: keep the UI thread free while listing sessions at launch` (at most 93 characters, `scripts/lint-commits --title`). Put the archive commit (`docs(openspec): archive list-sessions-off-ui-thread`) on the same PR, syncing the `session-restore` and `command-line` deltas into `openspec/specs/`.
- List the manual checks in 5.3 and 7.4 in the PR body (`testing R16`).
