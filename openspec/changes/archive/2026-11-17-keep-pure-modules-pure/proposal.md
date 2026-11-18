# Proposal

## Why

`fcis` says pure code takes time, environment values and facts about the system as arguments, and takes and returns plain values (`fcis R2`, `R3`, `R5`). The 2026-11-16 audit (`docs/architecture/audits/2026-11-16.md`, finding 13, "Enforcement gaps" and "Doc drift"; change 7 of `2026-11-16-remediation.md`) found pure functions and models that break this:
- they read the clock, a process-wide static, the environment or the disk;
- or they take a callback that reaches GPUI or a process.

Each one can only be tested with real time, a real home directory, temp trees or a headless GPUI app. Two of them contradict the docs: `layout.rs` and `terminal/cmdline.rs` are listed as pure in `fcis`, and both read the system.

Nothing stops new breaches. The architecture test holds `fcis R4` for term-core, but no rule for the app's pure modules, and the audit lists four gaps in its text search:
- grouped imports;
- `[workspace.dependencies]`, `[patch]` and `[replace]`;
- a doc comment that claims a direct `railway R2` check;
- `testing R3`'s runner features beyond `harness = false`.

The user answered D3 on 2026-11-16: `fcis R8` binds production code only, and the harnesses' pure helpers are held by their `--self-test`. The docs must say so. This change depends on change 5 (`enforce-platform-seams`), whose scanner already skips test-only items. The models of change 13 build on the shape this change sets.

## What Changes

- **Pure functions take values (code, no behaviour change).** Each site below gets its value as an argument, and its caller in the shell gathers it:
  - **Clock, pid and process-wide statics (`fcis R2`, `R3`):**
    - `layout::origin_id` is made once in `main.rs` and passed down (b01 #5);
    - the server core's `Session::mark_active`, `tell_foreground` and `adopted` take `now` (b11 #3);
    - `scope::command` takes the unit name, and a pure `scope::unit_name(role, pid, nanos)` builds it (b20 #1);
    - `connection.rs`'s `State::closing` and `give_up_closes` take `now` (b25 #6);
    - `host_token` takes the client token (b24 #12).
  - **Environment and home directory (`fcis R3`):**
    - the Unix editor command becomes `cmdline::unix_editor_command(visual, editor, path)`, the twin of `windows_editor_command` (b07 #8);
    - `config::expand_tilde` and `Profile::working_directory` take `home` (b23, `config.rs:285`);
    - `instance::launch_token` takes the two variables' values (b23, `instance.rs:57`);
    - `server_dir`'s choice of directory is split from making it absolute, which reads the working directory (b20 #2).
  - **The disk (`fcis R3`):** these take a read-only probe (`is_file`, `is_dir`) that the shell passes as `Path::is_file`, so their tests need no temp trees:
    - `cmdline::windows_default_program` (b28 #1), which makes `fcis`'s claim that `cmdline` is pure true;
    - `remote::find_helper_in` and `find_server_helper_in`, which also take the executable's paths already resolved (b27 #8);
    - `session::resolve_tab` (b27 #9).
  - **Callbacks into GPUI or a process (`fcis R5`):**
    - `layout::record` and `session::record_zones` take maps the view gathers (b01 #6; `record_zones` has the same shape and was found while reading it);
    - `Settings::new` takes the installed font families, not `&App` (b03 #7, and so b03 #8's headless test becomes a plain one);
    - the tab title helpers, `profile_rows` and `palette_commands` take values, not closures (b07 #9);
    - `relay_link_ended` takes the relay's account as a value (b25 #7).
  - **A log line (`fcis R3`, b27 #11):** `resolve_tab` stops logging. It returns the missing profile's name on `RestoredTab`, and the shell logs it as today. Change 12 can then show it on the tab (`railway R6`, b27 #14).
- **Rule text:**
  - **`fcis R5` gains a definition of a probe:** a read-only question about a path the function derived, which the shell answers. `fcis R9` names it beside the stream case. The plan's "or a probe" needs this, because R5 as written forbids any callback that reaches the world, and the b15 batch counted one.
  - **`fcis R8` binds production code** (change 5's definition: each crate's `src/` and build scripts). The e2e and perf harnesses' pure helpers are held by their `--self-test` (D3).
  - **`testing R3`** says what `--self-test` covers.
  - **`ports R3`** says patches and replacements count.
- **Docs:**
  - `fcis`'s "How it applies here" table splits into whole pure modules, which are checked, and pure items in shell modules, which review holds. `layout.rs` and `remote-protocol/src/upgrade.rs` become whole modules, and `session::resolve_tab` is added to the items.
  - The Enforcement sections of `fcis`, `testing` and `ports` name the new checks.
  - `CLAUDE.md` says where the layout origin id comes from.
  - `.claude/rules/{app,tests}.md`.
- **Checks in `tests/architecture.rs`:**
  - **`the_pure_modules_read_nothing_ambient`:** the whole-module rows read no clock, environment, file, process, thread, network or mutable static in production code. Its list must match `fcis`'s table.
  - **Smarter needles:** they follow grouped, nested, multi-line and renamed `use` items and paths written in full, for every existing source check.
  - **Manifests:** the dependency checks resolve `workspace = true` entries through `[workspace.dependencies]`, and refuse `[patch]` and `[replace]`.
  - **Opt-in runners:** each runner names its enabling variable, `--list` and `--self-test` (`--shard` is change 11's).
  - **Doc comment:** it says `railway R2` is held only through the closed lists.
  - Every new check has a planted-text test showing it finds what it should.

No user-visible behaviour changes.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. No requirement changes, so `.openspec.yaml` sets `skip_specs: true`. These requirements hold as written:
- **`session-restore`:** "a missing profile SHALL be replaced by the default profile, with a log line" (the line moves to the shell), the directory fallbacks, and layouts kept with sessions (the origin id is made once per process, as today);
- **`tabs`:** the title order: manual, program title, foreground process, profile name;
- **`profiles`:** Windows' default program order;
- **`configuration`:** ctrl-, opens `$VISUAL` or `$EDITOR`;
- **`terminal-server`:** the server directory, and Linux roles in their own scope.

## Impact

- **Code:**
  - **`src/`:** `layout.rs`, `zone_view.rs`, `workspace.rs`, `main.rs`, `settings.rs`, `session.rs`, `config.rs`, `instance.rs`, `actions.rs`, `terminal/cmdline.rs`, `terminal/mod.rs`, `remote/mod.rs`, `remote/local.rs`, `remote/local/connection.rs` and `remote/local/unix.rs`;
  - **the shared crates:** `remote-protocol/src/{scope,server_dir}.rs`;
  - **the helper:** `remote-agent/src/server/core/{sessions,holders}.rs` and `remote-agent/src/server/platform/unix/daemon.rs`. The server directory's callers keep their signatures.
- **Check:** `tests/architecture.rs`.
- **Docs:** `docs/architecture/{functional-core-imperative-shell,testing-architecture,ports-and-adapters}.md`, `CLAUDE.md`, `.claude/rules/{app,tests}.md`.
- **Unchanged:** wire messages, protocol versions, config keys, the state file's format, the layout blob's format, and dependencies.
- **Overlap with other changes:**
  - **Change 5 (`enforce-platform-seams`, a dependency):**
    - the pure-module check reuses its skipping of test-only items;
    - it lists `workspace.rs`, `session.rs`, `remote/mod.rs` and `settings.rs`, whose `cfg`s this change leaves where they are;
    - both edit `fcis`'s Enforcement section and `.claude/rules/{app,tests}.md`.
  - **Change 2 (`carry-session-identity-and-host`):** its `open_config` calls `editor_command`, which keeps its name and signature, and its `zone_server_session` keeps the key's string form that `layout::record`'s map holds.
  - **Change 4 (`isolate-test-runs`):** it also edits `src/remote/local.rs`'s tests. Its design leaves a `set_var` search to change 11, on the needles this change reworks.
  - **Change 6 (`add-clippy-to-ci`):** it adds its own tests to `tests/architecture.rs`, so a rebase meets adjacent lines only.
  - **Change 8 (`list-sessions-off-ui-thread`):** both edit `main.rs`, this change near `parse_args` and `open_window`, that one the launch closure.
  - **Change 10 (`test-platform-logic-everywhere`):**
    - it moves `terminal_automatic_title`'s `cfg!(windows)` (b07 #10), which this change leaves in the new `program_title`. Whichever lands second carries the other's shape;
    - b07 #11, on its list, closes here, because the `#[cfg(unix)]` `shell_quote` moves ungated into `cmdline.rs`.
  - **Change 11 (`deterministic-tests`):** it adds `--shard` to perf and extends the runner check, and writes its `thread::sleep` search on the reworked needles.
  - **Change 12 (`surface-swallowed-failures`):** it shows `RestoredTab`'s `missing_profile` on the tab (b27 #14), and edits `config.rs` near `expand_tilde`.
  - **Change 13 (`extract-decisions-into-models`):** it takes the other `fcis R3` entries in the settings dialog's models (b04 #1, b05 `fonts.rs:196`, `profiles.rs:558`), which keep `FontsState` from being pure until then.
  - **`update-dependencies`:** a `toml` 1.x bump may change the `toml::Table` API the manifest checks use. Whichever lands second adjusts.
