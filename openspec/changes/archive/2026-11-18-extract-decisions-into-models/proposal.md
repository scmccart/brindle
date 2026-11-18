# Proposal

## Why

The 2026-11-16 audit found the shell still holding decisions, and state still held as flags, sentinels and positions (`docs/architecture/audits/2026-11-16.md`, findings F6, F10 and F12; remediation plan, change 13):
- **F6, `fcis R1`, `R10`, `R9`, `R3`:** what to show, keep, send or open is decided inside views, entities, reader threads and the server core's loop handlers. Such a rule can only be tested with a window or a running server, and most have no test at all.
- **F10, `types R1`, `R7`, `R3`, `R5`, `R6`, `R12`:** lifecycles spread over several `bool`s, `0` and `""` standing for "none", and positions kept across a refresh. Combinations that can't happen compile, and a list rebuilt between a press and its handling is addressed by an old index.
- **F12, `fcis R8`:** pure functions and models with no unit test in their own file.

Changes 1–12 fixed the defects, added the checks and moved the platform logic. What is left is the long tail. It is too large for one sweep, and the README's policy brings most of it into line when each file next changes. This change takes the hotspots, where most entries sit and most churn happens, so the next breaches don't grow from them. Everything else is listed by entry in the design, with who changes it next.

## What Changes

Refactor only, in seven task groups. Each group lands as its own PR, and the change is archived after the last.

1. **`brindle-ui`:**
   - `Select`, `Combobox`, `Command`'s kept highlight, `DropdownMenu` and the tab drag name their items by an owner-given `ItemKey`, never by a position held across a refresh;
   - an unmeasured trigger is `None`, not a zero `Bounds`;
   - `Menu::label` and the overlay depth return `Option`;
   - the overlay's kind is one enum;
   - the crate's view-side choices become pure functions with tests.
   - This is the one shared contract the later groups build on.
2. **Untested pure code, and four small extractions (two handed over by changes 7 and 10):**
   - tests for `fit_grid`, `wants_links`, `Theme::indexed`, `unescape`, `terminal_font`, the CLI's status helpers, the wire helpers, `notice::Source::name` and the Windows loop's `millis`;
   - `session::state_dir`'s choice, and the relay's login shell and zombie test, as pure functions;
   - `--wait-for`'s handles are never reused.
3. **Server roles (`remote-agent`):**
   - the core's start, grace, idle and exit rules become a pure `Lifecycle`, and its handler policies pure functions (HELLO options, queue marks, holder placement, a lost client's sessions, exits, attach refusals, `LIST` rows, foreground looks);
   - the holder's spawn refusal, drain test and core-start backoff become pure functions;
   - a core session's flags become two enums, `Life` and `Ownership`, and the holder's become `Life` and `Farewell`;
   - client tokens are `Option` in memory.
4. **Brindle's client (`connection.rs` `State`):**
   - the reader's and idle thread's decisions become tested `State` methods: `connect_failed`, `idle_step`, `link_lost` and `route_*`, plus `retry_cap`;
   - `Sent` and `Link` enums replace `Pending`'s and `State`'s flags, and `Option<Negotiated>` replaces protocol `0`;
   - the role start's outcome is one enum;
   - a connection is known by its id, not its address.
5. **Lifecycles and the spawn request:**
   - `Terminal`'s `exited`, `attaching` and `failed` become one `Life` and an `Origin`;
   - `Session`'s "left while starting" becomes its own `Phase`;
   - "no command" is an `Option` up to the wire;
   - `spawn_request` takes the home directory and a probe.
6. **`Workspace`:**
   - a pure `Tabs` model keeps the active tab by id and does the strip's index arithmetic;
   - the new tab's directory rule is a pure function;
   - the overlays are one `Overlay` enum with a pure open-or-toggle rule;
   - theme and profile previews are a `Previews` model, and zoom a pure step;
   - `ActivateTab` takes a `TabTarget`, and the sessions palette carries the key to its kill.
7. **Settings dialog:**
   - the section models take the config file's text and return a `ConfigEdit` for the view to write, instead of reading and writing `config.toml` themselves;
   - the themes get their own `ThemesState`;
   - selection is kept by theme name and profile key, not by position;
   - buttons, rows, labels and focus moves are decided by the models.

**Left out:**
- **The rest of F6, F10 and F12** stays under the README's "brought into line when it is next changed" policy. `design.md` lists each entry, with the reason and who is expected to change it next.
- **`terminal_view.rs` and `terminal_element.rs`:**
  - their `fcis R1` entries (cursor shape and pointer, run batching, mark bars, blink, selection type, wheel routing) go to the in-flight `add-blocks` change, which rewrites those handlers and must follow `fcis` anyway;
  - so do the `fcis R10` and `types R1` entries in the same mouse handlers, and `Terminal::jump_to_prompt`, whose offset `add-blocks` moves onto its own view position;
  - this change only adds the missing `fcis R8` tests for `fit_grid` and `wants_links`.
- **b07 #20, the sessions palette's kill:**
  - the kill carries the session's key from the row down to the connection;
  - `KILL` itself has no key, so checking it needs a new client protocol version (`process R4`–`R6`), which this refactor doesn't add.

**Behaviour:**
- **No requirement changes.** No wire message, protocol version, config key, state file or layout blob changes.
- **The only observable differences** are in three races where today's code addresses the wrong item: a menu or select whose items are replaced while it is open, a tab dragged while the strip changes, and a settings list reread while an item is selected. There, the item the user chose is used, or nothing is (design, Risks).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. No requirement changes, so `.openspec.yaml` sets `skip_specs: true`.

## Impact

- **Code, by group:**
  1. `ui/src/components/{select,combobox,command,menu,dropdown_menu,tabs,checkbox,mod}.rs`, `ui/src/primitives/{overlay,layer}.rs`, `ui/src/style.rs`, `ui/src/text_input.rs`, and the gallery's uses;
  2. `src/{terminal_element,terminal_view,theme,main,settings,session}.rs`, `src/terminal/mod.rs`, `remote-agent/src/server/{cli,relay}.rs`, `remote-agent/src/server/platform/windows/event_loop.rs` and new `platform/wait.rs`, `remote-protocol/src/wire.rs`, `term-core/src/notice.rs`;
  3. `remote-agent/src/server/core.rs` and `core/{clients,sessions,holders,manage,process}.rs`, new `core/policy.rs`, `holder.rs` and new `holder/policy.rs`, `check.rs`, `remote-protocol/src/endpoint.rs`;
  4. `src/remote/local/connection.rs`, `src/remote/local.rs`, `src/remote/local/{unix,windows,link,relay}.rs`, `src/terminal/server.rs`;
  5. `src/terminal/mod.rs`, `src/remote/local/session.rs`, `src/remote/local/relay.rs`;
  6. `src/workspace.rs`, new `src/workspace/{tabs,launch,overlays,previews}.rs`, `src/actions.rs`, `src/config.rs`, `src/settings.rs`, `src/picker.rs` (its `SessionsModel` moves to `src/picker/sessions.rs`);
  7. `src/settings_dialog.rs`, `src/settings_dialog/{fonts,profiles}.rs`, new `src/settings_dialog/themes.rs`, `src/config_edit.rs`.
- **Tests:** unit tests in each new model's or function's own file. Existing real-process tests (`remote-agent/tests/`) and e2e cases run unchanged; no new e2e case, since no behaviour changes.
- **Docs:**
  - `fcis`'s "How it applies here" table, and `tests/architecture.rs`'s `PURE_MODULES` (change 7), list the new whole pure modules: `src/workspace/{tabs,launch,overlays,previews}.rs`, `src/picker/sessions.rs`, `remote-agent`'s `core/policy.rs`, `holder/policy.rs` and `platform/wait.rs`;
  - `CLAUDE.md`'s paragraphs on `Workspace`, the settings dialog and the server core name the models.
- **Depends on:**
  - change 7 (`keep-pure-modules-pure`, the pure-module check and the probe rule) and change 12 (`surface-swallowed-failures`, whose error values the models carry), as the plan says;
  - per group, also on changes 2, 5, 8, 9 and 10 (design, "Group order and dependencies").
- **Overlap with changes in flight:**
  - `add-blocks` takes the two view files' decisions (above);
  - `port-to-macos` edits `src/main.rs` and `src/settings.rs` near the functions group 2 tests;
  - `update-dependencies` (the `toml` bump) touches `config_edit.rs`'s `DocumentMut` uses that group 7's `ConfigEdit` wraps. Whichever lands second adjusts.
  - `add-pocket-client` doesn't overlap.
