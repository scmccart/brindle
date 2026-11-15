# Tasks

## 1. Protocol fields (`remote-protocol`)

- [x] 1.1 Add `client::Detach { session, layout: Option<serde_json::Value> }` (`layout` skipped when `None`) and make `ToCore::Detach` carry it instead of `SessionRef`; update the module's message table and field notes. Verify `cargo test -p brindle-remote-protocol` passes with the existing transcript bytes for `DETACH` unchanged.
- [x] 1.2 Add `SessionInfo.layout: Option<serde_json::Value>` (skipped when `None`) and document it in the `SESSIONS` row. Verify with the same test run.
- [x] 1.3 Add `holder::SessionMeta.layout: Option<serde_json::Value>`, kept in sorted field order and read by `from_value`. Verify by extending the `SessionMeta` round-trip test with a layout, and with a malformed one read as `None`.
- [x] 1.4 Extend the client transcript samples (`samples::to_core`, `samples::to_client`) with a `DETACH` carrying a layout and a `SESSIONS` entry with one, and pin their encodings. Verify `cargo test -p brindle-remote-protocol`.

## 2. Core keeps the layout (`remote-agent`)

- [x] 2.1 In `core/sessions.rs` `client_detach`, take the `Detach` message: store `layout` in `session.meta` when its encoding is at most `MAX_LAYOUT` (64 KiB), else log and drop it, before the existing `store_meta`. In `client_attach`, clear `meta.layout` and send `store_meta` when it was set, even when the owner is unchanged. Update `clients.rs` dispatch. Verify `cargo build -p brindle-remote-agent`.
- [x] 2.2 List `meta.layout` in `core/manage.rs` `client_list`. Verify by the tests in 2.4.
- [x] 2.3 Give the test client in `remote-agent/tests/support` a `detach_with(session, layout)`, with `detach` sending `layout: None`. Verify the existing `core.rs` tests still pass.
- [x] 2.4 Add tests to `remote-agent/tests/core.rs`, and verify with `cargo test -p brindle-remote-agent --test core`:
  - a layout given with `DETACH` is listed;
  - it survives `kill_core_and_wait_for_another`;
  - an `ATTACH` clears it;
  - an oversized one is dropped while the detach still applies;
  - a `DETACH` without one lists none.

## 3. Layout schema, recording and grouping (`src/layout.rs`)

- [x] 3.1 Create `src/layout.rs` with the serde types of design decision 3: `Layout { v, quit, index, title, zones }` and `Node` (a `key`/`active` leaf or a split with `split`, `first`, `second`), plus `quit_id()`. Verify by a unit test that pins the JSON shape.
- [x] 3.2 Add `record(tree: &ZoneTree, active, key_of, quit, index, title) -> Option<serde_json::Value>`. Zones without a key are pruned and their sibling takes their place. If the active zone was pruned, the first leaf is active. It returns `None` when no zone has a key. Verify with unit tests: a split, a pruned message zone, an all-message tab.
- [x] 3.3 Add `group(sessions: Vec<SessionInfo>) -> Vec<LayoutTab>` (design decision 6) and `LayoutTab::into_tree() -> (ZoneTree, Vec<(ZoneId, SessionInfo)>, ZoneId)`. Verify with unit tests for the restore merge:
  - a full split regrouped;
  - one member missing gives a single tab;
  - the active leaf pruned makes the first leaf active;
  - a blob that doesn't name its session, or has another `v`, or isn't JSON, opens flat;
  - two quits' tabs keep their recorded order within a quit and sort by each quit's oldest session;
  - flat sessions keep start order;
  - title trimming.

## 4. Brindle sends the layout on quit

- [x] 4.1 Add `Running::detach(layout)` in `src/remote/local/connection.rs`, `Session::detach_with(layout)` in `session.rs` (a session still starting detaches without one) and `Terminal::quit_with(layout)` in `src/terminal/mod.rs`. `quit`/drop keep sending none. Verify with the existing `session.rs` quit tests, plus a new one asserting that the `ToCore::Detach` carries the layout.
- [x] 4.2 Add `ZoneTreeView::quit_all(quit, index, title, cx)` in `src/zone_view.rs`. It builds the layout with `layout::record` from `zone_server_session`, then quits every terminal with it. `Workspace::quit` makes one `layout::quit_id()` and passes each tab's index and manual title. Verify with `cargo build` and the e2e case in 6.1.

## 5. Sweeps open grouped tabs

- [x] 5.1 Add `Workspace::insert_zone_tab(profile, host, LayoutTab, activate, …)` beside `restore_zones`. It builds terminals with `attached_terminal`, uses `ZoneTreeView::from_restored` and inserts at the end without changing the active tab. A one-leaf tab uses `insert_terminal_tab`. Both set the manual title. Verify with `cargo build`.
- [x] 5.2 Make `Workspace::attach_detached` group with `layout::group`, with the profile from `reattach::local_profile` of the active leaf's session. Verify with the e2e case in 6.1.
- [x] 5.3 Make `reattach::swept_tabs` return `Vec<(LayoutTab, usize)>` (profile from the active leaf's session through `session_profile`) and `attach_swept` open them with `insert_zone_tab`. Update `swept_tabs_keep_the_sweeps_order_with_each_sessions_profile` and add a grouped case. Verify with `cargo test -p brindle reattach`.

## 6. End to end and docs

- [x] 6.1 Add the `local-server-layout` case to `tests/e2e/cases/local_server.rs` and register it in `cases/mod.rs`, then verify it under `scripts/with-xvfb` with `BRINDLE_E2E=1` after `cargo build`:
  - the first run splits a tab and quits; the listing shows both sessions with a layout;
  - a second, `--no-restore` run quits with its own tab;
  - a third, restoring run has two tabs, the second with `--- zones count=2` after `next_tab`.
- [x] 6.2 Update CLAUDE.md: the "Core" paragraph (`meta.layout`, kept on `DETACH`, cleared on `ATTACH`, 64 KiB), "Protocols" (`DETACH`'s and `SessionInfo`'s `layout`), "Brindle's client" (the quit sends each tab's layout) and "Reattach" (`layout::group` in the sweeps). Verify by reading the changed paragraphs against the code.
- [x] 6.3 Run `cargo test --workspace --locked`, `cargo build --release --locked` and the `local-server` e2e cases under `scripts/with-xvfb`, and verify all pass.

## 7. Live layout updates (protocol 3)

- [x] 7.1 In `remote-protocol/src/client.rs`, set `PROTOCOLS` to 1–3. Add `MSG_SET_LAYOUT` (14), `SetLayout { session, layout? }` and `ToCore::SetLayout`, plus the docs table row and the protocol 3 note. Add it to the roundtrip samples. Verify with `cargo test -p brindle-remote-protocol`.
- [x] 7.2 Add a `client-set-layout` transcript (a new `Protocol::Client3`) and update the negotiation tests: a 1–3 client and a 1–2 core agree on 2, and two 1–3 sides agree on 3. Verify that `BRINDLE_REGEN_FIXTURES=1` only adds the new fixture, and that `cargo test --workspace --locked --test transcripts` passes.
- [x] 7.3 In the core, keep each client's negotiated protocol (`Client.protocol` in `core/clients.rs`). Add `client_set_layout` in `core/sessions.rs`: an `ERROR` on protocol < 3, from a client that doesn't have the session attached, or for an unknown session; the 64 KiB cap; `store_meta` only when the layout changed. Verify with `cargo build -p brindle-remote-agent`.
- [x] 7.4 Add `TestClient::hello_3` and `set_layout` to `remote-agent/tests/support`. Add tests to `remote-agent/tests/core.rs`: the latest `SET_LAYOUT` is listed after its client is SIGKILLed; it survives a core restart; it is refused from another client and on protocol 2. Verify with `cargo test -p brindle-remote-agent --test core layout`.
- [x] 7.5 Brindle:
  - `Connection` keeps the link's negotiated protocol (`State.protocol`, set in `up` from each `WELCOME`).
  - `Running::set_layout` sends `SET_LAYOUT` on protocol ≥ 3, only when the layout differs from the last one sent.
  - Add `Session::set_layout`, `Terminal::set_layout`, `ZoneTreeView::layout(index, title, cx)` (shared with `quit_all`) and `Workspace::send_layouts`, called from `main.rs`'s once-a-second `record_session` tick.
  - Rename `quit_id` to `origin_id` (made once per process) and the JSON field `quit` to `origin`.

  Verify with a unit test in `session.rs` or `connection.rs` (the same layout twice sends one `SET_LAYOUT`, and protocol 2 sends none) and the existing layout tests.
- [x] 7.6 Add the `local-server-layout-live` e2e case: split a tab, wait for both shells, then SIGKILL Brindle about 3 s later. `brindle-remote list --json` must then show both sessions with one layout naming both. Register it in `cases/mod.rs` and `cases/win.rs`, and verify it under `scripts/with-xvfb`.
- [x] 7.7 Update CLAUDE.md for protocol 3, `SET_LAYOUT` and the live sends. Re-run `cargo test --workspace --locked`, `cargo build --release --locked` and the `local-server` e2e cases under `scripts/with-xvfb`, and verify all pass.

## Workflow follow-up

- Archive with spec sync as `docs(openspec): archive session-layout-in-server and sync its specs` on the implementation PR.
