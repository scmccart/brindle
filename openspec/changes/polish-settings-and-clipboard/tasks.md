# Tasks

Four groups, one PR each, merged in order; the archive commit goes on the last group's PR. Within each group the order is pure code with its unit tests, then the shell, then the e2e cases, then docs. GUI runs go through `scripts/with-xvfb` (headless), never the user's desktop.

## 1. Fault switches are listed, and tests that need one declare it

PR title: `test: skip tests whose build lacks a fault switch they need`. This group comes first: it carries the one new contract between binaries and harnesses, `--list-fault-switches`.

- [ ] 1.1 Brindle's list: name constants for `BRINDLE_TEST_SINK_PANIC` (read by `terminal/server.rs`'s `inject_fault` through it) and `BRINDLE_TEST_LAUNCH_LISTING` (read by `main.rs`'s `launch_listing` through it), and `fault_switches()` in `main.rs`, `if cfg!(debug_assertions) { &[…] } else { &[] }`. Verify: a unit test `fault_switches_are_listed_only_in_debug_builds` in `main.rs` passes under `cargo test -p brindle fault_switches`.
- [ ] 1.2 `brindle --list-fault-switches` in `main.rs`'s argument loop beside `--list-actions` (`console_output`, one name per line, exit 0, before the config and GPUI), plus a line in `HELP`'s DEBUGGING section. Verify: `cargo run -- --list-fault-switches` prints both names and opens no window, and `target/release/brindle --list-fault-switches` (after `cargo build --release`) prints nothing and exits 0.
- [ ] 1.3 The helper's list: `fault::HOLDER_PANIC` and `fault::FEED_PANIC` constants, used by `holder.rs` and `core/screen.rs` in place of their literals, and `fault::honoured()`. Add `--list-fault-switches` to `server/cli.rs`'s commands (no arguments allowed; it never connects to a core) and to its `USAGE`. Verify: a unit test in `fault.rs` pins `honoured()` against `cfg!(debug_assertions)`, and a real-process test `the_helper_lists_its_fault_switches` in `remote-agent/tests/manage.rs` checks exit 0, that every line is a `BRINDLE_TEST_` name, and that an extra argument fails.
- [ ] 1.4 Real-process skips: `Capability::FaultSwitches` (`"fault-switches"`) in `remote-agent/tests/support/wait.rs`'s `Capability::ALL`, with `parse_required`'s test extended. Add a support probe that runs `BIN --list-fault-switches` once (`OnceLock`), parsed by a `parse_listing` with its own `#[test]`. The `faulty_holder` tests in `holder.rs` and `a_panicking_emulator_leaves_its_session_and_the_others_running` in `core.rs` call `skip(Capability::FaultSwitches, …)` and return when their switch isn't listed. Verify: `cargo test -p brindle-remote-agent --test holder --test core` passes. `cargo test --release -p brindle-remote-agent --test holder --test core` passes with those tests in `target/tmp/skipped.log`, and fails with `BRINDLE_TEST_REQUIRE=fault-switches`.
- [ ] 1.5 Harness decision: a new `tests/e2e/switches.rs` with `TestSwitch { SinkPanic, LaunchListing, HolderPanic }` (`name()`, `host() -> SwitchHost { Brindle, Helper }`), `parse_listing`, `readiness(needs, brindle, helper, required) -> Readiness { Run, Skip(String), Fail(String) }` and `undeclared(key, needs) -> Option<String>`. Add it to `--self-test` as "fault switches", covering:
  - a listed switch → `Run`;
  - a missing one → `Skip`, naming the switch, the binary's path and the remedy;
  - `required` → `Fail`;
  - a failed probe → its reason;
  - an undeclared `BRINDLE_TEST_*` key → a message, and any other key → `None`.

  Verify: `BRINDLE_E2E=1 cargo test --test e2e -- --self-test` passes.
- [ ] 1.6 Harness shell:
  - `Case` gains `needs` (`display(…)` in `cases/mod.rs` and `cases/win.rs` gives `&[]`, plus a constructor for cases with needs), and `Ctx` carries it.
  - The runner probes each host lazily, once per run, through `common::server::isolate` with a scratch directory and a 10 s `Scale`-bounded wait (killed past it). It applies `readiness` after the display check and before the case's directories are filled, reading `BRINDLE_E2E_SWITCHES_REQUIRED`.
  - `Run::env` makes `run()` fail on an undeclared `BRINDLE_TEST_*` key.
  - `local-server-sink-panic` and `local-server-reattach-slow-listing`/`-failed-listing` declare their Brindle switches, and `local-server-session-error` its helper switch. Drop their guesses ("is BRINDLE_E2E_BIN a release build?", "(the fault switch is in debug builds only)").

  Verify: `BRINDLE_E2E=1 cargo test --test e2e -- --list` lists the same cases as before, and the self-test passes.
- [ ] 1.7 A no-display e2e case `cli-list-fault-switches` (`cases/cli.rs`, unix and Windows lists) runs `ctx.bin` and `ctx.helper` with `--list-fault-switches` under `isolate`, and checks exit 0 and that every line is a `BRINDLE_TEST_` name. Verify: `BRINDLE_E2E=1 cargo test --test e2e -- cli-list-fault-switches` passes, opening no window.
- [ ] 1.8 CI: in `.github/workflows/ci.yml`, the `test` job's `cargo test` step gets `BRINDLE_TEST_REQUIRE: fault-switches`, and the `e2e` and `windows-e2e` jobs get `BRINDLE_E2E_SWITCHES_REQUIRED: "1"`. `build-windows.yml`'s `cargo test` step gets `BRINDLE_TEST_REQUIRE: fault-switches`. Verify: the PR's CI shows no `fault-switches` line in "Tests skipped for want of a capability", and no e2e shard reports the four cases skipped.
- [ ] 1.9 Docs:
  - `docs/architecture/testing-architecture.md`: add **R18**: a test or case that needs what a build or machine may lack (a fault switch, Docker, a user manager, the packaged ConPTY) MUST declare it and ask the binary or machine before it runs, never infer it from the harness's own build; without it, it is skipped with a reason naming what is missing and how to get it, unless a require variable (`BRINDLE_TEST_REQUIRE`, `BRINDLE_E2E_SWITCHES_REQUIRED`, `BRINDLE_REMOTE_E2E_REQUIRED`) makes it a failure, and CI sets them for what its runners provide. Add R18 to the Enforcement section (the e2e gate and `Run::env` guard, `skip(Capability…)`, CI's variables).
  - `docs/architecture/process-architecture.md` **R20**: append "and each binary lists the ones it honours with `--list-fault-switches`, so a test tells a missing switch from a broken feature".
  - `CLAUDE.md`:
    - line 193, after "(the e2e suite's fault switch).": add "`brindle --list-fault-switches` lists it and `BRINDLE_TEST_LAUNCH_LISTING`, and a release build lists none."
    - line 195: "(comma-separated: `scopes`, `packaged-conpty`)" → "(comma-separated: `scopes`, `packaged-conpty`, `fault-switches`)", and append "CI's `test` and Windows jobs set `BRINDLE_TEST_REQUIRE=fault-switches`; `brindle-remote --list-fault-switches` says which switches the helper has."
    - a new bullet after line 92 (the `remote-*` bullet): "A case that sets a fault switch declares it (`Case::needs`, `tests/e2e/switches.rs`). The runner asks each binary once (`--list-fault-switches`) and skips a case whose binary lacks one, saying which binary and why (a `--release` run, or `BRINDLE_E2E_BIN` naming a release build). `BRINDLE_E2E_SWITCHES_REQUIRED=1` makes that a failure, and `Run::env` refuses an undeclared `BRINDLE_TEST_*` variable."
    - line 98: "`e2e` (the suite under `scripts/with-xvfb` with `BRINDLE_E2E_SLOW=2`)" → "… with `BRINDLE_E2E_SLOW=2` and `BRINDLE_E2E_SWITCHES_REQUIRED=1`)".
  - `README.md`, "Testing", after the paragraph ending at line 293: "Cases that need a debug-only fault switch are skipped, with the reason, when the binary under test has none (`cargo test --release`, or `BRINDLE_E2E_BIN` naming a release build); `BRINDLE_E2E_SWITCHES_REQUIRED=1`, as CI sets, makes that a failure."

  Verify: `grep -n "list-fault-switches" CLAUDE.md README.md docs/architecture/*.md` shows each place.
- [ ] 1.10 Group check:
  - `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings` and both harness self-tests pass.
  - After `cargo build --release --locked`, `BRINDLE_E2E=1 scripts/with-xvfb cargo test --release --test e2e -- sink-panic session-error listing` reports the four cases skipped, each naming its binary and switch, with no window opened. With `BRINDLE_E2E_SWITCHES_REQUIRED=1` the same run fails.
  - The same filter without `--release` passes.

## 2. Settings from the profile menu, built-in themes marked, the close `X` closes

PR title: `feat(settings): open settings from the profile menu, mark built-in themes, close on X`.

- [ ] 2.1 Lock icon (`ui/src/icon.rs`): add `Icon::Lock` and its `LOCK` strokes as in design §2 (the 18×11 body at (3, 11) with radius 2, and the shackle polyline (7, 11), (7, 7), the radius-5 arc about (12, 7) in 22.5° chords, (17, 7), (17, 11)). Add it to `Icon::ALL` (now 15) and to `strokes()`. `every_icon_builds_at_component_sizes`, `every_icon_stays_inside_its_unit_box` and the gallery's `Icon::ALL` row take it without changes. Extend `shapes_are_lucides` with the lock: the body's `[3, 11, 18, 11, 2]` on the grid, the shackle starting `(7, 11), (7, 7)`, passing `(12, 2)` and ending `(17, 7), (17, 11)`, and every arc point within 0.01 of radius 5 from (12, 7). Add `Lock` to `ui/README.md`'s icon list (lines 241–245, "… `Columns2`, `Rows2` and `Lock`"). CLAUDE.md names no icons (its `brindle-ui` bullet says only that `Icon` draws lucide's strokes), so it is unchanged. Verify: `cargo test -p brindle-ui icon` passes.
- [ ] 2.2 Profile menu decision (`src/workspace.rs`): `profile_menu_entries(rows, keys, settings_shortcut)` appends a `Separator` and a "Settings" item keyed `settings`, and `ProfileMenuChoice { Profile { key, listed }, Settings }` comes from `profile_menu_choice(entries, key)`. Update `profile_rows_and_menu_items_match_the_picker` and `profile_menu_entries`'s doc (it now has a separator, after the profiles). Add tests:
  - `the_profile_menu_ends_with_settings` (separator, label, shortcut);
  - `the_settings_key_is_never_a_profile_or_an_index`;
  - `profile_menu_choice_decodes_profiles_and_settings` (twin names and an unkeyed row included).

  Verify: `cargo test -p brindle workspace::` passes.
- [ ] 2.3 Profile menu shell: `refresh_profile_menu` passes `shortcut(&OpenSettings, &focus, window)`, and the `MenuEvent` subscription matches on `ProfileMenuChoice`: `Profile` → `open_profile_keyed` as today, `Settings` → `open_settings`. Verify: task 2.4's cases.
- [ ] 2.4 e2e (`cases/profile_menu.rs`): `profile-menu-items` expects `items=Shell|Other|-|Settings` and a Settings row carrying the Open Settings shortcut. A new `profile-menu-settings`: `open_profile_menu`, `--key end`, `--key enter` → a `--- settings` line, focus `settings`, one tab; then `--key escape` → no dialog, focus `terminal`. Verify: `BRINDLE_E2E=1 scripts/with-xvfb cargo test --test e2e -- profile-menu` passes.
- [ ] 2.5 Built-in themes (`settings_dialog/themes.rs`):
  - `ThemeEntry::tag()` (`"built-in"`/`"custom"`);
  - `enabled(Btn::Edit)` only for a readable custom theme;
  - `start_edit` on a built-in returns `Reply::Refused("\"<name>\" is built in: New makes an editable copy")` instead of copying;
  - `request_delete`'s refusal reads "… is built in and can't be deleted";
  - the list line of `describe` ends with ` origin=built-in|custom` for the selected entry (appended, so the existing `contains` checks hold).

  Tests: `every_theme_is_tagged_built_in_or_custom`, `edit_is_unavailable_on_a_built_in`, `the_edit_key_on_a_built_in_says_new_copies`, `the_delete_key_on_a_built_in_says_why`; move the tests that reached a copy through Edit on a built-in (`tokyo-night-custom`, `spare-custom` in `settings_dialog.rs`) to New. Verify: `cargo test -p brindle settings_dialog` passes.
- [ ] 2.6 Built-in themes view (`settings_dialog.rs` `render_list`): a built-in row shows `Icon::Lock` at 0.875rem in `muted`, where a custom row keeps its "custom" tag. The icon is wrapped in `div().id(("theme-lock", ix))` with `.tooltip_for("Built in: New makes an editable copy", None)` (`brindle_ui::TooltipFor`). Verify: task 2.9's case (`origin=` in the dump), plus a look at a window capture under `scripts/with-xvfb` showing the lock on the five built-in rows and none on custom rows (the icon isn't in the dump). The tooltip needs a real pointer, so it goes in task 2.10's manual checks.
- [ ] 2.7 Close `X` (`settings_dialog.rs`): `Model::close() -> Step` returns `Step::Close` from every view. `Dialog::on_close` takes `self.step(self.model.close(), …)` in place of `request_cancel`, and the doc comments on `dismiss`/`request_cancel` drop "or the close X". Test: `the_close_button_closes_from_every_view`, covering the theme list, a dirty theme editor, a dirty profile editor, the delete question, and Fonts with a changed field. Verify: `cargo test -p brindle settings_dialog` passes.
- [ ] 2.8 e2e `settings-close-button` (`cases/settings.rs`), each run focusing the `X` by shift-tab (the dump's `field="close"` confirms it), then pressing `--key enter`:
  - a theme editor (New on `e2e`) with an unsaved background typed in → dialog closed, focus `terminal`, the capture's background `theme::BACKGROUND`, config unchanged;
  - the profile editor with `Shell` renamed to `work` → closed, config unchanged;
  - the delete question for `e2e-alt` → closed, `[themes.e2e-alt]` still in the config.

  Verify: `BRINDLE_E2E=1 scripts/with-xvfb cargo test --test e2e -- settings` passes, `settings-enter-on-button` and `settings-tab-wrap` included.
- [ ] 2.9 e2e `settings-builtin-theme`: `open_theme` then `settings_first` (`brindle-dark`) → `origin=built-in`. `--key e` → still `mode=list selected="brindle-dark"`, with the error naming built in and New. `--key d` → no `mode=confirm-delete`, error says built in. `settings_last` (`e2e-alt`) → `origin=custom`, and `--key e` → `mode=editor name="e2e-alt"`. Verify: the `settings` filter run in 2.8.
- [ ] 2.10 Docs:
  - `README.md`:
    - the profile-menu paragraph (lines 512–516): add "It ends with Settings, which opens the settings dialog.";
    - the ctrl-shift-, table row: add "(also the `⌄` menu's Settings)";
    - lines 543–545: replace "`e` edits a custom theme and `d` deletes one. Built-in themes can't be changed, so editing one starts from a copy." with "Built-in themes carry a lock and can't be edited or deleted: `n` makes an editable copy of the selected theme, `e` edits a custom theme and `d` deletes one.";
    - after "Otherwise escape discards the editor's changes and goes back to the list." (line 564): add "The dialog's close button (×) closes the whole dialog from anywhere, dropping unsaved changes, as ctrl-shift-, does."
  - `CLAUDE.md`:
    - line 222: "…, each keyed by its `ProfileKey`, and choosing one opens that profile as the picker does." → "…, each keyed by its `ProfileKey`, then a separator and a Settings item keyed `settings`. `profile_menu_choice` decodes the chosen key into a `ProfileMenuChoice`: a profile opens as the picker opens it, and Settings runs `open_settings`.";
    - "Manual checks", the profile menu item (line 112): add "its Settings item opening the dialog", and add an item: "a pointer click on the settings dialog's close × closes it from an editor". In the hover-tooltip item (the last), add "the Theme list's lock on a built-in" to the places a hover shows a tooltip.

  Verify: the README and CLAUDE.md diffs say exactly this.
- [ ] 2.11 Group check: `cargo test --locked --workspace`, clippy and both self-tests pass; `BRINDLE_E2E=1 scripts/with-xvfb cargo test --test e2e -- profile-menu settings input-settings` passes. Manual checks for the PR body: the press on `⌄` with Settings chosen by pointer, a pointer click on the × in an editor, and a hover on a built-in's lock showing "Built in: New makes an editable copy".

## 3. Right-click paste, and the clipboard settings read live

PR title: `feat: paste the clipboard on right-click when right_click = "paste"`.

- [ ] 3.1 Config (`src/config.rs`): `enum RightClick { Nothing, Paste }`, deserialised from `"none"` and `"paste"` with `Nothing` the default, and the `right_click` field. `assets/default-config.toml` gets `right_click = "none"` after `copy_on_select`, with a comment ("`"paste"`: a right-click pastes the clipboard when nothing is selected, and does nothing while text is; hold shift where a program uses the mouse"). Tests: `right_click_parses_none_and_paste` (absent → `Nothing`), and `an_unknown_right_click_value_is_a_config_error` (the message names `none` and `paste`); the default-config parse test still passes. Verify: `cargo test -p brindle config::` passes.
- [ ] 3.2 Decision (`src/terminal/mouse.rs`): `reports(mode, shift)` (moved from `TerminalView::reporting`), `RightPress { Report, Paste, Nothing }` and `right_press(reports, has_selection, setting)`. Tests over every mode × shift × selection × setting:
  - `a_right_press_is_reported_while_the_program_reports` (with or without a selection);
  - `right_click_pastes_only_when_set_to_paste`;
  - `a_right_press_over_a_selection_does_nothing` (both settings);
  - `shift_right_click_follows_the_selection_rule` (reporting on, shift held: `Paste` with no selection, `Nothing` with one). Verify: `cargo test -p brindle terminal::mouse` passes.
- [ ] 3.3 Pointer step (`src/terminal_view.rs`, `src/main.rs`): `PointerStep { …, action: PointerAction }` with `PointerAction { Hover, Click(PointerButton) }` and `PointerButton { Left, Right }`; `--right-click POS` parsed and refused before the run like `--click`; a `HELP` line. Extend `pointer_steps` (a right click keeps its modifiers; a bad position errs) and `main.rs`'s usage-error tests, if they cover `--click`. Verify: `cargo test -p brindle terminal_view:: main::` passes.
- [ ] 3.4 Shell:
  - `TerminalView::mouse_down` calls `mouse::reports`, and on a right press dispatches on `right_press(reports, terminal.read(cx).selection_text().is_some(), Settings::get(cx).config.right_click)`: `Paste` → the paste action's path (a shared helper); `Nothing` → return, leaving the selection; neither sets `reported_button`.
  - `mouse_up` passes `config.copy_on_select`, read at the release, to `Terminal::finish_selection(to_clipboard, cx)`, and `Terminal` drops its `copy_on_select` field.
  - `debug_pointer` presses the step's button.

  Verify: unit tests pass, `links-*` still pass (task 3.7), and task 3.5's cases.
- [ ] 3.5 e2e: a new `tests/e2e/cases/clipboard.rs`. Each case sets the clipboard through OSC 52 from the shell, to `echo pasted-$((6*7))` (content the case typed, `testing R6`):
  - `clipboard-right-click-paste`: with `right_click = "paste"`, `--right-click 0,0`, `--send '\r'`, `--wait-for pasted-42`;
  - `clipboard-right-click-off`: the default config, the same steps → no `pasted-42` on the screen;
  - `clipboard-right-click-selection`, two runs with `right_click = "paste"`. First, `--action select_all`, `--right-click 0,0`, then the dump: its `--- selection` line is still there and the prompt's line holds nothing typed. Second, the same steps, then `--click 0,0` to clear the selection, `--right-click 0,0`, `--send '\r'` and `--wait-for pasted-42`: no `--- selection` line, and `pasted-42` is shown;
  - `clipboard-right-click-mouse-mode`: a script that turns on modes 1000 and 1006, then `exec cat -v`; `--right-click 5,1` → `[<2;6;2M` and no clip text; `--right-click shift-5,2` with nothing selected → the clip text is echoed, with no `;6;3M` report; then `--action select_all`, `--right-click shift-5,3` → nothing more echoed, no `;6;4M` report, and the `--- selection` line still there;
  - `clipboard-right-click-reload`: the default config, a `call_at` that rewrites it with `right_click = "paste"`, `--action reload_config`, then a right-click pastes.

  Register them in `cases/mod.rs`, and `clipboard-right-click-paste` also in `cases/win.rs` with a pwsh variant of the OSC 52 step (the `sh(unix, windows)` pair). Extend `cli-usage-errors` with `--right-click 3` → exit 2 naming the step. Verify: `BRINDLE_E2E=1 scripts/with-xvfb cargo test --test e2e -- clipboard cli-usage-errors` passes.
- [ ] 3.6 Docs:
  - `README.md` line 504: "copy / paste (middle click pastes the primary selection)" → "copy / paste (middle click pastes the primary selection; with `right_click = "paste"`, right-click pastes the clipboard when nothing is selected and does nothing while text is: shift-right-click where a program uses the mouse)".
  - `CLAUDE.md` line 199 (Mouse bullet): append "A right press that isn't reported (`mouse::reports`; shift always bypasses) pastes the clipboard when `right_click = "paste"` and no text is selected, and does nothing while text is (`mouse::right_press`). `copy_on_select` and `right_click` are read from `Settings` at the event, so a reload reaches open tabs. `--right-click` is the right button's `--click`."
  - `CLAUDE.md` "Manual checks": add "right-click paste with `right_click = "paste"` (nothing selected; a right-click over a selection doing nothing; and shift-right-click over a program that uses the mouse, such as htop), and select-to-copy with `copy_on_select = true`, with a real pointer, on Linux and on Windows (no system menu on Windows)".

  Verify: the diffs say exactly this.
- [ ] 3.7 Group check: `cargo test --locked --workspace`, clippy and both self-tests pass; `BRINDLE_E2E=1 scripts/with-xvfb cargo test --test e2e -- clipboard links input cli` passes. Hand the manual check in 3.6 to the user with `scripts/run-throwaway.sh --config <file with right_click = "paste">`, and list it in the PR body.

## 4. Clipboard section in the settings dialog

PR title: `feat(settings): add a Clipboard section for copy on select and right-click paste`. The archive goes on this PR.

- [ ] 4.1 `src/config_edit.rs`:
  - `clipboard_values(doc)`, read past errors elsewhere; a key of the wrong type reads as its default, with a note naming the key;
  - `ClipboardChange { CopyOnSelect(bool), RightClick(RightClick) }`;
  - `ConfigEdit::SetClipboard(Vec<ClipboardChange>)`, writing the top-level keys in place.

  Tests: comments and order kept, only changed keys written, `right_click` written as `"paste"`/`"none"`, a wrongly typed key replaced only when its switch changed. Verify: `cargo test -p brindle config_edit::` passes.
- [ ] 4.2 `src/settings_dialog/clipboard.rs`: `ClipboardState` (`new`, `reload`, `reset`, `toggle(ClipboardField)`, `apply`, `describe`) and `ClipboardField { CopyOnSelect, RightClick }`. Tests:
  - values from the file's text;
  - the loaded config's values, read-only, when the file doesn't parse;
  - `apply` with no change → `Step::Close`;
  - only changed fields written;
  - read-only → `Err`;
  - the `--- settings section=Clipboard mode=form copy_on_select=… right_click=… field=… can_apply=… error=…` line.

  Verify: `cargo test -p brindle settings_dialog::clipboard` passes.
- [ ] 4.3 `Model` (`settings_dialog.rs`):
  - `Section::Clipboard` last in `SECTIONS`, and `View::Clipboard` (`ModeKind::Form`);
  - `select_section` resets it on leave;
  - `confirm` applies, `cancel` and `close` give `Step::Close`;
  - footer `[Cancel, Apply]`, and `describe_open` routes to it.

  Update the tests that count sections or wrap (`next_section` from Clipboard to Profiles), and extend `the_close_button_closes_from_every_view` to Clipboard. Verify: `cargo test -p brindle settings_dialog` passes.
- [ ] 4.4 View: two `brindle_ui::Switch`es labelled "Copy on select" and "Right-click pastes", with a hint line ("Pastes only when nothing is selected. Hold shift to paste where a program uses the mouse."). They are `FocusStops` with `field=` names. Space toggles, enter applies, and `Outcome::Applied` reloads as the Fonts section does. Verify: task 4.5's case.
- [ ] 4.5 e2e:
  - a new `settings-clipboard`: `open_settings`, `settings_section_last` → `section=Clipboard mode=form copy_on_select=false right_click=none`; tab to "Right-click pastes", `--key space`, `--key enter` → the dialog is closed and the config gains only `right_click = "paste"`, with its comments kept. In the same run, an OSC 52 clipboard and `--right-click` paste into the open tab.
  - a second run toggles both switches and presses `--key escape` → config unchanged.
  - update `settings-sections`, `settings-section-list` and `settings-section-shortcuts` for four sections.

  Verify: `BRINDLE_E2E=1 scripts/with-xvfb cargo test --test e2e -- settings` passes.
- [ ] 4.6 Docs:
  - `README.md`:
    - line 533: "three sections: Profiles, Theme and Fonts" → "four sections: Profiles, Theme, Fonts and Clipboard";
    - the ctrl-shift-, row: add "clipboard";
    - after line 577: add a paragraph: "The Clipboard section has two switches, Copy on select (`copy_on_select`) and Right-click pastes (`right_click = "paste"`, which pastes only when nothing is selected). Apply (or enter) writes the changed ones and closes the dialog, and open tabs follow at once; Cancel, escape or another section drop the changes."
  - `CLAUDE.md`:
    - line 72: after the Fonts section's dump form, add "the Clipboard section's is `--- settings section=Clipboard mode=form copy_on_select=… right_click=… field=… can_apply=… error=…`;";
    - after line 210 (the Fonts section bullet), add "**Clipboard section (`settings_dialog/clipboard.rs`):** `ClipboardState` is the GPUI-free model (the two values read from the file's text past errors elsewhere, `toggle`, `apply()` returning `ConfigEdit::SetClipboard` with only the changed keys); the view's two `brindle_ui::Switch`es are its stops."

  Verify: the diffs say exactly this.
- [ ] 4.7 Integration check:
  - `cargo test --locked --workspace`, clippy and both self-tests pass;
  - the whole e2e suite passes headless: `BRINDLE_E2E=1 scripts/with-xvfb cargo test --test e2e`;
  - `openspec validate polish-settings-and-clipboard --strict` passes.

## Workflow follow-up

- One branch and PR per group, in order (1 → 4), each squash-merged by the user; PR titles as given above, checked with `scripts/lint-commits --title`. Each PR body lists that group's manual checks.
- On group 4's PR, after its implementation commit: `openspec archive polish-settings-and-clipboard`, syncing the delta specs into `openspec/specs/`, committed as `docs(openspec): archive polish-settings-and-clipboard`.
- After the merges, offer to reinstall the daily build with `scripts/install.sh`.
