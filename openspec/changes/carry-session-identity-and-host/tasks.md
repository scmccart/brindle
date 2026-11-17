# Tasks

## 1. Session identity types (pure; the contract the client and the workspace share)

- [ ] 1.1 In a new `src/remote/identity.rs`, write the unit tests first, in its `#[cfg(test)] mod tests`:
  - `an_empty_wire_key_is_no_key`;
  - `a_listed_session_decodes_its_key_and_size_once`: `0x0`, `80x0` and `0x24` give no size;
  - `a_keyed_session_is_only_the_listed_one_with_its_key`: the design's table, covering the same key, another key, a keyless listing and another id;
  - `a_keyless_session_matches_any_listing_of_its_id`.
- [ ] 1.2 Implement the types (design, decision 1):
  - `SessionKey`: a private field, `from_wire(&str) -> Option<Self>`, `as_str`, `into_string`, and no `Default`;
  - `ListedSession { session, key, attached, size: Option<(u16, u16)> }`, with `ListedSession::of(&SessionInfo)` and `ListedSession::is(session, Option<&SessionKey>)`.
  - Declare the module in `src/remote/mod.rs` with the same `cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))` as `client` and `hosts`, and re-export both types from `remote`.
  - Verify with `cargo test -p brindle remote::identity`. `cargo test -p brindle --test architecture` must still pass, since no crate reference changes.

## 2. Client model: shown sessions and carried attaches (pure methods, `src/remote/local/connection.rs`)

- [ ] 2.1 Write the `State` tests first, building a `State` by hand with `fake::sink()` sinks and no link, thread or sleep. They replace `sessions_the_core_no_longer_lists_are_missing`:
  - `a_shown_session_is_kept_only_when_listed_with_its_key_and_a_client`: kept; another key; same key with no client; not listed; a keyless listing of a keyed session;
  - `a_keyless_shown_session_is_kept_by_id`;
  - `a_pruned_session_takes_its_window_and_foreground_with_it`;
  - `showing_a_session_under_a_shown_id_returns_the_one_it_displaced`;
  - `forget_leaves_a_session_shown_under_another_key`;
  - `shows_needs_the_same_id_and_key`.
- [ ] 2.2 Implement the model (decision 2):
  - `struct Shown { sink, key: Option<SessionKey>, parent: Option<u64>, foreground: Option<Foreground> }`;
  - `State.sessions: HashMap<SessionId, Shown>`, with the `parents` and `foreground` maps removed;
  - `State::show`, `shows`, `forget(id, key)` and `keep_listed(&[ListedSession]) -> Vec<Sink>`;
  - delete `missing` and `State::remove`, and update `in_use` and `lost_link`.
  - Verify with `cargo test -p brindle remote::local::connection`.
- [ ] 2.3 Write the `carried_attaches` table tests first. Each builds a `HashMap<RequestId, Pending>` with a `Waiting::Attach` and a `Vec<ListedSession>`, then checks `Decided.again` and `Decided.gone`, including the failure texts:
  - `a_recorded_attach_goes_out_at_the_listed_size_when_listed_with_its_key_and_no_client`, which also checks that the request's size stays when no size is listed;
  - `a_recorded_attach_shown_by_another_client_fails`;
  - `a_recorded_attach_without_a_key_or_with_another_has_ended`;
  - `a_carried_attach_listed_with_its_key_and_a_client_waits_for_its_replay`;
  - `a_carried_attach_listed_with_its_key_and_no_client_is_sent_again`;
  - `a_carried_attach_whose_id_has_another_key_fails`;
  - `a_carried_attach_not_listed_fails`.
- [ ] 2.4 Implement decision 3's types and table:
  - `AttachRequest { session, key: Option<SessionKey>, when: Attaching }` with `enum Attaching { Now { cols, rows }, Recorded }`;
  - `Waiting::Attach { sink, reply, attach, key, awaits: Awaits }` with `enum Awaits { Listing, Replay }`, replacing `check`. Update `Pending::is_check` and `Connection::attach`;
  - `carried_attaches(&mut requests, &[ListedSession], name)` per the table. Leave `Pending`'s `written`, `told` and `carried` as they are.
  - Every `match` on `Attaching` and `Awaits` names every variant (`types R2`). Verify with the 2.3 tests.

## 3. Client shell: the reader's arms and `Running` (`src/remote/local/connection.rs`, `session.rs`)

- [ ] 3.1 Write the `fake::link` tests first, beside `a_cut_off_attach_the_core_didnt_give_back_is_sent_again`:
  - `a_session_whose_id_the_reconnected_core_gave_another_is_lost`: two spawned sessions. The reconnect lists one id with another key and a client, and the other with its own key. The first tab gets the lost line, and the second keeps its replay.
  - `a_cut_off_attach_whose_id_has_another_key_fails_and_is_not_sent_again`.
  - `a_lost_session_handle_never_reaches_the_session_that_reused_its_id`: a spawned session is lost after a reconnect, and a new spawn on the new link gets the same id with another key. The old `Running`'s `input`, `resize`, `close` and `detach` write nothing, the new one's entry is still shown, and the new `Running::close` writes its `CLOSE`.
- [ ] 3.2 Implement the reader's arms:
  - `SPAWNED` decodes `Spawned.key` once and calls `State::show`. A displaced entry is told `lost`. The reply carries the decoded key into `Running`.
  - `REPLAY_START` with a `req`, and `attached_by_replay`, show the entry with the waiting attach's key. `attached_by_replay` keeps matching by id, with a comment saying why (decision 3).
  - `SESSIONS` decodes `Vec<ListedSession>` once, then calls `keep_listed` (after a loss) and `carried_attaches`. A recorded attach's `replay_size` comes from `ListedSession::size`. Listing hooks still get the wire `Vec<SessionInfo>`.
  - `REPLAY_END` reads `Shown.parent`, and `PROCESS` writes `Shown.foreground`.
- [ ] 3.3 Gate `Running` (decision 4):
  - `Running.key: Option<SessionKey>`, and `Running::key() -> Option<&SessionKey>`;
  - `input`, `resize`, `reparent`, `foreground` and `set_layout` act only when `State::shows(self.id, self.key)`;
  - `close` and `detach` go through `State::forget(id, key)`, and so does `Connection::forget`, which takes the key.
- [ ] 3.4 Update the existing tests:
  - give the `attach_request` and `recorded` helpers `Attaching` and `SessionKey` values;
  - make the listings in these tests carry the keys the fake core gave (`k{id}`), as a real core does:
    - `a_lost_relay_link_says_so_and_only_unlisted_sessions_are_lost`;
    - `attaches_cut_off_with_their_link_are_answered_by_the_reconnects_replays`;
    - `a_cut_off_attach_the_core_didnt_give_back_is_sent_again`;
    - `a_recorded_attach_waits_for_a_listing_and_goes_out_at_the_listed_size`;
    - `a_recorded_attach_fails_when_its_session_ended_was_replaced_or_is_shown`.
  - Keep one keyless listing, of a session spawned without a key, so the old-core rule stays pinned. Verify that the 3.1 tests and the updated ones pass with `cargo test -p brindle remote::local`.
- [ ] 3.5 Carry the key through `Session`:
  - in `src/remote/local/session.rs`, `requested` becomes `Option<(SessionId, Option<SessionKey>)>` and `Session::id_and_key` returns the same;
  - update its tests: the `attach_request` helper (`:433`), the `Spawned` built at `:697`, and `a_pending_attach_reports_the_requested_id_and_key` and its siblings;
  - `Terminal::server_session` (`src/terminal/mod.rs:980`) returns `Option<(SessionId, Option<SessionKey>)>`;
  - `ZoneTreeView::zone_server_session` (`src/zone_view.rs:593`) writes the state file's form, `key.map(SessionKey::into_string).unwrap_or_default()` (`types R9`).
  - Verify with `cargo test -p brindle remote::local::session` and `cargo test -p brindle session`.
- [ ] 3.6 Update CLAUDE.md's "Brindle's client" paragraph:
  - the reconnect's `LIST` finds the sessions the core no longer has *with their keys*;
  - a `Running` acts only while its connection shows that session by id and key;
  - `State.sessions` holds each session's window and foreground.
  - Check that the paragraph names no removed item (`parents`, `missing`).

## 4. Workspace (`src/workspace.rs`)

- [ ] 4.1 Build attach requests from the decoded values:
  - `attached_terminal` (`:781`) uses `ListedSession::of(info)` for the key and the size, falling back to `GridSize::default()` when no size is listed. This closes b07 #23.
  - `restored_terminal` (`:803`) builds `AttachRequest { key: SessionKey::from_wire(&recorded.key), when: Attaching::Recorded, .. }`.
  - Verify with `cargo build` and `cargo test -p brindle workspace`.
- [ ] 4.2 Write the `config_editor_launch` tests first, in `workspace.rs`'s tests module and gated as the existing `spawn_request` test there is:
  - `the_config_editor_takes_a_local_default_profile`;
  - `the_config_editor_runs_here_under_the_first_local_profile_when_the_default_is_remote`;
  - `the_config_editor_runs_here_under_the_default_when_no_profile_is_local`.
  - Each test also checks that `spawn_request(&profile, &host, Some(editor), None, &config)` asks for the editor command itself, not `SpawnRequest::remote`, and that its `doing` names no endpoint.
- [ ] 4.3 Implement decision 6:
  - the pure `config_editor_launch(config, editor) -> (LaunchRequest, Host)`;
  - `launch_on(request, host, window, cx)`, holding `launch`'s body, with `launch` calling it with `Host::of(&profile)`;
  - in `launch_on`, the requested-cwd filter and `active_cwd_for` read `host` instead of `profile.remote`;
  - `open_config` (`:1748`) gathers `Config::path()` and `editor_command`, then calls `config_editor_launch` and `launch_on`.
  - Verify with the 4.2 tests, and that the existing `workspace.rs` tests still pass, including the moved-profile split test at `:2783`.

## 5. End-to-end cases

- [ ] 5.1 Add `local-server-reused-id` to `tests/e2e/cases/local_server.rs`. Register it in `cases/mod.rs`, and in `cases/win.rs` beside `local-server-holder-lost` if its kills work there as that case's do. Steps:
  1. Run one tab with `keep_detached()` and `prompt()`, and note its session id from `Server::list`.
  2. Kill the session's holder, then the core (`Server::core`).
  3. Wait for the lost line (`--wait-for "Session lost:"`).
  4. Run `--action new_tab` and wait for its prompt, and `ensure!` that the new session got the noted id. Otherwise the case proves nothing, and says so.
  5. Run `--action prev_tab`, send `echo leaked\r`, then `--action close_tab`.
  6. Send `echo alive-$((1+1))\r` to the remaining tab and `--wait-for alive-2`.
  - Pass when the dump shows one tab, the screen has `alive-2` but not `leaked`, and `brindle-remote list` shows the session attached.
  - Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- local-server-reused-id`. Where possible, also show that it fails on a build without group 3.
- [ ] 5.2 Add `reload-config-editor-local` to `tests/e2e/cases/reload.rs`. Register it in `cases/mod.rs` only, since the editor command differs on Windows.
  - Config: a profile `Away` with `remote = { type = "ssh", host = "brindle-e2e.invalid" }`, made the default with `Run::default_profile("Away")`; launched with `-p Shell`.
  - Set `VISUAL` to `sh -c 'echo editing "$0"; sleep 60'`, then run `--action open_config` and `--wait-for editing`.
  - Pass when the dump shows two tabs and the screen has `editing` followed by the run's config path. Without the fix, that tab would try to reach `Away`'s unresolvable host instead.
  - Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- reload-config-editor-local`.

## 6. Integration checks

- [ ] 6.1 Run `cargo test --workspace --locked` (it includes `tests/architecture.rs`) and `cargo build --release --locked`, and check that both pass with no new warnings.
- [ ] 6.2 Run `scripts/with-xvfb env BRINDLE_E2E=1 BRINDLE_E2E_SLOW=2 cargo test --test e2e -- local-server restore reload palette remote`, covering attach, reattach, restore, the sessions palette and the remote cases (which skip without Docker). Check that all pass, and record which remote cases ran.
- [ ] 6.3 On the PR, check that the Windows CI jobs (`windows`, which runs `cargo test`, and the advisory `windows-e2e`) pass.

## Manual checks

None. Nothing here needs real keys or a pointer: the e2e cases drive input through `--send` and `--action`.

## Workflow follow-up

- Archive the change in the implementation PR (`docs(openspec): …`), syncing the three delta specs.
- After this lands, change 9 (`bound-queues-and-contain-panics`) copies `remote-connections` "Sessions come back after a reconnect" and `terminal-emulation` "A session lost for good is reported in the tab" from the synced specs, not from their text before this change.
