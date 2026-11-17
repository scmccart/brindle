# Tasks

This change depends on two earlier changes. Start from a tree with both merged:
- change 2 (`carry-session-identity-and-host`): groups 4 and 5 use its `State::shows(id, key)` and `State::forget(id, key)`;
- change 5 (`enforce-platform-seams`): tasks 1.6 and 2.7 use its `PathStyle` and `Parser::new(style)`.

## 1. Pure models and the shared-crate helper

- [x] 1.1 Add `src/queue_marks.rs` with `QueueMarks { high, low }`, a `const fn new` that asserts `low < high`, and `over(was_over, level) -> bool` (design, decision 1).
  - Write the unit tests first:
    - `a_queue_goes_over_at_the_high_mark`;
    - `it_stays_over_until_under_the_low_mark`;
    - `the_marks_themselves_are_over`, which checks `high` going up and `low` staying over.
  - Declare the module in `src/main.rs`.
  - Verify with `cargo test -p brindle queue_marks`.
- [x] 1.2 Add `src/terminal/events.rs` (decision 2) with:
  - `Kind { Merged(Merge), Life, Side { status: bool }, Ignored }`;
  - `Merge { Wakeup, Title, Foreground, Clipboard(ClipboardType) }`;
  - `Admit { Queue, Merged, Dropped, Ignored }`;
  - `Recovered { dropped: u64, statuses: bool }`;
  - `EventGate`, with `admit`, `took`, the title and clipboard slots, and `EVENT_MARKS = QueueMarks::new(2048, 512)`.

  It works on `Kind` only, so it doesn't touch `ChannelEvent` yet. Write the unit tests first:
  - `one_entry_waits_per_merge_kind_and_the_latest_value_wins`, covering the title, `ResetTitle` as `None`, and each clipboard selection;
  - `life_events_are_queued_past_the_high_mark`;
  - `side_events_are_dropped_from_the_high_mark_until_under_the_low_mark`;
  - `recovery_is_reported_once_with_the_count_and_whether_a_status_was_dropped`;
  - `ignored_events_are_never_queued`.

  Verify with `cargo test -p brindle terminal::events`.
- [x] 1.3 Add `src/remote/budget.rs` (decision 3) with:
  - `INPUT_MARKS = QueueMarks::new(4 << 20, 1 << 20)`;
  - `InputBudget { waiting, over }`, with `offer` and `handled`;
  - `Offer { Take, Refuse(Refusal) }`, `Refusal { First, Again }` and `Delivery { Queued, Refused(Refusal), Discarded }`.

  Declare the module in `src/remote/mod.rs` ungated, with the `cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))` that `identity` uses, so it builds and is tested on every platform (`fcis R7`). Write the unit tests first:
  - `input_is_taken_while_under_the_high_mark`;
  - `the_first_refusal_after_going_over_is_first_and_the_rest_again`;
  - `input_is_taken_again_once_under_the_low_mark`;
  - `one_call_past_the_high_mark_is_taken_whole`;
  - `handled_never_goes_below_zero`.

  Verify with `cargo test -p brindle remote::budget`.
- [x] 1.4 Add `Window` to `src/latency.rs` (decision 8): a `VecDeque<Sample>` capped at `SAMPLE_WINDOW = 10_000`, with a `dropped: u64`.
  - Write the unit tests first:
    - `a_window_keeps_the_newest_samples_and_counts_the_dropped`;
    - `the_summary_names_dropped_samples_only_when_there_are_some`, against `summary`, now given a window.
  - Verify with `cargo test -p brindle latency`.
- [x] 1.5 Add `Assembler::abandon(&mut self, session) -> bool` to `remote-protocol/src/holder.rs`. It drops that session's open blob, and its encoding is unchanged.
  - Write the unit test first: `an_abandoned_blob_lets_the_next_begin_start_clean`.
  - Verify with `cargo test -p brindle-remote-protocol holder` and that the transcript tests still pass unchanged.
- [x] 1.6 Write the unit tests first in `term-core/src/emulator.rs`'s tests (decision 10). They name the style explicitly, so both run on every platform:
  - `a_windows_style_emulator_replays_a_9_9_directory`: feed `ESC ] 9 ; 9 ; C:\src ST`. The emulator's marks hold `C:\src`, and `render` contains `ESC ] 7 ; file:///C:/src ST`.
  - `a_posix_style_emulator_keeps_osc_7_and_drops_a_windows_9_9_path`: OSC 7 `file://host/home/me` is kept and replayed, and `9;9;C:\src` is not.
  - `a_replayed_directory_survives_a_second_emulator`: a Windows-style emulator fed the first one's replay holds `C:\src`, as a new core does with a checkpoint.

  Then make `Emulator::new(cols, rows, scrollback, style: PathStyle)` pass `style` to `Parser::new`, and update the tests in `emulator.rs` that build one with `PathStyle::Posix`. Verify with `cargo test -p brindle-term-core emulator`, which fails before the change and passes after.

## 2. Holder and core (`remote-agent`)

- [x] 2.1 Add `remote-agent/src/server/fault.rs`, holding:
  - `describe(&(dyn Any + Send)) -> String`, moved from `core/screen.rs`'s guard;
  - `marker(var, cell) -> Option<&'static [u8]>`, the debug-only `OnceLock` read that `inject_fault` does today.

  `core/screen.rs` then uses both. Verify that `cargo test -p brindle-remote-agent --test core a_panicking_emulator_leaves_its_session_and_the_others_running` still passes.
- [x] 2.2 Write the unit tests first in `holder.rs`'s tests module (decision 6):
  - `core_backlog_resumes_only_under_the_low_mark`;
  - `read_limit_is_zero_for_a_core_over_its_mark_or_a_paused_session`;
  - `read_limit_without_a_core_is_the_tails_room`.

  Then implement the changes:
  1. Add `CORE_QUEUE_LOW = 2 MiB` and `Core.over`.
  2. Add `core_backlog`, `enum Dest { Core { over }, Tail { room } }` and `read_limit`.
  3. Replace the inline tests in `read_output` (`:875`) and `update_interests` (`:1093`) with them.

  Verify with `cargo test -p brindle-remote-agent --lib holder`, and that `an_exiting_holder_delivers_a_flood_and_its_exit_to_a_slow_core` and the rest of `--test holder` still pass.
- [x] 2.3 Write the real-process tests first in `remote-agent/tests/holder.rs` (decision 5).
  - Add `start_holder_with(dir, env)` to `tests/support/mod.rs`, with `start_holder` calling it.
  - Use the platform programs the existing holder tests use, so the tests run on Linux and Windows:
    - `a_holder_fault_while_spawning_fails_that_spawn_only`: one session running, then a `SPAWN` whose arguments carry `BRINDLE_TEST_HOLDER_PANIC`'s marker gets `SPAWN_FAILED` naming the failure. The first session keeps its pid and echoes input.
    - `a_holder_fault_on_input_reports_an_error_and_both_sessions_keep_running`: `INPUT` carrying the marker gets an `ERROR` for that session, and both sessions keep their pids and echo later input.
- [x] 2.4 Implement decision 5 in `holder.rs`:
  1. `enum Unit` and the pure `unit_of(&ToHolder)`, naming every variant, with a unit test `every_message_has_its_unit`.
  2. `Holder::guarded`, called from `process_core_frames` (`:615`).
  3. `Holder::fault`, covering `SPAWN_FAILED`, a session's `ERROR`, `Assembler::abandon` for checkpoints, and the holder-wide `ERROR`.
  4. The debug-only marker checks in `spawn` and `input`.

  Verify with the 2.3 tests and `cargo test -p brindle-remote-agent`.
- [x] 2.5 Write the refusal tests first (decision 7). Each test sends a refused opening, then 64 KiB more that the peer never reads, and expects the `ERROR`, then a read of 0 bytes, not an error:
  - `remote-agent/tests/core.rs`: `a_client_refused_for_its_first_message_reads_the_error_then_the_end` and `a_client_refused_for_its_versions_reads_the_error_then_the_end`;
  - `remote-agent/tests/holder.rs`: `a_core_refused_for_its_first_message_reads_the_error_then_the_end` and `a_core_refused_for_its_versions_reads_the_error_then_the_end`.

  Then implement:
  - rename `REFUSAL_FLUSH_LIMIT` to `REFUSAL_CLOSE_LIMIT` in `core/clients.rs`, and use `conn.close_for` at `:130` and `:155`;
  - add the holder's own `REFUSAL_CLOSE_LIMIT` (1 s) for `:456` and `:477`.

  Verify with the four tests on Linux and on the Windows CI job.
- [x] 2.6 Update `docs/architecture/process-architecture.md`:
  - R9: the holder's `CORE_QUEUE_HIGH`/`CORE_QUEUE_LOW` example, and that its input cap is lossy by design, behind Brindle's equal budget;
  - R3: the holder's per-message guard beside the core's per-session feed guard.

  Update CLAUDE.md's holder paragraph to match: the low mark, the per-message guard, `BRINDLE_TEST_HOLDER_PANIC` and the graceful refusals. Check that neither names `REFUSAL_FLUSH_LIMIT`.
- [ ] 2.7 Write the real-process test first in `remote-agent/tests/core.rs`'s `on_windows` module (decision 10): `a_detached_sessions_9_9_directory_is_replayed_on_attach_and_after_a_core_restart`.
  1. A `printer` session prints `ESC ] 9 ; 9 ; C:\src ST` and is detached.
  2. Attaching it replays `ESC ] 7 ; file:///C:/src ST`.
  3. Kill the core, and let a new one adopt the holder (as the existing restart tests do). Attaching again replays it again.

  Then:
  - add `pub const PATH_STYLE: PathStyle` to `server/platform/unix/pty.rs` (`Posix`) and `server/platform/windows/pty.rs` (`Windows`);
  - make `core/screen.rs`'s `Screen::new` pass `platform::pty::PATH_STYLE`;
  - make `remote-agent/tests/support/mod.rs:1093` pass `PathStyle::Posix`, or the platform's style if its callers compare Windows output.

  Update CLAUDE.md's `brindle-term-core` paragraph: `Emulator` is built with the path style of the platform its programs run on. Verify with `cargo test -p brindle-remote-agent --test core` on the Windows CI job, and that `cargo test -p brindle --test architecture` (`the_server_core_and_holder_name_no_platform_type`) passes.

## 3. Terminal events (`src/terminal/`)

- [x] 3.1 Extend `ChannelEvent` with `Merged(Merge)`, `Broken(String)` and `InputRefused`, and add `kind_of(&ChannelEvent) -> Kind` to `events.rs`, per the design's table. It names every variant of `ChannelEvent`, `Found` and alacritty's `Event`.
  - Write the test first: `every_event_has_the_kind_the_design_gives_it`.
  - Verify with `cargo test -p brindle terminal::events`.
- [x] 3.2 Make `Listener` `{ tx, output, gate }` with `Listener::send`.
  - `EventListener::send_event` advances the `OutputMarker` on `Wakeup`, then sends.
  - `Sink::send` (`server.rs:58`), `output` (`:96`), `replay` (`:107`) and `exited` (`:113`) all go through it.
  - `start_session`, `attach` and `message` (`mod.rs:413, 443, 481`) build the gate.
  - Update `server.rs`'s `sink()` test helper and `kinds()`, and add `many_reads_queue_one_wakeup_until_it_is_taken`. Check that `output_is_parsed_and_wakes_the_ui_like_a_pty_read` and the replay tests still pass.
  - Verify with `cargo test -p brindle terminal::server`.
- [x] 3.3 Rewrite `spawn_events`' handler (decision 2):
  - one `took` per chunk;
  - `Merged` markers read the gate's slots;
  - a match that names every `ChannelEvent` variant;
  - on `Recovered`, a `warn` log line, and `Records::reset` plus `TerminalEvent::Status(None)` when `statuses` is set;
  - `Batch` and `Batch::coalesce` removed.

  `Broken` and `InputRefused` get their real arms in 4.3 and 5.4. Until then they log. Verify with `cargo test -p brindle terminal` and `cargo build`.
- [x] 3.4 Update CLAUDE.md's "Alacritty events reach GPUI through an unbounded channel…" paragraph: merged at the source, side events dropped from 2048 waiting until under 512, and the gate as a leaf lock. Add the gate's mutex as a leaf to `process R14`'s lock-order note in `process-architecture.md`.

## 4. Brindle's client: a sink panic detaches its session

- [x] 4.1 Write the `fake::link` tests first in `src/remote/local/connection.rs`:
  - `a_sink_that_panics_is_told_broken_and_its_session_detached`: two spawned sessions; one sink panics on `DATA`. It gets `broken`, the link gets `DETACH` for its session, and the other sink receives its later `DATA`.
  - `a_sink_that_panics_while_the_link_is_down_is_detached_after_the_reconnect`: the panic comes from the lost line.
  - `a_broken_sessions_running_handle_writes_nothing`: after the panic, that `Running`'s `input`, `resize` and `close` write nothing.
- [x] 4.2 Implement the guard (decision 4):
  - `Connection::deliver`, `enum Delivered { Done, Panicked(String) }` and a payload-to-text helper, used for every per-session sink call in `handle`, `lost_link`, `ended` and the listing prune;
  - on a panic, `State::forget(id, key)`, `DETACH`, then a guarded `broken`;
  - `SessionSink::broken`, with `Sink::broken` sending `ChannelEvent::Broken`;
  - in debug builds, `BRINDLE_TEST_SINK_PANIC` in `Sink::output`, read once through a `OnceLock`.

  Verify with the 4.1 tests and `cargo test -p brindle remote::local`.
- [x] 4.3 Add `Terminal`'s `Broken(reason)` arm:
  - it replaces `Term`, `Parser` and `Marks` under parser → term → marks, and resets `Records`;
  - it writes the red line, then calls `mark_lost`.
  - `Terminal` is a GPUI entity, and Brindle has no headless GPUI harness, so this arm is checked by the e2e case in 7.1 (`testing R1`).
  - Verify with `cargo build` and `cargo test -p brindle terminal`.
- [x] 4.4 Add `no_profile_aborts_on_panic` to `tests/architecture.rs`: no `[profile.*]` in the root `Cargo.toml` sets `panic = "abort"`, since the guards in Brindle and in the core rely on unwinding. Verify with `cargo test -p brindle --test architecture`.
- [x] 4.5 Update CLAUDE.md's "Brindle's client" paragraph: every sink call is guarded, a panic detaches the session and the tab says so, and `BRINDLE_TEST_SINK_PANIC` exists.

## 5. Brindle's client: the input budget

- [x] 5.1 Write the tests first:
  - `connection.rs`, `fake::link`:
    - `input_past_four_mib_is_refused_while_the_link_is_down`: `First` once, then `Again`, and nothing refused is written after the reconnect.
    - `held_input_is_written_in_order_after_the_reconnect_and_frees_the_budget`: taken again once under 1 MiB.
    - `a_paste_past_the_high_mark_on_a_live_link_is_written_whole`.
    - `resize_and_detach_are_never_refused`.
  - `session.rs`: `input_held_while_starting_counts_once_across_the_handoff` and `input_while_starting_is_refused_past_the_high_mark`.
- [x] 5.2 Make the outgoing channel carry `Outgoing { message: ToCore, backlog: Option<(Arc<Mutex<InputBudget>>, usize)> }`, with `Connection::send` wrapping plain messages. The writer (`write_loop`, `write`) calls `handled` after writing or skipping a `DATA` that has a backlog. Verify with `cargo test -p brindle remote::local::connection`.
- [x] 5.3 Thread the budget through the session:
  - `Session` owns the budget;
  - `Phase::Starting` offers before it pushes;
  - `run()` hands the budget to `Running`, and the held input goes out without a second offer;
  - `Running::input` checks `State::shows`, then offers and queues, and returns `Delivery`;
  - `Session::input` returns `Delivery`.

  Verify with the 5.1 tests.
- [x] 5.4 Make `Terminal` keep a `Listener` clone. `Terminal::write` sends `ChannelEvent::InputRefused` on `Refused(First)`, and the event task's arm writes the dim line through parser → term → marks and notifies. Verify with `cargo test -p brindle terminal` and `cargo build`.
- [x] 5.5 Update `process R9` in `process-architecture.md` with Brindle's input budget (4 MiB / 1 MiB, refused past the mark and said in the tab) and the event marks. Update CLAUDE.md's "Brindle's client" paragraph to say input waits under the budget while starting and while the link is down.

## 6. Latency and config writes

- [x] 6.1 Make `Latency`'s `samples` and `all_samples` windows. `summary` and `log_summary` print ` dropped=D` only when D > 0. Verify with `cargo test -p brindle latency` and `BRINDLE_PERF=1 cargo test --test perf -- --self-test`, which checks that `parse_latency` still reads the header.
- [x] 6.2 Add `config::write_replacing(path, text)` (decision 9) and use it from `write_default` (`config.rs:434`) and `config_edit::save` (`config_edit.rs:33-40`).
  - Write the unit tests first, in a temp dir:
    - `a_replaced_file_has_the_new_text_and_no_temp_file_is_left`;
    - `write_default_creates_the_directory_and_the_default_config`.
  - Check that `config_edit`'s existing save tests, including the symlink case if there is one, still pass.
  - Verify with `cargo test -p brindle config`.

## 7. End-to-end

- [x] 7.1 Add `local-server-sink-panic` to `tests/e2e/cases/local_server.rs`, and register it in `cases/mod.rs` and `cases/win.rs`. Steps:
  1. Run with `BRINDLE_TEST_SINK_PANIC` set to a marker, and `prompt()`.
  2. Run `--action split_zone_right`, then wait for the new zone's prompt.
  3. Send a command that prints the marker without echoing it whole (`sh(…)`: `printf '%s-%s\n' brindle-test sink-panic` / its PowerShell form).
  4. Wait with `--wait-for "can't show this session"`.
  5. Run `--action focus_zone_left`, send `echo_sum("fine-", 1, 1)`, and `--wait-for fine-2`.
  6. A `call_at` step uses `listed_when` to wait, bounded, for one session listed detached with its pid alive and one attached.

  Pass when the dump shows `--- zones count=2`, the screen has `fine-2`, and there is no `--- wait-for timeout` line. Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- local-server-sink-panic`, and show it fails on a build without group 4: the whole window stops updating.

## 8. Integration checks

- [ ] 8.1 Run `cargo test --workspace --locked` (including `tests/architecture.rs`) and `cargo build --release --locked`. Check that both pass with no new warnings.
- [ ] 8.2 Run `scripts/with-xvfb env BRINDLE_E2E=1 BRINDLE_E2E_SLOW=2 cargo test --test e2e -- local-server remote notify status shell`, covering the event path (notifications, status dots, shell integration and titles), the server cases and the remote cases (which skip without Docker). Check that all pass, and record which remote cases ran.
- [ ] 8.3 On the PR, check that the Windows CI jobs pass: `windows`, which runs the holder and core tests above, and the advisory `windows-e2e`.

## Manual checks

- The refusal line, which needs a real paste:
  1. Open a tab on an SSH or container profile.
  2. Pause the host (`docker pause <container>`).
  3. Paste a file of more than 4 MiB with ctrl-shift-v, then type `pwd` and enter. The tab says once that input was not sent.
  4. Unpause it. The pasted text reaches the shell, and `pwd` doesn't.

  Hand it to the user with `scripts/run-throwaway.sh`.

## Workflow follow-up

- Land after change 5. When this change is archived, change 5's Open Question on Windows replays is settled; say so in the PR.
- Land after change 2. If change 2 was archived first, the MODIFIED requirements here must match its synced specs; they were copied from its deltas.
- Change 3 (`bound-and-fuzz-outside-input`) edits `holder.rs`'s checkpoint arms and `Assembler`. Whichever lands second rebases, keeping `Assembler::abandon` beside its cap.
- Archive the change in the implementation PR (`docs(openspec): …`), syncing the three delta specs.
