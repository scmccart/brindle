# Design

## Context

See proposal.md (Why). This change closes the audit's finding 13 (F13) entries for `fcis R2`, `R3` and `R5`, decision D3, and the architecture test's gaps from the report's "Enforcement gaps" section. "bNN #K" is item K under Violations in `docs/architecture/audits/2026-11-16/bNN.md`; b05 and b23 number nothing, so their entries are cited by `file:line`. The code is unchanged since the audit (`ab6462b`, re-checked on `846f07b`).

**The sites, as they read today:**

| Entry | Site | What it reaches |
| --- | --- | --- |
| b01 #5 | `src/layout.rs:69-77` `origin_id()` | `SystemTime::now()`, `std::process::id()`, a `static OnceLock` |
| b01 #6 | `src/layout.rs:84-91` `record(.., key_of: &mut impl FnMut(ZoneId) -> Option<String>, ..)` | entities through `cx` (`zone_view.rs:688-691`) |
| (same shape) | `src/session.rs:175` `record_zones(.., leaf_of)`, `:123` `record_node` | entities through `cx` (`workspace.rs:551`) |
| b03 #7, #8 | `src/settings.rs:86-102` `Settings::new(config, error, cx: &App)` | `cx.text_system()`; its tests need `Application::headless()` |
| b07 #8 | `src/workspace.rs:2490-2495` `editor_command` (unix) | `$VISUAL`, `$EDITOR` |
| b07 #9 | `workspace.rs:2252` `terminal_automatic_title(.., foreground_process: impl FnOnce)`, `:2268` `refresh_displayed_title(.., automatic_title: impl FnOnce)`, `:2325` `profile_rows(.., shortcut: impl Fn)`; `actions.rs:251` `palette_commands(available: impl FnMut)` | the process snapshot (`/proc`), the window's keymap, `window.is_action_available` |
| b11 #3 | `remote-agent/src/server/core/sessions.rs:156-158` `mark_active`, `:163` `tell_foreground`, `:171-195` `adopted` | `Instant::now()` |
| b20 #1 | `remote-protocol/src/scope.rs:127-133` `unit_name`, called by `command` | `SystemTime::now()`, `std::process::id()` |
| b20 #2 | `remote-protocol/src/server_dir.rs:46`, `:72` `resolve`, `resolve_windows` | the working directory, through `std::path::absolute` |
| b23 | `src/config.rs:285-295` `expand_tilde`, through `Profile::working_directory` | `dirs::home_dir()` |
| b23 | `src/instance.rs:57-62` `Request::launch_token()` | `$XDG_ACTIVATION_TOKEN`, `$DESKTOP_STARTUP_ID` |
| b24 #12 | `src/remote/local.rs:281-291` `host_token(host)` | the process's `client_token()` `OnceLock` |
| b25 #6 | `src/remote/local/connection.rs:330-341` `State::closing`, `give_up_closes` | `Instant::now()`, `elapsed()` |
| b25 #7 | `connection.rs:1737-1755` `relay_link_ended(.., why: impl FnOnce() -> String)` | `wire.why()`, which may wait for the relay process to exit |
| b27 #8 | `src/remote/mod.rs:49-119` `exe_paths`, `beside_exe`, `find_helper_in`, `find_server_helper_in` | `dunce::canonicalize`, `Path::is_file` |
| b27 #9 | `src/session.rs:446-459` `resolve_cwd`, from `resolve_tab` and `resolve_zone_node` | `Path::is_dir` |
| b27 #11 | `src/session.rs:502-504` in `resolve_tab` | `log::warn!` |
| b28 #1 | `src/terminal/cmdline.rs:48` `windows_default_program` | `Path::is_file` |

**What's in neither the plan nor this change:**
- b04 #1 and b05 (`settings_dialog.rs`, `fonts.rs:196`, `profiles.rs:558`): the settings dialog's models read `config.toml` themselves. These are change 13's group 2 ("section models that return edits"), which reshapes those models anyway.
- b15 (`remote-agent/src/server/relay.rs:613-619`, `lingers(user, exists)`): it takes a probe, which Decision 2 defines as allowed. The entry is dismissed, and nothing changes.
- b27 #10 (`session.rs:276` `state_dir`): an `fcis R1` decision in a shell function, left to change 13.

**The plan listed 16 entries and missed three.** The two b23 `fcis R3` entries are in no change of the plan, so they are taken here. `record_zones` has b01 #6's shape; it wasn't an audit entry, but it is in a file this change edits, and its caller sits next to `layout::record`'s.

**One of change 10's entries closes here as a side effect.** b07 #11 (`fcis R7`, the `#[cfg(unix)]` `shell_quote` at `workspace.rs:2505-2508`) closes because `shell_quote` moves, ungated, into `cmdline.rs` beside `unix_editor_command` (Decision 1). Change 10 drops it from its list.

**The architecture test today** (`tests/architecture.rs`):
- Its source checks match short needles (`std::env`, `env::`, `Instant::now`) line by line with `names_path`.
- `Scope::BeforeTests` stops at the first `#[cfg(test)] mod tests`.
- Its dependency checks read each member's `[dependencies]` tables (target-specific ones included), following a `package =` rename only when it is written in that table.
- None of the five manifests has `[workspace.dependencies]`, `[patch]` or `[replace]` today, and no checked file has a grouped `std` import. So the stricter checks find nothing in the current tree: the code fixes below are what make the new pure-module check pass.
- Change 5 adds a scanner that skips test-only items and module files (`platform_cfgs`, `test_modules`; its design §6), and this change builds on it.

**The runners:** `tests/e2e/main.rs` and `tests/perf/main.rs` both check their variable (`BRINDLE_E2E`, `BRINDLE_PERF`), `--self-test` and `--list`. Only e2e has `--shard`.

## Goals / Non-Goals

**Goals:**
- Every entry above takes its value, or a probe, as an argument, and its tests pass values in: no temp trees, no real clock, no headless app, no process-wide global.
- `fcis`'s list of pure modules is true, and a check keeps the whole-module part of it true.
- The architecture test's needles, manifest reading and runner check close the four gaps the audit named, each with a planted-text test.
- The docs say what D3 decided, what a probe is, and what each check holds.

**Non-Goals:**
- Moving decisions out of views and handlers (`fcis R1`, change 13), and pure Windows logic out of `cfg(windows)` (`fcis R7`, change 10).
- Changing behaviour. In particular, the Unix editor command keeps using an empty `$VISUAL` as it does today, though `windows_editor_command` treats blank as unset. Aligning them is a behaviour change.
- Checking pure items inside shell modules (`pace::step`, `SessionsModel`, `FontsState`, `session::resolve_tab`, the core's `Session`) by text search. A text search can't tell one item from its module's shell code; review holds them.
- `--shard` for the perf suite, and the `thread::sleep` and `set_var` searches (change 11).

## Layers

| Part | Layer | Crate |
| --- | --- | --- |
| `layout::{origin_id, record}`, `session::{record_zones, resolve_tab}`, `RestoredTab`, `ZoneRecord` | pure code | `brindle` |
| `cmdline::{unix_editor_command, shell_quote, windows_default_program}` | pure code (already listed) | `brindle` |
| `workspace::{program_title, terminal_automatic_title, refresh_displayed_title, profile_rows}`, `actions::palette_commands` | pure functions in a shell module (items) | `brindle` |
| `Settings::new` | a GPUI-free model's constructor | `brindle` |
| `config::expand_tilde`, `Profile::working_directory`, `instance::launch_token`, `remote::{find_helper_in, find_server_helper_in, beside_exe}`, `local::host_token`, `connection::{State::closing, State::give_up_closes, relay_link_ended, RelayAccount}` | pure functions and model methods | `brindle` |
| `Settings::from_platform`, `remote::exe_paths`, `ZoneTreeView::{session_keys, zone_records, active_title}`, `Workspace` (origin, gathering), `main.rs` (origin, home, tokens) | shell: gather, call, apply | `brindle` |
| `scope::{unit_name, command}`, `server_dir::{choose, choose_windows}` | pure functions inside a seam module | `brindle-remote-protocol` |
| `scope::new_unit_name`, `server_dir::{resolve, resolve_windows}` | the seam's effect: read the clock, pid or working directory, then call the pure function | `brindle-remote-protocol` |
| `Session::{mark_active, tell_foreground, adopted}` | methods of the core's session state | `brindle-remote-agent` |
| the core's handlers and the holder `WELCOME` handler | shell: read `Instant::now()` once and pass it | `brindle-remote-agent` |
| the needle scanner, manifest and runner checks | architecture tests: pure functions over text and `toml::Table`, tested on planted input | `tests/architecture.rs` |

- **No decision moves into a view, a thread or a seam.** The shell gains reads (the clock, the environment, the probe it passes) and loses no decision. Three places keep a one-line condition on a pure answer:
  - the zone title asks for the process name only when `program_title` returns none;
  - the relay's reason asks the process only when the watchdog didn't end the link;
  - the command palette keeps the actions GPUI says are available.

  Decision 3 says why each stays.
- **Crate references don't change** (`ports R1`, `R3`). The scope and server-directory functions stay in `remote-protocol`, which both binaries use, so both still compute the same unit and directory.

## Decisions

### 1. The shell gathers values; the pure function takes them

Each site gets the value as a parameter, and its shell caller reads it once. Signatures, by site:

- **`layout`:**
  - `origin_id(pid: u32, since_epoch: Duration) -> String` gives `{pid:x}-{nanos:x}`, today's format;
  - `record(tree, active, keys: &HashMap<ZoneId, String>, origin: &str, index, title)`.
- **`session`:**
  - `record_zones(tree, active, leaves: &HashMap<ZoneId, ZoneRecord>)` with `pub struct ZoneRecord { pub cwd: Option<PathBuf>, pub session: Option<ServerSession> }`. A zone that only shows a message has no entry, as `zone_record` returns `None` for one today.
  - `ZoneNodeState::record_node` takes the same map.
- **`Settings::new(config, config_error, installed: Vec<String>)`** is pure. The new `Settings::from_platform(config, config_error, cx: &App)` calls `installed_families(cx)`, then `new`, then logs the two "using … family" lines that `new` logs today. The production callers (`main.rs:421`, `:791`) and the test that needs a GPUI global (`cancelling_a_preview_restores_the_tokens`) call `from_platform`.
- **Editor:**
  - `cmdline::unix_editor_command(visual: Option<&str>, editor: Option<&str>, path: &str) -> (String, Vec<String>)` returns `/bin/sh -c "<editor> '<path>'"` with `visual.or(editor).unwrap_or("nano")`, which is today's handling, an empty `$VISUAL` included.
  - `cmdline::shell_quote` moves with it.
  - Both are ungated with `#[cfg_attr(windows, allow(dead_code))]`, the lint-only form change 5 allows.
  - `workspace.rs`'s two `editor_command` shells only read the variables. They keep their `cfg` pair; `workspace.rs` is on change 5's exception list.
- **Server core:** `Session::mark_active(&mut self, now: Instant)`, `tell_foreground(&mut self, now: Instant)` and `adopted(holder, held, now: Instant) -> Self`.
  - `session_spawned`, `session_output`, `client_input` and `replay` read `Instant::now()` once at their top.
  - The holder `WELCOME` handler (`core/holders.rs:415-431`) reads it once before its loop and uses the same `now` for `RECONNECT_GRACE`.
  - The other `Instant::now()` calls in handlers (`sessions.rs:343`, `:644`, `:760`) are shell code and stay.
- **`scope`:**
  - `unit_name(role: &str, pid: u32, subsec_nanos: u32) -> String` gives `brindle-{role}-{pid:x}{nanos:08x}`, today's format;
  - `command(decision, unit: &str, program, args)`;
  - `new_unit_name(role) -> String` reads the clock and pid. It is the seam's effect, in the seam module change 5's table names.
  - The callers are `remote-agent/src/server/platform/unix/daemon.rs:218` and `src/remote/local/unix.rs:55`.
- **`connection.rs`:**
  - `State::closing(&mut self, session, now: Instant)` and `State::give_up_closes(&mut self, limit, now: Instant)`;
  - `send_close` and `forget` pass `Instant::now()`;
  - `end_when_unused` reads `now` once per pass of its loop and uses it for `give_up_closes`, `idle_since` and `closes_due`.
- **`host_token(base: &str, host: &Host)`:** `shared` (`local.rs:152`) passes `client_token()`.
- **Home directory and tokens:**
  - `config::expand_tilde(path: &str, home: Option<&Path>)` and `Profile::working_directory(&self, home: Option<&Path>)`;
  - the callers: `main.rs`'s `parse_args_from(args, home)` (its three `expand_tilde` uses), `terminal/mod.rs:1184-1186` (which already falls back to `dirs::home_dir()`, now read once) and `session::resolve_cwd` (which already has `home`);
  - `parse_args()` passes `dirs::home_dir()`;
  - `instance::launch_token(xdg: Option<String>, startup_id: Option<String>) -> Option<String>` becomes a free function, and `Args::instance_request` (`main.rs:230`) reads both variables.

The origin id is made in `main.rs`, in the `Application::run` closure, and passed down:
- `open_window(launch, origin, cx)` and `Workspace::new(launch, origin, window, cx)`;
- `Workspace.origin: SharedString`;
- `ZoneTreeView::quit_all`, `send_layout` and `layout` take `origin: &str`.

`ports R11` puts process-wide state in `main.rs`. Brindle has one window, so this is still once per process. `ZoneTreeView::session_keys(cx) -> HashMap<ZoneId, String>` gathers what `key_of` read, keys that aren't empty only. `zone_records(cx)` gathers what `zone_record` returned.

**Alternatives:**
- **A GPUI global for the origin:** a global type in `layout.rs` would tie a pure module to GPUI (b24's `Hosts` smell). A global in `main.rs` hides the dependency from `ZoneTreeView`'s signature. An explicit parameter is what `fcis` asks for.
- **`cwd: &Path` for `server_dir::resolve`:** see Decision 5.

### 2. A probe for a path the function derived (`fcis R5` gains a definition)

Three sites test whether a path is a file or a directory:
- `windows_default_program`'s candidates are `<dir>\pwsh.exe`, then `<dir>\powershell.exe`, for each `PATH` entry;
- `find_helper_in` checks the variable's path, then each helper name beside each executable path;
- `resolve_cwd` checks the recorded directory, then the profile's.

Each takes a probe. The shell passes `Path::is_file` or `Path::is_dir`, and the tests pass a closure over a set:
- `windows_default_program(path_dirs, comspec, is_file: impl Fn(&Path) -> bool)`;
- `beside_exe(exes: &[PathBuf], names, is_file: &dyn Fn(&Path) -> bool)`;
- `find_helper_in(variable, exes: &[PathBuf], is_file: &dyn Fn(&Path) -> bool)` and `find_server_helper_in(server_variable, remote_variable, exes, windows, is_file)`. Their shells, `find_helper` and `find_server_helper`, gather `exe_paths(&current_exe)`, which keeps `dunce::canonicalize` and stays shell;
- `resolve_tab(tab, config, home, is_dir: &dyn Fn(&Path) -> bool)`, passed on to `resolve_zone_node` and `resolve_cwd`. It is `&dyn` because the walk recurses.

**`fcis R5` as written forbids "a callback that reaches the world"**, and b15 counted `relay::lingers(user, exists)` under it, while b20 left `scope::decide(…, exists, executable)` alone. The plan's "or a probe" needs the rule to say which callbacks are allowed. R5 gains:

> A probe is allowed: a read-only `Fn(&Path) -> bool` (or a check of the same kind) that answers one question about a path the function itself derived, such as whether a candidate is a file. The shell passes the real check; tests pass a closure over plain values. A callback that acts, waits, or reads an entity, a window or a process is not a probe.

R9's "Reach for a callback parameter only when the output must interleave with a stream in order" gains "or for a probe (R5)". The Idioms section gets one example, `scope::decide`, which already has this shape.

**Alternatives:**
- **Values only, by two phases:** a pure `candidates()`, then a shell `find(is_file)`, then a pure function of what was found. This is legal under R5 as written. But the shell would then own "the first candidate that exists, and a named path that isn't a file is an error", which is the decision itself (`fcis R1`). For `resolve_tab`, it would also need a pure walk that lists every directory the restore might look at, so that the shell can stat them first, and that repeats `resolve_tab`'s own walk.
- **Pre-gathered sets** (`existing: &HashSet<PathBuf>`): the same repetition, and a stat for every `PATH` entry even when the first one has `pwsh.exe`.

If the user prefers R5 unchanged, the two-phase form replaces Decision 2 for these three functions, and only tasks 2.2, 2.3, 2.5 and 2.9 change.

### 3. Laziness without a callback

Three of the callbacks exist only to avoid work:
- **Zone and tab titles (b07 #9).** `terminal_automatic_title` asks for the process name only when the program set no title of its own. The process name costs a process snapshot.
  - **Zones:** split the filter out as `program_title(osc_title: Option<&str>) -> Option<&str>`: the OSC title when it is the program's own, so not empty and not a Windows console title. Its `cfg!(windows)` stays inside, and moving it is change 10's (b07 #10). `terminal_automatic_title(osc_title, profile_name, process_name: Option<String>)` then decides.
    - `ZoneTreeView::refresh_zone_title` asks for `foreground_process_name(snapshot)` only when `program_title(...)` is `None`.
    - The new zone's first title (`zone_view.rs:491`) passes `None`, as its closure did.
  - **Tabs:** `refresh_displayed_title(displayed, manual_title: Option<&str>, automatic_title: &str) -> bool`.
    - `Tab::refresh_title` reads `ZoneTreeView::active_title()`: the active zone's title that `refresh_titles` cached a moment before, from the same terminal, profile name and snapshot, or the profile name when there is no active zone.
    - That is the string `ZoneTreeView::automatic_title` recomputes today, so `automatic_title` is removed, and with it the tab title's second walk of the snapshot.
    - The automatic title is now computed even under a manual title, but it is a clone of a cached `SharedString`.
- **The relay's reason (b25 #7).** `wire.why()` may wait for the relay process, so it is asked only when the watchdog didn't end the link.
  - `relay_link_ended(name, ending, account: RelayAccount)`, with `enum RelayAccount { Silent(Duration), Said(String) }`:
    - `Silent(dead)`: the watchdog ended the link after `dead` without a read, and the process isn't asked;
    - `Said(why)`: the process's own account, empty when it said nothing.
  - The shell (`connection.rs:949`) builds `Silent(timings.dead)` when `stamp.expired()`, else `Said(wire.why())`.
  - Today, `expired: bool` and the closure allow "expired, and asked anyway"; the enum rules that out (`types R1`).
- **The command palette (b07 #9).** `palette_commands()` returns every labelled action in table order, built from `ACTIONS` with the hidden openers left out. `open_command_palette` keeps those for which `types.contains(..) || window.is_action_available(..)`.
  - Availability is GPUI's answer about the focused tab. Filtering by it is applying a gathered answer, as `drop_lost_zones(window, alive)`'s caller does.
  - What the palette lists by rule stays pure and tested: labelled actions only, in table order, no hidden opener, no tmux entry.
- **`profile_rows(profiles, shortcuts: &[Option<SharedString>])`:** `Workspace::profile_rows` gathers one shortcut per profile from the keymap first. It costs nine keymap lookups at most, the same as today.

### 4. A log line becomes data (b27 #11)

`resolve_tab` logged when a recorded tab names a profile the config no longer has. `RestoredTab::Local` gains `missing_profile: Option<String>`, the recorded name, `None` when it was found.
- `Workspace::restore` logs the same line from it. `session-restore`'s "with a log line" holds.
- Change 12 shows it on the tab (b27 #14, `railway R6`). This change doesn't, because that is a behaviour change.

`Option<String>` and not a `bool`: the name is what the message needs, and `None` is the ordinary case (`types R7`'s "no sentinel" holds: there is no empty-string "none").

### 5. Choosing the server directory is pure; making it absolute is the seam's

`server_dir::choose(server_dir, runtime_dir, home) -> io::Result<PathBuf>` and `choose_windows(server_dir, local_app_data, elevated)` hold today's decision and return the path as given, relative when the variable was. `resolve` and `resolve_windows` keep their names and signatures and become `choose…(…).and_then(std::path::absolute)`.
- Every caller is unchanged: `unix/dir.rs`, `windows/dir.rs` (two) and `src/remote/local.rs:426`, `:434`.
- Brindle and the helper still share one function, so they agree on the directory.
- The tests move to `choose`, with exact values in place of `ends_with`, and run on every platform. Today `the_variables_are_used_in_order` is `#[cfg(unix)]` because `std::path::absolute` adds the current drive on Windows.

**Alternatives:**
- **`cwd: &Path` joined onto a relative choice:** `Path::join` isn't `std::path::absolute`. On Windows the latter resolves drive-relative paths through `GetFullPathNameW`, so the directory could differ from today's in an edge case.
- **Returning the relative path to every caller:** five callers would each have to remember the wrapper, on two binaries that must agree.

### 6. The pure-module check

`tests/architecture.rs` gains `PURE_MODULES`, the whole-module rows of `fcis`'s table, and `the_pure_modules_read_nothing_ambient`:
- **Files:** `src/terminal/{keys,mouse,cmdline}.rs`, `src/notify/policy.rs`, `src/layout.rs`, `src/reattach.rs`, `ui/src/filter.rs`, `ui/src/primitives/placement.rs`, `remote-protocol/src/{client,holder,wire,version,relay,upgrade}.rs`, `remote-agent/src/conpty/cmdline.rs`. Term-core is a crate row with its own test.
- **Lines:** production code only. It uses change 5's test-item skipping, factored out of `platform_cfgs` as `production_lines(text) -> Vec<(usize, String)>`, so a `#[cfg(test)]` helper's `Instant::now()` (as in `pace`'s tests) doesn't count.
- **What it rejects** (`fcis R2`, `R3`):
  - the IO and clock needles, resolved through imports (Decision 7): `std::{fs, net, process, env, thread}`, `std::time::Instant::now`, `std::time::SystemTime::now`;
  - mutable statics: a `static` whose type names `OnceLock`, `LazyLock`, `Mutex`, `RwLock`, `Cell` or `Atomic…`, a `static mut`, or `thread_local!`;
  - file-system methods, matched as text: `.is_file()`, `.is_dir()`, `.exists()`, `.try_exists(`, `.metadata()`, `.symlink_metadata()`, `.canonicalize()`, `.read_dir()`, `.read_link()`.

  So a probe is passed in, never called as a method.
- **Doc sync:**
  - `the_pure_module_list_matches_the_fcis_table`: `fcis_whole_modules(doc)` reads the backticked paths in the first column of `fcis`'s "Whole pure modules" table, expanding `{a,b}`, and they must equal `PURE_MODULES`;
  - `every_pure_module_exists`.
- **Positive direction:** `a_pure_module_reading_the_world_is_found` plants each kind (a grouped `use std::{env, fs};`, `static ID: OnceLock<String>`, `path.is_file()`, `std::time::SystemTime::now()` written in full, a renamed `Instant`) and a test-only `Instant::now()` that must not count.

`pace.rs`, `session.rs`, `picker.rs`, `settings_dialog/fonts.rs`, `text_input.rs` and `settings.rs` stay item rows: each mixes pure items with shell code.

**Alternatives:**
- **Checking items by name (`fn step` to its closing brace):** a text scanner can't follow a function's calls into the rest of its module, so it would hold less than it seems to.
- **Splitting those modules** so that each pure part is a whole module (`pace/step.rs`, `picker/sessions.rs`): that is worth doing, but as refactors in change 13, not under a check-only heading.

### 7. Needles that follow imports

All the source checks share one matcher. Today's short needles stay, so nothing that fails today passes. The matcher adds:
- **`use_paths(text) -> Vec<Import>`**, with `Import { local: String, path: String }`. It reads each `use` item to its `;`, across lines, with `pub` or `pub(…)` before it. It expands nested `{…}` groups, `self`, `as` renames and `*`, where a glob gives `local: "*"`.
- **`resolved_paths(line, imports) -> Vec<String>`:**
  - each `a::b::c` path in a line, with its first segment replaced by the import that names it (`Clock::now` after `use std::time::Instant as Clock` is `std::time::Instant::now`);
  - for a first segment that no import names, the glob prefixes;
  - a path that starts with `std::`, `core::` or `::std::` is taken as written.
- **A needle matches** a resolved path that equals it or continues it at a `::` (`std::env` matches `std::env::var`, not `std::environment`), and an import whose `path` does the same. So `use std::{fs, env};` is itself an offence in a checked file.

The existing checks (`term_core_does_no_io_and_reads_no_clock`, `brindle_ui_does_no_io_of_its_own`, `the_wire_modules_do_no_io`, `the_server_core_and_holder_name_no_platform_type`) keep their scopes and needle lists and gain the matcher. The search above found no grouped or multi-line `std` import in their files today. If the stricter matcher finds a site, it was a real breach that the old needles missed, so it is fixed in this change when small, or raised.

**Alternative:** `syn`. It is a new dev-dependency, which `CLAUDE.md`'s `Cargo.lock` rule discourages. Change 5 also rejected it for the same file.

### 8. Manifests: inherited entries and patches

- **`[workspace.dependencies]`:** `names()` follows a `{ workspace = true }` entry to the root's `[workspace.dependencies]` and takes the package name from that table's `package =`, so a rename written there counts as the crate it names.
- **`[patch.*]` and `[replace]`:**
  - `patched(table) -> Vec<String>` lists every patched or replaced crate in a manifest (the package name, `package =` followed);
  - `no_dependency_is_patched_or_replaced` reads all five manifests, fails on any entry, and names the crate.

  A patch swaps the code behind a name that a closed list vouches for, which `ports R3` is about. `CLAUDE.md` says vendoring a patched copy was rejected. `ports R3` gains: "A `[patch]` or `[replace]` of any crate is a change to this rule and to the test, as adding a dependency is."
- **Positive direction:** `a_workspace_dependency_counts_by_its_package` and `a_patch_or_replacement_is_found` feed planted `toml::Table`s.
- **Doc comment:** `crates_below_the_app_name_only_their_allowed_dependencies` says `railway R2`'s dependency half holds because `anyhow` is on no closed list, and that R2's own-error-types half is review's.

**Alternative:** allowing patches of crates on no closed list (the app's own). That would need a list of allowed patches with nothing on it, and the user's standing rule is no patches at all.

### 9. The opt-in runners (`testing R3`)

`each_opt_in_runner_has_its_variable_list_and_self_test`:
- For each `[[test]]` that `the_opt_in_suites_are_harness_false_targets` names, it reads the target's `path` source.
- It asserts that the runner names `BRINDLE_<SUITE>` (uppercase), `"--list"` and `"--self-test"`.
- `runner_gaps(suite, text) -> Vec<&'static str>` is the pure part, and `a_runner_without_a_self_test_is_found` plants one.

`--shard` is left out because perf lacks it. Change 11 adds it there and adds it to `runner_gaps`, rather than this change listing perf as an exception for one change's time.

### 10. The docs say what D3 decided

All of these land in the same commit as the checks:
- **`fcis R8`:** "binds production code (as `ports R4` defines it: each crate's `src/` and build scripts). Test code is out of its scope. The e2e and perf harnesses' own pure helpers are held by the harness's `--self-test` (`testing R3`)."
- **`testing R3`:** the `--self-test` clause becomes "a `--self-test` that tests the harness's own parsers, and SHOULD test its other pure helpers (`fcis R8` doesn't reach harness code)".
  - Parsers stay MUST, as today.
  - The other helpers are SHOULD, because about 25 existing ones have no self-test (`design-smells.md`: b32, b33, b35, b36 entries). A MUST would leave a gap that the README's rule says must be written down with what ends it, for helpers that `fcis R8` never covered.
  - The layer table's "unit tests on pure code" row gets a sibling: "a pure helper in a harness | its `--self-test`".
- **`fcis` "How it applies here":** two tables.
  - **"Whole pure modules (checked by `tests/architecture.rs`)":** the term-core crate row and the rows in Decision 6. `layout.rs` (`record`, `group`) becomes a whole module, now that `origin_id` is pure. `upgrade::is_newer` becomes `remote-protocol/src/upgrade.rs`.
  - **"Pure items in shell modules (held by review)":** `pace::step`, `session::{merge_windows, drop_lost_zones, record_zones, resolve_tab}`, `picker::{SessionsModel, session_palette}`, `FontsState`, `EditState`.
- **`fcis` R5, R9, Idioms:** Decision 2.
- **`fcis` Enforcement:** the new check holds R2 and R3 for the whole-module rows. The line "Nothing stops a pure module in `src/` from calling `Instant::now()` except the reviewer" becomes true only of the item rows.
- **`testing` Enforcement:** the runner check. **`ports` R3 and Enforcement:** Decision 8.
- **`CLAUDE.md`:**
  - "the process's `layout::origin_id`" says it is made once in `main.rs`;
  - the `cmdline.rs` mention adds the Unix editor command;
  - nothing else in `CLAUDE.md` names a changed signature.
- **`.claude/rules/app.md`:** "Enforced by" names the pure-module check. **`.claude/rules/tests.md`:** harness pure helpers go in `--self-test` (D3), and "Enforced by" names the runner check.
- **`src/terminal/cmdline.rs`'s module doc** says it also builds the Unix editor command. Its "Pure, so they are tested on every platform" becomes true.

## Wire, state file, terminal server

- **No wire message, protocol version or config key changes** (`process R4`–`R6`), so no transcript changes.
- **The state file's format** and **the layout blob's** (`origin` is the same `{pid:x}-{nanos:x}` string) are unchanged (`process R15`).
- **The server core:** `Session.active_at` and `detached_since` are in-memory only, never in `SessionMeta`, and are set from the same instant as before, read at the top of the handler.
  - A core restart builds adopted sessions with `adopted(.., now)` as today.
  - A Brindle crash and an upgrade don't touch these fields (`process R1`–`R3`, `R12`).
- **The scope's unit name** keeps its format, so `brindle-<role>-<hex>.scope` and `CLAUDE.md`'s description hold.

## Risks / Trade-offs

- **[The import resolver misreads a form]** → The planted tests pin each form it handles. A false positive names the line. A false negative falls back to today's short needles, which stay.
- **[`fcis R5`'s probe wording is read as a loophole]** → The definition is narrow: a read-only check of a derived path, and nothing that acts, waits or reads an entity, a window or a process. The anti-pattern list says so, and the user's review of this change is where it is settled.
- **[The cached tab title differs from today's recomputed one]** → They are computed from the same inputs in the same call (`refresh_title` runs `refresh_titles` first). e2e `commands-rename-tab` and `zones-*` cover the shown titles.
- **[`Settings::from_platform` and `new` drift]** → `from_platform` is three lines: gather, call, log. The pure `new` has the tests.
- **[A rebase against change 10 on `terminal_automatic_title`]** → Both reshape the same function. Whoever lands second keeps both `program_title` and change 10's platform argument.
- **[`expand_tilde`'s new parameter ripples into `parse_args_from`]** → `parse_args_from` is already tested with argument lists. Its tests pass `Some("/home/u")` and assert exact paths, which is better than today's `ends_with`.

## Migration Plan

- **Order:** pure functions and their tests first, then the shared crates (`scope`, `server_dir`), then the helper's core, then the app's shell, then the checks and the docs in one commit.
- **Deploy:** nothing to migrate. No file, protocol or config changes.
- **Rollback:** revert the PR.

## Open Questions

None. Decision 2's `fcis R5` wording follows the plan's "or a probe", and the user confirms the wording when they review this change. Decision 2 gives the fallback if they decline it.
