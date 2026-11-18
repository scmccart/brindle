# Design

## Context

See proposal.md, "Why". This change closes:
- the `process R13` entries of the audit's finding 8: b01 #27 and b24 #1 (one site, the launch listing), b01 #28, b02 #9 and b02 #10;
- its one `process R14` entry, b01 #29;
- the "Doc drift" item "`process R13` against CLAUDE.md" ("Verified by hand" row 6), settled by decision D2 of `docs/architecture/audits/2026-11-16-remediation.md`.

What the five sites, and the instance listener this change aligns them with, do today (code unchanged since the audit):
- **Launch listing (`src/main.rs:862-887`).** Inside `Application::run`'s closure, `running_sessions()` calls `remote::local::running_sessions(2 s)` when the launch restores or attaches. That function (`src/remote/local.rs:379-401`) connects without starting a core. The connect waits up to 2 s for a busy pipe on Windows and returns at once on Linux. It then says `HELLO` with `HELLO_LIMIT` (10 s), longer than the ≤ 5 s a new core takes to adopt its holders, and reads with a 2 s timeout per frame. Its `Option` result goes into the `Detached` global and into `restorable_window` (`:642-656`). That function also reads `session.toml` on the UI thread and drops lost zones by reading the global back. Only then does `open_window` run. The decision about what opens first is written inline in the closure.
  - A failure is `None`. A launch that didn't list is `Some(Vec::new())`. `--attach` then reads a failure as an empty listing (`listed.as_deref().unwrap_or_default()`) and says "the terminal server has no session N".
- **Instance listener (`main.rs:765-775`).** It lists on its own thread for a forwarded `--attach`. Its result travels as `Option<Option<Vec<SessionInfo>>>` (`Forwarded.listing`), and `forwarded_launch` (`:552-578`, unit-tested) turns a failure into "Brindle could not list the terminal server's sessions".
- **Config (`main.rs:412-433`, `:495-516`).** `watch_config` runs on the foreground executor and calls `std::fs::metadata` every second. `reload_config_renaming` calls `Config::load_or_default()`, which reads and parses the file and writes the default one if it is missing, on the UI thread. Its callers are the watch, the `ReloadConfig` action (deferred) and the settings dialog's `ConfigWritten(renames)` (deferred, `workspace.rs:1645-1648`). The first read, `main.rs:790`, is also inside the run closure.
- **Notification log (`notify/mod.rs:231-243`).** `Sink::Log` opens and appends to `BRINDLE_NOTIFY_LOG` inside `Notifier::send`, on the UI thread, on purpose: `--dump-screen-after` may quit right after.
- **Wayland activation (`notify/activation.rs:92-122`).** On its first use, `with_attached` wraps GPUI's `wl_display`, then does a registry round trip and binds `xdg_activation_v1` on the UI thread (`registry_queue_init`, `globals.bind`). `raise` runs it on a notification click (`notify/mod.rs:324`) and on a forwarded launch (`main.rs:541`). The later `connection.flush()` doesn't wait: wayland-backend's `flush` calls `wl_display_flush`, which reports would-block instead of waiting (`wayland-backend-0.3.11/src/sys/client_impl/mod.rs:286-296`). So only the round trip blocks.
- **Find highlights (`find_bar.rs:255-261`).** `TerminalElement::prepaint` (`terminal_element.rs:176`) calls `find_highlights`, which takes the term lock and runs `search::visible_matches` (a regex scan of the viewport, allocating a `Vec<Match>`) on every frame while the bar has a regex. Any `cx.notify()` in the window repaints every zone (GPUI 0.2.2), so other zones' output triggers it too. The same `prepaint` takes the term lock again at `:210` to paint, and reads `display_offset` there.

## Goals / Non-Goals

**Goals:**
- Nothing on the UI thread waits on the terminal server, the file system or the Wayland compositor at the four waiting sites (`process R13`), and `prepaint` does no scan (`process R14`).
- The launch restores exactly what it restores today for the same listing, and its window opens no later than today.
- The launch's decision is a pure function over plain values, with unit tests (`fcis R1`, `R5`, `R8`). A failed listing is a value that names its cause (`railway R1`, `R8`; `types R7`).

**Non-Goals:**
- **Opening the window before the listing answers.** See decision 2.
- **Showing a failed listing in the window.** It is logged, as today. Restored zones run new shells, and the sessions palette lists this machine's sessions afresh, so a transient failure hides nothing for long. Drawing it on the window is `railway R6` work for change 12 (`surface-swallowed-failures`).
- **Other blocking work the audit recorded only as smells.** These are b07's 1 s title poll (`/proc` and Toolhelp), b03's `refresh_zone_title`, b01's debug-step timers, and the settings dialog's own config write (`config_edit`). They are still in `design-smells.md`, and the README's adoption policy covers them.
- **Changing `process R13`.** Per D2 it stays as written, with no exception.

## Decisions

### 1. The launch reads on a `launch-list` thread started in `main`

- **Where it starts.** `main` knows everything the reads need once the session lock is decided:
  - `restoring = !args.no_restore && lock.is_some()`, which is `SessionStore::is_owner()`;
  - `args.attach`;
  - `state_dir`.
  
  `main` starts a named thread, `launch-list`, right after the instance socket and before `Application::new()`. The thread computes `LaunchReads { listing: reattach::Listing, recorded: Option<WindowState> }`:
  - `listing` is `running_sessions()` when the launch restores or attaches, else `Listing::Skipped`;
  - `recorded` is, when restoring, `session::load`, logged on `Err` as `restorable_window` logs today, then `session::merge_windows`.
  
  It sends the result on a `futures::channel::oneshot`.
- **Why a thread of its own.** The listing waits on a socket or pipe for up to about 12 s. Brindle already uses named threads for blocking connects (`launch-connect` in `sweep_launch_hosts`). A blocking call on GPUI's background executor would hold one of its workers for that long.
- **Why in `main`.** The listing then runs while GPUI connects to the display, loads the config and builds `Settings` (font enumeration). The window opens at about max(startup, listing), where today it opens after startup plus listing. So it opens no later than today.
- **The first config read** (`Config::load_or_default()`, `main.rs:790`) moves into `main` too, before `Application::new()`. It is a plain value that `Settings::new` takes in the closure, and no event loop exists yet.
- **If the thread can't be spawned**, `main` logs it and does the same reads inline before `Application::new()`, where nothing waits on them yet. The launch is never left without them.
- **If the thread ends without sending** (a panic, which would be a bug: the protocol decoders and `session::load` return errors, `railway R5`), the awaiting task logs an error and treats the listing as `Failed`. It reads the state file again on the background executor, so a bug in the listing can't cost the saved tabs: the next record would otherwise overwrite them.

### 2. The window waits for the answer, with today's limits; nothing new is drawn

- **The closure no longer opens the window itself.** After the globals it sets today (`Settings`, `Notifier`, keys, `SessionStore`) and `watch_hosts`, the closure spawns one foreground task. The task awaits the oneshot and then, in one `cx.update`, does what `main.rs:863-975` does today:
  - the decision (decision 3) and `Detached`;
  - `open_window`, `attach_detached` and the requested tab;
  - `sweep_launch_hosts`, `watch_config`, `record_session` and `take_launches`;
  - the debug steps and the dump timer.
  
  Their order is unchanged, so `--send` steps and `--dump-screen-after` still count from the window's opening.
- **`watch_hosts` moves ahead of the wait.** It is independent of this machine's listing: it sets `Hosts` and starts the eager endpoints' connections. Remote endpoints that connect at launch start connecting while the local listing runs. With no window yet, their reports reach `Hosts`, and `hosts_changed` finds no window to tell. That is what its doc comment already promises ("Before any window opens").
- **While the listing is pending there is no window**, exactly as today. The UI thread runs its loop: GPUI's own work, the `Hosts` reports and, on Windows, the message loop.
  - A later launch's request waits in the `Forwarded` channel until `take_launches` runs after the window opens, as today. Its `FORWARD_LIMIT` (5 s) applies, as today.
  - Nothing records the session before the window opens (`record_session` starts after it), so a crash during the wait leaves `session.toml` as it was.
- **No new deadline.** The wait ends when `running_sessions` returns. On Linux that is at most `HELLO_LIMIT` (10 s) plus 2 s per read; on Windows, 2 s more for a busy pipe. A timeout there is a failure like any other (decision 3).
  - The remediation plan's "within the same 2 s limit" reads only the argument to `running_sessions`. Its `HELLO` wait is 10 s, because a new core accepts clients only after adopting its holders (≤ 5 s).
  - A 2 s cap on the whole wait would make a launch against a core that is still adopting restore less than today. Its zones would start new shells while their sessions stayed detached (decision 3), and those sessions would come back as extra tabs at the next launch.
- **Rejected: open the window first and apply the listing when it arrives**, as remote endpoints are swept (`attach_swept`).
  - The zones that need the answer would have to be drawn before it. A zone whose session is gone would appear and then vanish. A live zone would start a shell and then gain a duplicate tab for its session. Swept tabs would arrive late and could change the tab order the spec fixes ("Launch reattaches detached terminals").
  - Remote endpoints accept that because they can be offline for good. The local server answers or fails within the limits above.
- **Rejected: list in `main` on the main thread before `Application::new()`.** It obeys R13 in letter, but the window would still wait for startup plus listing, and the work is no less blocking.

### 3. The launch's decision is a pure function over a closed `Listing`

- **`reattach::Listing`** (pure, `src/reattach.rs`), what listing this machine's server gave:
  - `Skipped`: not asked, because the launch neither restores nor attaches;
  - `Listed(Vec<SessionInfo>)`: attached or not; empty when no server runs (none is started);
  - `Failed(String)`: the cause, as `running_sessions` reports it.
  
  It replaces today's `Option<Vec<SessionInfo>>`, where `Some(vec![])` stood for both "not asked" and "no server", and `Forwarded`'s `Option<Option<…>>` (`types R1`, `R7`). On a platform with no server (the future macOS build), `main.rs`'s `running_sessions` keeps returning an empty listing, `Listed(vec![])`, as `Some(Vec::new())` today.
- **`Listing::attach_target(&self, id) -> Result<SessionInfo, String>`**, one rule for the launch and for `forwarded_launch`:
  - `Listed`: today's `reattach::attach_target`;
  - `Failed(why)`: `Err("Brindle could not list the terminal server's sessions: {why}")`, naming the action and its cause (`railway R8`);
  - `Skipped`: the same message without a cause. That case can't arise, because a launch with `--attach` always lists.
- **`startup_launch`** (pure, in `src/main.rs` beside `forwarded_launch`):
  - signature: `startup_launch(listing: &Listing, recorded: Option<WindowState>, restoring: bool, attach: Option<u32>, asked: Option<LaunchRequest>, profile: usize) -> Startup`;
  - result: `Startup { detached: Vec<SessionInfo>, first: Launch, then: Option<Launch> }`.
  
  The rules are today's, moved out of the closure:
  - `detached` is the `Listed` sessions that aren't attached and aren't `attach`'s, when restoring; otherwise empty.
  - The restore is `recorded` when restoring. With `Listed`, the zones whose session isn't in `detached` are dropped (`session::drop_lost_zones` over a local `Detached`, never the GPUI global, `fcis R3`). With `Failed` or `Skipped`, every zone is kept, since nothing tells a lost zone from a live one.
  - The asked tab is `Launch::Attach` for `--attach`, else the `-p`/`-d`/`-e` request.
  - With a restore, `first` is the restore and `then` the asked tab. Without one, `first` is the asked tab or a default-profile tab.
  
  The shell sets `Detached` from `startup.detached`, logs the restore and opens the rest. `restorable_window` goes.
- **Why `main.rs`.** `Launch` and `LaunchRequest` are `workspace.rs` types, and `reattach.rs` doesn't depend on `workspace`. `forwarded_launch` is the precedent: a pure helper in `main.rs` tested in its `mod tests`. Change 13 may later move both into a model.
- **A failed listing on the tab.** A failed listing reaches the user in three ways:
  - `running_sessions` logs it (`not attaching detached sessions: …`), as today;
  - a restored local zone with a session record starts its profile's program in its recorded directory, because `take_detached` finds nothing in an empty `Detached` (`workspace.rs:771-776`, `:826-834`);
  - `--attach` shows the cause in its tab.

### 4. Config: stat and read on the background executor, applied in order

- **`watch_config`** awaits `background_executor().spawn(config_mtime)` each second instead of calling it inline.
- **Reload requests go through one queue.** A `ConfigReloads` global holds the sender of an unbounded channel of rename lists. The `ReloadConfig` action, the watch and the settings dialog's `ConfigWritten(renames)` each send one request.
  - **One worker.** One foreground task takes a request and drains what else is queued. It merges the renames in order with the pure `merge_renames(Vec<Vec<(String, String)>>) -> Vec<(String, String)>`. It reads with `Config::load_or_default()` on the background executor, then applies on the UI thread.
  - **The apply** is today's `reload_config_renaming` body from `Config` onward, renamed `apply_config(config, error, renames, cx)`.
  - **In order.** One worker keeps reloads in order: a slow read can't land after a newer one, and the dialog's renames are applied with the read that follows its write.
  - **Bounded.** The producers are a user action, a one-per-second watch and dialog writes, and each turn drains the queue. The channel is unbounded but never holds more than a turn's requests (`process R9`).

### 5. Notification log: a writer thread, flushed on quit

- **The writer.** `Sink::Log` holds a `LogWriter`: a std channel to a `notify-log` thread that appends each line exactly as today (open in append mode, write, log a warning on failure). Id assignment (`next_id`) and `remember` stay synchronous on the UI thread, so clicks and bell replacement don't change.
- **The quit flush.** `Notifier::init` registers an `on_app_quit` handler for the log sink. It sends a `Flush(oneshot::Sender<()>)` marker and returns a future that resolves once the thread has written everything queued before it. GPUI polls quit futures for up to `SHUTDOWN_TIMEOUT` (100 ms, `gpui-0.2.2/src/app.rs:56`); a line takes microseconds. The quit path does this explicitly, not a `Drop` (`process R16`).
- **Bounded.** The queue's producer is rate-limited: across all tabs at most 10 notifications at once, then one every 2 s (`notify/policy.rs`). So the queue can't grow without bound, and its consumer is not the slower side (`process R9`).
- **If the thread can't be spawned**, `init` logs a warning naming the file and the sink drops lines. It is a test sink, and an e2e case reading the file then fails with the log line to explain why.

### 6. Wayland activation: attach once the window is open, off the UI thread

- **`activation::prepare(window, cx)`**, called right after `open_window`, reads GPUI's display pointer (`wayland_pointers`, which already returns `None` off Wayland). It does the wrap, the round trip and the bind on the background executor. A foreground continuation stores the result.
- **The state** is a UI-thread `RefCell` holding a closed enum, `Attachment`. It has four variants:
  - `Unused`: not Wayland, or `prepare` hasn't run;
  - `Pending { waiting: Vec<(AnyWindowHandle, String)> }`;
  - `Ready(Attached)`;
  - `Failed(String)`.
- **`Attached` moves between threads.** It holds the display as a `usize` key, plus wayland-client's `Connection` and `XdgActivationV1`, both `Send + Sync`, so it is `Send`.
- **`raise(window, token, cx)`** asks the pure `raise_plan(&Attachment, has_token) -> RaisePlan { WithToken, AfterAttach, Plain }`:
  - `Ready` with a token activates at once: a surface proxy, `activate` and a flush that doesn't wait;
  - `Pending` with a token queues the window's handle and token, and the continuation that stores `Ready` or `Failed` raises each queued window;
  - anything else calls `activate_window`, as today. A `Failed` attachment is logged once, when it fails.
  
  `raise`'s two callers (`Notifier::show`, `take_launches`) already run inside an update with `cx` and pass it.
- **Rejected: attach lazily on the first raise, on the background executor.** The first click would then always take the slower path.
- **Rejected: record it as a bounded exception.** D2 keeps R13 without exceptions, and the plan's scope names this site.

### 7. Find highlights: computed at event time, cached, read in `prepaint`

- **`FindBar` gains `visible: Option<VisibleMatches>`**, with `VisibleMatches { key: ViewKey, matches: Arc<[Match]> }` and `ViewKey { display_offset, cols, rows }`. The key and the lookup rule go in `src/terminal/search.rs` (pure, tested there): `cached(visible: Option<&VisibleMatches>, now: ViewKey) -> Cached { Fresh(Arc<[Match]>), Stale, Empty }`. `Empty` means no query has been searched yet, so nothing is drawn and nothing is refreshed; `Stale` means the view moved since the matches were found. The enum keeps "no cache" and "cache for another view" apart (`types R1`).
- **When the cache is refreshed.** Under the term lock those paths already take, the cache is refreshed by:
  - `research`, `step_match` and `recount`;
  - `find_terminal_changed`, which runs from the view's `cx.observe(&terminal)` on every terminal notify: output, and the view moves through `Terminal::scroll`, `jump_to_prompt` and the scroll to the bottom on input. Today it only schedules a count; it now also refreshes the cache at once while the bar has a regex.
  
  `close_find_bar` drops the cache with the bar.
- **What `prepaint` does.** It takes the cached `Arc` (no lock, no scan) and checks the key against the `display_offset` and grid size it already reads under its painting lock (`terminal_element.rs:210-215`).
  - When the key is stale (the grid was just resized in this `prepaint`, or output moved the view of a scrolled-back terminal before its notify arrived), it draws no matches for that frame and asks the view, deferred, to refresh and notify.
  - Drawing nothing for one frame is better than drawing matches on the wrong rows.
- **Cost.** The scan runs once per change to this terminal's view or content. Today it runs on every frame of the window, any zone's output included.

### 8. One client token per listing

`running_sessions` says `HELLO` with `"{client_token}-list"`. A forwarded `--attach` listed on the instance's thread while the launch is still listing uses the same token. The core's `take_over` (`remote-agent/src/server/core/clients.rs:217`) then closes the first connection, and that listing fails. That race is as old as the listener. This change makes both listings start near the same moment, so it closes it:
- each listing uses `list_token(client_token, n)` = `"{client_token}-list-{n}"`, with `n` from an `AtomicU64` in `local.rs`;
- `list_token` is pure, and tested to differ by `n` and never to equal the client's own token.

The core treats tokens as opaque strings, so any core build accepts them (`process R4`, `R5`: nothing on the wire changes shape).

### 9. Test switch and tests

- **`BRINDLE_TEST_LAUNCH_LISTING`** is read on the `launch-list` thread in debug builds only (`#[cfg(debug_assertions)]`), as `process R20` requires of a fault injector:
  - `fail` makes the listing `Failed("BRINDLE_TEST_LAUNCH_LISTING=fail")`;
  - a number of milliseconds waits that long, then lists.
  
  The parse is pure: `test_listing(Option<&str>) -> Option<TestListing>`, with `TestListing { Delay(Duration), Fail }`. e2e runs debug builds by default (`BRINDLE_E2E_BIN` unset).
- **Layers (`testing` R1, R13).**
  - **Unit:**
    - `startup_launch` and `Listing::attach_target`, as tables;
    - `forwarded_launch` with `Listing`;
    - `merge_renames`, `raise_plan`, `search::cached`, `list_token` and `test_listing`.
  - **e2e:**
    - `local-server-reattach-slow-listing`, `local-server-reattach-failed-listing` and `notify-log-at-exit` (new);
    - the `restore-*`, `local-server-reattach-*`, `local-server-attach-flag`, `instance-attach`, `notify-*`, `search-*`, `reload-theme-attached`, `colors-reload`, `settings-apply` and `settings-profile-rename` cases, unchanged.
  - **Manual (`testing` R16):**
    - a notification click and a forwarded launch raising the window under GNOME on Wayland, which Xvfb can't show;
    - launch timing on main against the branch, from a new debug log line.
  - The plan's "`--latency-log` look at time to first paint" can't be done: latency probes time key to paint only (`src/latency.rs`). So the launch logs, at debug level, how long after `main` started the listing answered and the window opened.

### 10. Layers, versions and the server

- **Pure:**
  - `reattach::Listing` and `Listing::attach_target` (`src/reattach.rs`);
  - `startup_launch` and `test_listing` (`src/main.rs`);
  - `merge_renames` (`src/main.rs`);
  - `raise_plan` (`src/notify/activation.rs`);
  - `ViewKey` and `cached` (`src/terminal/search.rs`);
  - `list_token` (`src/remote/local.rs`).
- **Shell:**
  - `main` and the launch task;
  - the `launch-list` and `notify-log` threads;
  - the config worker task;
  - `activation::prepare`;
  - `TerminalView`'s find bar methods and `TerminalElement::prepaint`.
- **Seams.** The Wayland code stays in `notify/activation.rs`. `main.rs`'s existing `cfg(any(target_os = "linux", windows))` around the listing stays (`main.rs` is on the `ports R4` exception list). `cfg(debug_assertions)` isn't a platform gate.
- **Crates.** Everything is in `brindle`. No crate reference changes, and `tests/architecture.rs` is untouched.
- **Versions (`process R4`–`R6`, `R15`).** No wire message, config key or state-file field changes, and no transcript changes. The listing's token is an opaque string.
- **Terminal server (`process R1`–`R3`, `R12`).**
  - A core restart during the launch's listing fails it, and the launch takes the failed path. The core keeps every session, which the next launch or the sessions palette finds.
  - A Brindle crash during the wait writes nothing, because recording starts after the window opens.
  - An upgrade is untouched: `running_sessions` never offers one (it says only `HELLO`), and the tabs' own connections do it as today.

## Risks / Trade-offs

- [A launch against a core that never answers still waits up to about 12 s before its window opens, as today] → That is the existing bound, and decision 2 explains why a shorter one would restore less. The wait no longer holds the UI thread, and it now overlaps GPUI's startup.
- [`startup_launch` moves code that spans the restore, the attach and the asked tab, and a slip changes what a launch opens] → Its table tests pin today's outcomes (including `Skipped` with no restore, and `--attach` with a restore), and the `restore-*` and `local-server-reattach-*` e2e cases run unchanged.
- [The Wayland round trip now runs on a background thread while GPUI's thread reads the same display] → libwayland's queues are thread-safe, and each reader uses its own queue (ours comes from `registry_queue_init`), which is what `from_foreign_display` with a separate queue is for. The manual check covers a click and a forwarded launch under GNOME on Wayland. If either hangs, the change stops and the design is revisited rather than shipped.
- [A reload now lands a few milliseconds after the dialog's write, so a preview the user starts in between is ended by it] → That is what happens today when the watch notices the write a second later. `settings-apply` and `settings-profile-rename` cover the dialog's path.
- [For one frame after a resize, or after output moves a scrolled-back view, a match in view isn't highlighted] → The next frame has it, and drawing nothing beats drawing matches on the wrong rows. The `search-*` cases run with output and scrolling.
- [`add-blocks` changes how the find bar moves the view] → Any new place that moves the view must refresh the cache, or it waits for the next notify. That is noted in proposal.md's overlap for whichever change lands second.
- [The notification log's last lines depend on GPUI polling the quit future] → It is polled for up to 100 ms and the flush takes microseconds. A new e2e case, `notify-log-at-exit`, checks it with no timing: a `-e` program sends two notifications and exits, which quits Brindle.

## Migration Plan

None. No file, key or protocol changes, and nothing persists across versions differently. Rollback is a revert.
