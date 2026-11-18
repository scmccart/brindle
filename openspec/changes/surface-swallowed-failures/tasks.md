# Tasks

The work is in seven groups. Each group is one PR that lands alone, once the changes it names have landed (design, "Groups and order"). Every group needs change 2 (`carry-session-identity-and-host`) and change 9 (`bound-queues-and-contain-panics`) merged first. Within a group, the tasks go pure code first, then the shared crates, then the shell, then UI and e2e. "Decision N" is design.md's.

Each group ends with its own checks: `cargo test --workspace --locked` (which runs `tests/architecture.rs`), `cargo build --release --locked` with no new warnings, and the group's e2e cases under `scripts/with-xvfb`. On its PR, the Windows CI jobs (`windows`, and the advisory `windows-e2e`) must pass.

## 1. Terminal server, helper and protocol (shared contracts)

Needs change 3 (its `SessionMeta::from_value` fuzz target) and change 10 (the seam decisions it moves).

- [x] 1.1 `SessionMeta::from_value(&Value) -> Result<SessionMeta, MetaFault>` in `remote-protocol/src/holder.rs` (decision 11).
  - `MetaFault { malformed: Vec<&'static str>, read: SessionMeta }`: `"meta"` for a non-object, else each field of the wrong type. `read` keeps the well-formed fields.
  - A missing field stays its default and is `Ok`.
  - Write the unit tests first:
    - `a_missing_field_is_its_default`;
    - `a_field_of_the_wrong_type_is_named_and_the_rest_kept`;
    - `a_non_object_meta_is_refused`.
  - Check that every transcript test (`holder-set-meta-layout` among them) still decodes to `Ok`, byte for byte.
  - Change 3's fuzz target asserts `Ok` or `Err(MetaFault)`.
  - Verify with `cargo test -p brindle-remote-protocol holder`.
- [x] 1.2 `ready::ReadyRead { Pending, Listening, Ended, Failed(io::Error) }` and the pure `ready_read(io::Result<usize>, &[u8]) -> ReadyRead`, in `remote-protocol/src/ready.rs` beside the Linux rule, ungated so it is tested on every platform.
  - Unit tests: `one_byte_is_listening`, `the_end_without_a_byte_is_ended`, `a_read_error_is_kept`.
- [x] 1.3 `prune_helper_copies` returns `Pruned { removed: usize, kept: Vec<(PathBuf, String)> }` (`remote-protocol/src/win.rs`).
  - Change 10's pure `pruned_builds` still decides what to try.
  - A Windows unit test: a copy held open is kept, with its reason.
- [x] 1.4 `Conn::input_waiting() -> io::Result<bool>` on both platforms (decision 11): `platform/unix/conn.rs` (a failed `FIONREAD`) and `platform/windows/{conn,pipe}.rs` (`port.pump` and `PeekNamedPipe`).
  - `holder_behind` in `core/holders.rs` reads `Err` as behind, so the next read meets the error and `drop_holder` logs its cause.
  - The test helpers in `conn.rs` `.unwrap()`.
  - Verify with `cargo test -p brindle-remote-agent conn`.
- [x] 1.5 `adopt_meta(Result<SessionMeta, MetaFault>) -> (SessionMeta, Option<String>)`, pure, in `core/sessions.rs`.
  - A fault keeps `read`, empties `owner`, sets `policy: Detach`, and gives a fresh `key` when the key was malformed.
  - It returns the `core.log` line naming the session and the fields.
  - Unit tests: `a_faulty_meta_is_adopted_detached_with_no_owner`, `a_malformed_key_is_replaced`, `a_good_meta_is_adopted_as_it_is`.
  - `Session::adopted` calls it, logs the line and queues `SET_META` with the repaired meta.
- [x] 1.6 Write the real-process test in `remote-agent/tests/core.rs`, on both platforms: `a_session_whose_meta_is_malformed_is_adopted_detached`.
  1. A holder with one session.
  2. `SET_META` with `{"owner": 7}`.
  3. Kill the core.
  4. The next core lists the session as detached with its program running, and an attach replays it.

  Show it fails before 1.5: the session gets the default `Kill` policy and is hung up after `RECONNECT_GRACE`.
- [x] 1.7 `queue -> io::Result<()>` in `core/holders.rs` and `core/clients.rs` (b11 #11, b10's `clients.rs:404` smell).
  - **For the holder:**
    - a failed `SPAWN` fails the waiting spawn with an `ERROR` to its client: `the terminal server couldn't pass the request to its holder: …`;
    - a failed `SET_META` leaves `storing` unset;
    - any other message for a session sends its attached client an `ERROR` with the session.
  - **For the client:** a reply that can't be encoded is replaced by an `ERROR` for its `req`.
  - Every `match` on the message names its variants (`railway R3`).
  - Unit tests on the pure parts: `an_unencodable_spawn_fails_its_request` and `an_unencodable_session_message_is_reported_to_its_client`, using a payload over `MAX_FRAME_PAYLOAD_SIZE`.
- [x] 1.8 Error texts name their action (b10 #13): `server/core.rs:124-143`, and `server/cli.rs:96, 202, 207, 259, 281, 308`.
  - Add to `remote-agent/tests/manage.rs`: `a_failed_command_names_its_action`, running `brindle-remote list` against a server directory that is a file, whose stderr names the action (`can't read the server directory` or the CLI's equivalent) and the system's error.
- [x] 1.9 The Windows seams (decision 11):
  - **The ready pipe.** `ReadyPipe::read` returns `ReadyRead`, and the starter's error appends `reading the ready pipe failed: …`.
  - **`daemonize`.** It logs a failed `SetConsoleCtrlHandler` or `set_current_dir` to the role's log.
  - **Snapshots.** `end_its_programs -> io::Result<usize>`, passed up by `kill_remaining`. The holder logs a failed snapshot with the session id.
  - **ConPTY loading.** `packaged() -> Result<Api, String>`, and `PACKAGED` caches only `Ok`, through a generic `cached_ok(cell: &OnceLock<T>, load: impl FnOnce() -> Result<T, String>) -> Result<&T, String>`. The holder logs each failure and tries again at the next spawn.
  - **Reparenting.** `reparent -> io::Result<Reparented>`, with `Reparented { Done, Unsupported }`. The holder's `REPARENT` arm logs `Unsupported` once and `Err` each time.
  - **`missing(name)`** names the export.
  - **The core's `prune_copies`** logs `Pruned.kept`.

  Unit tests in the agent, ungated where the code is: `cached_ok`'s `a_failure_is_not_cached` and `a_success_is_cached` (closures counting their calls), and `missing_names_the_export`. Verify on Windows CI: `cargo test -p brindle-remote-agent` runs them and `launch.rs`.
- [x] 1.10 The relay's build id (b15 `relay.rs:256`).
  - `Server.build` becomes a `Result`, and an `Err` writes one `core.log` line.
  - `RELAY_HOST.build` stays `""`, through a pure `host_build(&Result<String, String>) -> &str`.
  - Unit test: `an_unreadable_build_id_is_sent_as_unknown`. The relay tests in `tests/relay.rs` keep passing unchanged.
- [x] 1.11 Docs:
  - `docs/architecture/railway-oriented-programming.md`, "How it applies here": the `remote-agent` row's "the role's log; the client's tab" also names the `ERROR` a session's client gets for an unsendable message.
  - CLAUDE.md, the core paragraph: a malformed stored meta is adopted detached with no owner.

## 2. The config line and the settings dialog

Needs change 7 (textual: `config.rs`) and change 9 (`config::write_replacing`).

- [x] 2.1 `ConfigFinding`, `Loaded` and `ConfigReport` in `src/config.rs` (decision 2).
  - `Config::parse` returns `Loaded`: `normalized` returns its clamps, and the removed-key warnings are findings, not log lines.
  - `Config::findings` reports unknown global and per-profile themes.
  - `Config::theme` stops logging.
  - Write the unit tests first:
    - `a_clamped_font_size_is_a_finding`;
    - `a_clamped_line_height_is_a_finding`;
    - `an_unknown_theme_is_a_finding`;
    - `an_unknown_profile_theme_names_the_profile`;
    - `a_removed_key_is_a_finding`;
    - `a_valid_config_has_no_findings`.
- [x] 2.2 `actions::user_bindings(&[(String, String)]) -> (Vec<KeyBinding>, Vec<ConfigFinding>)`, pure. `bind_keys(cx)` returns the findings.
  - Unit tests: `an_unknown_action_is_a_finding`, `invalid_keys_are_a_finding`, `none_unbinds_and_is_not_a_finding`.
- [x] 2.3 The pure `config_line(error, findings, notices) -> Option<String>`.
  - Unit tests:
    - `no_error_and_no_findings_is_no_line`;
    - `findings_are_joined_after_an_error`;
    - `each_finding_says_what_was_used`;
    - `notices_come_last`.
- [x] 2.4 `config_edit::{custom_themes, profiles} -> Result<Vec<…>, Shape>`, with `Shape { Unparsable, NotATable, NotAnArray }`. `theme_entries` is pure, and `join_words`/`quote` return `Result<String, Unquotable>` (decision 9).
  - Unit tests:
    - `themes_that_are_not_a_table_are_a_shape_error`;
    - `profiles_that_are_not_an_array_are_a_shape_error`;
    - `a_missing_key_is_empty`;
    - `theme_entries_name_the_path_and_cause`;
    - `a_word_with_nul_is_unquotable`.
- [x] 2.5 `has_font_family -> io::Result<bool>` (`src/win.rs`). `platform_has_family` keeps the family on `Err` and logs once per process.
- [x] 2.6 Shell:
  - `load_or_default` returns `(Config, ConfigReport)`, with a failed `write_default` as a finding.
  - `Settings` keeps `config_findings` and the precomputed `config_line`, built by `Settings::new` and by `reload_config` after `bind_keys`.
  - `Workspace::render` reads `config_line` in place of `config_error`.
  - `dump_active_screen` prints `--- config <escaped>` after the `--- fonts` line when there is one.
- [x] 2.7 The settings dialog:
  - `reload_entries` uses `theme_entries` and shows its message in the Theme section's error line;
  - the Profiles section's read-only text is `reading <path>: <error>`;
  - `NotFound` reads as an empty document;
  - the profile editor marks an unquotable field invalid and blocks Save.

  Unit test on the profile editor's pure model in `settings_dialog/profiles.rs`: `an_unquotable_argument_blocks_save`.
- [x] 2.8 e2e:
  - Teach `tests/e2e/brindle.rs`'s dump parser the `--- config` line.
  - New case `reload-config-findings` (`tests/e2e/cases/reload.rs`):
    1. A config with `theme = "tokyonight"`, `font.size = 200` and `"ctrl-shift-y" = "split_zone_rihgt"`. The dump's `--- config` line names all three, and the active tab uses `brindle-dark`'s background (sampled as `reload-theme-attached` does).
    2. Fix the file while it runs. A later dump has no `--- config` line.
  - Extend `settings` (`tests/e2e/cases/settings.rs`): with `themes = 3`, the Theme section's error line names `themes`.
  - Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- reload settings`.
- [x] 2.9 Docs:
  - CLAUDE.md: `--dump-screen-after`'s `--- config` line, and the `settings.rs` paragraph's config line;
  - the railway doc's "config and settings" row: the config line lists the findings.

## 3. Tab notices

Needs change 2 (`Shown`, `State::shows`), change 9 (the event kinds), change 7 (`missing_profile`) and change 8 (the launch code).

- [ ] 3.1 `src/terminal/tab_note.rs` (decision 1): `line(text)` and `NoteGate`.
  - Write the unit tests first:
    - `controls_are_dropped`, covering C0, C1, `ESC` and `BEL`;
    - `lines_are_joined`;
    - `a_repeat_without_output_is_refused`;
    - `a_repeat_after_output_is_admitted`.
- [ ] 3.2 Pure helpers in `connection.rs`:
  - `reconnect_gave_up(limit, last: Option<&str>)`;
  - `lost_line(name, reason)`;
  - `layout_to_send(layout) -> Result<Option<Value>, LayoutTooLarge>`;
  - `ToCore::session()`, naming every variant.

  Unit tests:
  - `the_last_error_is_named`;
  - `only_timeouts_keep_todays_text`;
  - `the_lost_line_carries_the_first_line_of_the_reason`;
  - `a_layout_over_max_layout_is_too_large`.
- [ ] 3.3 `links::OpenError` and the pure `failure_text` (decision 3).
  - Unit tests: `no_scheme_says_so`, `a_missing_program_names_it_and_the_error`, `an_exit_status_is_named`.
- [ ] 3.4 `profile_for(config, name) -> Chosen` and the notice text, pure, in `workspace.rs`.
  - Unit tests: `a_name_matches_case_insensitively`, `an_unknown_name_gives_the_default_and_names_it`, `no_name_is_the_default_without_a_notice`.
  - The first launch and `forwarded_launch` both call it, which closes b01 #10.
- [ ] 3.5 The sink:
  - `SessionSink::note`, and `Sink::note` through the gate and `output`, so a held replay keeps it after;
  - `Sink::output` calls `gate.output()`.

  Unit test in `terminal/server.rs`: `a_notice_during_a_held_replay_is_written_after_it`.
- [ ] 3.6 `Connection`:
  - a session `ERROR` with no request goes to the shown session's sink as `note`, else is logged at `warn`;
  - an encode failure in `write` is noted in the session `ToCore::session` names;
  - `Running::set_layout` records `LayoutSent`;
  - `Running::detach` drops a too-large layout and still sends `DETACH`;
  - `reconnect_local` keeps the last error;
  - `run_relay` passes `relay_link_ended`'s reason to `lost_link`.

  Fake-link tests (`fake::link`):
  - `a_session_error_is_noticed_once_in_its_tab`;
  - `an_error_for_a_session_not_shown_is_not_delivered`;
  - `a_too_large_layout_is_not_sent_and_detach_still_goes`;
  - `a_remote_loss_names_its_cause`.
- [ ] 3.7 `Terminal::note` writes through parser, then term, then marks. It holds notes while `attaching` and writes them on `Replay(false)`, and writes after a `Lost` or `Failed` line.
  - This is entity code with no decision of its own beyond `attaching`, so the e2e cases hold it: `restore-fallbacks` (a note due before a replay) and `local-server-session-error`.
- [ ] 3.8 Callers:
  - **Links.** `TerminalView`'s ctrl-click awaits `links::open`'s receiver and calls `note` on the clicked terminal on `Err`, so `links::open` returns the receiver.
  - **Launches.** `LaunchRequest.note` is set from `Chosen` and handed to the new tab's terminal.
  - **Missing indexes.** `Workspace::launch` with a missing index opens `open_message_tab` (b07 #15).
  - **Restores.** `Workspace::restore` calls `note` with `missing_profile` on the restored zone's terminal.
- [ ] 3.9 e2e cases:
  - **`links-opener-fails`** (`tests/e2e/cases/links.rs`, modelled on `links-local-click`): `BRINDLE_LINK_OPENER` names a missing program, and a `--click ctrl-…` on a printed URL. The screen shows `couldn't open` and the URL.
  - **`local-server-session-error`** (`local_server.rs`):
    1. `BRINDLE_TEST_HOLDER_PANIC=<marker>`; send the marker as input.
    2. `--wait-for` the notice.
    3. Send `echo_sum("ok-", 1, 1)` and `--wait-for ok-2`. The screen shows the line once.
  - **`local-server-no-core-back`:**
    1. Kill the holder and the core of a tab's session, after making the private server directory unreadable.
    2. `--wait-for` the lost line, which names the start error.
    3. The case restores the mode before its cleanup.
  - **`profile-shortcut-missing`:** one profile, `--key ctrl-alt-5`. A message tab says there is no profile 5.
  - **`launch-unknown-profile`:** `-p wrk`. The active tab says no profile is named `wrk`.
  - **Extend `restore-fallbacks`:** the restored `Removed` tab says its profile is gone.
  - **Extend `remote-ssh-network`:** the tab's lost line names `ssh` or `nothing was read`.
  - Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- links local-server profile launch restore remote-ssh-network`, and show each new case fails on a build without this group.
- [ ] 3.10 Docs:
  - CLAUDE.md's `terminal/` paragraph names the notice line and `Terminal::note`;
  - the railway doc's `src/remote/` row adds "the dim notice line".

## 4. Endpoints and the sessions palette

Needs change 4 (textual: `local.rs`).

- [ ] 4.1 `Listing::Unlisted(String)` in `picker.rs`. `session_palette` draws it, and every `match` on `Listing` names it.
  - Unit tests:
    - `an_unlisted_endpoint_is_a_disabled_row_with_its_cause`;
    - `unlisted_does_not_add_the_no_sessions_note`.
- [ ] 4.2 `listing_from(host, state, Result<Vec<SessionInfo>, String>)`, and the action-naming texts (b07 #13, #16).
  - Unit tests: `this_machines_failure_is_unlisted`, `a_remote_failure_is_offline`.
- [ ] 4.3 `SessionsModel` takes a kill's outcome (decision 7).
  - Unit tests:
    - `a_failed_kill_brings_the_row_back_and_adds_its_note`;
    - `the_note_goes_at_the_endpoints_next_answer`;
    - `the_highlighted_row_stays_through_a_failed_kill`.
- [ ] 4.4 `relay::Ended { Status, Unread }` (decision 8). Remove `failed_status` and its `cfg` blocks, and word `why()` and `start()` for each case.
  - Unit tests:
    - `an_unread_exit_names_the_error`;
    - `a_watcher_that_cant_start_kills_and_reports`, with the spawn injected.
- [ ] 4.5 `client::StderrDrain`. `spawn_stderr_drain -> io::Result<StderrDrain>`, and `await_stderr` keeps what has arrived.
  - Unit tests on the pure part: `a_late_drain_keeps_what_was_read` and `a_read_error_is_appended`.
- [ ] 4.6 `upgrade::Offered { Upgraded, Current, Kept }`. `shared` logs `Kept`'s reason at `warn`.
  - A unit test on the decision that gives `Kept`: `a_failed_check_keeps_the_running_core_with_its_reason`.
- [ ] 4.7 `local::report_offline(host, &error)`, with `next_connection_id()`. The eager-connect thread and `when_listed` call it.
  - Unit test: `hosts_orders_an_offline_report_after_an_earlier_connection`, on `Hosts`'s pure update.
- [ ] 4.8 `Workspace`: `list_sessions` returns `Listed { sessions, kill }`, and `sessions_answered` passes the kill's outcome to `SessionsModel`.
- [ ] 4.9 e2e:
  - New case `local-server-palette-unlisted`: `BRINDLE_SERVER_HELPER` names a missing file, and the "Attach Session…" palette is dumped. Its row says this machine's sessions couldn't be listed, and there is no "No detached sessions" row.
  - Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- local-server-palette`.
- [ ] 4.10 Docs: CLAUDE.md's sessions palette paragraph names the `Unlisted` row and the failed-kill row.

## 5. Command line, latency and the state directory

Needs change 8 (the launch code), change 4 (`StateFile`), change 1 (the state-file text), change 9 (`latency::Window`) and group 2 (the config line's `notices`).

- [ ] 5.1 `parse_args_from` (decision 10):
  - every option without its value, and `-e` without a command, is a usage error naming the option;
  - so is `--dump-screen-after` without a number;
  - `--action`, `--key`, `--hover` and `--click` are checked at parse time;
  - `Step::Pointer` holds a `PointerStep`.

  Write the unit tests first:
  - `an_option_without_its_value_is_a_usage_error`, for each option;
  - `dash_e_without_a_command_is_a_usage_error`;
  - `an_unknown_action_is_a_usage_error`;
  - `a_bad_key_is_a_usage_error`;
  - `a_bad_pointer_position_is_a_usage_error`.

  Remove the run loop's `log::error!` arms.
- [ ] 5.2 `latency::LogState { Writing, Stopped { path, error } }`. The summary adds `--- latency-log stopped "<path>": <error>`.
  - Unit tests: `a_failed_write_stops_the_log_and_keeps_timing`, `the_summary_names_the_stopped_log`.
  - Run `BRINDLE_PERF=1 cargo test --test perf -- --self-test` to show `parse_latency` still reads the header.
- [ ] 5.3 `session::state_dir() -> Option<PathBuf>`, with the environment injected as change 7 shapes it.
  - Unit tests: `no_home_and_no_override_is_none`, `the_override_wins`.
- [ ] 5.4 `main`: with no state directory, run alone, with no `session.lock`, restore, reattach or recording. Add the window notice to `Settings`' config line `notices`.
- [ ] 5.5 e2e:
  - Extend `restore-fallbacks` (or add `restore-no-state-dir`) with a run under `env -u HOME -u XDG_STATE_HOME`, without `BRINDLE_STATE_DIR`, started in the case directory:
    - its `--- config` line says the session can't be saved;
    - no `session.toml` appears in the case directory.

    The harness sets `BRINDLE_STATE_DIR` for every run, so this one needs a `Run` option that unsets it.
  - Add a no-display case `cli-usage-errors`, which runs the binary with `-p`, `--action split_zone_rihgt` and `--dump-screen-after soon`. Each exits 2 naming the option, without a window.
  - Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- restore cli-usage`.
- [ ] 5.6 Docs: CLAUDE.md's debug-hook paragraph says bad steps are usage errors at launch.

## 6. Test harnesses

Needs change 4 (`tests/common/winproc.rs`).

- [x] 6.1 `tests/e2e/brindle.rs`:
  - the timeline thread's `join` error is an error;
  - `list()` checks `brindle-remote list`'s status;
  - the hang-up's kill status is checked.

  `capture.rs::save_ppm` returns `io::Result`, and callers `?` it.
- [x] 6.2 `tests/perf/main.rs`: a failed `summary.toml` write prints the error and exits 1.
- [x] 6.3 `tests/common/winproc.rs`: `all` and `elevated` return `io::Result`, and the leftover-role check fails on `Err`.
- [x] 6.4 Self-tests where the code is pure: `e2e --self-test` covers `list`'s status check through its parser, and `perf --self-test` covers the summary write's error path with an unwritable directory.
  - Verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test` and `BRINDLE_PERF=1 cargo test --test perf -- --self-test`.

## 7. `brindle-ui` icon path

- [x] 7.1 `Icon::path(&self, side, min_stroke) -> Result<Path<Pixels>, PathError>`, split out of `paint` in `ui/src/icon.rs`. `paint` logs an `Err` once per icon name.
  - Unit test: `every_icon_builds_at_component_sizes`.
  - Verify with `cargo test -p brindle-ui icon`.

## Manual checks

- A real ctrl-click on a link in a tab whose opener fails: `BRINDLE_LINK_OPENER=/nonexistent` on Linux, and on Windows a URL with a scheme no program handles. The tab shows the dim line. `--click` drives the same path, but only a real pointer goes through the platform's events. Hand it to the user with `scripts/run-throwaway.sh` (`scripts/run-throwaway.ps1` on Windows).

## Workflow follow-up

- Copied requirement texts:
  - `terminal-emulation` "A session lost for good is reported in the tab" is change 9's version, which includes change 2's;
  - `command-line` "Launch options" is change 8's;
  - `session-restore` "State file location and writes" is change 1's.

  If any of those was archived first, match this change's MODIFIED text to the synced spec.
- Archive the change in the PR of whichever group lands last (`docs(openspec): …`), syncing the nine delta specs. Groups 6 and 7 need no spec delta and may land in any order.
