# Tasks

Each group lands as its own PR, titled `refactor(<scope>): …`, in this order. Before each PR:
- run `cargo test --workspace --locked`, which includes `tests/architecture.rs`;
- run `cargo build --release --locked`;
- leave no `_ =>` arm on Brindle's own enums in a match the group rewrote (`railway R3`).

Behaviour doesn't change. Existing unit, real-process and e2e tests pass unmodified, except where a test reads a field that became an enum (design, Goals). Tests for a moved decision are written against today's code first, then the code moves (design, Decision 1).

## 1. brindle-ui: items by key, absence as Option, pure helpers

- [x] 1.1 Add `ItemKey` (`ui/src/components/mod.rs`), and a `key` on `CommandItem`, `SelectOption` and `MenuItem` that defaults to the label. Verify with a unit test that `new(label)` keys by the label and `.key(…)` overrides it.
- [x] 1.2 Key `Select`: the selection is a key, `set_options` keeps it only if an option has it, and `SelectEvent::Changed(ItemKey)`. Verify with unit tests `a_refresh_keeps_the_selection_by_key` and `a_refresh_without_the_key_clears_it` (b40 #1).
- [x] 1.3 Key `Combobox` and `Command`'s keep:
  - `ComboboxEvent::Changed(ItemKey)`;
  - `Command::set_groups(.., keep: Option<ItemKey>)`;
  - the pure `CommandRows::scroll_after -> Scroll { Top, Keep { reveal } }`.

  Verify with unit tests for the kept highlight and for each `Scroll` (b39 #2, #3, #4).
- [x] 1.4 `MenuEvent::Selected(ItemKey)` for `Menu` and `DropdownMenu`. `MenuEntry::label() -> Option<&str>`, and typeahead skips `None`. Verify with unit tests of typeahead over a separator and of the selected key (b40 #2, #3).
- [x] 1.5 `TabList`'s drag carries the trigger's `ElementId`: `TabMove { from: ElementId, to }`, and `can_drop` compares ids. Verify with a unit test of the drop target's index computation (b41 #1).
- [x] 1.6 `Combobox::panel` places only for a measured trigger, `panel_width(Option<Pixels>, min)`, and the overlay's depth is `Option<usize>`. Verify with unit tests of `panel_width(None, min)` and of an overlay missing from the stack (b39 #5, b42's smell).
- [x] 1.7 `Overlay`'s `kind: OverlayKind { Modal, NonModal { blocks_outside } }`. Verify that the gallery and the app build, and with a unit test that a modal overlay blocks outside (b42 #2).
- [x] 1.8 The pure helpers, each with a table test:
  - `LayerStack::prune` and `is_empty` (b41 #2);
  - `overlay::dismiss_target` (b42 #3);
  - `menu::page_key` and `menu::release_over` (b40 #5, #6);
  - `tab_target` without the active trigger, and `Closes` (b41 #6);
  - `EditState::run_lens` and `shows_placeholder` (b43 #2);
  - `text_input::hit_for_y` and `EditState::press` (b43 #3).

  Verify with `cargo test --locked -p brindle-ui`.
- [x] 1.9 Add the tests that are missing: `click_key` (taking the click's kind as a value if needed), `interaction`, and `tab_drag_style_comes_from_the_tokens`. Verify with `cargo test --locked -p brindle-ui components::` (b39 #1, b40 #4, b42 #1).
- [x] 1.10 Add `style::DEFAULT_REM` and `rems_at_default`, with a unit test.
- [x] 1.11 Move the owners onto the keyed events, each mapping the key back to today's index:
  - the gallery;
  - `settings_dialog`'s selects and `fonts.rs`'s combobox;
  - `workspace.rs`'s profile menu, context menu and `TabMove`.

  Verify that `cargo test --workspace --locked` passes and that the e2e cases for the palette, menus, tabs and settings pass, run as `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- palette menu tab settings`.
- [x] 1.12 Update `ui/README.md`'s component notes on keys, and `CLAUDE.md`'s `brindle-ui` paragraph. Verify that both name `ItemKey`.
- [ ] 1.13 Manual checks for the user, on `scripts/run-throwaway.sh`:
  - dragging tabs to reorder them;
  - the profile menu's press, drag onto an item and release;
  - a right-click on a background tab, whose items then act on that tab.

## 2. Untested pure code and small extractions

- [x] 2.1 Add unit tests for `fit_grid` and `wants_links`. Verify with `cargo test --locked -p brindle -- terminal_element:: terminal_view::` (b06).
- [x] 2.2 Add unit tests for `Theme::indexed`, `Color::mix`, `luminance`, and `Display` round-tripping `parse`. Verify with `cargo test --locked -p brindle theme::` (b06).
- [x] 2.3 Add unit tests for `unescape` and `terminal_font`. Verify with `cargo test --locked -p brindle -- main:: settings::` (b01 #12, b03 #9).
- [x] 2.4 `resolve_color` takes the `Theme`, not `&Terminal`. Add unit tests for it, for `term_config` and for `parser_for`, as change 5 leaves it. Verify with `cargo test --locked -p brindle terminal::` (b28 #14).
- [x] 2.5 Add unit tests for `cli.rs`'s `status_text`, `status_json`, `json_flag`, `no_args` and `unexpected`. Verify with `cargo test --locked -p brindle-remote-agent server::cli::` (b10 #9).
- [x] 2.6 Add unit tests for `wire.rs`'s `unknown_type`, `json` and `from_json`, and for `notice::Source::name`. Verify with `cargo test --locked -p brindle-remote-protocol wire::` and `cargo test --locked -p brindle-term-core notice::` (b21 #5, b30 #3).
- [x] 2.7 Move `millis` to an ungated `server/platform/wait.rs`, with tests for zero, rounding up and the cap. Verify on Linux, and on the Windows CI job (b13 #2).
- [x] 2.8 Add unit tests for `relay::no_listener` and `timed_out`. Extract `relay::login_shell` and `relay::stat_runs`, with tests. `Reached::host` maps `None` to the wire's empty field. Verify with `cargo test --locked -p brindle-remote-agent server::relay::`, and with `tests/relay.rs` unchanged (b15 `relay.rs:221-245`, `:449`, `:502`, `:560`).
- [x] 2.9 Extract `session::state_dir_from(over, platform, home) -> Option<PathBuf>`, unless change 12's task 5.3 already did. Add its table test. Verify with `cargo test --locked -p brindle session::` (b27 #10).
  - Change 12's task 5.3 did: `state_dir(&StateEnv)` takes the three values, which `StateEnv::read` gathers. This group adds the table test over every combination.
- [x] 2.10 `WaitLog` never reuses a handle. Verify with a unit test that begins, clears and begins again (b01 #17).
- [x] 2.11 Add `server/platform/wait.rs` to `fcis`'s whole-module table and to `PURE_MODULES`. Verify with `cargo test --locked -p brindle --test architecture`.

## 3. Server roles

- [ ] 3.1 Add the pure `core/policy.rs` `Lifecycle`:
  - `Lifecycle { phase, grace_until, idle_since }`, with `tick(now, Seen) -> Tick` and `should_exit(now, Ending)`;
  - table tests over startup adoption, grace, idle, stop and step-down.

  Then `Core::tick` and `should_exit` only gather and apply. Verify with `cargo test --locked -p brindle-remote-agent server::core::policy::` and `--test core` (b10 #5).
- [ ] 3.2 Add the rest of `policy.rs`, each with table tests:
  - `client_options` and `holder_protocols` (b10 #6);
  - `queue_mark` (b10 #7, which takes change 9's constants);
  - `placement` and `adoption` (b11 #4);
  - `gone_fate`, `exit_fate` and `grace_fate` (b11 #5, #6);
  - `attach_refusal`, with `Refusal`'s `Display` giving today's texts, and `returning` (b11 #7);
  - `look` and `foreground_changed` (b11 #9).

  Verify that their handlers call them, and with `cargo test --locked -p brindle-remote-agent`.
- [ ] 3.3 Make the core session's state the enums: `Life`, `Ownership`, `Option<ClientToken>` for the owner, and `Rebuild`:
  - the flags, the pid sentinel and the owner sentinel go;
  - `SET_META` still writes `owner` as a string.

  Verify with unit tests of the enums' predicates, with `Session::info` (b11 #8), and with `--test core` and `--test upgrade` unchanged (b11 #13, #14, #17).
- [ ] 3.4 Add `ClientToken::from_wire`, decoded once in `welcome`, `enum Peer` for `close_peers` and `forget_peer`, and `endpoint::Holder.build`, parsed once. Verify with unit tests in `remote-protocol`'s `endpoint.rs` and the core's `clients.rs`, and with `--test core` (b10 #10, #11, b11 #18).
- [ ] 3.5 Make `check::refusal_reason` pure, with tests. Verify with `--test manage` (b10 #8).
- [ ] 3.6 Add the tests that are missing: `kept_layout`, `command_line`, `allocate_id`'s wrap and skip of 0, and `holders.rs`'s `is_adopted` and `status`. Verify with `cargo test --locked -p brindle-remote-agent server::core::` (b11 #10).
- [ ] 3.7 Add the pure `holder/policy.rs`:
  - `spawn_refusal`, whose `Display` gives today's texts;
  - `drained` with `Output`;
  - `CoreStart::step`, taking over the backoff.

  Table tests for each. Verify with `cargo test --locked -p brindle-remote-agent server::holder::` and `--test holder` (b12 `holder.rs:731-741`, `:1000-1005`, `:1044-1050`).
- [ ] 3.8 The holder's `Farewell { Staying, Goodbye, Handover }` and its session `Life`/`Output`. `held.exit` is written only on `Life::Finished`. Verify with `--test holder` and `--test upgrade` unchanged, on Linux and on the Windows CI job (b12 `holder.rs:224-254`).
- [ ] 3.9 Add `core/policy.rs` and `holder/policy.rs` to `fcis`'s whole-module table and to `PURE_MODULES`, and name `Lifecycle`, `Life` and `Ownership` in `CLAUDE.md`'s core paragraph. Verify with `cargo test --locked -p brindle --test architecture`.

## 4. Brindle's client

- [ ] 4.1 `State.link: Link { NeverUp, Up(Negotiated), Down(Negotiated) }` replaces `connected`, `linked` and `protocol`. `SET_LAYOUT` is sent only when `sets_layout()`. Verify with unit tests of the transitions, and with the `fake::link` tests unchanged (b25 #12, #16).
- [ ] 4.2 `Pending.sent: Sent { Queued, Written, Carried }`, with `told` on `Waiting::Attach`, on change 2's `Awaits`. Verify with unit tests that only `link_lost` and recorded attaches make `Carried` (b25 #11).
- [ ] 4.3 Add the `State` methods `connect_failed`, `idle_step` (with `Idle`), `link_lost`, `route_spawned` and `route_replay_start`, and the free `retry_cap`. Each comes with table tests written against today's thread code first. Verify with `cargo test --locked -p brindle remote::local::connection::` (b25 #1–#5).
- [ ] 4.4 Add tests for `in_use`, `closes_due`, `waits_for_listing` and `Stamp`, which takes `now`. Verify with the same command (b25 #9, #10).
- [ ] 4.5 The platform `start` returns `Start { Ready, Exited, TimedOut }` on both platforms, and `start_failure` is tested on every platform. `forget` compares `Connection::id()`. Verify with `cargo test --locked -p brindle remote::local::`, and on the Windows CI job (b24 #6, #8, #9, b26 #8).
- [ ] 4.6 Add `replay_resize` in `terminal/server.rs`, with tests, and `Wire::why() -> Option<String>` over change 12's wording. Verify with `cargo test --locked -p brindle -- terminal::server:: remote::local::relay::` (b24 #10, b26 #1).
- [ ] 4.7 Run the client's end-to-end coverage. Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- local-server remote`, and with `BRINDLE_REMOTE_DOCKER_TEST=1 cargo test -p brindle --locked remote::client` after rebuilding the musl helper.

## 5. Lifecycles and the spawn request

- [ ] 5.1 `Terminal`'s `life: Life { Running, Ended, Failed }` and `origin: Origin { Spawned, Attached }`, with the pure `Life::after(LifeEvent)`. Change 9's `Broken` goes through `mark_lost`. Verify with unit tests of `after` and `shows_only_message`, and with `cargo test --locked -p brindle terminal::` (b28 #6).
- [ ] 5.2 `Session`'s `Phase { Starting(Queued), Left(Leaving), Running, Gone }` and `Queued::into_ops`. The `_ => None` arms name every variant. Verify with unit tests of the hand-over order and of a session left while starting (b26's `session.rs:31-283`).
- [ ] 5.3 `SpawnRequest.command: Option<String>`, mapped at encode, and `spawn_request` matches on it. Verify with a unit test that the encoded bytes are unchanged for `None` and `Some` (b26 #2, b28 #7).
- [ ] 5.4 `spawn_request` takes `home` and an `is_dir` probe, and its test drops the `/var/log` path. Verify with `cargo test --locked -p brindle terminal::` (b28 #4).
- [ ] 5.5 Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- local-server` that tabs that exit, are lost or fail still show today's lines.

## 6. Workspace

- [ ] 6.1 Add the pure `src/workspace/tabs.rs`, with table tests over orders of 0–4 tabs for each method:
  - `TabId` and `TabTarget`;
  - `Tabs` with `active_in`, `insert_at`, `removed`, `pick` and `shift`.

  Verify with `cargo test --locked -p brindle workspace::tabs::` (b07 #2, #3, #4, #19).
- [ ] 6.2 `Workspace` keeps `Tabs` instead of `active: usize`, and `reorder_keeping` goes. `ActivateTab(TabTarget)` is built by `last_tab`, `activate_tab_N` and alt-N. Verify with `actions::` tests and with the e2e tab cases (b00 #3, b07 #22).
- [ ] 6.3 Add `ProfileKey` with `Config::profile_key` and `find_profile`. The `NewTab` palette and the profile menu resolve their items by key. Verify with unit tests over duplicate names (b07 #19's profile half).
- [ ] 6.4 Add the pure `src/workspace/launch.rs` `new_tab_cwd`. Verify with table tests over host kinds, requested paths and pins (b07 #1).
- [ ] 6.5 Add the pure `src/workspace/overlays.rs` `OverlayKind` and `toggle`, and the shell's `overlay: Overlay`, with `MenuFor`. `ActionMenu` is keyed by action name and takes shortcuts as values. Verify with table tests of `toggle`, and with the e2e palette, menu and settings cases (b07 #17, #18, b00 #5).
- [ ] 6.6 Add the pure `src/workspace/previews.rs` `Previews` and `Settings::zoomed`. Verify with unit tests, including zoom's clamp (b07 #5, #7).
- [ ] 6.7 Move `SessionsModel` to `src/picker/sessions.rs`, with `Endpoint::answered: Option<u64>`. The kill carries `SessionRef` to the listing. Verify with the moved tests and a round-0 test (b02 #6, b07 #20 in part).
- [ ] 6.8 `TAB_BAR_HEIGHT` uses `rems_at_default`. Verify that the e2e tab-strip geometry cases pass (b07 #21).
- [ ] 6.9 Add the four `workspace/` modules and `picker/sessions.rs` to `fcis`'s whole-module table and to `PURE_MODULES`, and name the models in `CLAUDE.md`'s `Workspace` paragraph. Verify with `cargo test --locked -p brindle --test architecture`.
- [ ] 6.10 Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e` (the whole suite). Then hand group 1's manual checks to the user again.

## 7. Settings dialog

- [ ] 7.1 Add `ConfigEdit` and `Applied` in `config_edit.rs`, with `apply` calling the existing functions. Verify with one unit test per variant on a parsed document.
- [ ] 7.2 Add `ThemesState` in `settings_dialog/themes.rs`. `Model` becomes `{ section, themes, profiles, fonts, error }`, with a mode per section. Verify with the existing `settings_dialog::` tests, moved (b04 #5, #11).
- [ ] 7.3 The models take the file's text and return `Step`, and the view writes and then calls `written`. Verify with unit tests that each operation returns the expected `ConfigEdit` without touching a file (b04 #1, #2; b05 `fonts.rs:196`, `:293`, `profiles.rs:558`, `:715`, `:769`, `:778`).
- [ ] 7.4 Selection is kept by theme name and `ProfileKey`, `Target { New, Saved }` is added, and a `ProfileRef` is built only where it is written. Verify with unit tests in which a reload keeps the selection by key, and a deleted key falls back by position (b04 #7, b05 `profiles.rs:249-251`, `:520-530`, b00 #11).
- [ ] 7.5 Move the decisions out of the view:
  - `Model::enabled`, `active_row` and `focus_target`;
  - `ProfilesState::rows`, `editor_title`, `default_note` and `theme_labels`;
  - `FontsState::choice_for`.

  Verify with unit tests (b04 #3, #4; b05's `fcis R1` and `R10` sites).
- [ ] 7.6 Clean up the types:
  - `FieldId { Name, Color(usize) }`;
  - `focused_label`/`field_label -> Option` (the dump prints `""`);
  - remove `Choice`;
  - a blank profile name is `None`.

  Verify with unit tests, and that the `--- settings` dump lines are unchanged (b04 #8, #9, #10, b05 `profiles.rs:575-579`).
- [ ] 7.7 Add the tests that are missing: `start_duplicate`, `move_profile`, `theme_names`, `take_renames`, `profile_preview`, `move_selection`, `describe` in `ConfirmDelete`, and `command_groups`. Verify with `cargo test --locked -p brindle settings_dialog::` (b04 #6, b05 `profiles.rs:627`, `fonts.rs:478`).
- [ ] 7.8 Name the models in `CLAUDE.md`'s settings paragraph, and verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- settings`.

## Workflow follow-up

- Archive the change with `openspec archive extract-decisions-into-models` on group 7's PR, as its last commit: `docs(openspec): archive extract-decisions-into-models`. `skip_specs` means there is no spec sync.
