# Tasks

Changes 5 (`enforce-platform-seams`) and 7 (`keep-pure-modules-pure`) land first (design §8).

Every site follows the same steps:
1. Write the pure function's tests against today's behaviour, taking the cases from the seam it replaces, and watch them fail to compile.
2. Add the function.
3. Switch the seam to call it, and delete the code it replaces.

Error texts and `io::ErrorKind`s stay byte for byte. Tests are named as sentences (`testing R15`).

Code under `cfg(windows)` builds only on Windows. Those tasks are verified by the `windows` CI job (task 5.2), and on Linux only by `cargo test` of the new ungated modules.

## 1. Shared contracts in `brindle-remote-protocol`

- [x] 1.1 `remote-protocol/src/hresult.rs` (design §5.5):
  - add `pub mod hresult;` (ungated) and `pub fn win32_code(hresult: u32) -> Option<u32>`, which is `Some(code & 0xffff)` for `0x8007xxxx` and `None` otherwise;
  - tests: `a_win32_hresult_gives_its_code` (`0x8007_0005` gives 5, `0x8007_00E7` gives 231), `another_facility_gives_none` (`0x8004_0005`, `0x8000_4005`) and `a_plain_code_gives_none` (0, 5);
  - switch `win::os` to it, falling back to `io::Error::other(error)` as today.

  Verify: `cargo test --locked -p brindle-remote-protocol hresult::`.
- [x] 1.2 DACLs (b21 #1, #2; design §5.5):
  - move `Ace`, `Dacl`, `Dacl::allows_only` and `SYSTEM_SID` from `win.rs:322-353,587` into `access.rs`;
  - `win.rs` keeps `impl Dacl { read, of_handle, of_path }` and adds `pub use crate::access::{Ace, Dacl, SYSTEM_SID};`, so `win::SYSTEM_SID`'s users don't change;
  - add `a_dacl_allows_only_the_named_sids`, built from values: one that passes, plus the five refusals (unprotected, `others > 0`, an extra SID, a missing SID, a duplicated SID).

  Verify:
  - `cargo test --locked -p brindle-remote-protocol access::`;
  - on Windows, `a_server_pipe_allows_only_the_user_and_system` and `a_private_directory_allows_only_the_user_and_system` pass unchanged.
- [x] 1.3 The server's client check (b14 #1; design §5.4):
  - add `access::admit_client(own: &Principal, client: io::Result<Principal>) -> io::Result<()>`, with the body of `windows/listen.rs:237` `check`: `PermissionDenied`, "refused a connection whose user can't be told: …" and "refused a connection: …", through `peer_allowed(.., Side::Server)`;
  - move the value assertions of `listen.rs`'s `the_client_must_be_the_same_user_at_no_lower_integrity` here, asserting the whole message texts. The listener's switch is task 2.4.

  Verify: `cargo test --locked -p brindle-remote-protocol access::`.
- [x] 1.4 The pipe-name check (b14 #1; design §5.4):
  - add to `server_dir.rs` `pub fn check_pipe_name(name: &str) -> Result<(), BadPipeName>`, with `pub enum BadPipeName { NoPrefix, BadName, TooLong }`, whose `Display` gives `listen.rs:104`'s three problem texts;
  - `PIPE_PREFIX` and `MAX_NAME` (256) move with it, and the length is counted with `encode_utf16`;
  - tests: `a_pipe_name_needs_the_prefix_a_plain_name_and_at_most_256_characters` (the cases of `paths_are_pipe_names`: a socket path, an empty name, `a\b`, `\\server\pipe\name`, a NUL, 256 and 257 characters) and `every_pipe_the_server_names_is_a_pipe_name` (`core_pipe`, `holder_pipe` with a 64-character build id, `pty_pipe`).

  Verify: `cargo test --locked -p brindle-remote-protocol server_dir::`.
- [x] 1.5 Holder sockets (b20 #3; design §5.5):
  - add `server_dir::holder_socket_stem(name: &str) -> Option<&str>`: the stem of `holder-….sock`, else `None`;
  - the `#[cfg(not(windows))]` `Endpoint::shown` builds the socket, lock and log paths from it;
  - test `a_holder_socket_gives_its_stem` covers a socket, a lock file, a log, `holder-.sock` and a name without the prefix.

  Verify: `cargo test --locked -p brindle-remote-protocol server_dir:: endpoint::` (`sockets_are_in_the_directory_and_show_the_holders` passes unchanged).
- [x] 1.6 Helper copies (b21 #3; design §5.5):
  - add to `server_dir.rs`:
    - `copy_temp_name(pid, n)`;
    - `is_current(copy_build: Option<&str>, build: &str) -> bool`;
    - `after_lost_rename(copy_build, build, copy_exists) -> LostRename { UseWinner, Replace, Fail }`;
    - `pruned_builds(entries: Vec<(OsString, EntryKind)>, keep: &str) -> Vec<OsString>`, with `EntryKind { Dir, Other }`;
    - `after_image_removal(ImageRemoval) -> PruneDir`, with `ImageRemoval { Removed, Missing, Failed }` and `PruneDir { Remove, Keep }`;
  - tests:
    - `a_lost_rename_uses_the_winner_replaces_another_file_or_fails`;
    - `pruning_keeps_the_current_build_and_skips_files`;
    - `a_running_image_keeps_its_build_directory`;
    - `the_temporary_copy_is_named_by_pid_and_count`.
  - `win.rs`'s `ensure_helper_copy`, `place_copy` and `prune_helper_copies` call them, keeping the reads, `MoveFileExW`, the removals and `NEXT`.

  Verify:
  - `cargo test --locked -p brindle-remote-protocol server_dir::`;
  - on Windows, `the_copy_is_named_by_build_and_keeps_the_last_write_time`, `a_wrong_file_under_the_copys_name_is_replaced`, `racing_copies_end_with_one_valid_copy` and `pruning_skips_a_build_whose_helper_is_running` pass unchanged.
- [x] 1.7 Docs for this group (design §7):
  - `functional-core-imperative-shell.md`'s "Whole pure modules" table gains `remote-protocol/src/{access,hresult}.rs` (who may use a pipe, and an `HRESULT`'s Win32 code), and change 7's `PURE_MODULES` gains both;
  - `ports-and-adapters.md` R7's examples add `access::admit_client`.

  Verify: `cargo test --locked -p brindle --test architecture`. Change 7's table-sync and purity checks pass with the two new modules.

## 2. Neutral modules beside the agent's seams (`brindle-remote-agent`)

- [x] 2.1 `server/platform/ready_arg.rs` (b13 #1 and the Linux twin; design §5.1):
  - add `pub enum ReadyArg { Fd, Handle }` (`flag()`, `placeholder()`), `role_args(arg, role)`, `parse_fd(role, args) -> Result<Option<i32>, String>` (`>= 0`) and `parse_handle(role, args) -> Result<Option<usize>, String>` (`!= 0`);
  - declare it in `server/platform/mod.rs` with lint-only `cfg_attr`s;
  - tests:
    - `a_descriptor_of_zero_or_more_is_taken`;
    - `a_nonzero_handle_is_taken`;
    - `each_parser_refuses_the_other_flag`;
    - `role_arguments_name_the_flag_and_its_placeholder`;
    - `anything_else_is_the_usage_text` (asserting the texts).

    Move into them the assertions of `unix/daemon.rs`'s `ready_fd_arguments` and `windows/daemon.rs`'s `ready_handle_arguments`, and delete those two tests.
  - `ready_from_args`, `role_args` and `start` in both `daemon.rs` files call the new module; the trampoline's `inherited_ready` takes `ReadyArg::Handle.flag()`.

  Verify: `cargo test --locked -p brindle-remote-agent ready_arg:: platform::daemon::`.
- [x] 2.2 `server/platform/ensure_dir.rs` (b13 #6; design §5.2):
  - add `Found`, `Protect`, `First`, `AfterCreate`, `first(found, protect)` and `after_create(found)`;
  - tests:
    - `an_existing_directory_is_used_or_protected_as_asked`;
    - `anything_but_a_directory_is_refused`;
    - `a_missing_directory_is_created`;
    - `an_unreadable_path_fails_the_read`;
    - `after_creating_only_a_directory_is_used`.
  - `windows/dir.rs`'s `ensure_with` matches on them, keeping the messages and the `io::Error`s.

  Verify:
  - `cargo test --locked -p brindle-remote-agent ensure_dir::`;
  - on Windows, `dir::` (`an_existing_directory_is_protected_only_when_it_is_the_default`, `a_file_is_refused_and_a_failed_create_names_the_directory`) passes unchanged.
- [x] 2.3 `server/platform/pipe_state.rs` (b14 #7; design §5.3):
  - add:
    - the Win32 constants;
    - `ReadEnd`, `read_ended`, `WriteEnd` and `write_ended`;
    - `Handle { Open, Closed }`;
    - `ReadState` (`ready`, `apply`) and `WriteState` (`unsent`, `ready`, `fail`, `apply`, `rotate -> Rotation { Send, Idle, Close }`);
    - `Sides` (`readable`, `writable`, `sent`, `hung_up`, `meets`), `Need`, `WRITE_MARK` and `KEEP`.
  - tests:
    - `a_read_of_bytes_fills_the_buffer_and_a_read_of_nothing_is_the_end`;
    - `a_broken_pipe_or_eof_is_the_end`;
    - `a_cancelled_read_changes_nothing`;
    - `another_read_error_is_kept`;
    - `a_closed_reader_is_at_its_end_whatever_the_read_said`;
    - `a_write_takes_at_most_what_was_sent`;
    - `a_write_that_took_nothing_fails_with_write_fault`;
    - `a_failed_write_drops_what_was_queued`;
    - `the_writer_is_writable_under_the_mark_or_once_failed`;
    - `rotating_drops_a_large_queues_capacity`;
    - `a_shut_writer_closes_once_everything_is_sent`;
    - `hung_up_is_the_readers_end_or_the_writers_error_without_a_reader`;
    - `a_wait_is_over_when_any_need_is_met`.
  - `windows/pipe.rs` keeps handles, `Op`, the buffers and the unsafe slices. `Reader` and `Writer` hold the states, `read_done`, `write_done` and `send` call them, and `State`'s readiness methods go through `Sides`.
  - add the `#[cfg(windows)]` test `the_win32_codes_match_the_windows_crate`.

  Verify:
  - `cargo test --locked -p brindle-remote-agent pipe_state::`;
  - on Windows, `pipe::`, `listen::` and `event_loop::` (the flood, graceful-close and cancel tests) pass unchanged.
- [x] 2.4 The listener and the loop (b14 #1; design §5.4, §5.5):
  - `windows/listen.rs`'s `pipe_name` calls `server_dir::check_pipe_name`, keeping `to_str` (not UTF-8 is `NoPrefix`), `encode_wide` and the `InvalidInput` wrapping;
  - `check` becomes a call to `access::admit_client`, and its test moved in 1.3 goes from here;
  - `event_loop::win32` returns `hresult::win32_code(code).unwrap_or(code)`.

  Verify: on Windows, `paths_are_pipe_names`, `a_client_of_another_user_or_lower_integrity_is_closed_unread` and `a_taken_name_fails_the_bind` pass unchanged.
- [x] 2.5 The program and environment (b15; design §5.6):
  - add to `conpty/cmdline.rs` `program(command, env) -> &str` and `environment(given, exact, own) -> Vec<(String, String)>`;
  - tests:
    - `the_program_is_the_command_else_a_nonempty_comspec_else_cmd` (moved from `pty.rs`'s `the_program_is_the_command_or_the_command_interpreter`);
    - `an_exact_environment_is_the_given_one`;
    - `given_names_replace_own_names_of_any_case`.
  - `pty::program_name` (still the platform item `holder.rs:762,787` calls) and `pty::environment` call them, the latter passing its UTF-8 `vars_os`.
  - If change 4 has landed, `environment_block` is a re-export in this file; leave it alone.

  Verify:
  - `cargo test --locked -p brindle-remote-agent conpty::cmdline::`;
  - on Windows, `the_sessions_environment_picks_the_host` passes unchanged.
- [x] 2.6 `conpty/choice.rs` (b09 #3, b15; design §5.6):
  - add `INBOX`, `wants_inbox(env)`, `extract_dir(local_app_data, version)` and `shipped(exe_dir, extract_dir, holds_embedded) -> Shipped { Beside, Extracted, ToExtract, Nowhere }`;
  - tests:
    - `only_inbox_asks_for_the_builtin_host` (the name in any case, the value exactly `inbox`; moved from `pty.rs:1332`);
    - `the_extraction_is_under_local_app_data_by_version`;
    - `an_empty_local_app_data_is_unset`;
    - `matching_files_beside_the_exe_win`;
    - `a_stale_or_missing_extraction_is_to_extract`;
    - `no_extract_dir_is_nowhere`.
  - `api.rs`'s `packaged` and `shipped_dir` keep the reads, writes, `load`, the `OnceLock`s and the log lines, passing `same` as the probe.

  Verify:
  - `cargo test --locked -p brindle-remote-agent conpty::choice::`;
  - on Windows, `api::` (`matching_files_beside_the_exe_are_used`, `a_different_dll_beside_the_exe_is_ignored`, `a_stale_extraction_is_rewritten`, `no_candidates_is_an_error`) passes unchanged.
- [x] 2.7 `conpty/hosts.rs` (b15; design §5.7):
  - add `Listed`, `HostSeen`, `is_session_host`, `is_ours`, `candidates` and `ends`;
  - tests:
    - `a_host_that_started_after_the_root_is_not_the_sessions`;
    - `an_exited_host_or_an_unknown_start_is_not_the_sessions`;
    - `a_host_is_ours_only_when_this_process_is_its_parent`;
    - `candidates_leave_out_this_process_and_the_host`;
    - `a_candidate_ends_only_on_the_host_and_started_after_it`.
  - `windows/pty.rs`'s `Group::host` and `Host::end_its_programs` map `PROCESSENTRY32W` to `Listed` and keep the opens, queries and terminations;
  - `plain` uses `hresult::win32_code`.

  Verify:
  - `cargo test --locked -p brindle-remote-agent conpty::hosts::`;
  - on Windows, `a_kill_ends_the_sessions_processes_and_no_others`, the three `a_hang_up_…` tests and `a_kill_after_the_grace_ends_a_root_that_left_its_console` pass unchanged.
- [x] 2.8 `pty/program.rs` and `pty/stat.rs` (b10 #3, #4; design §4, §5.8):
  - add `shell_from_env`, `passwd_shell`, `home_from_env`, `expand_home`, `start_dir` and `default_term` to `program.rs`, and `STAT_PGRP`, `STAT_TPGID` and `parse_stat_field` to `stat.rs`;
  - declare both ungated in `pty.rs`, with `cfg_attr(windows, allow(dead_code, …))`;
  - tests:
    - `the_shell_is_the_requests_then_this_processs`;
    - `an_empty_shell_counts_as_unset`;
    - `the_passwd_shell_is_used_only_if_it_exists_else_bin_sh`;
    - `home_is_the_requests_then_this_processs`;
    - `a_tilde_expands_to_home` (moved from `cwd_tilde_uses_remote_environment_home`);
    - `a_missing_start_directory_falls_back_to_home_then_to_none`;
    - `term_defaults_only_for_a_non_exact_environment_without_one`;
    - `stat_fields_count_from_after_the_command` (moved), with an empty line, a missing `)`, a short line and a command holding `) (`.
  - `pty/unix.rs` keeps `passwd_entry`, which it reads only when the environment step gives `None`, plus arg0, `Command` and the `/proc` reads. `home_dir(env)` stays for `platform/unix/dir.rs`, as `home_from_env(…).or_else(passwd)`.

  Verify:
  - `cargo test --locked -p brindle-remote-agent pty::`;
  - `cargo test --locked -p brindle-remote-agent --test holder` (Linux spawns, the login shell and the home fallback).
- [x] 2.9 Docs for this group (design §7):
  - the module docs of `server/platform/mod.rs`, `conpty/mod.rs` and `pty.rs` name the new modules and say each is pure and tested on every platform;
  - the "Whole pure modules" table gains `remote-agent/src/server/platform/{ready_arg,ensure_dir,pipe_state}.rs`, `remote-agent/src/conpty/{choice,hosts}.rs` and `remote-agent/src/pty/{program,stat}.rs`, and `PURE_MODULES` gains them;
  - `ports-and-adapters.md` R7's examples add `ready_arg::role_args` and `pty::program`;
  - `CLAUDE.md`'s "Platform layer" bullet says the decisions sit in neutral modules beside the seams.

  Verify:
  - `cargo test --locked -p brindle --test architecture`;
  - `cargo build --locked --workspace` with no warnings.

## 3. Neutral items in the app (`brindle`)

- [x] 3.1 `desktop.rs` (b01 #7; design §5.9):
  - add at module level `without_verbatim_prefix(path: &str) -> Option<String>`, with `cfg_attr(not(windows), allow(dead_code))`;
  - tests: `a_verbatim_disk_path_loses_its_prefix`, `a_verbatim_unc_path_becomes_a_unc_path`, `a_plain_path_is_left_alone`;
  - `start_menu::resolved` calls it.

  Verify: `cargo test --locked -p brindle desktop::`.
- [x] 3.2 `escape_markup` (b02 #4): move it and `markup_is_escaped` from `notify/dbus.rs` to `notify/mod.rs` beside `toast_xml`, with `cfg_attr(not(target_os = "linux"), allow(dead_code))`; `dbus.rs:114` calls `super::escape_markup`.

  Verify: `cargo test --locked -p brindle notify::`.
- [x] 3.3 Console titles (b07 #10; design §5.9):
  - `workspace::program_title` (change 7's) takes `style: PathStyle` and drops a console title only for `PathStyle::Windows`;
  - `zone_view.rs:491`, `:613` and `workspace.rs:118` pass `terminal::process::LOCAL_PATH_STYLE`;
  - add `a_console_title_is_dropped_only_for_windows_paths`, run with both styles on every platform.
  - If change 7 hasn't landed, `terminal_automatic_title` takes the argument instead (design §8).

  Verify:
  - `cargo test --locked -p brindle workspace::`;
  - `windows-title-child` in `windows-e2e` (task 5.2).
- [x] 3.4 The Unix default program (b28 #3):
  - add `terminal::cmdline::unix_default_program(shell: Option<String>, args: &[String]) -> (String, Vec<String>)`, with `cfg_attr(windows, allow(dead_code))`;
  - test `the_default_program_is_shell_as_given_else_bin_sh` covers an empty `SHELL`, kept as today;
  - the `#[cfg(unix)]` `default_program` passes `env::var("SHELL").ok()`.

  Verify: `cargo test --locked -p brindle terminal::cmdline::`.
- [x] 3.5 Instance names (b23; design §5.9):
  - add ungated `socket_path_in(runtime_dir, cache_dir, temp_dir, state_dir) -> PathBuf` and `pipe_name_for(user, state_dir) -> String` to `instance.rs`;
  - tests:
    - `the_socket_is_in_the_runtime_dir_else_the_cache_else_temp`;
    - `an_empty_runtime_dir_counts_as_unset`;
    - `the_pipe_names_its_user_in_letters_digits_and_underscores`;
    - `a_missing_user_leaves_the_user_part_empty`.
  - `imp::socket_path` and `imp::pipe_name` gather and call.

  Verify:
  - `cargo test --locked -p brindle instance::`;
  - `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- instance` (headless).
- [x] 3.6 Docs for this group: `fcis`'s "Pure items in shell modules" table gains:
  - `desktop::without_verbatim_prefix`;
  - `notify::escape_markup`;
  - `instance::{socket_path_in, pipe_name_for}`;
  - `workspace::program_title`'s path style.

  The `terminal/cmdline.rs` row adds "the Unix default program".

  Verify: `cargo test --locked -p brindle --test architecture`.

## 4. The never-gated check, with its docs in the same commit

- [x] 4.1 Write the planted tests first and watch them fail (`testing R13`):
  - `a_gated_neutral_module_is_found`: a production `#[cfg(windows)] fn`, a `#[cfg(unix)] mod x;`, and an ancestor gated `cfg(windows)`;
  - `a_gated_neutral_item_is_found`: an item inside `#[cfg(windows)] mod imp {`, and one under `#[cfg(unix)]`;
  - `a_lint_only_cfg_attr_and_test_code_do_not_count`.

  Then add `NEUTRAL_MODULES` (`remote-protocol/src/server_dir.rs`), `NEUTRAL_ITEMS` (design §6) and `the_neutral_code_is_never_gated`. It covers `PURE_MODULES` and `NEUTRAL_MODULES`: production lines, the module's own declaration, and its ancestors, where only `cfg(any(target_os = "linux", windows))` is allowed. It also covers `NEUTRAL_ITEMS`: column 0, and no platform `cfg` among the attributes above. Add `every_neutral_module_and_item_exists`.

  In the same commit:
  - `functional-core-imperative-shell.md`'s Enforcement says the check holds R7 for the whole pure modules, `server_dir.rs` and the named items, and review holds the rest;
  - R7's paragraph "The other platform's data" names `pipe_state` and `server_dir::check_pipe_name`;
  - `.claude/rules/remote-agent.md` points its `ports R7` line at the new modules, and its "Enforced by" names the check;
  - `.claude/rules/remote-protocol.md` names `access.rs`, `hresult.rs` and `server_dir.rs`.

  Verify:
  - `cargo test --locked -p brindle --test architecture`;
  - locally (not committed), gating `mod pipe_state;` with `#[cfg(windows)]` fails the check.

## 5. Integration

- [ ] 5.1 Linux:
  - `cargo test --workspace --locked`;
  - `cargo build --release --locked` with no new warnings;
  - `scripts/with-xvfb env BRINDLE_E2E=1 BRINDLE_E2E_SLOW=2 cargo test --test e2e` (headless; covers `local-server-*`, `instance-*` and titles).

  Verify: all pass, and the counts include the new tests of groups 1 to 4.
- [ ] 5.2 Windows: push the branch, then:
  - the `windows` job (`cargo test`, both self-tests) and `windows-e2e` (`windows-conpty-host`, `windows-conpty-fallback`, `windows-title-child`, `windows-close-mid-output`, `windows-pwsh-integration`, `local-server-*`) pass;
  - the moved tests run there for the first time on that platform, so the PR says so (`testing R17`).

  Verify: both jobs green on the PR.
- [ ] 5.3 Run `openspec validate test-platform-logic-everywhere --strict` and verify it passes.

No manual checks: nothing here can only be shown with real keys or a pointer.

## Workflow follow-up

- The archive commit goes on the implementation PR, as `docs(openspec): …`.
- The PR title is a `refactor:` and is not breaking.
