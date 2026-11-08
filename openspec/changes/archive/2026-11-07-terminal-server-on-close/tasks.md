# Tasks

## 1. Protocol

- [x] 1.1 Add `key` (random 128-bit hex) to `SPAWNED` and `SessionInfo`, and `title` and `started` (unix seconds) to `SessionInfo`, as additive JSON fields with defaults; extend the protocol-1 transcript tests and verify old frames still decode (`cargo test -p brindle-remote-protocol`)
- [x] 1.2 Add H1 version 2 with `SET_META { session, meta }` (new message number, version range 1–2), transcript test it, and verify a v1-only peer negotiates v1 (`cargo test -p brindle-remote-protocol`)
- [x] 1.3 Extend the protocol fuzzer to the new message and fields, and verify `remote-protocol/src/fuzz.rs` passes

## 2. Server

- [x] 2.1 Generate the session key at spawn, keep it, `started` and the policy in `SessionMeta`, report key, title (from the emulator) and `started` in `SESSIONS`; verify in `remote-agent/tests/core.rs` that a fresh core reusing id 1 gives a different key
- [x] 2.2 Holder: store a `SET_META`'s meta in place of the spawn's, and return it in its `WELCOME`; verify in `remote-agent/tests/holder.rs`
- [x] 2.3 Core: on `ATTACH` set the owner to the client's token; on `DETACH` clear the owner and switch `kill` to `detach`; send `SET_META` for each change to H1 v2 holders, and log and skip it for v1 holders; verify the "Close policies" and "Attaching takes ownership" scenarios in `remote-agent/tests/core.rs`
- [x] 2.4 Verify across a core restart (kill the core, let a holder restart it) that a detached session stays detached and gets no replay, and that a session attached by client B returns to B and not to A (`remote-agent/tests/core.rs` or `upgrade.rs`)
- [x] 2.5 `brindle-remote list`: show policy, age and title, and in `--json` the new fields; verify in `remote-agent/tests/manage.rs`
- [x] 2.6 Start `serve` and `hold` through `systemd-run --user --scope --collect --quiet --unit=brindle-<role>-<random>` when `$XDG_RUNTIME_DIR/systemd/private` exists and `systemd-run` is on `PATH`, unless `BRINDLE_NO_SCOPE=1`, falling back with one log line; unit-test the decision, and verify by hand that a core started from Brindle runs in its own `brindle-serve-*.scope` (`systemctl --user status`)
- [x] 2.7 Update CLAUDE.md's terminal-server section (owner rules, H1 v2, scopes) and verify it matches the code

## 3. Brindle's client

- [x] 3.1 `Connection::spawn` takes the policy and returns the key; `Connection::attach(id, size, sink)` sends `ATTACH` and resolves on `REPLAY_START` (with its `req`) or `ERROR`, with the sink in the map before the first replay `DATA`; verify with a scripted core in `local/connection.rs` tests
- [x] 3.2 `Session::attach` mirrors `Session::start` (non-blocking, queued input and resizes); verify with a slow scripted core as in `session.rs`'s tests
- [x] 3.3 `Session::detach` sends `DETACH`, `Session::close` stays `CLOSE`, and a session that already exited sends neither; verify in `session.rs` tests
- [x] 3.4 `Connection::list()` returns `SessionInfo`s, and `Connection::kill(id)`; verify against a scripted core
- [x] 3.5 `Connection::flush(deadline)` (added for quitting, removed again in 4.3: quitting sends nothing)
- [x] 3.6 An attached terminal starts at the session's listed size, and its sink feeds the first replay at that size, then resizes back under the term lock; verify with a `terminal/server.rs` unit test and the `local-server-reattach-inactive` e2e case

## 4. Closing ends, quitting keeps

- [x] 4.1 Remove `on_close` (`Profile`, `DEFAULT_CONFIG`, the settings dialog field, `config_edit`) and its tests; verify `cargo test -p brindle config settings_dialog config_edit`
- [x] 4.2 `Closing`: profile tabs spawn with `detach` and close with `CLOSE`; `-e` tabs spawn with `kill`; `Drop` and quit send nothing; remove `Leave::Keep` paths that only `ask` used; verify with unit tests over the decision
- [x] 4.3 Remove the close prompts (zone, tab, window, quit) and the should-close prompt: zone and tab close end their terminals, the window's close and `Close Window` quit; quit no longer flushes; verify with the e2e cases in 7.1
- [x] 4.4 Update README (closing ends, quitting keeps) and CLAUDE.md's client section, and verify they match

## 5. One window

- [x] 5.1 Remove `new_window` (action, binding, palette entry, `ACTIONS`) and every path that opens a window after launch (`Target::Window`, "New window: <profile>" rows, `open_window` from the workspace); verify `actions.rs` tests and that `--list-actions` has no `new_window`
- [x] 5.2 tmux: drop cross-window lookups (`find_session` over other windows, `go_to_session`, `SessionLocation::Elsewhere`, "Go to" rows, "in another window (switches)"); a session this window shows activates its tab; keep `ShownByAnotherBrindle`; verify `tmux` unit tests and the `session-*` and `tmux-*` e2e cases
- [x] 5.3 Restore: merge a multi-window state file into one window (tabs in order, first window's size and active tab, each control-mode session once); record one window; verify `session.rs` tests
- [x] 5.4 Remove or rewrite the multi-window e2e cases (`multi_window.rs`, `windows.rs`, the `--- window N` dump lines where nothing produces them) and the harness's multi-window parsing; verify the suite under `scripts/with-xvfb`
- [x] 5.5 Update README's keybindings (no ctrl-shift-n) and CLAUDE.md's workspace notes, and verify they match

## 6. Launch, restore and reattach

- [x] 6.1 Remove `restore_session`: accept and ignore the key with one logged warning, drop it from `Config`, `DEFAULT_CONFIG`, `--help` and README, and record and restore unconditionally (`--no-restore` still skips); verify `config.rs` tests (old key loads, no banner) and `session.rs` tests
- [x] 6.1a Give every e2e and perf case its own `BRINDLE_STATE_DIR` (`tests/e2e/brindle.rs`, `cases/win.rs`, `tests/perf`), drop `restore_session = true` from the restore cases' configs, and verify the restore cases and a few unrelated ones pass under `scripts/with-xvfb` with nothing written under `~/.local/state/brindle`
- [x] 6.2 Record `{ id, key }` per local Linux zone in the state file (optional field, `VERSION` stays 1); verify a round trip and that an old file loads (`session.rs` tests)
- [x] 6.3 On a restoring launch, `LIST` once, attach each zone whose id and key match a detached session, and drop the zones with a record whose session doesn't match (sibling takes the split, empty tabs dropped, a default tab when nothing is left); zones without a record restore as before; verify with `session.rs`/`workspace` unit tests and the e2e cases in 7.2
- [x] 6.4 After restore, attach every remaining detached session as a tab in start order, with its profile by name (default when gone), keeping the active tab; skip for `--no-restore` and when not the instance; verify with e2e cases
- [x] 6.5 Every instance launch restores, then opens its request as the active tab (`-p`/`-d` a tab, `-e` a command tab, `--attach` an attached tab; a default tab only when nothing was restored); update `is_plain` and `--help`; verify `parse_args` tests and e2e cases
- [x] 6.6 `attach_session` ("Attach Session…") palette: rows of detached sessions (title or command, directory, age), newest first, enter attaches after the active tab, `picker_kill` (ctrl-shift-delete) kills and refreshes, an empty list says so; verify with an e2e case
- [x] 6.7 `--attach <ID>` (Linux): parse it, make it a request, open a tab attached to that session, or a message tab naming the id and the reason; verify `parse_args` tests and an e2e case
- [x] 6.8 Update README for restore (lost zones dropped, every launch restores) and reattaching, and CLAUDE.md, and verify the commands shown run as written

## 7. One instance

- [x] 7.1 `src/instance.rs`: the socket path (`$XDG_RUNTIME_DIR/brindle-instance-<hash of the state dir>.sock`, else under `~/.cache/brindle/run`), the request (launch options, working directory, `XDG_ACTIVATION_TOKEN`/`DESKTOP_STARTUP_ID`) as one JSON line, `ok`/error reply; unit-test encoding and the path
- [x] 7.2 Instance side (Linux): bind after taking `session.lock`, mode 0600, `SO_PEERCRED` same uid; accept on a background thread and hand requests to the GPUI thread; raise the window (activation token on Wayland, GPUI activation on X11) and open the request; verify with an e2e case
- [x] 7.3 Launcher side: a launch that can't take the lock and has no debug options connects (retrying up to 5 s), sends, waits for `ok`, exits 0, or prints why and exits 1; a debug launch runs standalone without restore, reattach or state writes; verify with e2e cases
- [ ] 7.4 Windows: a named pipe `\\.\pipe\brindle-instance-<hash>-<user>` with the same request, and `AllowSetForegroundWindow` for the instance's pid before sending; verify `cargo build` and `cargo test` on Windows (CI's `windows` job) and a `windows-*` e2e case for a second launch
- [x] 7.5 Update README (launching again raises Brindle; `-e`/`--attach` from a shell) and CLAUDE.md, and verify

## 8. End-to-end checks

- [x] 8.1 `local-server-close-*` cases: closing a zone or tab ends its program (process gone, nothing listed), including a reattached tab; quitting and closing the window (`WM_DELETE_WINDOW`) leave sessions detached with the same pids; an `-e` tab ends at quit; remove the `ask` cases
- [x] 8.2 `local-server-reattach-*` cases: quit and relaunch, SIGKILL and relaunch, a killed zone in a split is dropped and its sibling reattached, every session gone opens one default tab, a `--no-restore` launch leaves sessions for the next, an inactive tab's replay lands where it was, no bell from a replay
- [x] 8.3 `instance-*` cases: a second plain launch exits 0 and leaves the first with its tabs; `-e` and `--attach` from a second launch open tabs in the first; a debug run beside an instance writes no state
- [x] 8.4 `local-server-palette` and `local-server-attach-flag` cases updated for one window
- [x] 8.5 Run the whole suite and `cargo test --workspace --locked` and `cargo build --release --locked`, and verify all pass
- [ ] 8.6 Hand the manual checks to the user: a GNOME-launched Brindle survives `systemctl --user stop app-gnome-brindle-*.scope` with its sessions; launching Brindle again from GNOME and from a shell raises the running window on Wayland; the sessions palette

## Workflow follow-up

- Land through a squash-merged PR titled `feat(terminal-server)!: keep terminals across restarts, one window, single instance`.
- Archive the change and sync its specs after merge.
