# Tasks

No behaviour changes. Each task changes a pure function's parameters, gives its tests plain values (no temp tree, no real clock, no headless app, no process-wide global: `testing R1`, `R7`), and updates the shell callers to gather the value. Tests are named as sentences (`testing R15`). Nothing here sleeps or asserts on elapsed time (`testing R8`). Line numbers are from `846f07b`.

This change depends on `enforce-platform-seams` (change 5). Group 5 builds on its scanner, so start group 5 after change 5 has landed. Groups 1–4 don't need it.

## 1. Shared crate: the scope's unit name and the server directory (`brindle-remote-protocol`)

- [ ] 1.1 `remote-protocol/src/scope.rs` (b20 #1, design §1):
  - add the pure `pub fn unit_name(role: &str, pid: u32, subsec_nanos: u32) -> String`, giving today's `brindle-{role}-{pid:x}{nanos:08x}`;
  - `command` takes `unit: &str` in place of `role`;
  - add the shell `pub fn new_unit_name(role: &str) -> String`, which reads `SystemTime::now()` and `std::process::id()`, and delete the private `unit_name`.
  - Update both callers to pass `&scope::new_unit_name(…)`: `remote-agent/src/server/platform/unix/daemon.rs:218` and `src/remote/local/unix.rs:55`.
  - Tests:
    - `unit_names_are_the_role_pid_and_nanoseconds_in_hex`: `unit_name("serve", 0x4d2, 0x2a)` is `brindle-serve-4d20000002a`;
    - the existing `command` test asserts `--unit=<the unit it was given>` exactly, not `starts_with`.

  Verify: `cargo test --locked -p brindle-remote-protocol scope::`, then `cargo build --locked --workspace`.
- [ ] 1.2 `remote-protocol/src/server_dir.rs` (b20 #2, design §5):
  - add the pure `choose(server_dir, runtime_dir, home) -> io::Result<PathBuf>` and `choose_windows(server_dir, local_app_data, elevated) -> io::Result<PathBuf>`, with today's bodies minus `std::path::absolute`;
  - `resolve` and `resolve_windows` become `choose…(…).and_then(std::path::absolute)`, with doc comments saying they read the working directory. Their callers are unchanged.
  - Move the tests to `choose`/`choose_windows`, comparing exact `PathBuf`s built with `join` on both sides, and drop the `#[cfg(unix)]` and the "differs per platform" comments. A relative variable (`rel/dir`) comes back as given.

  Verify: `cargo test --locked -p brindle-remote-protocol server_dir::` and `cargo build --locked --workspace`.

## 2. App: pure helpers take values or a probe

- [ ] 2.1 Editor command (b07 #8, design §1):
  - in `src/terminal/cmdline.rs`, add `pub fn unix_editor_command(visual: Option<&str>, editor: Option<&str>, path: &str) -> (String, Vec<String>)`, which is `/bin/sh -c "<editor> <quoted path>"` with `visual.or(editor).unwrap_or("nano")`. Its doc comment says an empty `$VISUAL` is used as given, unlike `windows_editor_command`.
  - move `shell_quote` there from `src/workspace.rs:2506`. Both get `#[cfg_attr(windows, allow(dead_code))]`.
  - the `#[cfg(unix)]` `editor_command` in `workspace.rs:2492` passes `std::env::var("VISUAL").ok()` and `std::env::var("EDITOR").ok()` to it.
  - update the module doc: it also builds the Unix editor command.
  - Move the `shell_quote` test from `workspace.rs:3027`, and add `the_unix_editor_is_visual_then_editor_then_nano` (VISUAL wins, then EDITOR, then nano; `code -w` passes through; the path is quoted).

  Verify: `cargo test --locked -p brindle terminal::cmdline::` and `cargo test --locked -p brindle workspace::`.
- [ ] 2.2 Windows' default program (b28 #1, design §2):
  - `windows_default_program(path_dirs, comspec, is_file: impl Fn(&Path) -> bool)`;
  - the `#[cfg(windows)]` `default_program` in `src/terminal/mod.rs:1223` passes `|path: &Path| path.is_file()`.
  - Replace the temp-tree tests with tables over a set of "files" (`|p| files.contains(p)`): pwsh before powershell in any directory, PATH order within each, then `%COMSPEC%`, then `cmd.exe`. Delete `RemoveOnDrop` and `bin_dirs`. The "a directory named like the program" case goes, because that is `Path::is_file`'s behaviour, not this function's.

  Verify: `cargo test --locked -p brindle terminal::cmdline::` on Linux. The Windows caller is compiled by the PR's `windows` job.
- [ ] 2.3 The helper's lookup (b27 #8, design §2), in `src/remote/mod.rs`:
  - `beside_exe(exes: &[PathBuf], names, is_file: &dyn Fn(&Path) -> bool)`;
  - `find_helper_in(variable, exes: &[PathBuf], is_file)` and `find_server_helper_in(server_variable, remote_variable, exes, windows, is_file)`;
  - `exe_paths` stays (the shell's `canonicalize`), and `find_helper` and `find_server_helper` pass `&exe_paths(&exe)` and `&|path: &Path| path.is_file()`.
  - Rewrite the tests at `:272-366` as tables over a set of files:
    - the variable's path is used when it is a file, and is an error naming the variable when it isn't;
    - helper names in order;
    - the resolved executable's directory after the link's (`exes = [link, target]`);
    - Windows looks only for `brindle-remote.exe`.

  Keep one test of `exe_paths` with a real symlink, `#[cfg(unix)]` (D1 allows a test-only platform `cfg`), so the shell's resolution keeps a test. Verify: `cargo test --locked -p brindle remote::tests`.
- [ ] 2.4 Home directory (b23, `config.rs:285`, design §1):
  - `config::expand_tilde(path: &str, home: Option<&Path>)` and `Profile::working_directory(&self, home: Option<&Path>)`;
  - callers:
    - `src/main.rs`'s `parse_args_from(args, home: Option<&Path>)`, with `parse_args()` passing `dirs::home_dir().as_deref()`, for its three uses (`-d`, `--latency-log`, the prefix);
    - `src/terminal/mod.rs:1184-1186`, which reads `dirs::home_dir()` once and uses it for both the profile's directory and the fallback;
    - `session::resolve_cwd`, with `Some(home)`.
  - Tests:
    - `profiles_and_overrides` (`config.rs:469`) asserts `Some("/home/u/src")` exactly with `home = Some("/home/u")`;
    - add `a_tilde_without_a_home_stays_as_written`;
    - `main.rs`'s argument tests pass `Some(Path::new("/home/u"))`, and one asserts `-d ~/src` gives `/home/u/src`.

  Verify: `cargo test --locked -p brindle config::` and `cargo test --locked -p brindle tests::` (main's module).
- [ ] 2.5 Session restore (b27 #9, #11; `record_zones`; design §1, §2, §4), in `src/session.rs`:
  - `resolve_tab(tab, config, home, is_dir: &dyn Fn(&Path) -> bool)`, passed through `resolve_zone_node` and `resolve_cwd`;
  - `resolve_tab` no longer logs. `RestoredTab::Local` gains `missing_profile: Option<String>`, the recorded name when the config has no such profile.
  - add `pub struct ZoneRecord { pub cwd: Option<PathBuf>, pub session: Option<ServerSession> }`. `record_zones(tree, active, leaves: &HashMap<ZoneId, ZoneRecord>)` and `ZoneNodeState::record_node` take the map.
  - Shell:
    - `Workspace::restore` (`workspace.rs:474`) passes `&|dir: &Path| dir.is_dir()` and logs `missing_profile` with today's message;
    - `ZoneTreeView::zone_record` becomes `zone_records(cx) -> HashMap<ZoneId, ZoneRecord>`, leaving out zones that only show a message, and `workspace.rs:551` passes its result.
  - Tests: `resolve_fallbacks` (`:853`) and `zone_tree_round_trip_and_fallbacks` (`:1215`) take a set of existing directories, with no `/nonexistent`, `std::env::temp_dir()` or `/` lookups. Add `a_missing_profile_is_named_on_the_restored_tab`, and update the `record_zones` tests (`:1042-1058`) to build maps.

  Verify: `cargo test --locked -p brindle session::` and `cargo test --locked -p brindle workspace::`.
- [ ] 2.6 Launch token (b23, `instance.rs:57`): `Request::launch_token()` becomes the free function `instance::launch_token(xdg: Option<String>, startup_id: Option<String>) -> Option<String>`, the first that is set and not empty. `Args::instance_request` (`main.rs:230`) reads the two variables. Add `the_launch_token_is_the_first_one_set` (empty skipped, none set gives `None`). Verify: `cargo test --locked -p brindle instance::`.
- [ ] 2.7 Endpoint tokens (b24 #12): `host_token(base: &str, host: &Host)`, with `shared` (`src/remote/local.rs:152`) passing `client_token()`. `each_remote_endpoint_has_a_token_of_its_own` (`:1120`) uses a fixed base (`"t"`), asserts `host_token("t", &Host::Local) == "t"` and keeps its stability and distinctness checks, and stops reading the global. Verify: `cargo test --locked -p brindle remote::local::tests`.
- [ ] 2.8 The connection's closes and the relay's reason (b25 #6, #7, design §1, §3), in `src/remote/local/connection.rs`:
  - **Closes:**
    - `State::closing(&mut self, session, now: Instant)` and `State::give_up_closes(&mut self, limit, now: Instant)`;
    - `send_close` and `forget` pass `Instant::now()`;
    - `end_when_unused` reads `now` once per pass and uses it for `give_up_closes`, `idle_since` and the wait from `closes_due`.
    - Add `closes_are_given_up_once_the_oldest_waited_the_limit_unlinked`, on `State::default()`: none before the limit, none while `linked`, and all of them, oldest first, at the limit.
  - **The relay's reason:**
    - add `enum RelayAccount { Silent(Duration), Said(String) }` and `relay_link_ended(name, ending, account: RelayAccount)`;
    - the reader (`:949`) builds `Silent(timings.dead)` when `stamp.expired()`, else `Said(wire.why())`.
    - Rewrite `a_relay_links_reason_is_the_silence_then_the_process_then_the_stream` (`:3095`) over the enum, with no panicking closure.

  Verify: `cargo test --locked -p brindle remote::local::connection::`.
- [ ] 2.9 The rule text the probes need (design §2), in `docs/architecture/functional-core-imperative-shell.md`, in the same commit as 2.2, 2.3 and 2.5:
  - R5 gains the probe definition, worded as in design §2;
  - R9's callback sentence adds "or for a probe (R5)";
  - Idioms gets a "A probe for a path the function derived" entry citing `scope::decide` and `cmdline::windows_default_program`;
  - the anti-patterns gain "A callback that reads an entity, a window or a process inside pure code. Instead: the shell gathers the value (R5)."

  Verify: `grep -n "probe" docs/architecture/functional-core-imperative-shell.md` shows R5, R9, Idioms and the anti-pattern.

## 3. Helper: the server core's sessions take `now` (`brindle-remote-agent`)

- [ ] 3.1 b11 #3 (design §1, "Wire, state file, terminal server"):
  - in `remote-agent/src/server/core/sessions.rs`: `Session::mark_active(&mut self, now: Instant)`, `tell_foreground(&mut self, now: Instant)` and `adopted(holder, held, now: Instant)`;
  - `session_spawned` (`:363`), `session_output` (`:437`), `client_input` and `replay` read `Instant::now()` once at their top;
  - in `core/holders.rs`, the `WELCOME` handler reads it once before its loop (`:418`) and uses it for `adopted` and for `RECONNECT_GRACE`'s `until` (`:430`).
  - Add to `sessions.rs`'s tests:
    - `an_adopted_session_is_detached_and_active_since_now`;
    - `marking_activity_records_the_given_instant` (`tell_foreground` also clears `foreground`).

  Verify: `cargo test --locked -p brindle-remote-agent --bin brindle-remote server::core::`, then the real-process `cargo test --locked -p brindle-remote-agent --test core` (adoption, `PROCESS`), unchanged.

## 4. App shell: the origin id, titles, settings, rows and the palette

- [ ] 4.1 The layout's origin and keys (b01 #5, #6, design §1):
  - **Pure:**
    - `layout::origin_id(pid: u32, since_epoch: Duration) -> String`, giving today's `{pid:x}-{nanos:x}`;
    - `record(tree, active, keys: &HashMap<ZoneId, String>, origin, index, title)`.
  - **Shell:**
    - `main.rs` makes the id once in the `Application::run` closure and passes it to `open_window(launch, origin, cx)` and `Workspace::new(launch, origin, window, cx)`, which keeps `origin: SharedString`;
    - `Workspace::quit` and `send_layouts` pass it to `ZoneTreeView::quit_all`, `send_layout` and `layout`, which take `origin: &str`;
    - `ZoneTreeView::session_keys(cx) -> HashMap<ZoneId, String>` gathers the keys that aren't empty.
  - Tests:
    - `origin_ids_are_the_pid_and_nanoseconds_in_hex`, exact;
    - the layout tests build a `keys` map in place of `key_of`, and drop the "made once per process" assertions;
    - the doc comment on `Layout::origin` stops linking a function that reads the clock.
  - `CLAUDE.md`'s "the process's `layout::origin_id`" says it is made once in `main.rs`.

  Verify: `cargo test --locked -p brindle layout::` and `cargo test --locked -p brindle zone_view::`.
- [ ] 4.2 Settings (b03 #7, #8, design §1):
  - `Settings::new(config, config_error, installed: Vec<String>)` is pure;
  - add `Settings::from_platform(config, config_error, cx: &App)` (`installed_families(cx)`, `new`, the two "using … family" log lines);
  - `main.rs:421`, `:791` and `cancelling_a_preview_restores_the_tokens` call `from_platform`.
  - `a_font_preview_overrides_the_config_until_cleared` becomes a plain `#[test]` on `Settings::new(Config::default(), None, names(&[…]))`, without `in_headless_app` or the `settings.installed =` overwrite. Add `font_sizes_are_clamped_to_5_to_96` (a profile size of 1 and of 200, with zoom).

  Verify: `cargo test --locked -p brindle settings::`.
- [ ] 4.3 Titles (b07 #9, design §3), in `src/workspace.rs` and `src/zone_view.rs`:
  - add `pub(crate) fn program_title(osc_title: Option<&str>) -> Option<&str>` (today's filter, `cfg!(windows)` included);
  - `terminal_automatic_title(osc_title, profile_name, process_name: Option<String>)`;
  - `refresh_displayed_title(displayed, manual_title: Option<&str>, automatic_title: &str) -> bool`;
  - `ZoneTreeView::refresh_zone_title` asks `foreground_process_name(snapshot)` only when `program_title(…)` is `None`, and the first title (`zone_view.rs:491`) passes `None`;
  - replace `ZoneTreeView::automatic_title` and `Tab::automatic_title` with `ZoneTreeView::active_title() -> SharedString`, the cached title of the active zone or the profile name, read in `Tab::refresh_title` after `refresh_titles`.
  - Rewrite the title tests (`workspace.rs:3045-3083`) with values. Add `a_programs_own_title_is_not_empty_or_blank` for `program_title`.

  Verify: `cargo test --locked -p brindle workspace::`.
- [ ] 4.4 Rows and commands (b07 #9, design §3):
  - `profile_rows(profiles, shortcuts: &[Option<SharedString>])`, with `Workspace::profile_rows` gathering one shortcut per profile;
  - `actions::palette_commands()` returns every labelled action in table order;
  - `open_command_palette` (`workspace.rs:1383`) keeps those with `types.contains(…) || window.is_action_available(…)`.
  - Update `profile_rows_and_menu_items_match_the_picker` (`workspace.rs:2859`) and the `actions.rs` tests (`:494`, `:504`, `:626-629`). The availability case filters in the test and still asserts the labels and their order.

  Verify: `cargo test --locked -p brindle actions::` and `cargo test --locked -p brindle workspace::`.

## 5. Architecture checks, with their docs in the same commit

Write each check's planted-text test first and watch it fail, then the check itself (`testing R13`). Each check's docs change in the same commit as it (`tests/architecture.rs`'s own rule). Verify each task with `cargo test --locked -p brindle --test architecture`.

- [ ] 5.1 The import-following matcher (design §7):
  - add `struct Import { local: String, path: String }`, `use_paths(text) -> Vec<Import>` and `resolved_paths(line, imports) -> Vec<String>`;
  - `offenders` matches a needle against resolved paths and imports as well as with today's `names_path`.
  - Planted tests:
    - `a_needle_follows_grouped_nested_and_renamed_imports`: `use std::{fs, env};`, a multi-line `use std::{\n time::{Instant, SystemTime},\n env,\n};`, and `use std::time::Instant as Clock;` with `Clock::now()`;
    - `a_needle_matches_a_path_written_in_full`: `std::time::SystemTime::now()` and `::std::env::var`;
    - `a_needle_does_not_match_a_longer_name`: `std::environment`, `prefs::load()`, a commented-out `use`.
  - The four existing source checks still pass. If one now finds a site, fix it here when small, else stop and raise it (design §7).
- [ ] 5.2 The pure-module check (design §6):
  - factor change 5's test-item skipping into `production_lines(text) -> Vec<(usize, String)>`, reused by `platform_cfgs`;
  - add `PURE_MODULES`, `MUTABLE_STATICS` and `FS_METHODS`, and the tests `the_pure_modules_read_nothing_ambient`, `every_pure_module_exists` and `the_pure_module_list_matches_the_fcis_table` (with `fcis_whole_modules(doc) -> BTreeSet<String>`, expanding `{a,b}` and skipping the term-core crate row, which `term_core_does_no_io_and_reads_no_clock` covers);
  - add `a_pure_module_reading_the_world_is_found`, whose planted file has a grouped `std` import, a `static OnceLock`, `.is_file()`, a full-path `SystemTime::now()`, a renamed `Instant`, and a `#[cfg(test)]` `Instant::now()` that must not count.
  - In the same commit, `functional-core-imperative-shell.md` (design §10):
    - "How it applies here" splits into "Whole pure modules (checked by `tests/architecture.rs`)" and "Pure items in shell modules (held by review)", with the rows design §6 and §10 list;
    - Enforcement names the new tests and says R2 and R3 are checked for the whole modules, and review holds the items;
    - `.claude/rules/app.md`'s "Enforced by" names the check.
  - Verify also: the check passes only with groups 2 and 4 done. Reverting 4.1's `origin_id` locally makes it fail on `src/layout.rs`; don't commit that.
- [ ] 5.3 Manifests (design §8):
  - `names` follows `{ workspace = true }` to the root's `[workspace.dependencies]`, taking its `package =`;
  - add `patched(table) -> Vec<String>` and `no_dependency_is_patched_or_replaced` over the five manifests;
  - add the planted tests `a_workspace_dependency_counts_by_its_package` and `a_patch_or_replacement_is_found`;
  - `crates_below_the_app_name_only_their_allowed_dependencies`'s doc comment says `railway R2`'s dependency half holds through the closed lists (`anyhow` is on none), and that the other half is review's.
  - In the same commit, `ports-and-adapters.md` R3 gains the patch sentence, and its Enforcement section names the two checks.
- [ ] 5.4 Runners and D3 (design §9, §10):
  - add `runner_gaps(suite, text) -> Vec<&'static str>`, `each_opt_in_runner_has_its_variable_list_and_self_test` (reading each `[[test]]` target's `path`) and `a_runner_without_a_self_test_is_found`.
  - In the same commit:
    - `functional-core-imperative-shell.md` R8 says it binds production code and that the harnesses' pure helpers are held by `--self-test` (D3);
    - `testing-architecture.md` R3's `--self-test` clause (parsers MUST, other pure helpers SHOULD) and the "harness helper" row in "How it applies here";
    - `testing-architecture.md`'s Enforcement names the runner check;
    - `.claude/rules/tests.md` says harness helpers are tested in `--self-test` and names the check under "Enforced by".
- [ ] 5.5 Verify the group:
  - `BRINDLE_E2E=1 cargo test --locked -p brindle --test e2e -- --self-test` and `BRINDLE_PERF=1 cargo test --locked -p brindle --test perf -- --self-test` pass; neither opens a window;
  - each commit that changes `tests/architecture.rs` also changes the doc it enforces (`git log --stat` on the branch);
  - `functional-core-imperative-shell.md`'s Enforcement section says that only review stops an item row (not a whole module) from calling `Instant::now()`.

## 6. Final checks

- [ ] 6.1 `cargo test --locked --workspace` and `cargo build --locked` (which builds `brindle-remote` for the e2e run) pass on Linux.
- [ ] 6.2 Run the e2e cases this change's shell edits reach, headless (no window on the desktop, so no asking first): `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --locked --test e2e -- commands-rename-tab zones- palette-lists-for-tab profile-menu- restore- local-server-layout local-server-reattach local-server-session colors-reload settings-fonts instance-`. They cover titles, rows and the palette, restore, layouts and the origin, the core's start (scope unit), reloads (`Settings::from_platform`) and the second launch (launch token). All pass, unchanged.
- [ ] 6.3 Windows: the PR's `windows` job builds and tests the `cfg(windows)` callers (`default_program`, `resolve_windows`, `editor_command`), and `windows-e2e` runs its cases. Both green. This machine can't run them (`testing R17`: don't report Windows as verified from Linux).
- [ ] 6.4 `scripts/lint-commits`: `refactor` for the code, `test(architecture)` for the checks, `docs(architecture)` for doc-only commits, subjects lowercase. Manual checks: none, since nothing needs real keys or a pointer.

## Workflow follow-up

- Archive in the implementation PR: `openspec archive keep-pure-modules-pure` (`skip_specs`, so no spec sync), committed as `docs(openspec): archive keep-pure-modules-pure`.
