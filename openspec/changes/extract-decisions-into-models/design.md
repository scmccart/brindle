# Design

## Context

See proposal.md (Why). The sources:
- **The audit:** `docs/architecture/audits/2026-11-16.md`, findings F6, F10 and F12.
- **The plan:** `2026-11-16-remediation.md`, change 13, and its "Adjusted while writing the changes".
- **Entry citations:** "bNN #K" is item K under Violations in `2026-11-16/bNN.md`. b05, b06, b12, b15 and b23 number nothing, so their entries are cited by `file:line`.
- **Code:** unchanged since the audit (`ab6462b`). `git diff ab6462b HEAD` touches no source file.

### What the sibling changes already take

These entries of F6, F10 and F12 are closed elsewhere and are not repeated here:

| Change | Entries |
| --- | --- |
| 2 `carry-session-identity-and-host` | b07 #23 and #24; b07 #6, which is the same `attached_terminal` size check that `ListedSession::size` replaces; b25 #8, by its table tests for `carried_attaches`; b25 #13–#15; b26 #3 (`id_and_key` returns `Option<SessionKey>`) |
| 7 `keep-pure-modules-pure` | b03 #7, #8; b07 #8, #9, #11, #12; `palette_commands`, half of b00 #5; b11 #3; b20 #2; b23's two `fcis R3` entries (`config.rs:285`, `instance.rs:57`); b24 #12; b25 #6, #7, and `give_up_closes`' test, part of b25 #9; b27 #8, #9, #11; b28 #1 |
| 8 `list-sessions-off-ui-thread` | b01 #8 (`startup_launch`) and b01 #15 (`reattach::Listing`). Change 8 doesn't cite them, but its two types replace both sites |
| 9 `bound-queues-and-contain-panics` | b12's duplicated backpressure policy (`holder.rs:874-880`, `:1091-1095`) |
| 10 `test-platform-logic-everywhere` | b07 #10; b13 #2's `win32`; b15 `pty.rs:257-317` (which processes a hang-up ends), `pty.rs:374-426` (`environment`, `plain`); b21 #1; b23 `instance.rs:124-130, 257-264` |
| 12 `surface-swallowed-failures` | b01 #10: its `profile_for(config, name) -> Chosen` is the one rule for both launch paths |
| D3 (change 7's doc text) | b34 #5–#8: e2e harness helpers, held by `--self-test`, outside `fcis R8` |

Otherwise changes 11 (`deterministic-tests`) and 12 (`surface-swallowed-failures`) take `testing` and `railway` entries only. Where change 12 edits the same functions, the overlap is named in Decision 2. Change 11 leaves b01 #11 (`parse_args_from`, `fcis R9`) to "change 12 or 13". Change 12 moves the steps' checks into it but keeps its exits, so it stays on the list at the end.

### What the sibling changes hand to this one

| From | Entry | Here |
| --- | --- | --- |
| 2 | b07 #20 (the sessions palette kills by bare id) | group 6, as far as a refactor can go (Decision 9) |
| 2 | b25 #11 (`Pending`'s flags), b25 #16 (`protocol: u32`) | group 4 |
| 7 | b04 #1, b05 `fonts.rs:196`, `profiles.rs:558` (the dialog's models read the file) | group 7 |
| 7 | b27 #10 (`session::state_dir`) | group 2 |
| 7 | splitting `SessionsModel` into a whole pure module | group 6, which edits it anyway. `pace::step` stays where it is, because nothing here changes `pace.rs`. |
| 9 | b10 #7 (the core's client-queue hysteresis) | group 3 |
| 9 | `Terminal`'s flags, with the `Broken` arm change 9 adds | group 5 |
| 10 | b13 #2's `millis` | group 2 |
| 10 | the relay's login shell (b15 `relay.rs:221-245`) | group 2 |
| 10 | b02 #2, #3 (notify's retry and toast ids); b14 #6 (`ReadState`'s flags) | left, with the reason in the list at the end |

### Corrections to the plan

- **Group 5's batches:**
  - `Terminal`'s flags are b28 #6, not b23;
  - `Session`'s are the `Phase` state machine in b26's design smells (`session.rs:31-283`);
  - b25 has no `session.rs` entry;
  - the sentinel pid the audit names is the core's (b11 #13), which is group 3's.
- **Group 3's list:** "b25 #1–#5, #8–#12, #16" includes b25 #8, which change 2 closes.
- **The non-session `types R3` entries** the plan's Closes line names. Four of them don't fit a refactor:
  - **b01 #16:** the layout blob's tab identity. It needs a new field in the layout JSON (`types R9`, `process R15`), which is a format change;
  - **b03 #16:** the audit flags it only as a risk. Each returned index is used inside the call that computed it, which `types R3` allows;
  - **b27 #18:** `WindowState::active` is the persisted form, which `types R9` allows. Its in-memory fix comes with b27 #17's tree enum.
  - **b07 #20:** see the Proposal and Decision 9.

  All four are on the list at the end.
- **The order of the groups** differs from the plan's list (Decision 2).

## Goals / Non-Goals

**Goals:**
- **Each hotspot's decisions live in a pure function or a GPUI-free model** with unit tests in its own file (`fcis R1`, `R8`, `R10`, `R9`). The view, entity, thread or handler around it gathers, calls and applies, and dispatches on a closed enum.
- **The state those decisions read is one enum per lifecycle** (`types R1`). There is no `0` or `""` for "none" past the point where a value is decoded (`types R7`, `R9`), and no position is kept across a refresh (`types R3`).
- **Behaviour is unchanged**, including texts, log lines, focus moves and wire bytes. Existing unit, real-process and e2e tests pass unmodified, except where a test reads a field that became an enum.
- **Every new whole pure module** is listed in `fcis`'s table and in `PURE_MODULES`, so change 7's check holds it.

**Non-Goals:**
- **The entries listed at the end.** They follow the README's adoption policy.
- **`terminal_view.rs` and `terminal_element.rs` decisions:** `add-blocks` rewrites those handlers (proposal).
- **Any wire, protocol-version, config-key, state-file or layout-blob change** (`process R4`, `R5`, `R15`). Sentinels in wire structs stay. They become `Option`s or enums where they are decoded (`types R9`).
- **New failure reporting.** Change 12 owns that. The models carry its values and don't add new ones.

## Layers

| Part | Layer | Crate and file |
| --- | --- | --- |
| `ItemKey`; the keyed `Select`, `Combobox`, `Command` keep, `DropdownMenu`/`Menu` events and `TabMove`; `OverlayKind`; `LayerStack::prune`; pure helpers in `menu.rs`, `tabs.rs`, `overlay.rs` and `text_input.rs`'s `EditState` | pure items in a shared crate | `brindle-ui` |
| the components' entities (render, events, focus) | shell | `brindle-ui` |
| `fit_grid`, `wants_links`, `Theme::indexed`, `unescape`, `terminal_font`, `resolve_color`, `term_config`, `parser_for`, `state_dir_from`, `WaitLog` | pure items, tests only or a small extraction | `brindle` |
| `millis` | pure, a new whole module, `server/platform/wait.rs` (ungated) | `brindle-remote-agent` |
| `cli.rs` status helpers; `relay::{login_shell, stat_runs, no_listener, timed_out}` | pure items | `brindle-remote-agent` |
| `wire.rs` helpers | pure items | `brindle-remote-protocol` |
| `notice::Source::name` | pure | `brindle-term-core` |
| `core/policy.rs` (`Lifecycle`, `client_options`, `queue_mark`, `placement`, `adoption`, `gone_fate`, `exit_fate`, `grace_fate`, `attach_refusal`, `returning`, `look`) | pure, a new whole module | `brindle-remote-agent` |
| `holder/policy.rs` (`spawn_refusal`, `drained`, `CoreStart`) | pure, a new whole module | `brindle-remote-agent` |
| `Life`, `Ownership`, `Rebuild`, `Session::info` (core); `Life`, `Farewell` (holder); `ClientToken`, `Peer`; `check::refusal_reason` | state types and pure methods inside shell modules | `brindle-remote-agent` |
| the core's and holder's loop handlers | shell: gather, call, apply | `brindle-remote-agent` |
| `endpoint::Holder.build` | parsed once at the boundary | `brindle-remote-protocol` |
| `State` methods, `Sent`, `Link`, `Negotiated`, `retry_cap`, `Stamp` | GPUI-free model, inside a shell module | `brindle`, `src/remote/local/connection.rs` |
| `Start`, `start_failure`; `replay_resize` | outcome type and pure functions | `brindle`, `src/remote/local.rs`, `src/terminal/server.rs` |
| the platform `start` | seam: returns `Start` on both platforms | `src/remote/local/{unix,windows}.rs` |
| `Life`, `Origin` (`Terminal`); `Phase`, `Queued`, `Op` (`Session`); `SpawnRequest.command: Option` | state types and pure methods | `brindle`, `src/terminal/mod.rs`, `src/remote/local/session.rs` |
| `Tabs`, `TabId`, `TabTarget`; `new_tab_cwd`; `OverlayKind`, `toggle`; `Previews`; `Settings::zoomed`; `ProfileKey` | pure, new whole modules `src/workspace/{tabs,launch,overlays,previews}.rs`, plus items in `settings.rs` and `config.rs` | `brindle` |
| `SessionsModel` | pure, moved to the whole module `src/picker/sessions.rs` | `brindle` |
| `Overlay` (what is open, with its view-side data) | shell state of `Workspace` | `brindle` |
| `ConfigEdit`, `Applied` | pure (document edits) | `brindle`, `src/config_edit.rs` |
| `ThemesState`, `ProfilesState`, `FontsState`, `Model`, `Step`, `FieldId`, `FocusMove` | GPUI-free models | `brindle`, `src/settings_dialog/{themes,profiles,fonts}.rs`, `src/settings_dialog.rs` |
| `SettingsDialog`: reads the file, writes `ConfigEdit`s, dispatches on `Step` | shell (entity) | `brindle`, `src/settings_dialog.rs` |

- **No decision moves into a view, a thread or a seam:** every row that is shell only gathers and applies.
- **No crate reference changes direction.** `brindle-ui` still knows nothing of the app: `ItemKey` is an opaque string the owner gives. The agent still reaches the protocol only through `brindle-remote-protocol`.
- **No new `cfg`:** `millis` leaves its Windows-only file for an ungated one, which is `fcis R7`'s direction.

## Decisions

### 1. One shape for every group

- **Where things go.** A decision becomes a function, or a method of a model, in the same module as its caller. When it is a whole set, it goes in a new whole pure module beside the caller (`core/policy.rs`, `workspace/tabs.rs`); a Rust 2018 path module (`holder.rs` plus `holder/policy.rs`) needs no file move.
- **Plain values in and out** (`fcis R5`):
  - `now` is an argument (`fcis R2`);
  - a path check is a probe, as change 7 defines it (`fcis R5`, D5);
  - entity handles, `Token`s and `Sink`s stay in the shell. Where a decision picks one, it returns an index or an id the shell resolves.
- **Closed enums, matched with every variant named** (`types R2`). The `_ =>` arms on Brindle's own enums in a match this change rewrites are removed, under the `railway R3` exception's "when next changed".
- **Tests** go in the same file's `mod tests`: tables over values, no window, server, clock or sleep (`fcis R8`, `testing R1`).
  - **Before the move:** a function moved out of a shell gets its tests from what the shell does today, so the move is checked against the old code.
  - **After the move:** the existing real-process and e2e tests still cover the wiring.
- **Texts:** each enum that replaces a string decision gets a `Display` giving today's text byte for byte. Examples are the holder's spawn refusals and the core's attach refusals.

### 2. Group order and dependencies

The groups follow `openspec/config.yaml`'s task order. Group 1 is the shared contract: the keyed `brindle-ui` events, which groups 6 and 7 use. Group 2 is pure code with its tests. Groups 3 to 5 are the server, the client and the entities. Groups 6 and 7 are the views. This differs from the plan's list, which started with `workspace.rs`. The plan's order would have the views adopt the keyed events twice.

| Group | Lands after | Overlaps |
| --- | --- | --- |
| 1 `brindle-ui` | — | change 3 orders `range_from_utf16` in `text_input.rs`; change 12 edits `icon.rs`. Both are textual only. |
| 2 Untested pure code | change 5 (`parser_for` takes the host's path style); change 10 (`event_loop.rs` keeps only seams). Change 12's `state_dir` outcome may land before or after (Decision 4) | `port-to-macos` edits `main.rs`/`settings.rs` |
| 3 Server roles | changes 7 (`now` in `Session`), 9 (holder fault handling, the client-queue marks' constants), 3 (the `Assembler` cap) and 12 (`SessionMeta::from_value`'s refusal: the core keeps such a session `Ownership::Detached`) | change 10's `pty/stat.rs` |
| 4 Brindle's client | changes 2 (`Shown`, `Awaits`, `SessionKey`), 7 (`closing`/`give_up_closes` take `now`, `RelayAccount`), 9 (the outgoing byte budget) and 12 (the lost line's cause, the relay's exit reading) | — |
| 5 Lifecycles | changes 2 (`id_and_key`), 7 (`working_directory` takes `home`, the probe rule), 9 (`Broken`) and 12 (the tab's notice line) | `add-blocks` edits `terminal/mod.rs`'s view position |
| 6 `Workspace` | group 1; changes 2 (`launch_on`, `ListedSession`), 7 (`profile_rows` and the title helpers take values), 8 (the launch's `LaunchRequest`) and 12 (the listing and kill failure rows, ctrl-alt-N's message tab) | — |
| 7 Settings dialog | group 1, group 6 (`ProfileKey`), change 12 (the section's read errors) | `update-dependencies`' `toml` bump |

### 3. Group 1: `brindle-ui`

**`ItemKey`** (`ui/src/components/mod.rs`):
- a newtype over `SharedString`, given by the owner for each item;
- `CommandItem`, `SelectOption` and `MenuItem` gain `key: ItemKey`, and their `new(label)` defaults it to the label;
- it is the item's identity across a refresh, as shadcn's `value` is (`types R3`).

| Component | Today | After | Entry |
| --- | --- | --- | --- |
| `Select` | `selection: usize` kept by position through `set_options`; `SelectEvent::Changed(usize)` | the selection is the chosen option's key, resolved to an index per render; `set_options` keeps it only if an option has that key. `SelectEvent::Changed(ItemKey)` | b40 #1 |
| `Combobox` | `selected: Option<usize>`, `ComboboxEvent::Changed(usize)`; the highlight is re-found by label in `sync_panel` | keyed the same way. `sync_panel` keeps the highlight by key through `Command::set_groups(.., keep: Option<ItemKey>)` | b39 #4, b39 #3 |
| `Command` | `set_groups(.., keep: Option<usize>)` | `keep: Option<ItemKey>`. A pure `CommandRows::scroll_after(keep_found: bool) -> Scroll` with `enum Scroll { Top, Keep { reveal: usize } }` decides the scroll (b39 #2) | b39 #2 |
| `DropdownMenu`/`Menu` | `MenuEvent::Selected(usize)`, an index into the entries the menu opened with, which the owner may have replaced since | `MenuEvent::Selected(ItemKey)`. The owner resolves the key against its current items | b40 #2 |
| `TabList` | `DraggedTab.index: usize` from the start of the drag; `TabMove { from: usize, to }` | `DraggedTab.trigger: ElementId`, the trigger's own id. `TabMove { from: ElementId, to: usize }`, where `to` is computed in the drop handler that sends it. `can_drop` compares ids | b41 #1 |

`CommandEvent::Confirmed(usize)` stays: the palette handles it in the same update that computed it. It is not an audit entry.

**`Option`s for absence** (`types R7`):
- **b39 #5:** `Combobox::panel` places the panel only when `self.trigger` is `Some`, and `panel_width(Option<Pixels>)`. An unmeasured trigger waits a frame, which is what it does today in practice.
- **b40 #3:** `MenuEntry::label() -> Option<&str>`. Typeahead skips `None`.
- **b42 smell:** the overlay's depth is `Option<usize>`, and an overlay not in the stack is drawn unlifted at the base. Today `unwrap_or(0)` does the same, by accident.

**Kinds** (`types R1`):
- **b42 #2:** `Overlay`'s `modal: bool` and `blocks_outside: bool` become `kind: OverlayKind { Modal, NonModal { blocks_outside: bool } }`. `block_outside_pointer()` exists only on a non-modal builder.

**Decisions out of the entities**, each a pure function with a table test:
- `LayerStack::prune(&mut self, live: impl Fn(LayerId) -> bool) -> Vec<LayerId>`, and `WindowLayers::remove`'s empty-stack rule as `LayerStack::is_empty` (b41 #2);
- `overlay::dismiss_target(focused_inside: bool) -> DismissTarget { Focused, Own }` (b42 #3);
- `menu::page_key(key, modifiers) -> Option<MenuNav>` (b40 #5) and `menu::release_over(item: Option<usize>, inside: bool) -> ReleaseOver` (b40 #6);
- `tabs::tab_target` returns `None` for the active trigger, and `trigger_press` takes `closable && !disabled` as one `Closes` value (b41 #6);
- `EditState::run_lens(&self) -> Vec<(usize, Underline)>` and `EditState::shows_placeholder()` (b43 #2);
- `text_input::hit_for_y(y, top, bottom) -> Hit { Start, End, Line }` and `EditState::press(offset, clicks, extend)` (b43 #3).

**Tests with none today** (`fcis R8`):
- `click_key`, taking the click's kind as a plain value if `ClickEvent` can't be built without a window (b39 #1);
- `components::interaction` in `mod.rs` (b40 #4);
- `tab_drag_style_comes_from_the_tokens` (b42 #1).

**One named conversion** (`types R5`, for b07 #21's use in group 6): `style::DEFAULT_REM: Pixels` and `const fn style::rems_at_default(Rems) -> Pixels`. `TAB_BAR_HEIGHT` uses it instead of `* 16.0`.

**Owners:** the gallery, and the app's uses, take the keyed events in this group, mapping each key back to the index they use today (`settings_dialog`'s selects, `fonts.rs`'s combobox, `workspace.rs`'s two menus and `TabMove`). So the app builds and behaves as before. Groups 6 and 7 then key their own items properly.

### 4. Group 2: untested pure code, and four small extractions

Tests (`fcis R8`), each in the function's own file:

| Function | Cases | Entry |
| --- | --- | --- |
| `terminal_element::fit_grid` | the `max(2)` floor, padding on both sides, rounding down | b06 |
| `terminal_view::wants_links` | the four combinations of control and alt | b06 |
| `Theme::indexed`, `Color::mix`, `Color::luminance`, `Display for Color` | cube corners 16, 21, 231; ramp ends 232, 255; `mix` at 0, 0.5, 1; luminance of black and white; `Display` round-trips `parse` | b06 |
| `main::unescape` | `\r`, `\n`, `\e`, `\t`, a lone backslash, other text unchanged | b01 #12 |
| `settings::terminal_font` | the fallbacks are `FALLBACK_FAMILIES` that are installed, minus the family itself | b03 #9 |
| `terminal::resolve_color`, `term_config`, `parser_for` | `resolve_color` takes the `Theme` (all `default_color` reads) instead of `&Terminal`: spec, named and indexed colours with and without a palette override. `term_config`: the `osc52` choice and scrollback. `parser_for`: local and remote path styles, as change 5 leaves it | b28 #14 |
| `cli::{status_text, status_json, json_flag, no_args, unexpected}` | `status_json` drops `req` and adds `running`; `status_text` shows `, draining`; `json_flag` refuses extra arguments | b10 #9 |
| `wire::{unknown_type, json, from_json}` | an unknown field ignored, a missing optional defaulted, malformed JSON giving `InvalidData` naming the message, the `unknown_type` text | b21 #5 |
| `notice::Source::name` | the four names (config and log keys) | b30 #3 |
| `millis`, moved ungated to `server/platform/wait.rs` | zero, sub-millisecond rounding up, the cap at `INFINITE - 1` | b13 #2 (`fcis R8`, and `R7` for its gate) |
| `relay::{no_listener, timed_out}` | each `ErrorKind` they accept and one they don't | b15 `relay.rs:449-454, 502-507` |

Extractions:
- **b27 #10:** `session::state_dir_from(over: Option<OsString>, platform: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf>`:
  - a non-empty variable wins;
  - else the platform's directory, else `<home>/.local/state`, joined with `brindle`;
  - `None` when there is none, which is change 12's outcome for `state_dir` (b27 #13). Until change 12 lands, the caller maps `None` to `./brindle`, as today.

  `state_dir()` gathers the three values. The `cfg!(windows)` choice of `dirs` function, and passing no home on Windows, stay in the gathering, as today. Tests cover each case.

  Change 12's task 5.3 expects this injection "as change 7 shapes it", but change 7 hands b27 #10 here. Whichever of the two lands first does the extraction, and the other builds on it.
- **b15 `relay.rs:221-245`:** `relay::login_shell(shell_var: Option<String>, passwd_shell: Option<String>) -> Option<String>`. A non-empty `SHELL` wins, else the passwd shell. `Reached::host` reads both once per relay start and maps `None` to the wire's empty string where it builds `RelayHost` (`types R9`).
- **b15 `relay.rs:560-568`:** `relay::stat_runs(stat: &str) -> bool`, tested on `R`, `S`, `Z`, `X` and lines with no `)` or nothing after it. `runs(pid)` reads the file and calls it.
- **b01 #17:** `WaitLog` keeps a `next: u64` and never reuses a handle. Its test clears the last entry and begins again.

### 5. Group 3: server roles

**`core/policy.rs`** (new, pure). `now` is always an argument.

| Item | Takes over | Returns | Entry |
| --- | --- | --- | --- |
| `Lifecycle { phase: Phase, grace_until: Option<Instant>, idle_since: Option<Instant> }`, `enum Phase { Adopting, Serving }` | `Core::accepting`, `grace_until`, `idle_since` | `tick(&mut self, now, seen: Seen) -> Tick`, where `Seen { startup_adopted, awaiting_owner, busy }` are what the core gathers and `Tick { accept: Accept, end_grace: bool }` with `enum Accept { No, Now { grace: bool } }`. Then `should_exit(&self, now, ending: Ending) -> bool`, `enum Ending { No, Stopping { sessions_left: bool }, SteppingDown }` | b10 #5 |
| `client_options(protocol, &Hello) -> ClientOptions { linger, process_info }`, `holder_protocols(..) -> Vec<u32>` (sorted, deduplicated) | the `welcome` arithmetic | — | b10 #6 |
| `queue_mark(over: bool, queued: usize) -> Option<Mark>`, `enum Mark { Pause, Resume }` | the hysteresis in `send_client` and `flush_client` | — | b10 #7 |
| `placement(holders: &[HolderView], build, starting: bool) -> Placement { Spawn(usize), Start, Wait }` and `adoption(holder_build, core_build) -> Adoption { Keep, Drain }` | `spawn_holder`, `place_waiting`, `adopt`'s `drain` | the index is into `holders` | b11 #4 |
| `gone_fate(policy, starting, linger) -> Fate { HangUp, Linger(Duration), Keep }` | `client_gone`'s match | — | b11 #5 |
| `exit_fate(&Ownership) -> ExitFate { KeepForOwner, Report }` and `grace_fate(policy, &Life) -> GraceFate { Forget, HangUp, LeaveDetached }` | `session_exited`, `end_grace` | — | b11 #6 |
| `attach_refusal(Option<SessionView>, client) -> Option<Refusal>`, `enum Refusal` with `Display` giving today's texts; `returning(&[(SessionId, &Ownership)], &ClientToken) -> Vec<SessionId>` | `client_attach`'s chain, `reattach`'s filter | — | b11 #7 |
| `look(active: bool, active_at, now) -> Look { Look, Keep, Skip }` and `foreground_changed(last: Option<&Foreground>, seen: &Foreground) -> bool` | `look_at_processes`' settle and compare | — | b11 #9 |

**Session state** (`core/sessions.rs`, `types R1`, b11 #17):
- **`Life`:** `Starting(client::Spawn)`, `Running { pid: NonZeroU32 }`, `HungUp { pid }` and `ExitedAwaitingOwner(Option<i32>)`. It replaces `spawning`, `pid` (0 until reported, b11 #13), `hung_up` and `exited: Option<Option<i32>>`.
- **`Ownership`:** `Attached(Token)`, `Lingering { until, since }`, `AwaitingOwner { since }` and `Detached { since }`. It replaces `attached`, `awaiting_owner`, `linger_until` and `detached_since`. The owner's token is `Option<ClientToken>` beside it (b11 #14).
- **`Rebuild`:** `Ready` or `Pending { truncated }`. It replaces `rebuilding: Option<bool>`.
- **What stays:** `paused`, `storing` and `pending_replay`, which are independent facts.
- **Methods:** `is_running`, `reportable` and `checkpoint`'s bail-outs become `Life`'s and `Ownership`'s methods. `Session::info(&self, id, now) -> SessionInfo` builds the `LIST` row (b11 #8).
- **Ruled out:** a session both hung up and awaiting its spawn, an exit kept while a client is attached, and a pid read before it was reported.

**Smaller types:**
- `ClientToken::from_wire(&str) -> Option<ClientToken>`, decoded once in `welcome` (b10 #11);
- `enum Peer { Holder(Token), Client(Token) }` for `close_peers` and `forget_peer` (b10 #10);
- `endpoint::Holder.build: String`, parsed from the lock's name once in `remote-protocol`, so `adopt` compares builds and never re-parses the file name (b11 #18).

**`check::refusal_reason(wanted, supported, version, build) -> Option<String>`** is pure. `refusal` reads the build and logs (b10 #8).

**`holder/policy.rs`** (new, pure):
- **`spawn_refusal`:** `spawn_refusal(draining, exists, &Spawn) -> Option<SpawnRefusal>`, whose `Display` gives today's four texts (b12 `holder.rs:731-741`).
- **`drained`:** `drained(now, reaped_at, last_output, output: Output) -> bool`, with `enum Output { Open, Closed }` (b12 `holder.rs:1000-1005`).
- **`CoreStart`:** `CoreStart { next: Option<Instant>, backoff: Duration }` takes over `next_core_start` and `backoff`. `CoreStart::step(&mut self, now, needed: bool) -> bool` says whether to start a core now. It applies the grace and then doubles the backoff up to the cap (b12 `holder.rs:1044-1050`).

**The holder's state** (`types R1`):
- `Core`'s `handover` and `goodbye` become `farewell: Farewell { Staying, Goodbye, Handover }`, which rules out a handover without a goodbye (b12 `holder.rs:244-254`);
- `Session`'s `output_closed`, `reaped` and the moment `held.exit` is set become `life: Life { Running, Reaped { code: Option<i32>, at: Instant }, Finished }` and `output: Output` (b12 `holder.rs:224-242`);
- `pty: Option<Pty>` stays the resource. `held` stays the wire entry, and its `exit` is written only on `Life::Finished`.

**Tests:**
- `policy.rs` and `holder/policy.rs`: a table per function;
- `sessions.rs`: `Session::info`, `kept_layout`, `command_line`, `allocate_id`'s wrap and its skip of 0, and `Life`'s and `Ownership`'s predicates;
- `holders.rs`: `is_adopted` and `status`.

These close b11 #10's three entries. The real-process suites in `remote-agent/tests/` run unchanged.

**Restart, crash, upgrade** (`process R1`–`R3`, `R12`):
- Nothing persisted or sent changes. `SET_META` still carries `owner` as a string, built from `Option<ClientToken>` where the message is made.
- **A core restart:** the new core builds `Life`, `Ownership` and `Rebuild` from the holder's `WELCOME` and meta, as it sets the flags today. An adopted session is `AwaitingOwner` or `Detached` (a malformed meta, as change 12 leaves it, is `Detached`), and `Rebuild::Pending` until its checkpoint and tail are fed.
- **A Brindle crash:** it touches none of this state.
- **An upgrade:** step-down, checkpoint and `GOODBYE { handover: true }` behave as today. The holder's `Farewell::Handover` is the old `(goodbye, handover) = (true, true)`.
- **Mixed builds:** a holder or core of another build meets the same H1 messages.

### 6. Group 4: Brindle's client

This group builds on change 2's `Shown`, `Awaits` and `SessionKey`, and on change 7's `now` arguments.

**`State`'s link and requests:**
- `connected`, `linked` and `protocol: u32` (0 before the first link) become `link: Link { NeverUp, Up(Negotiated), Down(Negotiated) }` (b25 #12, #16):
  - `Negotiated(u32)` has `sets_layout()` for protocol 3 or later;
  - `Link::protocol() -> Option<Negotiated>` gives the current or the last link's;
  - this rules out "linked but never connected" and a version of 0.
- `Pending`'s `written`, `carried` and `told` become `sent: Sent { Queued, Written, Carried }`, and `told: bool` moves onto `Waiting::Attach` (b25 #11):
  - `Carried` means "the next `LIST` decides": an attach carried over a lost link, or a recorded one;
  - only `State::link_lost` and the recorded-attach constructor make it.

**`State` methods** (pure, with tests over `State::default()` and `fake::sink()`):

| Method | From | Returns |
| --- | --- | --- |
| `connect_failed(&mut self, refused, eager, message) -> ConnectFailed` | `failed_to_connect` (b25 #1) | `{ failed: Vec<Pending>, told: Vec<Sink>, next: Next }`, with `enum Next { Retry, End }`. On `End` it sets `ended` under the same lock, as today |
| `idle_step(&mut self, now, eager, idle, close_limit) -> IdleStep` | `end_when_unused`'s loop (b25 #2) | `{ given_up: Vec<SessionId>, then: Then }`, with `enum Then { Wait(Duration), End(String) }`. `idle_since` and the loop's local `given_up` become `idle: Idle { InUse, Since { at, given_up: Option<String> } }` |
| `link_lost(&mut self) -> Vec<Pending>` | `disconnected` (b25 #3) | the requests that fail; attaches become `Carried` |
| `retry_cap(eager, nothing_waits, &Timings) -> Duration`, a free function | `Connection::retry_cap` (b25 #4) | — |
| `route_spawned(&mut self, Spawned) -> Routed` and `route_replay_start(&mut self, req, session) -> Routed` | `handle`'s `SPAWNED` and `REPLAY_START` arms (b25 #5) | `enum Routed { Shown { sink, reply }, Close(SessionId), Detach(SessionId) }`, plus the request to fail with today's text |

The threads keep only the lock, the clock, the waits, the sends and the sink calls.

**Tests with none today** (`fcis R8`):
- `in_use`, `closes_due` and `waits_for_listing`. `give_up_closes` already has one from change 7 (b25 #9).
- `Stamp`: `touch`, `hold` and `deadline` take `now`, and the tests cover the `HELD` case and the `try_from` fallback (b25 #10).

**Around the connection:**
- **b24 #6:** `local.rs`'s `forget` compares `Connection::id()`, not `Arc::as_ptr` as `usize`.
- **b24 #8 and b26 #8:** the platform `start` returns `enum Start { Ready, Exited { status, stderr }, TimedOut { stderr } }`, the same item on both platforms. This replaces `ready` and `signalled`.
- **b24 #9:** `start_failure(&Start, limit) -> Option<String>` gives today's three texts, tested on every platform.
- **b24 #10:** `server.rs`'s `replay_resize(own, recorded: Option<GridSize>) -> Option<GridSize>`, with no `cols > 0` test. It takes change 2's decoded size.
- **b26 #1:** `Wire::why() -> Option<String>`. The reader builds change 7's `RelayAccount::Said` only for `Some`. Change 12 rewords `why` for a relay exit it couldn't read (`Ended::Unread`). Group 4 lands after it and keeps that wording, returning `None` only where no account exists.

### 7. Group 5: lifecycles and the spawn request

**`Terminal`** (b28 #6):
- **The new state:** `exited`, `attaching` and `failed` become `life: Life { Running, Ended, Failed }` and `origin: Origin { Spawned, Attached }`. `replaying` stays: it is independent.
- **The transitions:** a pure `Life::after(self, LifeEvent) -> Life`, with `enum LifeEvent { Exited, Lost, Failed }`:
  - `Failed` is final;
  - `Lost` after `Exited` stays `Ended`;
  - change 9's `Broken` arm goes through `mark_lost`, so it gives `Ended`.
- **What reads it:**
  - `shows_only_message()` is `origin == Attached && life == Failed`;
  - `set_layout` and `server_session` test `life == Running`.
- **What it rules out:** "failed but not exited".

**`Session`** (b26 design smell, `session.rs:31-283`):
- **The phases:** `Phase` becomes `Starting(Queued)`, `Left(Leaving)`, `Running(Running)` and `Gone`, where `Queued { input, resize, parent }`. Today "left while starting" is `Starting { left: Some(_) }`, and its input is already cleared.
- **The hand-over:** `Queued::into_ops(self) -> Vec<Op>`, with `enum Op { Resize(Size), Reparent(u64), Input(Vec<u8>) }`, is tested for today's order: resize, then reparent, then input.
- **`run`:** it matches `Left(how)` to close or detach the new session.
- **The `_ => None` arms on `Phase`** (`session.rs:131`, `:151`) name every variant.

**The spawn request:**
- **b26 #2:** `SpawnRequest.command` is `Option<String>`. The relay sets `None` for "the login shell", and the encode maps `None` to the wire's empty field (`types R9`).
- **b28 #7:** `spawn_request`'s "open a shell on …" text matches on that `Option`.
- **b28 #4:** `spawn_request(.., home: Option<PathBuf>, is_dir: impl Fn(&Path) -> bool)` takes the home directory and a probe (change 7's `fcis R5` probe), and `spawn` passes `dirs::home_dir()` and `Path::is_dir`. Its test stops depending on `/var/log/brindle-nowhere` being absent.

### 8. Group 6: `Workspace`

**`workspace/tabs.rs`** (new, pure) holds `TabId`, `TabTarget` and `Tabs`.

`TabId(u64)` replaces `Tab::id: u64` (`types R4`). The counter in `Tab::new` still mints it.

`Tabs { active: Option<TabId> }` takes over `Workspace::active: usize` (b07 #19, `types R3`). Its methods take the strip's order as `&[TabId]`, gathered from `self.tabs` per call. `Workspace` keeps `tabs: Vec<Tab>` as the one order, so nothing is kept twice.

| Method | Replaces | Entry |
| --- | --- | --- |
| `active_in(&self, order) -> Option<usize>` | every `self.tabs.get(self.active)` | b07 #19 |
| `insert_at(&self, order, requested: Option<usize>) -> usize` | `insert_tab_inactive`'s arithmetic (after the active tab; 0 when empty; clamped). The active tab needs no shift: it is an id | b07 #2 |
| `removed(&mut self, order_before, removed: TabId) -> Removed { Last, Active(TabId) }` | `remove_tab`'s rule: the tab after a removed active tab, else the one before; another tab's removal leaves it | b07 #3 |
| `pick(&mut self, order, Pick) -> Option<TabId>`, where `enum Pick { Tab(TabId), Target(TabTarget), Next, Prev }` | `next_tab`, `prev_tab` (wrapping), `activate_tab`, `select` | b07 #4 |
| `shift(&self, order, at: TabId, by: Shift) -> Option<(usize, usize)>`, where `enum Shift { Wrapping(isize), Within(isize) }` | `move_tab` (wraps) and `move_tab_at` (doesn't). Both then use the existing `reorder`, and `reorder_keeping` goes | b07 #4 |

**`TabTarget { Nth(usize), Last }`** is `ActivateTab`'s payload (`types R7`, b00 #3, b07 #22). `last_tab`, `activate_tab_N` and alt-9 build it. The action is `no_json`, so no config text changes.

**`ProfileKey { name: String, nth: usize }`** (`src/config.rs`) names a profile as the nth one with that name (b07 #19's profile half):
- profiles have no ids, and names may repeat;
- `Config::profile_key(ix) -> ProfileKey` and `Config::find_profile(&ProfileKey) -> Option<usize>`;
- the `NewTab` palette (`PaletteKind::NewTab { keys }`) and the profile menu's items (their `ItemKey` is the profile key's text) resolve the chosen row against the config as it is when the row is chosen;
- `LaunchRequest.profile` stays an index, used in the call that resolved it;
- `NewTabWithProfile(n)` stays positional: "the nth profile" is what ctrl-alt-N means.

**`workspace/launch.rs`** (new, pure) has `new_tab_cwd(requested: Option<PathBuf>, host: &Host, pins: bool) -> NewTabCwd`, with `enum NewTabCwd { Given(PathBuf), FollowActive, ProfileDefault }` (b07 #1):
- a remote host keeps a requested directory only if it is POSIX, `~` or `~/…`;
- `FollowActive` applies when nothing usable was asked and the profile pins no directory;
- the shell reads the active tab's directory only on `FollowActive`, through `active_cwd_for`, which keeps its `same_endpoint` test;
- it uses change 2's `launch_on` host.

**`workspace/overlays.rs`** (new, pure) holds the overlays' kinds and the toggle rule (b07 #17, #18):
- **The kinds:** `OverlayKind { Palette(PaletteRole), Settings, ProfileMenu, ContextMenu }`, with `PaletteRole { NewTab, Commands, Prompt, Sessions }`.
- **`toggle(open: Option<OverlayKind>, wanted: OverlayKind, how: Opens) -> Toggle`**, with `enum Opens { Toggles, Replaces }` and `enum Toggle { Open, CloseThenOpen, CloseOnly }`, is today's rule:
  - opening a palette of the kind already open closes it;
  - the profile menu toggles;
  - anything else replaces what is open.
- **The state:** `Workspace`'s `palette_kind`, `sessions_opening`, `_palette_subscription`, `context_menu_items` and `context_menu_tab` become one shell field, `overlay: Overlay`. Its variants are:
  - `Closed`;
  - `SessionsOpening(SessionsModel)`;
  - `Palette { kind: PaletteKind, _subscription }`;
  - `Settings`;
  - `ProfileMenu`;
  - `ContextMenu { menu: ActionMenu, on: MenuFor }`, with `enum MenuFor { Tab(TabId), Zone }`.
- **What it rules out:**
  - a sessions model opening while another overlay shows;
  - a `palette_kind` with no palette;
  - a tab menu's id outliving its menu.

  `palette_confirmed` matches the overlay instead of taking `palette_kind` out and putting it back.
- **`Overlay::kind()`** gives `toggle` its input.
- **`ActionMenu`'s items** are keyed by action name, so `context_menu_selected` resolves the group 1 key. `ActionMenu::new` takes the shortcuts as values (`&[Option<SharedString>]`), finishing b00 #5 after change 7's `palette_commands`.

**`workspace/previews.rs`** (new, pure) has `Previews { theme: Option<Theme>, profile: Option<(String, Profile)> }`, which takes over `theme_preview` and `profile_preview` (b07 #7). Its `theme_for(&Profile, &Config) -> Theme` and `profile_for<'a>(&'a self, &'a Profile) -> &'a Profile` replace `tab_theme` and `effective_profile`.

**`Settings::zoomed(zoom, Zoom) -> f32`**, with `enum Zoom { By(f32), Reset }`, is pure and keeps today's clamp of −10 to 60 (b07 #5).

**The sessions palette:**
- `SessionsModel` and its types move to `src/picker/sessions.rs`, a whole pure module;
- `Endpoint::answered: u64` (0 for none) becomes `Option<u64>`, so a model at round 0 isn't complete before any answer (b02 #6);
- `palette_kill` carries `SessionRef { host, session, key: Option<SessionKey> }` to `refresh_sessions_palette` and `list_sessions` (b07 #20). The connection still sends `KILL { session }`, which has no key field. Checking the key needs a new client protocol version, so b07 #20 stays open for that follow-up (list at the end).

**`TAB_BAR_HEIGHT`** uses `style::rems_at_default` (b07 #21).

### 9. Group 7: settings dialog

**The file in and out** (b04 #1, #2; b05 `fonts.rs:196`, `profiles.rs:558`, `fonts.rs:293`, `profiles.rs:715`, `:769`, `:778`; `fcis R3`, `R9`):
- **The read:** the view reads `config.toml` (`FileText = Result<String, String>`, with the read error as change 12 words it) and passes it to `new` and `reload`.
- **The decision:** an operation returns `enum Step { Nothing, Write(ConfigEdit), Close }`.
- **The write:** on `Write`, the view runs `config_edit::save(path, |doc| edit.apply(doc))`, reads the file again, and calls `Model::written(result, text) -> Outcome`, the existing `Outcome`.
- **`ConfigEdit`** (`config_edit.rs`) is a closed enum over today's edits:
  - `SetGlobalTheme { name }`;
  - `SaveTheme { original: Option<String>, name, theme }`;
  - `DeleteTheme { name }`;
  - `SaveProfile { at: Option<ProfileRef>, profile, default: DefaultChange }`, with `enum DefaultChange { Keep, Rename { old, new }, Set(String) }`;
  - `DeleteProfile { at: ProfileRef, name }`;
  - `MoveProfile { at: ProfileRef, up: bool }`;
  - `SetFonts(Vec<FontChange>)`.

  `ConfigEdit::apply(&self, &mut DocumentMut) -> Result<Applied, String>`, with `enum Applied { Done, MovedTo(usize) }`, calls the existing functions.

**The sections** (b04 #5, #11; `types R1`):
- **`ThemesState`** (`settings_dialog/themes.rs`, new) takes over `Model`'s `entries`, `active`, `selected`, `mode`, `fallback` and the editor.
- **`Model`** becomes `{ section, themes, profiles, fonts, error }` and dispatches once per operation on `section`.
- **`mode_kind()`** is each section's own (Fonts is always `Form`), so `(Theme, Form)` can't be built.

**Selection by identity** (`types R3`):
- **themes:** by name (`selected: Option<String>`, `Editor::return_to: Option<String>`), b04 #7;
- **profiles:** by `ProfileKey` (group 6), b05 `profiles.rs:520-530`:
  - `ProfilesState.selected: Option<ProfileKey>`;
  - `ProfileEditor.return_to: Option<ProfileKey>`;
  - `ProfileEditor`'s `at` and `original_name` become `target: Target { New, Saved(ProfileKey) }` (b05 `profiles.rs:249-251`, `types R1`);
  - a `ProfileRef` (position and name, checked by `profile_at`) is built from the key inside the call that writes, and never stored, which closes b00 #11.
- **After a reload:** each key resolves to an index. A key that is gone falls back to the old position, clamped, which is what a delete does today.

**Decisions out of the view**, each with a test:
- `Model::enabled(Btn) -> bool` (b04 #3);
- `Model::active_row() -> Option<usize>` and a pure `focus_target(placed_for, view, facts) -> FocusMove` (b04 #4);
- `ProfilesState::rows() -> Vec<ProfileRow>`, `editor_title()` and `default_note()` (b05 `profiles.rs:1068`, `:1040-1043`, `:1206-1209`, `:1281-1285`);
- `ProfileEditor::theme_labels(&self, known: &[String]) -> Vec<String>` (b05 `profiles.rs:905-918`);
- `FontsState::choice_for(&ItemKey) -> Option<FamilyChoice>`, over the option list the model built, which replaces "option 0 is Default" (b05 `fonts.rs:541-547`).

**Types:**
- `FieldId { Name, Color(usize) }` owns the field/slot conversion (b04 #8, `types R5`);
- `focused_label` and `field_label` return `Option<&'static str>`, and the dump prints `""` for `None` as today (b04 #9);
- `Choice` goes, and the select uses `profiles::Field` directly (b04 #10, `types R12`);
- `config_edit::profiles` gives a blank name as `None`, and the list shows `Profile N` for it (b05 `profiles.rs:575-579`).

**Tests with none today:**
- `start_duplicate`, `move_profile`, `theme_names`, `take_renames`, `profile_preview` and `enabled` (b04 #6);
- `ProfilesState::move_selection` and `describe` in `ConfirmDelete` (b05 `profiles.rs:627`);
- `command_groups` (b05 `fonts.rs:478`);
- `ConfigEdit::apply`, one per variant, on a parsed document;
- the "reload keeps the selection by key" rows.

### 10. Failures and where they show

No failure is added or moves, and each keeps the unit it is shown on:
- **Config writes:** a refused or failed write is the settings dialog's error line, from `written`'s `Err`, as from `save` today.
- **Server refusals:** the core's attach refusals and the holder's spawn refusals are the client's `ERROR`. On Brindle's side that is the tab's failure, through change 12's notice line.
- **Connecting:** a failed relay attempt is the endpoint's tabs' retry line, or their requests' failures (`ConnectFailed`). An idle end gives the end's reason (`IdleStep`).
- **The role start:** a failed start is the tab's "didn't start" text (`start_failure`).

The new enums carry these as values, so `railway R1` holds: no default replaces a failure.

### 11. Wire, config, state file

None changes:
- **The wire:** `SessionInfo`, `Spawn`, `SpawnRequest`'s wire form, `SessionMeta`, `RelayHost` and `ExitStatus` keep their fields, defaults and bytes. So any core, holder, relay and client version meets any other as today, and no transcript changes (`process R4`–`R6`).
- **The config:** `ActivateTab` is a `no_json` action, so no config text changes.
- **The state file and layout blob:** unchanged (`process R15`).

## Left on the adoption policy

Each entry of F6, F10 and F12 that no change takes, with its reason and the change expected to touch the code next. Under the README's "Scope of adoption", the next change to the code brings it into line.

| Entry | Site | Rule | Why not here; next change |
| --- | --- | --- | --- |
| b01 #1, #2, #3, #4, #13 | `src/find_bar.rs` | `fcis R1` ×3, `R8`, `types R1` | Not a hotspot. `add-blocks` edits the find bar to reveal matches. |
| b01 #9, #11 | `src/main.rs:735-761`, `:248-358` | `fcis R10`, `R9` (both SHOULD) | Changes 7, 8 and 12 reshape `main.rs` first (change 12 edits `parse_args_from`'s checks). The role choice and a `parse_args_from` that returns its exits as values belong to the next change to the launch. |
| b01 #14 | `src/latency.rs:29-38` | `types R1` | Change 9 bounds its samples. Debug-only. |
| b01 #16 | `src/layout.rs:26-39` | `types R3` | Needs a field in the layout blob, a format change (`types R9`, `process R15`). |
| b01 #18, b03 #12 | `layout.rs:241`, `zone_view.rs:691`, `reattach.rs:173` | `types R7` | Change 2 leaves these key readers. Decode to `SessionKey` when they next change. |
| b02 #1, #2, #3, #5, #7, #8 | `src/notify/{mod,dbus,toast}.rs` | `fcis R1` ×3, `types R7`, `R6`, `R1` | Not a hotspot. A `Registry` model and a D-Bus `Link` enum belong to the next notify change. Change 10 hands over #2 and #3; they wait with the rest. |
| b03 #1, #2, #3, #4, #13, #14 | `src/zone_view.rs`, `src/reattach.rs:209-215` | `fcis R1`, `R10`, `types R5`, `R12`, `R7` ×2 | Not a hotspot. `FocusOrder` and `UnitRect::to_pixels` come with the next zone change. |
| b03 #16 | `src/reattach.rs:35-63` | `types R3` | Each index is used in the call that computed it, which R3 allows. The audit flags it only as a risk. |
| b06 ×9 | `terminal_element.rs:235-247`, `:377-404`, `:429-445`, `:564-570`; `terminal_view.rs:241-253`, `:458-463`, `:533-563`, `:353-374`, `:424-531`, `:123-138` | `fcis R1` ×6, `R10` ×2, `types R1` | `add-blocks` rewrites these handlers (proposal). |
| b28 #5 | `src/terminal/mod.rs:757-778` | `fcis R1` | `add-blocks` moves prompt jumps onto its view position. |
| b10 #12, b19 #2, #3 | `cli.rs:121-133`, `remote-protocol/src/client.rs` | `types R7`, `R1`, `R7` | One typed view of `SessionInfo` at the protocol boundary. Change 2's `ListedSession` is the app's. A shared one is a `remote-protocol` change of its own. |
| b11 #15, #16 | `core/sessions.rs:287`, `:1100` | `types R7` | Protocol ids stay integers (`types R4`); group 3 tests the skip of 0. The 0×0 cell size is the H1 `Resize` field. |
| b12 `pty.rs:70-75`, `holder.rs:982`, `event_loop.rs:67`; b15 `pty.rs:156-161` | the unix and Windows pty seams, the unix loop | `types R1` ×2, `R7` ×2 | Seam types on both platforms. Change 12 edits the pty seams' outcomes next. The signal fd is async-signal-safe by need; its comment says so. |
| b13 #4, b14 #2, b14 #6 | `daemon.rs:189`, `windows/listen.rs:266`, `pipe_state` (after change 10) | `types R7`, `R7`, `R1` | Platform seam state. Change 12 edits the ready pipe's outcome next. Change 10 offers b14 #6 as optional. |
| b15 `relay.rs:572-577`, `:216-217`, `:429-439`, `:308-350`, `:389-394` | the relay | `types R1`, `R7`, `R3`, `fcis R10`, `R1` | Not a hotspot (group 2 takes two small items from it). Change 12 edits the relay's exit reading next. |
| b20 #4, #5, #6, #7 | `remote-protocol/src/{scope,ready,holder,relay}.rs` | `types R1` ×2, `R7` ×2 | Accessors at the protocol boundary. Change 12 reshapes `SessionMeta::from_value` first. |
| b23 `config.rs:195-197`, b27 #20 | `RemoteTarget::Container.engine` | `types R6` | Parsing the engine at config load moves an error from launch time to load time, which is a behaviour change, not a refactor. |
| b23 `remote/client.rs:368-391` | `launch_with` | `fcis R9` | SHOULD. Not a hotspot. |
| b24 #7 | `src/remote/hosts.rs:49, 80` | `types R7` | Not a hotspot. |
| b27 #17, #18, #19 | `src/session.rs:61-80`, `:45-47`, `:429-442` | `types R1`, `R3`, `R12` | The persisted forms and the restore model. Changes 1, 7 and 12 edit `session.rs` first; the in-memory tree enum is one change of its own. |
| b30 #1, #2; b31 #3–#8 | `term-core/src/{emulator,osc,shell_integration,status}.rs` | `types R7` ×2, `R1` ×5, `R5` | Shared by app and core (`fcis R6`). Changes 3 and 5 edit these files first. |
| b07 #20 (rest) | `KILL` | `types R3` | The kill carries the key (Decision 8). Checking it needs a keyed `KILL`, a new client protocol version (`process R4`–`R6`). |

These are not audit entries, and are not taken:
- the three `too_many_arguments` allows, which change 6 gives a reason;
- `CommandEvent::Confirmed(usize)` (Decision 3);
- `pace::step`'s split.

## Risks / Trade-offs

- **[A moved decision behaves differently]** → Each function's tests are written against today's code before the move (Decision 1). The e2e suite covers the wiring: tabs, palette, profile menu, settings, restore and `local-server-*`. So do `remote-agent/tests/` and the client's `fake::link` tests, which all run unchanged.
- **[Three races now resolve differently]:**
  - a menu or select whose items are replaced while it is open;
  - a tab dragged while the strip changes;
  - a settings list reread while an item is selected.

  → Today they act on whatever item took the old position. Now they act on the chosen item, or on nothing if it is gone. No requirement describes these races, and the outcome is the one the user asked for.
- **[`ProfileKey`'s `nth` shifts when a same-named profile is added before the selected one]** → The selection then moves to that profile's twin, which is no worse than today's position. Profile ids would need a config change.
- **[Group 3 is the largest group, in the code that holds the user's programs]** → It lands after changes 7, 9 and 12. The real-process suites (`core.rs`, `holder.rs`, `upgrade.rs`, `manage.rs`, `relay.rs`) run on Linux and Windows CI unchanged. The wire is unchanged, so a mixed-build upgrade meets the same messages.
- **[Group 1 changes `brindle-ui`'s public events]** → The same group maps every owner's use back to today's index, so the app behaves as before until groups 6 and 7 key their items. The gallery's `--check` exercises the components (a GUI run: ask first).
- **[Rebase churn with the in-flight changes]** → Decision 2 orders each group after the changes it builds on. Groups are independent PRs, so a late sibling delays only its group.

## Migration Plan

Nothing to migrate: no persisted, wire or config format changes. Each group is one PR, and the group is the unit of rollback: reverting it restores the shell code it moved. The change is archived after group 7.
