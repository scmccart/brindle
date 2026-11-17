# Design smells, 2026-11-16 audit

Every design smell the reading batches reported, verbatim, grouped by batch. A smell is something that looks wrong but that no rule's words cover; none of these are counted as violations. Batch contents are in `batches.md`.

## b00

- `src/actions.rs:105` `ACTIONS` is a tuple `(&str, Option<&str>, fn)`, and its fields are read by position in four places. A named struct would read better.
- `src/actions.rs:261-263` `menu_label` builds every action in the table (`build()`) on each lookup to compare. The same happens per item in `ActionMenu::new`.
- `src/actions.rs:93-101` `ActivateTab(usize)` and `NewTabWithProfile(usize)` carry positions. This is by user intent ("alt-3", "ctrl-alt-2"), so not a types R3 case, but the profile one shifts when profiles are reordered.
- `src/actions.rs:328-335` key contexts are string literals shared by hand with the views that set them. A shared constant per context would stop typos.
- `src/actions.rs:431` `bind_keys` has no test; its pure half would.
- `src/config_edit.rs:20-41` `save` does file IO (canonicalize, read, create_dir_all, write, rename) in a module that is otherwise pure document editing. Callers on the UI thread would block (process R13); the callers are outside these files, so this was not checked.
- `src/config_edit.rs:120` `theme_references(&mut DocumentMut, ..)` takes `&mut` for a read-only question, because `for_each_reference_slot` is mutable. A read-only twin would let callers hold `&DocumentMut`.
- `src/config_edit.rs:132-140, 198-208` `custom_themes` and `profiles` re-serialize the whole document to a string and re-parse it on each call.
- `src/config_edit.rs:404` `move_profile(.., up: bool)` takes a direction as a bool. A `Direction` enum is clearer.
- `src/config_edit.rs:482-548` `set_font_family(table: &str)` and `set_font_number(key: &str)` take open strings for closed sets (`font`/`ui_font`; `size`/`line_height`). Any other string silently creates an arbitrary table or key. An enum would reject it at compile time; no numbered rule covers it.
- `src/config_edit.rs:193` `IMPLICIT_PROFILE = "Shell"` duplicates a name `Config::normalized` also owns, kept in step by a comment.
- Tests assert on error text (`:710`, `:725`, `:733-734`, `:763`, `:786`, `:991`, `:1026`: `contains("reopen")`, `contains("not valid TOML")`, `contains("last profile")` and others). That is railway R4's "never compare error text" in spirit, and it comes with the `Result<_, String>` exception. The dialog's use of the same texts (`CHANGED`) was not checked.
- `Cargo.toml:41-79` the platform dependency blocks (`cfg(unix)`, `cfg(target_os = "linux")`, `cfg(windows)`) are manifest-level `cfg`s for platform concerns. The rule text speaks of source attributes, so this is not counted.
- `build.rs:6, 20, 25-26` `std::env::var(..).unwrap()` on Cargo-provided variables is bug-only, so not a railway R9 breach. `.expect("compiling assets/windows/brindle.rc")` is the build failing loudly, which is right for a build script.

## b01

- `src/layout.rs` (declared pure in fcis' table) imports `crate::zone_view::{SplitAxis, ZoneId, ZoneTree}` from the GPUI view module, so the model's tests compile only with gpui in the graph. `SplitAxis` also carries serde for the layout blob, mixing the persisted form with the in-memory type (types R9 flavour). Move those types to a GPUI-free module.
- `src/layout.rs:274-310` — `group` sorts on an anonymous tuple `(u64, u32, String, u32)` and an `anchors` map. A small `SortKey` struct would be clearer. `u32::try_from(index).unwrap_or(u32::MAX)` and `serde_json::to_value(layout).ok()` in `record` hide a failure that cannot happen.
- `src/desktop.rs:225-229` — `run_tool` turns an `update-desktop-database` or `gtk-update-icon-cache` failure into `log::warn!` only. Documented ("failures are only warnings"), but the user running `--install-desktop` never sees it.
- `src/desktop.rs:300-328` — the tests run `install`/`uninstall`, which call `refresh_caches` and therefore execute `update-desktop-database` from the host's `PATH` against a temp directory. They are not hermetic (testing R7 spirit). The temp names include the pid and the test name, so they do not collide. `prefix_is_rejected_on_windows` asserts on error text (`err.contains("winget install")`).
- `src/find_bar.rs:164-182, 144-162` — a full-scrollback `search::count(.., COUNT_CAP)` runs on the UI thread while holding the term lock (in `step_match` and `recount`). It is capped, but it contends with the reader thread. The `count` field (a derived value) names its refreshers only indirectly (`recount`, `step_match`, `research`), with no comment beside the field (types R10 flavour).
- `src/main.rs:195, 155` — `WaitLog` is `Arc<Mutex<Vec<(u64, String, WaitState)>>>` with an anonymous tuple. A struct entry would read better.
- `src/main.rs:213, 523` and `reattach::attach_target(session: u32, …)` — `--attach ID` and `Forwarded` name a session by its bare id (and `:867` compares `Some(s.session) != args.attach`). It resolves within one fresh listing, so it is not stale, but it is the "id alone" shape that `types R3` warns about. The CLI can't supply a key.
- `src/main.rs:447, 611` — `futures::channel::mpsc::unbounded` for host reports and launch listings, and `:762` for forwarded requests. The consumers drain eagerly and the payloads are small state reports, so this is probably fine, but they have no high/low marks (process R9 wording is about producer/slower-consumer queues).
- `src/main.rs:1058, 1014` — debug loops poll on 1 ms and 100 ms timers on the foreground executor (Windows pacing note in process R13 concerns tasks that wake without pause; 1 ms is paced but dense).
- `src/main.rs:1025-1032` — the `--latency-bench` jitter (xorshift) is an inline closure with its own seed from `SystemTime::now()`. Pure enough to extract and test.
- `src/latency.rs:232-239` — `samples` and `all_samples` grow without bound for the life of a `--latency-log` run (debug only).
- `src/main.rs:230-241` — `Args::instance_request` reads `std::env::current_dir()` and `Request::launch_token()` (env) inside a method on pure data. Its test compares against `current_dir()`, which is fine, but the ambient reads could be arguments (fcis R3 applies only to pure code).

## b03

- `src/zone_view.rs:543-547`, `504-513` and `887-899`: small `if id == self.active { emit } else if changed { notify }` conditionals in event handlers. The decision of when a title change is announced could be returned by a model method.
- `src/zone_view.rs:779-785` (`menu_anchor`): the decision "header bottom-left, else tree origin" is in the entity, but it only picks between two stored `Bounds`.
- `src/zone_view.rs:574-575`: double blank line. `:958`: `let card =Card::new`.
- `src/zone_view.rs:670-675`: two stacked, partly duplicated doc paragraphs ("Leaves every zone's program…" then "Brindle is quitting: …") on `quit_all`.
- `src/zone_view.rs:258-277` (`can_split_zone`): the `has_header` parameter is unused (`let _ = has_header;`).
- `src/zone_view.rs:35` (`ZONE_HEADER_HEIGHT: f32`) duplicates `brindle_ui::CARD_HEADER_HEIGHT`, guarded only by a test.
- `src/zone_view.rs:31`: `ZoneTreeView` imports `shortcut`, `terminal_automatic_title` and `TAB_BAR_HEIGHT` from `workspace`, which owns the tree view. That is a two-way module dependency, and `terminal_automatic_title` is a pure title decision that lives in the shell file.
- `src/zone_view.rs:540-547`, `626-635`, `607-614`: `refresh_zone_title` runs `terminal.foreground_process_name(snapshot)` (a `/proc` or Toolhelp read) on the UI thread inside the `TitleChanged` handler. This is the documented design (process R13/R14 are not breached in `render`), but on Windows cmd it runs once per command.
- `src/zone_view.rs:787-790` (`split_zone`): silently returns when the zone is too small. A key-bound split gives no feedback, while the menu disables the item.
- `src/zone_view.rs:1180`, `reattach.rs:519-521`: tests assert on error/label text. Fine while the error is `Result<_, String>` by exception.
- `src/settings.rs:292-307` (`resolve_family`): a configured family that is not installed falls back silently, with only `log::info!("using font family …")` in `Settings::new`. Compare railway R10 ("falls back to defaults and shows why"). Whether the Fonts section shows it is outside these files.
- `src/settings.rs:253-258`: `catch_unwind` around `resolve_font` to survive a panic inside GPUI. Acceptable as confinement of a foreign panic, but it is the only one in these files.
- `src/settings.rs:392-405`: two headless GPUI apps are spawned and leaked on threads, and may run in parallel with each other. Not shown to be a flake, but it is process-wide global GPUI state (testing R7 territory).
- `src/settings.rs:137-139` (`chrome_theme`): `cx.global::<ChromeTheme>()` panics if the global is unset. That is a bug-in-Brindle case, which railway R9 allows, but it carries no `expect` message.
- `src/reattach.rs:186-194` (`attach_target(id: u32, …)`): finds by session id alone. It is a user-typed id, within one listing, so types R3 does not bite.
- `src/primary.rs`: clean. The seam is thin, `PrimaryBuffer` is ungated and tested on every platform (fcis R7).
- `src/reattach.rs`: the best-placed file. It is pure, takes `now` as a parameter (`age`), and every function has a test.

## b04

- The module doc says `Model` "has no GPUI dependency" but it imports `gpui::Keystroke` for `Model::key`; plain data, but the claim is inaccurate.
- File IO (read at `Model::new`, `reload_entries`, `select_section`; write at `confirm`) runs on the UI thread inside event handlers. Small config file, not in `render`, and the rule's examples are about connecting, so not reported against process R13.
- `Model::save` returns `Result<(), ()>` while the real error goes into a side field `self.error`.
- `src/settings_dialog.rs:852-853`: the doc comment "Cycles through the sections (ctrl-tab)." sits on `edge_section`, not `next_section`.
- `src/settings_dialog.rs:270`: stray blank line before the closing brace of `impl Editor`.
- `render` clones the whole `Editor` each frame (`let editor = editor.clone();`, line 1876) only to satisfy the borrow checker.
- `render` calls `follow_focus(&mut self ...)`, which mutates scroll handles and `scrolled_to` during rendering.
- The view reads model internals directly (`model.profiles.read_only`, `profiles.selected`, `model.fonts.read_only`, `model.mode`, `model.selected`), so the model's encapsulation is nominal.
- `SettingsDialog` carries about 30 fields across three sections (shared focus handles by `Btn`); each section's view state could sit with its model.
- `render_list` calls `selectable_index` per row (O(n^2) over a small list).
- Test `Fixture` uses `std::env::temp_dir()/brindle-settings-<pid>-<name>` with a `Drop` guard and unique case names, so it is parallel-safe (testing R7 OK); several tests assert on hard-coded list positions (`model.select(5)`, `select(7)`), which break if a built-in theme is added.

## b05

- fonts.rs:12-466 — model, the probe and the view in one file, with `use gpui::…` after the model code (lines 460-470); `FontsState` also imports `FontPreview` from `settings.rs`. Not covered by a rule as worded.
- profiles.rs:433, 504 and fonts.rs:54 — the 4..=96 font-size limit and its message `"use a number from 4 to 96"` are written three times (and again in `Config::normalized`); one owner would avoid drift.
- profiles.rs:186-216 — `Values.texts` is a positional array whose literal in `from_profile` repeats `TEXT_FIELDS` order by hand (seven bare `String::new()`), with `at()` using `.expect("text field")`; a reorder compiles and breaks silently.
- profiles.rs:186-197 — `remote`, `engine` and the SSH/container texts all kept at once so values survive a kind switch (documented for `connect_at_launch`); `engine: String` is a closed set (`ENGINES`) held as a string.
- profiles.rs:855-856, 939, 1170 — which fields are selects is a const list (`SELECT_FIELDS`) plus `f if ... contains` guards, not a property of `Field`; adding a select field needs three edits.
- profiles.rs:23-65 — `Field` doc says "in the editor's (and tab) order" but declaration order differs from `FIELDS`.
- profiles.rs:1020, 1160 — `render_profiles` clones the whole `ProfileEditor` and `render_profile_editor` recomputes `editor_result()` (shlex splits, allocations) on every render; `render_fonts` recomputes `errors()` and `render_profile_list` calls `p.description()` per row per render. No IO (so not process R14), but whole-window re-render makes it run often.
- fonts.rs:455, 430 — `catch_unwind(...).unwrap_or(false)` and `cx.update(...).ok()`: a family that can't be probed counts as proportional and a failed update is dropped; documented in the comments.
- profiles.rs:761, 805-806, 831 — `unwrap_or("")` for a missing selected profile; the dump text is display only, but `delete_question` would read `Delete ""?`.
- fonts.rs:196, profiles.rs:558, 715-796 — config-file reads and writes happen on the UI thread in event handlers (not in render; small files). process R13 speaks of connecting/starting, so not reported.
- fonts.rs and profiles.rs both define a `Done` enum (`Nothing`/`Written`); fonts' derives `PartialEq`, profiles' doesn't.
- profiles.rs:1055-1061 — a click handler captures a row index from the render and calls `select(ix)` later; valid while the list doesn't change between render and click.

## b06

- `terminal_element.rs:176-201, 210-446` — `prepaint` does the whole frame's work: a `view.update` for find highlights, a `terminal.update` that resizes (which may queue a server message), the term `FairMutex` lock held for the whole cell loop, and per-frame `Vec`/`String` allocation per batch. process R14 asks `prepaint` to stay free of "heavy allocation and lock contention"; none of these is IO, and the doc's table treats `TerminalElement`'s paint as shell, so not reported, but this is the hottest function in the app and is where the fcis R1 extractions above would also shrink the locked section.
- `terminal_element.rs:43-46` — `cell_metrics` replaces a failed `text_system.advance(font_id, size, 'm')` with `font_size * 0.6` silently; the cell grid then drifts from the real glyph advance with no sign why.
- `terminal_element.rs:536, 549` — `line.paint(..).ok()` discards a paint error with no trace.
- `terminal_element.rs:96-112` — `Frame` keeps `arrow_cursor: bool` and `pointing_hand: bool` that are folded into one `CursorStyle` at paint; one `CursorStyle` field decided in `prepaint` would remove the pair.
- `terminal_element.rs:67` — `cell_highlight(bool, bool, bool)` takes three positional bools; easy to transpose (the test has to name each position).
- `terminal_element.rs:876-881` — the `cell_highlight` test sits inside `box_drawing::tests` (via `super::super`), far from the function it tests.
- `terminal_element.rs:247`, `terminal_view.rs:41-47` — `(line, col)` in `cursor_cell` against `(col, line, side)` from `GridLayout::cell_at` and `(col, line)` in `last_reported_cell`/`hover_cell`: same-typed `usize` pairs in opposite orders in one pair of files.
- `terminal_view.rs:709-723` — a `use gpui::{point, px};` in the middle of the test module after the first test.
- `terminal_view.rs:572` — the debug hook reports a failure with `log::error!` and returns; debug-only, so not railway R6, but the step silently does nothing in the dump.
- `terminal_view.rs:152-153, 149` — `hovered_link` and `layout` are stored derived values; their refreshers (`refresh_hover`/`hover`, `prepaint`'s `layout_cell.set`) are named only by distant doc comments, not beside the field (types R10 is met by the `GridLayout` doc "of the last painted frame", marginal for `hovered_link`).
- `theme.rs:317-321, 142` — built-in themes are chosen by string name and `Default` depends on a runtime lookup of a literal; a `BuiltinTheme` enum (or a `const BRINDLE_DARK: Theme`) would remove the `unwrap` and the `_ => return None`.
- `theme.rs:89, 56` — two luminance definitions (`luminance`, gamma-encoded, for `is_dark`; `relative_luminance`, WCAG, for contrast); easy to use the wrong one.
- `theme.rs:23-34` — `Color::parse` passes `s` to `u32::from_str_radix`, which accepts a leading `+`, so `"+12345"` (length 6) parses as `#012345`; the boundary parse (types R6) is looser than the documented form.
- `win.rs:25-31` — `write_replacing` is plain `std::fs` logic with no Windows API, living in the Windows-only module; its temp name `path.tmp<pid>` collides between two threads of one process writing the same path. fcis R7 says helpers should compile on every platform, though this one does IO and is not pure.
- `win.rs:35-41, 116-153` — `set_app_id` logs a failure and continues; `attach_parent_console` swallows every failure by design (documented). No unit's state shows them; acceptable for process start-up, but nothing tells the user why toasts or console output are missing.

## b07

- `workspace.rs` is 3087 lines: strip rendering, restore/attach, the sessions-palette driver, previews, debug dumps and key lookup in one module (`ports R13` asks for one module per concern). The sessions-palette driver (`refresh_sessions_palette`, `sessions_answered`, ...) and the debug dump (`dump_active_screen`, `describe_*`) are the natural splits.
- The doc comment at 2347-2349 ("The menu of tab `ix` of `count`...") sits on `TabMenuItem`, not on `tab_menu_rows` (2377), which has none.
- `Self::settings(cx).clone()` clones the whole `Config` on the UI thread at 470, 515, 654, 699, 731, 847, 872, 1144, 1611 and 1774 (`apply_tab_themes`, run on every theme-preview change). `render` and `render_tab_bar` clone a `Theme` each frame (`active_theme` at 2542, `Settings::chrome_theme(cx).clone()` at 1990): not IO, but `process R14` asks for no heavy allocation in render.
- `refresh_title` plus `record_session` run on the UI thread every second (1 s poll at 414-436): a process snapshot (`/proc`, or Toolhelp on Windows) and a full `snapshot()` of every tab. The docs sanction the poll, but they are blocking-style work on the UI thread (`process R13`).
- `restore_zones` returns silently when `config.profiles.get(profile_ix)` is `None` (516), dropping a restored tab with no log or message; `config.profiles[...]` direct indexing at 458, 483, 849, 883, 1614 relies on invariants from `default_profile_index`, `resolve_tab`, `local_profile`, `swept_tabs` and `session_profile`.
- `unix_now()` (213) maps a clock failure to `0` (`map_or(0, ..)`), which `SessionsModel::view` then reads as a real "now"; a sentinel, quiet.
- `change_font_size(delta: Option<f32>)` uses `None` for "reset" (1728); an enum would say so.
- `move_tab` (1191) and `reorder_keeping` (2227) are two implementations of tab reordering (see entry 4); `activate_from_strip` ignores `select`'s bool (1081).
- `describe_focus` (1937) is a long if-chain deciding a dump value inside the view; it is a debug hook, so it was not counted under `fcis R1`.
- `on_attention` mixes the bell-dot decision (`if !visible { bell = true }`, 1000) with notification dispatch in one handler; small.

## b09

- `remote-agent/src/main.rs:91` repeats the command list `"check" | "list" | "kill" | "status" | "stop"` that `server::cli::run` already owns, and `USAGE` is defined twice (lines 13 and 15); the two copies can drift.
- `remote-agent/src/conpty/console.rs:297-299` and `:304-308` decode the undocumented `NtQueryInformationProcess` answer (`host & !3`, `& CONNECTED`, non-zero) inline in Windows-only code with no unit test; a pure `decode(host: usize, own_pid) -> Console { NotYet, On(pid) }` would be testable everywhere, and the "when Windows doesn't say, count as connected" default (`is_none_or`) is an untested policy.
- `remote-agent/src/conpty/console.rs:288-290` polls every 1 ms for up to 1 s per closing session; a burst of closes means many polling threads.
- `remote-agent/src/conpty/console.rs:236-262` relies on a detached thread finishing `ClosePseudoConsole`; if the holder exits right after dropping the last session, the thread dies before the close runs (a cleanup in `Drop`, though not a GPUI quit, so `process R16` doesn't apply).
- `remote-agent/src/conpty/console.rs:169` the attribute-list buffer is a `Vec<u8>` (alignment 1) cast to `LPPROC_THREAD_ATTRIBUTE_LIST`; works in practice but is not guaranteed aligned.
- `remote-agent/src/conpty/console.rs:228` `try_clone().ok()` silently drops a failure that "doesn't fail" (the close then just doesn't wait).
- `remote-agent/src/conpty/cmdline.rs:49` sorts the environment with `to_uppercase()` (Unicode) where `CreateProcessW` uses the OS upcase table; equal for ASCII, may differ for non-ASCII names.
- `remote-agent/src/conpty/api.rs:42,122 (console.rs)` `#[cfg_attr(not(test), allow(dead_code, reason = "read by tests"))]` on production fields/functions that exist only for tests.
- `remote-agent/src/conpty/console.rs` tests use fixed 30 s limits (`Instant::now() + 30 s`, `recv_timeout(30 s)`) that don't scale with a factor; the loops poll for a condition, so `testing R7` is met, but `BRINDLE_E2E_SLOW`-style scaling is absent.
- `remote-agent/build.rs:49-52` parses the pinned version out of a PowerShell script by line prefix (`$Version = '`); a format change in the script panics the build with a terse message.
- `remote-agent/src/liveness.rs` is clean against the rules (time passed in, atomics only, three tests, no sleeps); the `as u64` on `as_millis()` is harmless.

## b10

- `remote-agent/src/server/build_id.rs:1-3` says the id "is computed once at startup", but `current()` re-reads and re-hashes the whole binary on every call (`check::refusal` and `core::run` each call it).
- `remote-agent/src/server/check.rs:38` `build_id::current().unwrap_or_else(|_| "unknown".into())` puts a sentinel string into a diagnostic. It is display only, so not `types R7`, and the failure is dropped from the message.
- `remote-agent/src/server/cli.rs:99-101` falls back to `now = 0` when the clock is before the epoch (`map_or(0, ..)`), which makes every age read `0s`.
- `remote-agent/src/server/core.rs:227` `claim` polls with `std::thread::sleep(20ms)` for up to 2 s at startup, before the readiness loop exists. Allowed in effect, but it is the one blocking wait outside the loop (`process R8` reads "one thread per role on a readiness loop").
- `remote-agent/src/server/core/clients.rs:130,155` and `core.rs:592-606` block the core's only loop thread for up to `REFUSAL_FLUSH_LIMIT` (1 s) per refused connection, and up to 10 s on a step-down. Bounded, but every other session's I/O stalls meanwhile. Up to 64 pending connections could each cost this.
- `remote-agent/src/server/core/clients.rs:404` `queue` logs an encode failure and drops the message. A reply a client waits for would then never come, and the failure never becomes the session's or client's state (`railway R6` reads "not a log line alone"; arguably it covers this). It only happens for an oversize frame, which the core's own chunking should prevent.
- `remote-agent/src/server/core/clients.rs:246-249` (`take_over`): the stale client's connection is dropped after one best-effort `flush()`, not a graceful close. Intended (the link is dead), and it is documented.
- `remote-agent/src/server/core/clients.rs:76-78`: `log!` for every refused connection past `MAX_PENDING` can flood `core.log` under a connection flood.
- `remote-agent/src/pty/unix.rs:399-405` `signal_process_group` returns `()` and discards `kill`'s result (also `set_utf8`, whose comment says that is deliberate). Failure is mostly `ESRCH`, which is an expected "already gone".
- `remote-agent/src/pty.rs:34-37,45-48`: `cfg_attr(windows, allow(dead_code ...))` is a platform attribute in the seam's root file, which sits at `remote-agent/src/pty.rs`, beside the named seam directory `pty/`. I treated it as part of the seam.
- `remote-agent/src/pty/unix.rs` is gated on `target_os = "linux"` while the module sets elsewhere use `unix`. It will matter for the macOS port.
- `remote-agent/src/server/conn.rs:326-336,355-358` and `core.rs:874-877`: tests assert how much a socket holds (`assert_eq!(sender.queued(), 0, "a socket holds this much")`, `queued() > 0` after 8 MiB). That depends on the kernel's buffer sizes, not on the rule (`testing R8` speaks of timing and machine speed, so not reported).
- `remote-agent/src/server/conn.rs` tests poll with a fixed 10 s deadline that does not scale with `BRINDLE_E2E_SLOW`. `testing R7` asks for "a scaled limit", but unit tests have no scale variable, so not reported.
- `remote-agent/src/server/core.rs` generic `Core<L: EventLoop>` is used only with `Loop` here (tests use `Core<Loop>`). `ports R6` warns against traits added for mockability, and I could not see another implementor in these files.

## b11

- `holders.rs:132` — `connect` calls `std::thread::sleep(20 ms)` in a loop for up to `CONNECT_RETRY_LIMIT` (1 s) per holder, on the core's single event-loop thread (in `discover(initial=true)`), so a stuck holder stalls every client during startup. No rule's words forbid it; process R8 is about one thread per role.
- `holders.rs:115-116` — `liveness` ignores the results of `endpoint::remove` and `fs::remove_file` with `let _ =`, and the caller then logs "removed what a holder that is gone left behind" even if removal failed.
- `holders.rs:249` — a holder's undecodable message is only logged and the loop continues; the units affected (its sessions) show nothing.
- `holders.rs:270` — `closing_holder_frames` discards the frame error with `Err(_)` and `discard_input()`. This is harmless during shutdown but drops the cause.
- `sessions.rs:218-221` — the doc comment `/// The command line `list` shows; empty for the login shell.` sits above `session_key` (it belongs to `command_line`), so the two comments run together.
- `sessions.rs:222-238` — `session_key` reads `/dev/urandom` directly in the core. On Windows that always fails and the key falls back to time + pid + counter. It is a platform concern (randomness) with no seam item. No rule's words cover it, since there is no cfg or platform type, but it is a seam candidate (ports R4 spirit).
- `sessions.rs:1079-1087` — `replay` queues the whole rendered screen as chunks without consulting the client's queue marks (process R9). It is bounded by scrollback size, and `client_is_over` is checked only after.
- `screen.rs:108-122` — `inject_fault` reads the environment inside the emulator wrapper that `Screen::restore` calls. It is a documented test hook (process R20), and cached in a `OnceLock`, but it is an ambient read inside model-like code (fcis R3 in spirit).
- `holders.rs:16-24`, `process.rs:28` — the core imports `server::platform::{conn::Stream, listen, lock::Lock, pty}` directly, while other items go through facade modules (`daemon`, `dir`, `event_loop`). It is allowed (same item names on both platforms), but the paths are inconsistent.
- `event_loop.rs:22` — `pub type Token = usize` is a bare alias used as the identity of sources and, in the core, of sessions' holder and client. Tokens are never reused (`next_token += 1`), so there's no stale-identity bug, but a newtype would stop mixing with other `usize`s (types R4, SHOULD, out of this batch's checked rule set).
- `manage.rs:59` — `detached_secs` uses `map_or(0, ...)` for "no data", and wire field semantics make 0 mean "not detached or unknown".

## b12

- holder.rs:441 — `pending.conn.read_available().unwrap_or(true)` turns a read error into EOF with no log. The cause is lost (railway R8 spirit).
- holder.rs:1160-1165 — `queue` logs an encode failure and drops the message; the core is never told. The only caller-visible effect is a missing reply.
- holder.rs, `handle` and `spawn` — the single holder thread has no `catch_unwind`, unlike the core's feed. `pty::spawn(&request)` on core-supplied command, args and env lives in `crate::pty` (not audited here); if it can panic, every session dies (process R3's reason for confinement). The holder gives no "why this can't panic" comment.
- holder.rs:665, 671, 677 — `.expect("a core sent this")` rests on `process_core_frames` calling `handle` only while `self.core` is Some. That is true today, but nothing in the type says so; passing `&mut Core` into `handle` would remove the expects.
- holder.rs, `Core::interest` and `Session::interest` — a stored derived value (types R10) refreshed each turn in `update_interests`; the field docs don't name the refresher.
- holder.rs:557-561 — `previous = if core.handover { Handover } else { Crash }` is a tiny decision inline in `lose_core`; the same applies to the `should_exit` rule (1115, takes `now` but is a method on `Holder`).
- platform/unix/dir.rs:37-67 — `ensure_for` decides what is an acceptable directory (owner, mode & 0o077) inside the seam (ports R7 spirit). It is Unix-only by nature, so not testable everywhere, but a pure `fn refusal(is_dir, uid, mode, own_uid) -> Option<String>` would be.
- platform/unix/conn.rs:36-41 — `input_waiting` turns an ioctl failure into `false`, and `wait` (86-106) ignores `poll`'s result; a failed `FIONREAD` or `poll` looks like "nothing waiting".
- platform/unix/daemon.rs:140-144 — `ReadyPipe::read` turns a read error on the pipe into `Some(false)` ("not listening"), no log. Documented in a comment.
- platform/unix/daemon.rs:299-304 — the results of `chdir` and the `dup2`s in `daemonize` are ignored.
- platform/unix/event_loop.rs:33 — `#[allow(dead_code, reason = "for the core's worker threads (task group 4)")]` on `Waker::wake` is a stale planning comment, used in tests only.
- platform/unix/event_loop.rs:388-399 — the signal test installs a process-wide SIGUSR2 handler and overwrites the static `SIGNAL_WAKE_FD`; it runs in parallel with other tests in the binary. Fine today (no other test registers a signal), but nothing guards it (testing R7 spirit, ports R10 is about env vars).
- platform/unix/pty.rs:42-49 — `Pty::write` with an empty slice returns `Err(last_os_error())` (stale errno). The holder never writes empty, but the seam contract is loose.
- holder.rs:1167-1250 — the tests are good (`Tail`, `Slot::claim`), but the policy functions listed under fcis R1 have none.

## b13

- `daemon.rs:143` and `Ready::signal`: any first byte counts as "listening" (`Ok(read) => Some(read > 0)`), where the Linux side counts the role ready only on exactly `ready::BYTE`. The doc explains that Windows reads the first byte, but the byte's value is never checked.
- `daemon.rs:81` and `:309`: the ready-handle placeholder is a string (`{ready_handle}`) substituted into arguments with `arg.replace`. This is an in-band sentinel, and an argument containing that text would be rewritten.
- `daemon.rs:937,941,1013` and `dir.rs:135,140`: tests assert on error text (`contains("inherited pipe")`, `starts_with("usage: …")`). This is acceptable while the text is a single-consumer message (railway R4), but it pins wording.
- `conn.rs:65-69`: `read_timeout: Option<Duration>` has meaning only while `nonblocking` is false, so the two fields are linked. It mirrors the std socket API, so not clear-cut types R1.
- `conn.rs:45-61` and `Need { readable, writable, sent }`: three bools, but not exclusive, so not a types R1 case.
- `event_loop.rs:459-492` (`Loop::wait`): the readiness policy (pump first so a busy source can't starve the others, poll interval while an operation reports through an event, wake handling) is inline in the loop handler. It is the seam's own job and is tested through real pipes, but it is not a pure function.
- `event_loop.rs:346` `Waker::wake_on_exit(BorrowedHandle)` is `pub` and takes a platform type. The only caller is `platform/windows/pty.rs:179`, so it does not leak to the holder (ports R8 holds in practice). It could be `pub(super)`.
- `event_loop.rs:410` `unsafe impl Send for ExitWatch` and the `Arc::into_raw` leak path in `Drop` (`UnregisterWaitEx` failure leaks the reference and the handle). This is documented and bounded.
- `event_loop.rs:262-279` (`Port::shut_down`): a 5 s `DRAIN_LIMIT` blocking wait inside `Drop for Loop`. It is not a quit-time dependency (process R16 concerns Brindle's UI quit), but a stuck packet blocks the role's exit for 5 s.
- `daemon.rs:206` `SystemTime::now()` and `dir.rs:18` `std::env::var_os` are in shell/seam code, so fcis R2/R3 do not apply. `dir.rs` passes the values to the pure `resolve_windows`, as it should.

## b14

- listen.rs:335-351: the `ConnectNamedPipe` result to `Connect` mapping (pending, connected already, client gone, failed) is written inline in `State::listen`. A pure `fn connect_outcome(error: u32) -> Connect` would be unit-testable (fcis R10/R9).
- listen.rs:83, 88: `Identity(Result<Principal, String>)` stores the token error as text only so the type can be `Clone + Eq`; the `ErrorKind` is lost. Not covered by railway R4 as worded (a field, not a function return).
- lock.rs:52: `is_at` reports any failure of `File::open(path)` or the identity read as `false`, so a permission error reads as "the file was deleted". Probably right for the caller, but it merges two outcomes.
- pipe.rs:1189-1210: `wait` chooses a wait strategy (pump one port, wait on events, take turns every `TURN` = 10 ms) and falls back to `thread::sleep` when nothing is in flight. This is polling inside the seam, and `pump` errors are dropped (:1194, :1202). It is the documented design, but it is a decision written in place.
- pipe.rs and listen.rs are 1536 and 1080 lines, mostly tests and `unsafe` state machines; `ports R13` (organise by concern) is met, but `pipe.rs` holds the I/O state machine, the readiness facade and a wait function.
- Tests: absence assertions after a fixed 30 ms wait (listen.rs:680, 688; pipe.rs:1464), a 120 s child sleep (lock.rs:88), and 300/500/100-round stress loops (listen.rs:1013, pipe.rs:1355). They don't assert timing numbers, but the 30 ms absence checks pass trivially on a slow machine.
- pipe.rs:1042-1050: `Pipe::write` returns a fail-fast error only after having queued the bytes (`count` bytes accepted, then `Err`), so the caller can't tell how much was taken; harmless, since the pipe has failed.

## b15

- relay.rs is Linux-only and names `std::os::unix::net::UnixStream`, `std::os::fd::AsFd`, `/proc/self/exe` and `/proc/<pid>/{exe,stat}` directly, outside the server platform seam. It connects with `UnixStream::connect` (299, 314, 475) although the unix module set already exports `connect`. It has no `cfg` of its own, so `ports R4` is not strictly broken. The gate is outside this batch: `#[cfg(unix)] pub mod relay;` in server/mod.rs:17, while main.rs:62-70 gates `relay` on `target_os = "linux"`. The two gates disagree, so a macOS build would compile relay.rs under `cfg(unix)` and hit the missing `/proc`. Not covered by the README's `ports R4` exception file list (remote-agent/src/server/mod.rs and remote-agent/src/main.rs are not named).
- relay.rs runs three blocking threads (stdin reader, core to stdout copier, heartbeat) plus the main thread, rather than the role's one readiness loop (`process R8`). The relay isn't a per-session thread pattern and the module docs justify it. Whether R8's "each server role" covers the relay is unclear, so this is not counted.
- relay.rs:813-824 `test_timings_replace_the_defaults_only_when_valid` asserts defaults only when the process environment happens to have neither variable set, so it silently asserts nothing otherwise. The name also says "only when valid", but no invalid value is exercised. The parse in `Timings::from_env` (132-139) reads env and parses in one closure; a pure `parse_millis(Option<&str>)` would be testable.
- relay.rs:827 `a_zombie_or_a_missing_pid_doesnt_run` never tests a zombie.
- relay.rs:722-724 `heartbeats` returns silently if `Heartbeat.encode()` fails, with no `ended` message, leaving the relay without heartbeats. A constant message can't fail today.
- relay.rs:511 `step_down` hard-codes `req = 1`, and its `Ok(None) => return Ok(())` treats a close before `OK` as success.
- pty.rs:398-408 `program_name`'s `cmd.exe`/COMSPEC fallback and 412-426 `environment`'s merge with the holder's own environment are policy in the holder's seam (`process R1` "no policy"). CLAUDE.md says Brindle always names the program (`windows_default_program`) and sends a complete environment, so both fallbacks are dead in the product. The test code relies on them (and on `exact_env`).
- pty.rs:138-144 `impl Drop for Pty` closes the pseudoconsole, and `hang_up` is documented as "like dropping the Pty". The holder's hangup depends on Drop. This is a holder session, not a Brindle quit, so `process R16` is not engaged.
- pty.rs tests skip with `eprintln!` when `pwsh.exe`/`powershell.exe` is absent (914-917) or when no packaged ConPTY is built (1247-1250). A skipped test reports as passed (`testing R17`/`R9` flavour).
- pty.rs:659-665 test helper `owner()` returns 0 for "no owner" (a sentinel, test-only).
- pty.rs:356-363 `reparent` returns `io::Result<()>` that is always `Ok`, so the signature promises a failure it can't report.
- sys.rs:12-14 Windows `random_u64()` always returns `None`, so `core.rs::random_instance` (core.rs:232-241) always uses the clock-and-pid fallback on Windows for the "random id for takeovers". The seam item exists only for name parity.
- pty.rs:296-299 `end_its_programs` takes a Toolhelp snapshot per kill and calls `OpenProcess` per process, a large per-session cost inside the holder's thread (the doc says a hang-up never waits, but `kill_remaining` does).

## b16

- Negative-existence assertions ("the session is still alive after N ms") cannot be written as a poll for a positive condition without an observable; the core logs could provide one (the core already logs adoption and hang-ups). Eleven of the fixed sleeps above are of this kind.
- core.rs:433 — `thread::sleep(Duration::from_millis(30))` deliberately throttles a slow client; intent is clear, but it makes the test's duration speed-dependent (60 s deadline).
- core.rs:1531 — `DETACH_RACE_ATTEMPTS = 20` repeats a race and asserts none was lost; it detects but does not guarantee the race occurred (the sleep(300) at 1558 is what tries to set it up). Probabilistic detection, not a rule check.
- core.rs:1739-1744 and core.rs:2221-2224 [Windows] — the scope test and the ConPTY REPARENT test `return` early with `eprintln!("skipped: ...")` and report green. `testing R9` ("no skip") is about flakes, so not a violation, but the pass is not "verified" (cf. `testing R17`, `R11`).
- core.rs:762 — `a_panicking_emulator...` needs a debug core (`inject_fault` is `#[cfg(debug_assertions)]` in src/server/core/screen.rs) but the test has no matching cfg; under `cargo test --release` it fails instead of skipping.
- core.rs:1746 — the scopes test sets `BRINDLE_NO_SCOPE=0` so roles really call `systemd-run --user --scope` in the developer's own user manager, outside the test's private directory (`testing R4`: touches nothing of the user's). Scopes end with their processes, so low impact.
- core.rs:1113 — uses `SystemTime::now()` against the core's wall-clock `started`; fine for a test, but ties to the host clock.
- Windows `on_windows` module (core.rs:1766-2316) is unverified from Linux (`testing R17`: its first run on Windows is its first run).

## b17

- holder.rs:398-403 `Throttled::read` calls `std::thread::sleep(self.pause)` (25 ms) before every read: a fixed sleep, but it is the stimulus (a slow core), not a wait; the rule's words ("no fixed sleeps") read broadly, so flagging it for a decision. The test's own assertion (all bytes and the exit) is rule-based.
- holder.rs:1477 `core.quiet_for(Duration::from_secs(1))` and relay.rs:197, :288-290 (`client.next(HEARTBEAT)` / `next(300ms)` expecting `Recv::TimedOut`) wait a fixed time to show an absence; unavoidable for a "nothing arrives" check, but each is a fixed wait that a slow machine only makes less sure.
- relay.rs:201 `client.heartbeats >= 3` and relay.rs:222 `blocked < 10` (10 consecutive 20 ms looks) derive counts from timings; loose enough to hold today, but they are timing-based counts.
- relay.rs:241 `relay_rss < 32 * 1024` KiB is an absolute memory figure that depends on allocator and platform, not on speed.
- holder.rs:1051 `adopted.tails[&1].len() < 4096` (Windows) is a size bound on what the console says, depending on the host's own output.
- support/mod.rs:47 `TIMEOUT` is a constant 30 s and holder.rs:1179 uses a literal 60 s, neither scaled by a factor (testing R7 says "a scaled limit"; `BRINDLE_E2E_SLOW` only reaches the e2e suite).
- holder.rs:183, :799, :877 hold raw `libc` calls (`pipe2`, `FIONREAD`, `setsockopt`) inline in the test file instead of `support/unix.rs`; test code, not a seam violation under ports R4 as worded, but the support module already holds the unix helpers.
- holder.rs:1492-1495 `reparent_...` returns early with "skipped: no packaged ConPTY" and still counts as passed; a capability skip, not a flake skip (testing R9), but the test reports green without running.
- holder.rs:238-245 and relay.rs:316-341 assert on error message text (`starts_with("hold wasn't ready")`, `contains("refusing the server directory")`); fine in tests, but they pin wording the code has no typed outcome for.
- launch.rs sets no `BRINDLE_SERVER_DIR` for its child runs; the commands fail on argument checks (daemon.rs) before any server directory is touched, so it is hermetic in practice, not by construction (testing R4).
- holder.rs:841-848 / :939-946 treat "the socket is full" as two consecutive polls with the same unread count, which depends on the poll interval.

## b18

- `support/mod.rs:1-5` says `unix` and `windows` export "the same items", but they don't: `stays_blocked`, `signal_set`, `program_of`, `context_switches`, `pass_fd_as`, `send_exec_error_down_fd_3`, `start_with` exist only in `unix.rs`; `working_set_kib`, `Window`, `CONSOLE_WINDOW`, `cmd_title`, `inherited_env` only on Windows. Fine while the users are cfg-gated, but the module doc overstates it.
- `support/unix.rs:138-146` and `support/relay.rs:60-71` repeat the same `ExecutableFileBusy` retry loop; one helper would keep the documented cause in one place.
- `support/windows.rs:203-226` `connect` retries `ERROR_PIPE_BUSY` in an unbounded `loop` (each `WaitNamedPipeW` is 5 s), so a pipe that stays busy hangs the test instead of failing at `TIMEOUT`.
- `upgrade.rs:29-32,72` (Windows) `QUIET = 1 s` decides when "the session has printed everything" by waiting for silence (`output_until_quiet`); `support/unix.rs:293-325` `stays_blocked` does the same with `BLOCKED_FOR = 250 ms`. Both are time-based quiet windows, not conditions; the unix one documents why it isn't a fixed sleep, the Windows one doesn't.
- `support/relay.rs:21-29` `IDLE = 1500 ms` / `HEARTBEAT = 300 ms` make the liveness tests (in `tests/relay.rs`, outside this batch) depend on real-time ratios under load.
- `support/windows.rs:394-419` finds server processes by parsing `core[pid]`/`holder[pid]` out of log text, so a log format change silently makes `TempDir::drop` leak roles on Windows.
- `support/windows.rs:599-603` `printed_window` and `support/mod.rs:1062-1089` (`numbered_line`, `check_lines`) are small parsers/checkers with no tests of their own; they are test support, so fcis R8 doesn't strictly reach them.
- `support/mod.rs:976-977` `attach_until` sleeps 50 ms between attach attempts; it is bounded by `TIMEOUT` and is a poll, so not a violation, but each try attaches and detaches a real session.
- `upgrade.rs:462-463` a string literal contains a literal line break instead of `\n` (`"no {entry:?} in` newline `{logs}"`); harmless, inconsistent with the other asserts.
- `support/mod.rs:1054` and `windows.rs` `inherited_env` read the test process's environment into spawns (read only, not a violation of ports R10).

## b20

- `remote-protocol/src/lib.rs:18-19, 23-24`: `#[cfg(unix)] pub mod scope;` and `#[cfg(windows)] pub mod win;` sit in the crate root, which is not a listed seam. They only gate the seam modules themselves (as `platform/mod.rs` does), so I did not count them under `ports R4`. A reviewer could still ask for the cfg to live in a small `platform` module.
- `remote-protocol/src/lib.rs:6` and `relay.rs:4` say "protocols 1 and 2" and "protocol 1 and 2 frames", but client protocols are 1-3 (`SET_LAYOUT`).
- `remote-protocol/src/relay.rs:159-161`: the collision test says "1-13 and 32-41" and skips message 14 (`SET_LAYOUT`), so a future relay number cannot be checked against it.
- `remote-protocol/src/scope.rs:17`: `NO_SCOPE_ENV` (`BRINDLE_NO_SCOPE`) is documented as "tests, the e2e suite", but the relay also sets it in production when no scope is possible (per CLAUDE.md), so it is not a test-only switch under `process R20`, and the doc comment is wrong.
- `remote-protocol/src/scope.rs:26`: `Decision::Plain(&'static str)` carries the reason as free text, and `command(role: &str)` takes `"serve"`/`"hold"` as a string. A small enum for each would make a typo a compile error.
- `remote-protocol/src/scope.rs:128-131`: `.unwrap_or_default()` on a clock error turns "clock before the epoch" into nanos 0. It cannot matter for a unit name.
- `remote-protocol/src/holder.rs:665`: `serde_json::to_value(self).unwrap_or_default()` would turn a serialisation failure into `Null` meta (erasing owner and policy). It cannot fail for this struct today; an `expect("SessionMeta serializes")` would match `railway R9`.
- `remote-protocol/src/holder.rs:670-694`: `from_value` repeats the struct's fields by hand, so a new field must be added in two places or it silently defaults.
- `remote-protocol/src/holder.rs:657-659` (`SessionMeta.started: u64`, `scrollback: u32`): units live only in doc comments (`types R5` covers spaces that meet; none meet here).
- `remote-protocol/src/upgrade.rs:13-16`: `Build.version` is a raw `String` parsed by `semver()` on every `is_newer` call, and the string comes from a remote relay's `RELAY_HOST`. Parsing once into a typed version (`types R6`) would remove the repeated `Option` handling.
- `remote-protocol/src/endpoint.rs:31, 78`: a pipe name is stored as `PathBuf` (`server_dir::holder_pipe(...).into()`), and `server_dir::{core_pipe, holder_pipe, pty_pipe}` return `String` and take the SID and build as `&str`. A `PipeName` type would keep names apart from paths.
- `remote-protocol/src/endpoint.rs:89-95`: `fs::read_dir(...).flatten()` and `into_string().ok()` skip unreadable entries and non-UTF-8 names without a trace.
- `remote-protocol/src/holder.rs:73-79` (`HELLO`/`Hello`): `min`/`max` are repeated from `VersionRange` instead of embedding it (the encoding is frozen, so only a smell).
- `remote-protocol/src/holder.rs:64-71` and the `meta_is_kept_as_json` test: key order of `meta`/`layout` (`serde_json::Value`) is sorted only while no crate in the build enables serde_json's `preserve_order`. No manifest enables it today (`grep` over `Cargo.toml` files found none), but the encoding's determinism (`process R6`) depends on feature unification across the workspace, and the test input is already sorted, so it passes either way.

## b21

- win.rs:554 — in `place_copy`, when another process won the rename, `return std::fs::remove_file(temp)` returns that cleanup error as the function's result even though a valid copy exists. A failed temp cleanup then fails `ensure_helper_copy`. Cleanup elsewhere (line 523) ignores errors. Make the two consistent.
- win.rs:509, 560 — the error text names only the path (`context(installed.display(), error)`, `context(copy.display(), os(error))`), not the action that failed. `railway R8` asks for action then cause. These are borderline; the other `context` calls in the file name the action ("can't open its token"). I did not count them.
- win.rs:208-211 — `check_server` flattens `Refusal` into a string-wrapped `io::Error` of kind `PermissionDenied` (`refuse(refusal.into())`). The typed `Refusal` (`OtherUser`, integrity mismatch) is lost to callers. `ports R9` names "a peer with another identity is a `Refusal`". The kind is a usable value, so I did not count a violation. Keeping the `Refusal` as the error source would let callers branch.
- win.rs:567-584 — `prune_helper_copies` returns `()` and swallows every failure (`let Ok(entries) = ... else { return }`, `let _ = remove_dir_all`). This is documented ("Nothing here fails"), but it is an expected failure with no return value or log (`railway R1`, `R6` spirit). A caller cannot tell a failed prune from nothing to prune.
- win.rs:586-587 — `SYSTEM_SID` is a pub const placed after all the code, used only for DACL checks and tests. It is likely duplicated with the SDDL string in `access.rs` (not read here).
- win.rs:955 — a test polls with `thread::sleep(50ms)` against a fixed 20 s deadline. That is a poll-with-limit as `testing R7` allows, but the limit isn't scaled. No violation.
- win.rs:490 and 507 — `build_of` and `ensure_helper_copy` read the whole helper into memory (twice per call, plus the temp copy). Fine for a few-MB exe, but unbounded.
- wire.rs:16-18 — `SessionId` and `RequestId` are plain aliases, so a session id and a request id can be swapped without a compile error. `types R4` accepts integers inside protocol structs, but these helpers' signatures (`session_bytes`, `split_session`) are the module boundary where a newtype would help. Not counted.
- wire.rs:72-84 — `use super::*;` sits after the first test in the test module, and the first test has an unrelated name style (`fixed_payloads_reject_other_lengths` is fine, the layout is odd). Style only.
- Not checked: whether `fuzz.rs` exercises `split_session`/`session_fixed` directly (`process R6`). Fuzz coverage of decoders that call them is outside the batch.

## b22

- `remote-protocol/tests/transcripts.rs:1-3` — the module doc says the file "pin[s] client protocols 1 and 2, the relay link and H1", but it also pins protocol 3 (`Protocol::Client3`, `client-set-layout`). The doc is stale.
- `remote-protocol/tests/transcripts.rs:14-17, 230-238` — `BRINDLE_REGEN_FIXTURES=1` rewrites every fixture, the shipped v1 and H1 ones included. Nothing in the harness keeps the older fixtures unchanged, as `.claude/rules/remote-protocol.md` and process R5 want. Only review of the diff does. A regen that wrote only fixtures that don't exist yet would enforce it.
- `remote-protocol/tests/transcripts.rs:296-330` — `every_message_type_appears_in_a_transcript` checks the numeric ranges `MSG_DATA..=MSG_SET_LAYOUT`, `MSG_WELCOME..=MSG_PROCESS` and so on. Three weaknesses:
  - A message type added past `MSG_SET_LAYOUT`, `MSG_PROCESS` or `MSG_SET_META` is not checked until someone edits the ranges.
  - It keys on `(kind, msg_type)`, so the two directions are merged. `Data` is type 1 in both directions, so dropping `ToClient::Data` from all transcripts would still pass.
  - It checks message types, not enum variants or optional fields, which is how violations 1-3 got through.
  An exhaustive `match` over `ToCore`, `ToClient`, `ToHolder`, `ToHolder` and `RelayMessage` variants, or a count of variants, would catch them.
- `remote-protocol/tests/transcripts.rs:610-620, 664-666, 1143, 1187-1196` — layouts and metas are free-form `serde_json::Value`s that the test hand-writes in sorted order, because serde_json's `preserve_order` can be switched on by feature unification. process R6 says "sort the keys of free-form maps". Nothing in this file tests that the codec sorts them, so the determinism is only as good as the test author's hand ordering. The codec side is outside this batch.
- `remote-protocol/tests/transcripts.rs:62-71` — `Step::name()` derives the message name by splitting the `Debug` output at `(`, and `describe` branches on `ends_with("End")` and `starts_with("Relay")`. This is fragile if a variant is renamed or gains a `Debug` form without a paren. It only affects fixture comments, which never fail a test.
- `remote-protocol/tests/transcripts.rs:140-141` — `describe` uses `unwrap_or([0; 4])` and so turns a short payload into session 0 in the comment. It is harmless because comments are ignored on decode, but `render` still compares them byte for byte in `encoding_the_messages_reproduces_the_fixtures` (`assert_eq!(rendered, text, …)`).
- `remote-protocol/tests/transcripts.rs:805-807` — the expected `client-hello-refused` error text comes from `client::negotiate(..).unwrap_err().to_string()`. The fixture still pins it, but the expectation is computed from the code under test (testing R6 asks for fixtures the test controls).
- `remote-protocol/tests/transcripts.rs:1087-1090` — misindented `Detach { … }` (formatting only).
- `Process` (lines 383-389, 823-825, 891) is only pinned as `(Some, Some)` and `(Some, None)`. A `name: None` form is not pinned. This is minor, because the field is optional and skipped on encode.

## b23

- `src/instance.rs:296-323` — the Windows listener's `read_request(&pipe)` has no read timeout (the Unix `serve` sets `FORWARD_LIMIT` timeouts at 173-174), so a client that connects and never writes blocks the one listener thread forever. The Unix peer-uid check also has no Windows counterpart (the server pipes check the SID, this pipe does not). No rule words cover either.
- `src/instance.rs:154, 298-304` — an `accept` error is logged and the loop continues (it could spin); a failed `create` on Windows ends the listener thread with only a log line, so the instance silently stops accepting launches.
- `src/instance.rs:173-174, 235-236` — `set_read_timeout(...).ok()` / `set_write_timeout(...).ok()` discard errors, so the "bounded" wait can silently be unbounded.
- `src/instance.rs:67-69` — `fnv1a` duplicates the FNV-1a hash that `remote-protocol` uses for server pipe names, and the peer-uid check duplicates the other seams' peer checks. This is a parallel IPC-naming seam (ports R4's "never start a second seam"), though it is not clearly the same concern.
- `src/instance.rs:22-43` — `Request` carries `profile`, `cwd`, `command` and `attach` as independent `Option`s although `--attach` presumably excludes the rest (types R1 smell; the CLI parsing may enforce it elsewhere).
- `src/config.rs:299` — `Config::path()` reads `BRINDLE_CONFIG` and `dirs::config_dir()` inside an impl of the data type, mixed with the pure parse/normalize methods. Fine as shell code, but it sits on the model type.
- `src/config.rs:337-346` — `Config::parse` logs removed-key warnings itself, so a pure parser has a logging side effect. The warnings are not returned to the caller (the config banner never shows them).
- `src/config.rs:399-408` — `default_profile_index`/`profile_index` hand out positions into `profiles`. They are fine within a call but would break types R3 if a caller kept one across a config reload; I did not trace the callers.
- `src/config.rs:233, 117` — `Profile` derives `Default` with an empty `name`, which `normalized` treats as invalid (types R11 SHOULD NOT; `default_shell()` and test builders use it).
- `src/config.rs:328-335` — `read_to_string` of the config is unbounded (railway R5 says to bound what is accepted from files; a hand-edited config is a trusted source, so this is minor).
- `src/remote/client.rs:98-104, 126-136` — `Unstarted.message` is stored beside `code` and `said`, which it is derived from (types R10 wants the refresher named); it is built in `local/relay.rs`.
- `src/remote/client.rs:146-152` — the `Run` trait exists so `launch_with` can be driven by `Scripted` (ports R6's mockability anti-pattern, though it wraps process IO and not a decision; the fcis R9 split above would make it unnecessary).
- `src/remote/client.rs:109-113` — `Helper::find` hashes the whole helper file on every launch, including every reconnect attempt with backoff. It runs on the connection thread, not the UI thread, so process R13 is not at stake.
- `src/remote/client.rs:187` — `thread::spawn` (panicking) for the stdin copier, where the stderr drain uses `Builder::spawn`.
- `src/remote/client.rs:1700, 1536, 1823` — tests branch on error text via `contains`, because the scenarios flatten errors to `String`. That is test assertion of the message (railway R8), not code branching, so I left it out of the violations.
- `src/remote/client.rs:1216-2019` — about 800 lines of opt-in Docker harness live inside the seam module's `#[cfg(test)]`. This is a file-size note only.

## b24

- `src/remote/local.rs:105-112` — `client_token` silently falls back to the clock (`unwrap_or_default()` gives nanos 0 on a clock error) when `getrandom::fill` fails. Nothing is logged, and the token's only uniqueness is the pid.
- `src/remote/local.rs:531` — `connect_started` retries with `std::thread::sleep(20 ms)` for up to `CONNECT_RETRY`. It runs on a session start thread, so the rule on UI blocking isn't broken, but it is a sleep-retry loop.
- `src/remote/local.rs:431` — `std::env::var_os("LOCALAPPDATA").unwrap_or_default()` turns a missing variable into an empty path (a sentinel) before it reaches `server_dir::helper_copies`.
- `src/remote/hosts.rs:60` — the pure `Hosts` model implements `gpui::Global`. This isn't an `fcis R5` item (the rule names Entity, Window, App, Context), but it ties the model to GPUI. A newtype wrapper in the shell could carry it.
- `src/terminal/server.rs:116-120` — `lost` calls `self.output(..)`, so if a replay is being held (`self.replay.is_some()`), the "Session lost" line is appended to the replay bytes and fed later, ahead of `ChannelEvent::Lost`.
- `src/terminal/server.rs:67, 136-138` — `term.columns() as u16`, and `replay_size(cols: u16, rows: u16)` using zero as "invalid" (filtered at `:68`). A `GridSize` constructor that refuses zero, parsed where the listing arrives, would keep the check out of the feed path.
- `src/terminal/server.rs:58` — `unbounded_send(event).ok()` drops the send error silently (receiver gone); there is no log line.
- `src/terminal/search.rs:8-11` and `server.rs:13-15` — the app imports `alacritty_terminal` directly (a declared dependency in `Cargo.toml`) next to `brindle-term-core`'s re-export. The README rule doesn't forbid it, but the layers in the dependency diagram suggest one route.
- `src/remote/local.rs` — `Endpoint`'s fields differ per platform (`copies` on Windows, `scope` elsewhere), so the struct isn't "the same items on every platform"; this is covered by the ports R4 exception today.
- `src/remote/local.rs:1000-1006` (and `:951-955`, `:1088-1092`, `:886-890`) — the polling `eventually` helper is copied four times in one test module, each with its own 10 s deadline and 5 ms sleep.

## b25

- `connection.rs:920, 937`: `run_local` and `run_relay` re-match `self.transport` and `unreachable!` on the other arm; a local-only method on a type that can be either is an illegal state kept out only at run time.
- `connection.rs:1905-1908`: `Running::reparent` guards "this machine only" with `self.conn.is_relay()` (a bool test on `Transport`) instead of a match on `Host::Local`; it works, but `types R8` names a `Host::Local` arm, and `Transport` duplicates `Host`.
- `connection.rs:366-374, 421-426`: `Slot { link: Option, closed: bool }` and `Watch { link: Option, closed }` pair an `Option` with a flag (`closed` implies no link); one enum would say so.
- `connection.rs:387, 398`: `Stamp::HELD = u64::MAX` is a sentinel inside an atomic (converted to `Option` in `deadline()`, so only a smell), and `Stamp::new`/`touch` read the clock themselves.
- `connection.rs:137`: `static NEXT_CONNECTION` is a process-wide mutable static that feeds report ids; an argument to `made` would keep tests independent.
- `connection.rs:135, 672-683`: `Report` is called under the state lock and "must not block"; a callback that runs with a lock held is a standing hazard for a UI-side `Hosts` handler.
- `connection.rs:1139-1140`: the `SESSIONS` arm locks `state` in a match guard and again in its body; two acquisitions with a gap between them (benign today, because only the reader removes `list`).
- `connection.rs:1093-1099`: an undecodable server frame is logged and skipped (allowed by `railway R5`), but a skipped `DATA` or `EXIT` frame is invisible to the tab; no unit shows it.
- `connection.rs:1126, 1145, 1155, 1173, 806`: failure texts such as `answered with SPAWNED` and `closed before it started` name neither the action nor a cause; the session layer is relied on to add them.
- `connection.rs:1896-1898`: `Running::foreground` takes the connection's `state` mutex from the UI thread (title poll); short, but it shares a lock with the reader thread.
- `connection.rs:140`: `SessionSink` and `Dial` (a fake in tests) are traits with test stand-ins; they sit at real seams (the terminal, the relay launch) rather than around a decision, so `ports R6` is not violated.
- `connection.rs:133-136`: `SessionId`, `RequestId` are plain integer aliases (tests do `spawn.req as SessionId`, 2192), so a request id and a session id can be swapped without a compile error (`types R4`, SHOULD, for ids crossing a module boundary; wire structs stay integers).
- `connection.rs` is 3457 lines with about 1950 of production code and one thread-owning type mixing transport, request bookkeeping, liveness and reporting; the `ports R13` organisation rule (one module per concern) would split request bookkeeping, liveness and state reporting.

## b26

- `src/remote/local/session.rs` `Phase` and `run` (lines 31-283): what to queue, drop, and send in what order once a session starts (clear input on leave, resize then reparent then input, `End` then `Keep`) is a decision written as a mutex-guarded state machine inside the shell. A pure `Starting` model with `leave()` and `finish() -> Vec<Step>` would make it unit-testable without a fake core (fcis R1, R9, R10). It is tested today only through a scripted core.
- `src/remote/local/session.rs:37` and `:176`: `parent: Option<u64>` and `reparent(hwnd: u64)` pass a raw window handle as a bare integer across modules (types R4 would suggest a newtype). The wire struct keeping `u64` is fine.
- `Start.status: String` ("still running", "exit code: N", `"exit status: 1"`, which `unix.rs:313` asserts by text) is display text built in each seam. A typed exit would keep the seams thin (ports R7, R8).
- `src/remote/local/unix.rs:47-50`: the seam decides `scope::current()` versus `Plain("the endpoint uses no scope")` from `endpoint.scope`. That is a small decision in a seam (ports R7). `unix.rs:123-127` returns `None` for both a timeout and a read error, so a read error is reported as "didn't signal", with the cause dropped (railway R8, borderline).
- `src/remote/local/upgrade.rs:135`: `step_down` sets a 60 s read timeout on the stream and never restores it. On the `Err` path `offer` returns that same stream (line 72) as the running connection. Check that `Connection::start` resets it.
- `src/remote/local/upgrade.rs:113-130`: `run` leaks a thread per hung helper, and `sys::kill(pid)` runs after a timeout with no guard against pid reuse (relay.rs documents that window for its own kill).
- `src/remote/local/relay.rs:102` (`Dial`) and `link.rs:14` (`Wire`): traits whose production implementations are one (`Deployed`) and two (`Stream`, `ChildWire`), with in-memory fakes in tests (`fake::dialer`, `fake::link`). Ports R6 says a seam is `cfg`-selected modules, not traits, and testing R12 says "Brindle has no mocks". They wrap IO rather than a decision, so this is not a clear breach.
- `src/remote/local/relay.rs:140-194` (`start`): the post-marker decision (`ending`, `late`, `problem`, `code`) is inline with spawning and killing. `unstarted_message` and `exit_text` are pure and tested, but the choice of whether to wait for the exit is not (fcis R9, R10).
- `src/remote/local/relay.rs:392-397` (`Drop for ChildWire`): kills the relay process on drop. No quit effect depends on it (`DETACH` and `CLOSE` are flushed first), so process R16 is not breached, but it is a Drop-time effect worth keeping that way.
- `src/remote/local/session.rs:33-34`: the `Starting` phase's `input: Vec<Vec<u8>>` is unbounded while a connect or bootstrap (up to 120 s) is pending. Process R9 asks for marks on every producer to slower-consumer queue, so a large paste during a hung connect grows memory without limit.
- `src/remote/local/pipe.rs:241-253`: `ended` and `code` (pure mappings of Win32 codes to `io` results) have no unit test in the file, but they take `windows::core::Error` and can't be built on Linux (fcis R8 vs R7). `address` drops the `Endpoints::of` error and prints "the pipe of <dir>" (railway R8, borderline).
- Test-only `cfg(windows)` and `cfg(unix)` helpers (`relay.rs:539-595`, `:643`) and `cfg!(unix)` branches (`:665`, `:683`) are test code and allowed by testing R17's platform pairs. They are not reported under ports R4.
- Unix and windows `start` duplicate the "ready" and "signalled" computation and the log of the trampoline's stderr, since the logic sits in each seam. Hoisting it to a shared pure function would satisfy ports R7.

## b27

- `src/remote/mod.rs:121-139` `wait_process` polls with `try_wait` and a 20 ms sleep; it is only called from `client.rs` on a background thread (deploy), so process R13 holds, but a `wait` on a helper thread with a kill signal would avoid the polling.
- `src/remote/mod.rs` bundles unrelated things: helper lookup, process waiting, window hiding and `remote_command` (ports R13 asks for organisation by concern; no `utils` module, so not a violation).
- `src/remote/mod.rs:30-38` `hide_window` mutates and returns `&mut Command` while `remote_command` builds an owned `Command`; the Ssh and Container arms both end in the same call (fine).
- `src/session.rs:192-195, 210-214` `TabState::Local` is documented "A local tab" and also carries `remote: Option<RemoteTarget>` and records remote tabs; `Host` exists but the persisted form uses `Option<RemoteTarget>` (types R9 allows it at the boundary, but the name misleads).
- `src/session.rs:27` `MIN_SIZE: (f32, f32)` and `WindowState::{width, height}: Option<f32>` are logical pixels by comment only; no newtype (types R5 is not broken here, but a `LogicalPx` type would keep units apart).
- `src/session.rs:36-50` `WindowState` keeps `width`/`height` as two independent `Option<f32>`, so a state with only one of them is representable.
- `src/session.rs:310-334` no fsync of the directory after the rename, so a crash can lose the rename; the temp name is fixed (safe under the `session.lock` instance lock only).
- `src/session.rs:398` `unwrap_or(ZoneId(1))` gives the active zone a made-up id when there are no leaves; a tree always has a leaf, so it is a placeholder, not a sentinel the wire sees.
- `src/session.rs:668` `SessionStore` derives `Default` and tests build it with `SessionStore::default()` then set the private `lock` field directly; `new(lock)` is the constructor that expresses the invariant (types R11 SHOULD, nothing invalid is representable, so not a violation).
- `src/remote/mod.rs:42` `HELPER_NAMES` includes the Linux archive's file name for every platform; on Windows `BRINDLE_REMOTE_AGENT` names the Linux helper, a coupling the tests pin.

## b28

- `src/terminal/pace.rs:33-42` (`drain`) reads `Instant::now()` and decides when to stop (`start.elapsed() < BUDGET`) inside a stream-draining helper, in a module whose `step` is deliberately pure with `now` as an argument. It is shell-side, because it takes a stream and a callback, so `fcis R2` (pure code) does not strictly apply. The stop decision is the reason `drain_stops_at_the_budget` needs sleeps (violation 13).
- `src/terminal/mod.rs:994-998` (`impl Drop for Terminal`) still detaches or closes in `Drop`. `process R16` is met because the quit path does it explicitly (`quit_with`). The Drop remains a second path, and `leave` has no "once" guard of its own despite the doc comments at 929-940 ("later calls, and the drop, do nothing"). The guard is presumably in `Session`, which I did not read.
- `src/terminal/mod.rs:942-960` — the doc comment above `set_layout` (946-951) is the leftover first half of `leave`'s comment. The text sits on the wrong function.
- `src/terminal/mod.rs:870-881`, `:903-922`, `:1184-1187` — the 1 s title poll, `working_directory` and `spawn_request` do `/proc` reads and `is_dir` stats on the UI thread (the Windows `dir.is_dir()`, `read_link` on `/proc/<pid>/cwd`, `read_to_string` on `/proc/<pid>/stat`). They are cheap, and `process R13`'s named calls return at once, but a network path could block.
- `src/terminal/mod.rs:1298` — `resolve_color` takes the whole `&Terminal` (which holds a `Task`, a `Session` and a `FairMutex`) only to call `default_color`. It should take `&Theme`, since `palette_color` is already pure over it. This is close to `fcis R5` for a function used as pure.
- `src/terminal/mod.rs:200-206` — the `Backend::Remote` name ("read-only message") reads as `Host::Remote` and invites confusion. `Backend::Message` would be clearer.
- `src/terminal/mod.rs:255,258` — `pub theme` and `pub last_activity` are public fields on a type with invariants.
- `src/terminal/mod.rs:457,1169,1209` — `host.is_local()` bool tests are used where `Host` is a two-variant enum. The `Host::Local` match arms elsewhere (873, 906, 910) are the better shape. `types R8` is satisfied either way.
- `src/terminal/mod.rs:514-543` (`flush_sync`) holds a small decision (`t <= Instant::now()`) inside an entity. `fcis R10` says to question it. It is a one-line comparison.
- `src/terminal/links.rs:111-116` (`open`) reads `BRINDLE_LINK_OPENER` and chooses the program inside the function that also acts. It is the documented stand-in at the seam, but a pure `choose_opener(env: Option<String>) -> Opener` would make the choice testable. `open_default` (Windows) repeats `open_with`'s scheme check and thread spawn.
- `src/terminal/keys.rs`, `mouse.rs` — clean: pure, tested, no panics on input. `mouse::report` indexes nothing and `32 + 1 + v` is in `usize`.
- `Cargo.toml:16` — the app depends on `alacritty_terminal` directly as well as through `brindle-term-core`, and `keys.rs`, `mouse.rs` and `mod.rs` import it. The dependency rule's picture routes it through `term-core`. `ports R3` binds only the crates below the app, so no rule is broken.

## b30

- `term-core/src/replay.rs:92-241` — `render` is a 150-line function that decides what to emit and writes it in one pass. It is pure (it returns bytes), so `fcis R9` doesn't apply, but it is hard to review in parts. Splitting out the mode, cursor and tail sections would help.
- `term-core/src/replay.rs:542-559` — `Out::char` and `Out::text` encode a char to UTF-8 in two near-duplicate ways.
- `term-core/src/replay/tests.rs:20-58, 73-92, 114-153` — the test harness re-declares `Size`, `Titles` and the stream-order status logic (a hand-rolled `status_events` scanner) that `emulator.rs` already provides through `Emulator`, `Titles` and `osc::Scanner`. It is test-only, but it is a second copy of "status lifetimes in stream order", which could drift from `Emulator::feed`. Driving the round trips through `Emulator` would remove it.
- `term-core/src/emulator.rs:74-76, 198-200` — `Emulator::new` and `resize` silently clamp 0 columns or rows to 1 (`size()`), where a typed non-zero size would show the invalid value at the boundary.
- `term-core/src/emulator.rs:138-142` — `Found::Notice(_) => {}` drops notices, which is intended (the server doesn't notify). The comment says only that the listener drops events, not that notices are dropped here.
- `term-core/src/replay.rs:304` — `char::from_u32(0xf0000 + l as u32).unwrap_or(' ')` silently degrades the scroll-region probe if the screen has more than about 65k lines. The failure is impossible at real sizes.
- `term-core/src/emulator.rs:46-49` — `Titles` exposes its inner `Arc<Mutex<Option<String>>>` to the whole module (`self.titles.0.lock()` in four places). A small accessor type would keep the title cell in one place.

## b31

- `shell_integration.rs:366` `.max().unwrap_or(0)` in `Command::last_line`: 0 as a stand-in for "no lines" is unreachable (the prompt is always present), but it is a sentinel in shape.
- `shell_integration.rs:98` `windows_paths: bool` is really a path-style choice and would read better as a two-variant enum.
- `shell_integration.rs:566` `Box<dyn Iterator>` closure in `track`, allocated on every processor call (hot path); a pure-function split would also help `fcis R9`.
- `shell_integration.rs:384,382` `RowId = *const Cell`, a row's identity taken from the address of its cells. It relies on alacritty's row rotation (documented) and is compared only, but it is fragile against a grid reallocation on resize (resize goes through `snapshot`/`restore` instead).
- `Parser::advance` takes the `Mutex<Marks>` and locks it in several places per call, so the lock order (parser, then term, then marks) lives in this code's call structure alone.
- `shell_integration.rs:991,994` the test depends on the host platform (`cfg!(windows)`) rather than building `Parser::posix()` or a Windows-style parser on both.
- `status.rs:441 describe()` is dump formatting for the app's `--dump-screen-after` inside the shared state crate (it has a test, so it is only a placement smell).
- `status.rs:14-15` the `BASE64` engine config is repeated in `encode`, which uses `general_purpose::STANDARD`: encode and decode use different engines (padding differs only on decode), so they are consistent today.

## b32

- tests/architecture.rs:44-54 — the I/O needles are text matches on path spellings. `use std::{fs, env};`, `std::io::`, `File::open` after a renamed import, `Command::new` and `std::os::` (term-core, ui, wire modules) are not caught. `the_wire_modules_do_no_io` and `term_core_does_no_io_and_reads_no_clock` therefore only catch the common spellings. A grouped import is the likeliest miss.
- tests/architecture.rs:148-167 — comment skipping is line-prefix only, so a trailing `// std::fs` or a string literal containing `env::` could false-positive. Block comments are not skipped. `Scope::BeforeTests` stops only at `#[cfg(test)]` immediately followed by `mod tests`, so any other test layout (an extra attribute between, or a separate test file in `server/core/`) is scanned as production code, which can false-positive but not hide a violation.
- tests/architecture.rs:298-312 — `the_server_core_and_holder_name_no_platform_type`: the `windows::` needle also matches `platform::windows::…` paths (the preceding char is `:`), so a legitimate import of the seam's Windows module would fail the test. It passes today, so none exist. `libc::` is not matched in the form `use libc;` or `extern crate libc`.
- tests/architecture.rs:314-330 — doc comment says "the opt-in suites keep their own runner" and cites testing R3, but it asserts only `harness = false`. The runner features R3 lists (env var, `--list`, `--self-test`, `--shard`, no `#[ignore]`) are unchecked. testing-architecture.md's Enforcement section is honest that only the harness flag is covered.
- tests/architecture.rs — every check selects real files (checked: `term-core/src`, `ui/src`, `remote-agent/src/server/core/` with 6 files, `core.rs`, `holder.rs`, and the six wire modules `client`, `holder`, `wire`, `version`, `relay`, `fuzz`, all exist). Every check asserts non-vacuity (`!files.is_empty()`, `!named.is_empty()`, the positive-direction test). `the_wire_modules_do_no_io` has a non-empty list by construction but would panic on a renamed file, which is loud, not vacuous. Dependency checks do not look at `[workspace.dependencies]`, `[patch]` or `[replace]`.
- tests/architecture.rs:200 — doc comment claims railway R2 ("no anyhow below the app"). That holds through the closed allow-list (anyhow is not on any), not through a direct check. Railway R2's "return their own error types" half is not checked, as expected for review-only.
- tests/architecture.rs — helpers `manifest`, `dependencies`, `sources` have no unit test (only `names_path` does). The `offenders` scope logic (`BeforeTests`) has no test.
- tests/e2e/brindle.rs:456-496 — `Run::run` assembles the argument list, config text and environment inline in the shell. That is decision code (what `--step-interval`, `-p`, `--no-restore`, `-e` to pass) that fcis R9 would split into a pure builder returning an argv, testable by `self-test`.
- tests/e2e/brindle.rs:514-518 — `Event::Theme` rewrites the config with `config(theme)`, dropping the run's `extra_config`, `top`, `padding` and `default_profile`. A run that mixes `.config(..)` with `.theme_at(..)` loses them on the reload.
- tests/e2e/brindle.rs:476-496 — the child inherits the parent's environment apart from the removed `WAYLAND_DISPLAY`. It sets `BRINDLE_STATE_DIR`, `BRINDLE_CONFIG`, `BRINDLE_SERVER_DIR`, `BRINDLE_NO_SCOPE`, `DISPLAY`, but not `BRINDLE_NOTIFY_LOG`, `BRINDLE_LINK_OPENER`, `BRINDLE_SERVER_HELPER` and `BRINDLE_REMOTE_AGENT`. A developer's own value of those leaks in unless the case overrides it. Also `HOME` and `XDG_*` are inherited. Whether a case touches the user's D-Bus or `xdg-open` depends on each case remembering. A hermetic default (`env_clear`-then-allow, or explicit removals) would back testing R4.
- tests/e2e/brindle.rs:218 and 557 — `.ok()` on the kill command and on `read_to_string` of the dump. If the dump read fails the error shows later as "no screen in the dump", a misleading cause.
- tests/e2e/capture.rs:254-259, 267 — `watch_window` returns silently on X connect or atom failure. The receiver sees a closed channel and `run` then reports "no viewable window for pid … within N s", which misattributes an X connection failure to a missing window.
- tests/e2e/capture.rs:300-311 — on the 3 s deadline `grab` returns the all-black frame without saying so; the later colour check then fails with a colour summary. A "never presented a frame" error would name the cause.
- tests/e2e/capture.rs:329-347 — `request_close` sends a `WM_DELETE_WINDOW` client message with XSendEvent to a window by id. It is not XTest/xdotool and goes to the window named, not the focused one, so testing R2's reason does not apply. But R2's list is "synthetic OS input", and this is an event the harness injects; CLAUDE.md lists the same on Windows (`WM_MOUSEMOVE`) as "a message, not input". Worth stating in testing R2 that targeted window messages are allowed.
- tests/e2e/brindle.rs:41-70 — `CHROME_BACKGROUND`, `CHROME_BORDER`, `ALT_CARD_BORDER`, `ALT_CARD_MUTED` are hand-computed copies of the app's mixing formulas (`Theme::ui_tokens`, `card_border`). They come from the `e2e` theme so testing R6 is met, but a change to the mix constants silently drifts these unless a case fails.
- tests/e2e/brindle.rs:227, 554 and capture.rs:265, 295, 310 — 100 ms polling sleeps in the harness are poll-the-condition waits (acceptable under testing R7's "poll for the condition" in spirit; R7 names unit and real-process tests anyway).
- tests/e2e/brindle.rs:512, 540 — timeline waits are `sleep` until a scaled due time. Not a fixed sleep before an assertion.

## b33

- `tests/e2e/cases/mod.rs:194` and `cards.rs`: `step_time` hard-codes "Brindle runs its steps one second apart, the first 1.5 s in", a copy of the app's debug-hook timing. If `--step-interval` or the first-step offset changes, the pixel cases drift without any error.
- `cards.rs`, `colors.rs`, `commands.rs`, `input.rs`, `links.rs` and others use absolute `dump_at(N)` and `capture_at(N)` times after a step instead of `--wait-for`, for example `links.rs:68` "Late in the hover step, so the repaint has landed even under load". These are scaled but are still timing padding.
- `local_server.rs:550`, `links.rs:32` and `notify.rs:8`: `read_to_string(...).unwrap_or_default()` makes a missing log file and an empty log the same. In `reattach_quit`, `replayed.is_empty()` would pass vacuously if the log path were wrong. Check that the file exists, or write it first.
- `local_server.rs:1311`: `std::fs::write(saved, ...).ok()` drops a write error, and the case later asserts `report.exists()`.
- `local_server.rs:297` `mark` opens `/proc/<pid>/fd/1` with no `cfg(unix)` gate. It is only called by Linux-only paths, but it compiles on Windows.
- `instance.rs:39` `Launcher::launch` sets `BRINDLE_STATE_DIR`, `BRINDLE_CONFIG` and `BRINDLE_SERVER_DIR` but not `BRINDLE_NO_SCOPE=1`, which `Run` sets. If the instance hand-off failed, the second launch would start a core through `systemd-run --user --scope` outside the case. It also does not set `DISPLAY` explicitly. This only matters on the failure path, and the server directory is still private.
- `fonts.rs:40`: `Outcome::Skip` when the terminal and UI fonts coincide. This is an environment skip, not a flake skip, but it is a skip nonetheless.
- Many cases, such as the `reattach_*` family, are long multi-run sequences with magic wait limits (`10.0`, `15.0`). The limits are scaled through `Server::wait`.
- `cards.rs:257` and `ring_between`: the ring check `count >= band * 0.5 && count <= band * 2.0` is a loose pixel-count heuristic, but it is not timing- or machine-speed-dependent. `cards.rs:332`: `color.0 > color.2 + 60 && color.0 > color.1` is a weak "orange" test with a message that names `#854b08`.
- Pure test helpers have no `--self-test` coverage: `near_px`, `dev`, `step_time`, `shown_pid`, `slashed`, `sh`, `echo`, `echo_sum`, `echo_pid`, `ring_between`, `Line`. The brief lists fcis R8 for pure functions, but the e2e crate has no libtest.
- `mod.rs:23`: `mod restore;` and `mod reload;` are out of alphabetical order, which is cosmetic.

## b34

- tests/e2e/cases/search.rs:161: `panel()` re-derives the chrome token (`mix(BACKGROUND, black, 0.35)`, then `mix(.., FOREGROUND, 0.04)`) that `Theme::ui_tokens` computes. A change to the derivation silently breaks the case; the e2e theme could expose the popover colour as a constant.
- Timing-coupled setup: many cases rely on "one step per second" and `dump_at`/`capture_at` constants instead of a `--wait-for` barrier. Examples: palette.rs `step_time` (comment: "Brindle runs its steps one second apart, the first 1.5 s in"); search.rs `PRINT_LINES` followed by `.find(..)` and `capture_at(4.5)` with no wait for `line-300`; reload.rs `theme_at(4.0)` and `capture_at(6.0)`; restore.rs `close_window` (comment "cd at ~1.5 s, recorded by the 2 s poll, window closed at ~2.5 s", where the case depends on the poll beating the close). These are the "extra padding" anti-pattern, but testing R7 is worded for unit and real-process tests, so none are counted.
- tests/e2e/cases/restore.rs:196: `ctx.sleep(2.0)` orders the second launch after the first. It is scaled, but it is a fixed sleep (the anti-pattern list names it); waiting for the first run's window or lock would be a condition. `thread::scope` plus two dumps at 7.0 and 3.0 make the case order-by-clock.
- tests/e2e/cases/remote.rs and restore.rs hand-roll Brindle spawns (`run_brindle`, `run_until_window_closes`) parallel to `Run`. This is what caused violations 1-3: each hermetic setting has to be repeated by hand.
- palette.rs and profile_menu.rs duplicate `step_time`, `expect_focus` and `expect_closed`.
- remote.rs `logged`/`log_line` and several restore.rs reads use `read_to_string(..).unwrap_or_default()`. An unreadable log looks the same as an empty one, so a poll just times out without the cause. This is test code; railway R1/R6 are not claimed.
- remote.rs `ClosedTab::run` proves absence with a 10 s sampling loop (`started` pids, 100 ms sleeps). The comment admits a short-lived `ssh` can be missed; the failed-attempt count is the real check.
- remote.rs `FIXTURE_IMAGE` caches the first case's `Result` process-wide. A failed first build repeats for every later remote case.
- restore.rs `run_until_window_closes` calls `child.kill()` on timeout without `wait()`, and has no guard like remote.rs's `ChildGuard` for an early `?` return.
- remote.rs `mark_tty`/`Docker::mark` write straight to another process's tty output through `/proc/<pid>/fd/1`. This is not synthetic OS input (testing R2 holds), but it bypasses the debug hooks; the file is Linux-only and relies on the mod-level gating of the cases.

## b35

- tests/e2e/cases/win.rs:206-210: `tab_menu_background` gates its probe on `start.elapsed() < ctx.secs(4.0)` from the harness spawn. `Probe`'s own doc (win.rs:299-304) says the harness's clock and Brindle's differ by seconds on a cold start, so a time gate is the unreliable thing it warns against. It is a padding wait, not a poll on a condition (testing R7's wording, though R7 names unit and real-process tests).
- tests/e2e/main.rs:75-77: `Ctx::sleep` is a fixed-sleep helper (scaled). Its callers in other files (`restore.rs`, `multi_window.rs`) are outside this batch. The testing doc lists "sleep before an assertion" as an anti-pattern.
- tests/e2e/main.rs:287, 296, 301 and win.rs:115, 360, 712: fixed, unscaled limits (5 s, 2 s, +30 s, 120 s) on the server stop, role wait and zoom wait. They are limits, not delays, so not counted under testing R5, but under SLOW on a loaded runner they are the first to flake.
- Fixed `dump_at` and `capture_at` times (for example settings.rs:37, 121, 173 with 11.7, 13.3 and 17.3 s; zones.rs; shell.rs) are scaled by `Run`, but they are padding against step counts, not `--wait-for` conditions. The `step_time` helper is copied into five case files (`mod.rs:196`, `tab_menu.rs:12`, `zone_menu.rs:12`, `tabs.rs:70`, plus `palette.rs` and `profile_menu.rs` outside this batch). One shared helper would remove the drift risk.
- tests/e2e/cases/win.rs:80, 179, 387-390: probes return strings ("no window", "no holder", "waiting", "clicked") that callers then match with `==` or `contains`. They are stringly-typed outcomes (railway R1 and types R7 spirit), but they are test-harness polling values, so not counted.
- tests/e2e/main.rs:81, 314, 319 and status.rs:9, win.rs:381, 407, 513: `.ok()` and `unwrap_or_default()` on file reads and writes. A missing artifact or log becomes an empty value. Later assertions fail anyway, but a failed `ctx.save` is invisible. Not counted: these are test helpers.
- tests/e2e/main.rs:111: unknown `--flags` are silently ignored (needed for cargo's libtest args), so a typo such as `--shrad 2/3` runs every case without a message.
- Pure helpers with no unit test in the self-test: `tabs.rs` `measure_plus`, `distance` and `Strip::rect`; `settings.rs` `with_shell_profile_theme` and `shown_section`; `win.rs` `count`, `under`-like helpers (only `under` is self-tested). They are test code, so fcis R8 (own-file unit tests for pure functions) and testing R1 are not counted as violations. The harness's own parsers (`parse_shard`, `take_shard`, `query::replies`, `palette_reply`) are self-tested.
- tests/e2e/cases/win.rs CONFIG sets `[notifications] when = "always"` but only `passthrough` sets `BRINDLE_NOTIFY_LOG`. Any other case that causes a notification would reach the real desktop (testing R4's "touches nothing of the user's"). No case here does today.
- tests/e2e/cases/win.rs:100-134, 182-188 send `WM_*` messages to Brindle's own window with `SendMessageW`. This is allowed by CLAUDE.md and does not touch other windows, but it sits close to testing R2's "synthetic OS input" line, so keep it limited to the message set it uses.
- Every case function returns `Result<Outcome, String>` (the shared `Result` alias). It is covered by the railway R4 exception in spirit, but the tests do not compare error text, so nothing is tagged or counted.

## b36

- `tests/perf/cases.rs:103`: the throughput script starts with a fixed `sleep 1` to wait for the first layout resize. Strictly `testing R7` covers unit and real-process tests, but a layout-resize wait is exactly the "poll the condition" case. The e2e `--wait-for` hook is not available here because the script is the `-e` command.
- `tests/perf/cases.rs:53`: the `try_wait` loop with a 100 ms sleep and a deadline is a bounded poll, which is fine. `limit` is not scaled by any slow factor, and the perf suite has no `BRINDLE_*_SLOW` equivalent (`testing R5` is worded for e2e).
- `tests/perf/cases.rs:73`: `ensure!(result.samples == ctx.samples)` fails the case on a count mismatch. It is not a numeric threshold, so it is consistent with `testing R14`. Exiting 1 for a case that "couldn't measure" (`main.rs:215`) is documented in a comment.
- `tests/perf/main.rs:157`: `date -u` shells out, and on failure falls back to the fixed directory name `run`. A second run would write into the first one's directory and overwrite its `summary.toml`. A shared name is a `testing R4`-flavoured hazard. Taking the time from `SystemTime` would be pure Rust.
- `tests/perf/main.rs:65`: `Ctx::save` swallows write errors with `.ok()`. The files are diagnostic artifacts, so low severity.
- `tests/perf/vtebench.rs:29-31`: `head()` ignores the exit status of `git rev-parse` and returns an empty string. A broken checkout therefore reads as "not at the pinned commit" and triggers fetch/checkout. The recorded `commit` could be `""`.
- `tests/perf/main.rs:120-122`: the filter and selection logic (`filters.iter().any(contains)`) sits inline in `main`. It is untested by `--self-test`, which covers only the report parsers and renderer. This is minor, since there is no libtest here.
- `tests/e2e/winproc.rs`: `first_argument` is a pure function, but its test is in `self_test()`, which only compiles and runs on Windows (`#[cfg(windows)] mod winproc`). The Linux CI therefore never checks it. That is acceptable under `testing R17` but worth knowing.
- `tests/e2e/winproc.rs:122` `alive` returns false when the process can't be opened (access denied), which is indistinguishable from "exited". `parent()` calls `all()`, which takes a full snapshot per call.
- `tests/perf/report.rs:64-65`: `field(...)? as u64` casts an `f64`. A negative or NaN value saturates silently. This is a harness parser of Brindle's own output, so low risk.
- Placement: `report.rs` is clean on `fcis`. `parse_*`, `cell`, `table` and `render` take values and read no clock, env or files. Only `Summary::load` does IO, as a shell function, and `render` is covered by the fixture self-test.
- Imports: nothing points the wrong way. `winproc` uses `brindle_remote_protocol::win` and `windows`, both allowed for the app's tests. The perf suite uses only std, serde, toml and x11rb.

## b39

- ui/src/components/card.rs:58 — `CardHeader::new` documents "A header with no title" as `SharedString::default()` (empty string for none) while `Dialog::title` uses `Option`; the header always renders `trigger_label(title)`. No rule covers it as it is display-only, but it is the pattern types R7 names.
- ui/src/components/combobox.rs:118 / :154 — `placeholder: SharedString::default()` is the "no placeholder" value, same pattern.
- ui/src/components/command.rs:350 — `reveal_pending: Option<usize>` keeps a row index across a frame; it is cleared by `rebuild`, so it is safe today, but it is a position held between calls (types R3's spirit).
- ui/src/components/button.rs:194 — `attach(button, self.tooltip.filter(|_| !self.disabled))` and :145 (`padding_x` with an icon) are small decisions made in `render` with no unit test; `button_style` carries the rest. Under fcis R10's "questioned in review"; harmless.
- ui/src/actions.rs:129,142 — the test pins `29` and `7` binding counts, so adding a binding breaks it; fine as a pin but brittle.
- ui/src/components/command.rs:565, 524-528 — the `let Some(item) = self.item(item_ix) else { return div() }` and "list asks for a row the filter has since removed" arms silently draw an empty row for a bug state; the code is defensive rather than wrong.
- ui/src/components/card.rs:196-199 — `on_header_bounds` runs a user callback during prepaint with the documented rule "must not notify"; that is a constraint on the owner, held by documentation only (process R14 concerns IO, which is not involved).
- ui/src/components/context_menu.rs — nothing to report; it is a thin shell over `Menu` and `menu_overlay` (its pure parts, `MenuAnchor::at` and `menu_item_release`, are in menu.rs, outside this batch). It has no unit tests of its own, and has no pure logic.

## b40

- `ui/src/components/dropdown_menu.rs:132`, `ui/src/components/popover.rs:274`, `ui/src/components/select.rs:615`: an unmeasured trigger (`None`) becomes `Bounds::default()`, a zero rect at the window origin, and the menu is placed against that on the first frame (`select.rs:441` does the same with `px(0.)`). The `Option` is handled by defaulting rather than by a distinct case.
- `ui/src/components/menu.rs:379`: `Menu::bounds` reads "not painted yet" as `width <= 0`. It is turned into an `Option` there, which is fine, but it leans on a zero-width sentinel from `ScrollHandle`.
- `ui/src/components/select.rs:698` and `:739`: `SelectMenu` re-checks `!self.options[ix].disabled` before emitting `Picked`, although `ReleaseOver::Option` is documented as "an enabled option" and only enabled rows carry the handler. Two places decide "enabled".
- `ui/src/components/select.rs:222-226`: `(false, _) => Open` on `Activation` is a wildcard in a tuple pattern. It is not a `_ =>` arm, but a new `Activation` variant would silently open.
- `ui/src/components/select.rs:664-690`, `:792`: `SelectMenu` copies `options` and `selected` as snapshots at open. They are fixed for the menu's life, so this is safe, but they duplicate `Select`'s state with no refresh rule written beside them.
- `ui/src/components/select.rs:444` and `menu.rs:46-50`: the private `select::MenuEvent` shares a name with `menu::MenuEvent`, and `menu.rs` imports `MenuRelease`/`ReleaseOver` from `select.rs`, so `menu` depends on `select`'s pure types. The shared pointer-release model could live in its own module.
- `ui/src/components/mod.rs:130-156`: `FocusRing` mixes pure geometry (in `style::ring_geometry`, tested elsewhere) with drawing. It is fine, but the drawing half has no test here.

## b41

- ui/src/primitives/focus_visible.rs:87-101, 139-164 — the `Observers` trait exists only so a test can substitute a `Recorder`. This is close to ports R6 and testing R12 (no mocks), but it wraps registration, not a decision, so it is not counted as a violation. A test that asserts "init binds no keys" through a recorder is weaker than a structural check.
- ui/src/primitives/focus_visible.rs:33-40 — the per-window `Modalities` map never drops a closed window's entry (documented as accepted). Not covered by a rule.
- ui/src/primitives/layer.rs:166-173 — `is_top` takes `&mut App` and prunes, so a query mutates global state. `stack` and `depth` are the read-only variants.
- ui/src/primitives/nav.rs:19 and ui/src/filter.rs:14 — a persisted `highlight: Option<usize>` and `filter` returning indices. They are valid within a call (types R3), but a caller that keeps them across a list rebuild would break the rule. This is OK as written (the app's `Command` uses `keep`), but it relies on the callers.
- ui/src/components/tabs.rs:131-147, 393-398 — `TabActivate`, `TabContextMenu` and `on_close(&usize)` all deliver bare indices to the owner. They are valid for the render that built the closure, but the owner must not store them. Only `DraggedTab` (violation 1) crosses frames.
- ui/src/components/tabs.rs:497-511 — the list handles left/right/home/end itself in `on_key_down` with raw key strings, instead of through actions in `default_bindings()`. The module doc and the `list_keys_reach_the_list` test justify it, and it is not binding. Still, it is the one place the crate interprets keys outside its bindings.
- ui/src/components/toggle.rs:42-43 — `label: Option` and `icon: Option` allow `(None, None)` (types R1 in spirit). The constructors prevent it, and the builder methods cannot reach it.
- ui/src/icon.rs:144-159 — `Icon::ALL` is a hand-kept copy of the variant list. `every_icon_is_listed_once` checks duplicates, not completeness, so a new variant omitted from `ALL` would skip the geometry test (types R2/R10 spirit).
- ui/src/icon.rs:192-245 — the geometry (centring, arcs, rect corners) is computed inside `paint` against `PathBuilder`, so only `stroke_width` and the tables are unit-tested.
- ui/src/primitives/tooltip-related: ui/src/components/tooltip.rs:80-93 — the Escape-dismiss rule (`key == "escape"`, `was_shown`) is an inline closure in `Tooltip::build`, with no pure helper or test.
- Test names that are not sentences (testing R15, outside this brief's rule list): `fuzzy`, `no_stops`, `nowhere_to_go`, `nothing_to_highlight`, `from_outside_the_stops`.
- ui/src/lib.rs:20-31 — wide glob re-exports (`pub use actions::*; components::*; primitives::*`) make the public surface implicit. Not covered by a rule.

## b42

- `ui/src/primitives/overlay.rs:57-61` `backdrop()` hardcodes `hsla(0,0,0,0.5)`, documented as "isn't a token"; a theme cannot change it.
- `ui/src/primitives/overlay.rs:232` `stack.and_then(|stack| stack.depth(id)).unwrap_or(0)` turns "layer not in the stack" into depth 0 (a sentinel-like default; harmless because `lifted` is false then, but the value is meaningless).
- `ui/src/primitives/overlay.rs:212-312` `render_with` mixes element construction with three structural branches (`self.modal`, `self.blocks_outside`, `lifted`); the lifted choice is in a tested pure fn, but the modal/blocks_outside shape is not (fcis R10 would question it; it is layout, not a policy).
- `ui/src/primitives/overlay.rs:77` `Overlay` has no test for the `open`/`close`/subscription lifecycle (needs a window; only its pure helpers are tested). Covered by the gallery `--check`, per ui rules.
- `ui/src/primitives/placement.rs:103-113` `available_height` ignores `trigger` and `offset` for `Left | Right` (parameters unused on those arms); fine but the signature suggests otherwise.
- `ui/src/primitives/placement.rs:310` `Anchored::prepaint` rounds the shift (`.round()`) outside `place`, so the placement tested in pixels differs by up to half a pixel from what is drawn; the rounding is not in a tested function.
- `ui/src/style.rs` `Tokens::dark` is a stored hint beside the colours and tests (`the_dark_hint_picks_the_dark_refinements`, `switch_colours_for_each_state_and_set`) construct tokens whose hint disagrees with their colours (`Tokens { dark: false, ..neutral_dark() }`); the flag is defined in `theme.rs` (outside this batch), where types R1/R10 should be checked (what refreshes it).
- `ui/src/style.rs:760-761` and tests rely on `crate::components::RING_WIDTH` equalling `TAB_LIST_PADDING` by a test only, two constants for one fact.
- `ui/src/style.rs:1416` `const { assert!(... <= ... + f32::EPSILON) }` in a test compares floats with an epsilon fudge; fine but brittle.

## b43

- `ui/src/text_input.rs:1-8, 281, 383` — doc comments name app concepts ("the palettes' filter", "settings dialog's fields", "`--dump-screen-after`", "the `--send` debug hook"), and `describe()`/`insert()` exist for the app's debug hooks. This is not a code dependency, but it sits against `ports R2`'s "knows nothing of ... the app".
- `ui/src/text_input.rs:317-321` — `last_line: Option<ShapedLine>`, `last_bounds: Option<Bounds>` and `text_origin_x: Pixels` are always set together in `paint` and read together in `offset_for_position`. That is three fields for one fact (a `Option<LastPaint>` would be one). It is close to `types R1` but not clearly covered by it.
- `ui/src/text_input.rs:482, 321, 315` — `offset_for_position` returns `0` when there is no painted line yet, and `text_origin_x` and `scroll_x` start at `0`: `0` stands for "unknown" (close to `types R7`). An `Option<usize>` return would make the caller decide.
- `ui/src/text_input.rs:291-302, 738` — `scroll_for_caret` takes and returns bare `f32` pixels, and the call site converts with `.into()`. Taking `Pixels` would make the unit visible, though only one unit is involved.
- `ui/src/text_input.rs:774` — `line.paint(...).ok()` silently drops a paint error. It is an expected failure value discarded with no trace (touches `railway R1`/`R6`, but a per-frame paint has no unit to show it on).
- `ui/src/text_input.rs:752` — `prepaint` mutates the entity (`input.scroll_x = scroll`) without a notify. This is an idiom GPUI allows, and it makes the render pass stateful.
- `ui/src/text_input.rs:244` — `let word = |c: char| is_word_char(c);` is a redundant closure.
- `ui/src/theme.rs:163-171` vs `197-208` — `ui_theme()` panics when `UiTheme` is unset (documented, a Brindle bug, so within `railway R9`), while `ui_font()` falls back to `SYSTEM_UI_FONT`. The two accessors are inconsistent. `ui_font`'s fallback arm is not unit-tested, because it needs an `App`.
- `ui/src/theme.rs:55-57` — `hex` returns `Hsla` via `rgb(..).into()`, so the "source" hex values are only checked through the `bytes()` round trip in tests. That is fine, just noting that the equality tests all go through float conversion.
- `ui/Cargo.toml` — no findings. The only dependency is `gpui = "0.2.2"` (`ports R2`, `R3`), with no dev-dependencies. The `text_input.rs` and `theme.rs` imports are `std::ops::Range`, `gpui` and `crate::` only.

Checked and clean: no IO, env, fs, clock or global reads in `EditState` or `scroll_for_caret` (`fcis R2`, `R3`, `R5`); no `cfg` attributes in the three files (`ports R4`); no `anyhow` (`railway R2`); no `_ =>` arm on `Motion` (its `match` names every variant, so there are no `[EXCEPTION railway R3]` instances); no non-test `unwrap` or `expect`; no key binding; UTF-16 and byte offsets are converted only by `EditState` methods (`types R5`); `EditState`, `single_line` and `scroll_for_caret` have deterministic unit tests in-file with no sleeps or env (`fcis R8`, `testing R7`, `R8`); the `Tokens` methods and constructors have in-file tests.

