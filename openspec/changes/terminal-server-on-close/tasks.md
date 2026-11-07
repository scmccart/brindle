# Tasks

## 1. Protocol

- [ ] 1.1 Add `key` (random 128-bit hex) to `SPAWNED` and `SessionInfo`, and `title` and `started` (unix seconds) to `SessionInfo`, as additive JSON fields with defaults; extend the protocol-1 transcript tests and verify old frames still decode (`cargo test -p brindle-remote-protocol`)
- [ ] 1.2 Add H1 version 2 with `SET_META { session, meta }` (new message number, version range 1–2), transcript test it, and verify a v1-only peer negotiates v1 (`cargo test -p brindle-remote-protocol`)
- [ ] 1.3 Extend the protocol fuzzer to the new message and fields, and verify `remote-protocol/src/fuzz.rs` passes

## 2. Server

- [ ] 2.1 Generate the session key at spawn, keep it, `started` and the policy in `SessionMeta`, report key, title (from the emulator) and `started` in `SESSIONS`; verify in `remote-agent/tests/core.rs` that a fresh core reusing id 1 gives a different key
- [ ] 2.2 Holder: store a `SET_META`'s meta in place of the spawn's, and return it in its `WELCOME`; verify in `remote-agent/tests/holder.rs`
- [ ] 2.3 Core: on `ATTACH` set the owner to the client's token; on `DETACH` clear the owner and switch `kill` to `detach`; send `SET_META` for each change to H1 v2 holders, and log and skip it for v1 holders; verify the "Close policies" and "Attaching takes ownership" scenarios in `remote-agent/tests/core.rs`
- [ ] 2.4 Verify across a core restart (kill the core, let a holder restart it) that a detached session stays detached and gets no replay, and that a session attached by client B returns to B and not to A (`remote-agent/tests/core.rs` or `upgrade.rs`)
- [ ] 2.5 `brindle-remote list`: show policy, age and title, and in `--json` the new fields; verify in `remote-agent/tests/manage.rs`
- [ ] 2.6 Start `serve` and `hold` through `systemd-run --user --scope --collect --quiet --unit=brindle-<role>-<random>` when `$XDG_RUNTIME_DIR/systemd/private` exists and `systemd-run` is on `PATH`, unless `BRINDLE_NO_SCOPE=1`, falling back with one log line; unit-test the decision, and verify by hand that a core started from Brindle runs in its own `brindle-serve-*.scope` (`systemctl --user status`)
- [ ] 2.7 Update CLAUDE.md's terminal-server section (owner rules, H1 v2, scopes) and verify it matches the code

## 3. Brindle's client

- [ ] 3.1 `Connection::spawn` takes the policy and returns the key; `Connection::attach(id, size, sink)` sends `ATTACH` and resolves on `REPLAY_START` (with its `req`) or `ERROR`, with the sink in the map before the first replay `DATA`; verify with a scripted core in `local/connection.rs` tests
- [ ] 3.2 `Session::attach` mirrors `Session::start` (non-blocking, queued input and resizes); verify with a slow scripted core as in `session.rs`'s tests
- [ ] 3.3 `Session::detach` sends `DETACH`, `Session::close` stays `CLOSE`, and a session that already exited sends neither; verify in `session.rs` tests
- [ ] 3.4 `Connection::list()` returns `SessionInfo`s, and `Connection::kill(id)`; verify against a scripted core
- [ ] 3.5 `Connection::end_all_and_flush(deadline)` sends `CLOSE` for the given sessions and returns once the writer has written them; verify with a scripted core

## 4. Config and terminals

- [ ] 4.1 Add `on_close = "detach" | "ask"` to `Profile` (default `detach`; any other value, `kill` included, a config error naming the key and both values) and to `DEFAULT_CONFIG`'s comments; verify `config.rs` tests for the default, each value and the `kill` error
- [ ] 4.2 `Terminal` with `Backend::Server` records at spawn whether it asks; profile tabs spawn with `detach`, `-e` command tabs with `kill`; `Terminal::close(Keep | End)` sends `DETACH` or `CLOSE`, and `Drop` keeps (ends for `-e`); verify with unit tests over the decision
- [ ] 4.3 A `Terminal` built from `Session::attach`, with the replay quiet, asking per its profile's `on_close`; verify a replay with a bell raises none in a unit test
- [ ] 4.4 Settings dialog: the on-close field on the profile page (hidden for tmux and remote profiles), saved via `config_edit` without writing the default; verify `settings_dialog` and `config_edit` tests

## 5. Closing

- [ ] 5.1 Zone, tab and window close collect the local terminals they would close; with no `ask` among them close as today with each terminal's behaviour, otherwise open one prompt (keep running / end / cancel) and act on the answer; verify with the e2e cases in 7.1
- [ ] 5.2 The caption ✕ and window-manager close go through GPUI's should-close callback with the same prompt; verify with an e2e case closing the window by action and by `WM_DELETE_WINDOW`
- [ ] 5.3 `Quit` and closing the last window: one prompt for every asking terminal in every window; "end" calls `end_all_and_flush` before quitting, "keep" quits; verify with an e2e case
- [ ] 5.4 Update README's keybindings/behaviour section for `on_close`, and verify the documented default matches `DEFAULT_CONFIG`

## 6. Launch reattach, restore, palette and `--attach`

- [ ] 6.1 Remove `restore_session`: accept and ignore the key with one logged warning, drop it from `Config`, `DEFAULT_CONFIG`, `--help` and README, and record and restore unconditionally (`--no-restore` still skips); verify `config.rs` tests (old key loads, no banner) and `session.rs` tests
- [ ] 6.1a Give every e2e and perf case its own `BRINDLE_STATE_DIR` (`tests/e2e/brindle.rs`, `cases/win.rs`, `tests/perf`), drop `restore_session = true` from the restore cases' configs, and verify the restore cases and a few unrelated ones pass under `scripts/with-xvfb` with nothing written under `~/.local/state/brindle`
- [ ] 6.2 Record `{ id, key }` per local Linux zone in the state file (optional field, `VERSION` stays 1); verify a round trip and that an old file loads (`session.rs` tests)
- [ ] 6.3 On a restoring launch, `LIST` once, then attach each zone whose id and key match a detached session, and respawn the rest in their directories; verify with the e2e cases in 7.2
- [ ] 6.4 After restore (or the launch's usual tab), attach every remaining detached session as a tab of the first window in start order, with its profile by name (default when gone), keeping the active tab; skip for `-e`, `--no-restore` and when not holding the lock; verify with e2e cases
- [ ] 6.5 `attach_session` ("Attach Session…") palette: rows of detached sessions (title or command, directory, age), newest first, enter attaches after the active tab, `picker_kill` (ctrl-shift-delete) kills and refreshes, an empty list says so; add both to `ACTIONS`; verify with an e2e case using `--action` steps and the `--- palette` dump
- [ ] 6.6 `--attach <ID>` (Linux): parse it, make the launch non-plain, open the window with a tab attached to that session, or a message tab naming the id and the reason; update `--help`/usage and verify `parse_args` tests and an e2e case
- [ ] 6.7 Update README for reattaching, the palette and `--attach`, and CLAUDE.md's client section, and verify the commands shown run as written

## 7. End-to-end checks

- [ ] 7.1 `local-server-close-*` cases: zone/tab close under `detach` (listed detached, same pid), `ask` keep/end/cancel through `--action picker_confirm`/escape, and an `-e` window close (process gone, nothing listed); verify `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- local-server-close`
- [ ] 7.2 `local-server-reattach-*` cases: quit and relaunch (same pid, same screen), SIGKILL Brindle then relaunch (split tree with `vim` and scrollback back), a session killed meanwhile (respawn in the directory), a closed tab comes back after the restored ones, `--no-restore` leaves sessions detached, a second Brindle attaches nothing, no bell from a replay (`BRINDLE_NOTIFY_LOG` empty); verify under `scripts/with-xvfb`
- [ ] 7.3 `local-server-palette` and `local-server-attach-flag` cases; verify under `scripts/with-xvfb`
- [ ] 7.4 Run the whole suite and `cargo test --workspace --locked` and `cargo build --release --locked`, and verify all pass
- [ ] 7.5 Hand the manual checks to the user: a GNOME-launched Brindle survives `systemctl --user stop app-gnome-brindle-*.scope` with its sessions; the caption ✕ prompt under `ask`

## Workflow follow-up

- Land through a squash-merged PR titled `feat(terminal-server)!: keep local terminals running after close and reattach them`.
- Archive the change and sync its specs after merge.
